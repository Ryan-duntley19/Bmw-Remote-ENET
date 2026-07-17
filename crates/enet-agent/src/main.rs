//! Laptop ENET agent — LAN auto-discover or remote relay / WireGuard.

use anyhow::Context;
use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use clap::Parser;
use enet_core::config::{GatewayConfig, NetworkMode, Role};
use enet_core::discover_gateways;
use enet_core::discovery::{
    adapter_link_up, detect_candidate_interfaces, pick_enet_interface,
};
use enet_core::logging::init_logging;
use enet_core::stats::backoff_delay;
use enet_core::state::ConnectionState;
use enet_protocol::magic::DEFAULT_AGENT_API_PORT;
use enet_tunnel::{
    EthernetPort, RelayTunnelEngine, RelayTunnelOptions, SimulatedEthernet, TunnelEngine,
    TunnelHandle, TunnelOptions,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Keep the process singleton so Host↔Client state cannot split across two agents.
struct InstanceGuard {
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

// Windows CreateMutex handles are not Send in the type system; process-global is fine.
unsafe impl Send for InstanceGuard {}
unsafe impl Sync for InstanceGuard {}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            extern "system" {
                fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
            }
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }
}

fn acquire_single_instance() -> Option<InstanceGuard> {
    // After a self-update restart the old process may need a moment to exit
    // and release the mutex — retry briefly instead of giving up.
    for attempt in 0..12u32 {
        if let Some(g) = try_acquire_instance() {
            return Some(g);
        }
        if attempt == 0 {
            eprintln!("  Another BMW ENET Agent is running — waiting up to 8s (update restart?)…");
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    None
}

fn try_acquire_instance() -> Option<InstanceGuard> {
    #[cfg(windows)]
    {
        extern "system" {
            fn CreateMutexA(
                lp: *mut std::ffi::c_void,
                initial: i32,
                name: *const u8,
            ) -> *mut std::ffi::c_void;
            fn GetLastError() -> u32;
        }
        const ERROR_ALREADY_EXISTS: u32 = 183;
        let name = b"Global\\BMW-ENET-Agent-SingleInstance\0";
        let handle =
            unsafe { CreateMutexA(std::ptr::null_mut(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Some(InstanceGuard { handle });
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                extern "system" {
                    fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
                }
                CloseHandle(handle);
            }
            return None;
        }
        return Some(InstanceGuard { handle });
    }
    #[cfg(not(windows))]
    {
        Some(InstanceGuard {})
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "enet-agent",
    about = "BMW ENET laptop agent — LAN, relay, or WireGuard"
)]
struct Args {
    #[arg(short, long, default_value = "config/agent.toml")]
    config: PathBuf,
    #[arg(long)]
    peer: Option<IpAddr>,
    #[arg(long)]
    pair_code: Option<String>,
    /// Override relay host:port for remote mode
    #[arg(long)]
    relay: Option<String>,
    #[arg(long)]
    simulate: bool,
    /// Local status page bind port (default 47903).
    #[arg(long)]
    status_port: Option<u16>,
}

/// Shared live state for the laptop status page / console indicator.
struct LiveStatus {
    handle: RwLock<Option<TunnelHandle>>,
    pair_code: RwLock<String>,
    desktop_peer: RwLock<String>,
    configured_peer: RwLock<Option<IpAddr>>,
    /// When true, next resolve_peer skips the cached IP and re-runs LAN discovery.
    force_discover: AtomicBool,
    enet_name: Arc<RwLock<String>>,
    enet_link: Arc<AtomicBool>,
    /// True when Npcap capture/inject is live on the ENET adapter.
    l2_active: Arc<AtomicBool>,
    /// Human adapter label or reason L2 is inactive.
    l2_label: Arc<RwLock<String>>,
    force_reconnect: AtomicBool,
    config_path: PathBuf,
    tunnel_port: u16,
    /// Newer release found by the periodic / manual check.
    update_available: RwLock<Option<enet_core::updater::UpdateInfo>>,
    /// Update settings snapshot (repo, token, auto).
    update_repo: String,
    update_token: String,
    /// Auto-install new releases when idle.
    auto_update: RwLock<bool>,
}

#[derive(Serialize)]
struct StatusJson {
    version: String,
    pair_code: String,
    desktop_connected: bool,
    desktop_peer: String,
    configured_peer: Option<String>,
    enet_interface: String,
    enet_link: bool,
    l2_active: bool,
    l2_label: String,
    vehicle_awake: bool,
    vehicle_link: bool,
    rtt_ms: f64,
    loss_rate: f64,
    friendly: String,
    update_available: Option<String>,
    auto_update: bool,
}

#[derive(Deserialize)]
struct ConnectRequest {
    peer: String,
    pair_code: Option<String>,
}

#[derive(Serialize)]
struct ConnectResponse {
    ok: bool,
    message: String,
    peer: Option<String>,
}

/// Npcap port that reports OS carrier (shared refresher) instead of a sticky flag.
#[cfg(windows)]
struct LinkedPcap {
    inner: Arc<enet_tunnel::PcapEthernet>,
    link: Arc<AtomicBool>,
}

#[cfg(windows)]
#[async_trait]
impl EthernetPort for LinkedPcap {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn link_up(&self) -> bool {
        self.link.load(Ordering::Relaxed)
    }
    async fn recv(&self) -> anyhow::Result<Bytes> {
        self.inner.recv().await
    }
    async fn send(&self, frame: Bytes) -> anyhow::Result<()> {
        self.inner.send(frame).await
    }
}

/// Placeholder NIC that still reports real OS link/carrier for the ENET adapter.
struct MonitoredEthernet {
    name: Arc<RwLock<String>>,
    preferred: String,
    link: Arc<AtomicBool>,
}

#[async_trait]
impl EthernetPort for MonitoredEthernet {
    fn name(&self) -> &str {
        // Trait requires &str; callers use the shared name via status UI.
        "monitored-enet"
    }
    async fn link_up(&self) -> bool {
        // Never block the tunnel runtime with PowerShell here — a background
        // refresher updates `link`. Keepalive/RTT must stay on the fast path.
        {
            let cur = self.name.read().clone();
            if cur.is_empty() || cur == "pending-enet" {
                if let Some(iface) = pick_enet_interface(&self.preferred) {
                    *self.name.write() = iface.name;
                }
            }
        }
        self.link.load(Ordering::Relaxed)
    }
    async fn recv(&self) -> anyhow::Result<Bytes> {
        // Npcap capture not wired yet — keep the task parked.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Err(anyhow::anyhow!("monitored ethernet closed"))
    }
    async fn send(&self, _frame: Bytes) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn build_ethernet_port(
    cfg: &GatewayConfig,
    simulate: bool,
    enet_name: Arc<RwLock<String>>,
    enet_link: Arc<AtomicBool>,
    l2_active: Arc<AtomicBool>,
    l2_label: Arc<RwLock<String>>,
) -> anyhow::Result<Arc<dyn EthernetPort>> {
    l2_active.store(false, Ordering::Relaxed);
    if simulate {
        let (port, _peer) = SimulatedEthernet::pair("sim-enet", "sim-car");
        info!("using simulated ENET interface");
        *enet_name.write() = "sim-enet".into();
        *l2_label.write() = "simulated (test mode)".into();
        enet_link.store(true, Ordering::Relaxed);
        std::mem::forget(_peer);
        return Ok(port);
    }

    #[cfg(windows)]
    {
        if !enet_tunnel::PcapEthernet::npcap_available() {
            eprintln!();
            eprintln!("  *** Npcap not found — launching installer ***");
            eprintln!("  Enable “WinPcap API-compatible Mode”, then Finish.");
            eprintln!();
            let installed = tokio::task::spawn_blocking(|| {
                enet_core::ensure_npcap_installed(|msg| {
                    eprintln!("  {msg}");
                })
                .unwrap_or(false)
            })
            .await
            .unwrap_or(false);
            if !installed || !enet_tunnel::PcapEthernet::npcap_available() {
                *l2_label.write() = "Npcap not installed".into();
                eprintln!();
                eprintln!("  *** Npcap required for ISTA ***");
                eprintln!("  Install from https://npcap.com (enable WinPcap API compatibility),");
                eprintln!("  then restart BMW ENET Client.");
                eprintln!("  Tunnel can still connect, but car frames will NOT reach the desktop.");
                eprintln!();
            }
        }
        if enet_tunnel::PcapEthernet::npcap_available() {
            if let Some(iface) = pick_enet_interface(cfg.enet_interface.as_str()) {
                // Only open the scored ENET candidate — never the Wi‑Fi / LAN tunnel NIC.
                let try_open = |target: &str| enet_tunnel::PcapEthernet::open(target).ok();
                let opened = try_open(&iface.name).or_else(|| {
                    // Fall back: match Npcap description against the OS adapter name.
                    let want = iface.name.to_lowercase();
                    enet_tunnel::PcapEthernet::list_devices()
                        .unwrap_or_default()
                        .into_iter()
                        .find(|line| line.to_lowercase().contains(&want))
                        .and_then(|line| {
                            let npf = line.split('|').next().unwrap_or(&line).to_string();
                            try_open(&npf)
                        })
                });
                if let Some(port) = opened {
                    let shown = port.display_name().to_string();
                    info!(adapter = %port.name(), %shown, "Client L2 ENET capture ready");
                    // Keep the OS adapter name so the carrier refresher can poll it.
                    *enet_name.write() = iface.name.clone();
                    *l2_label.write() = shown.clone();
                    l2_active.store(true, Ordering::Relaxed);
                    enet_link.store(adapter_link_up(&iface.name), Ordering::Relaxed);
                    eprintln!("  ENET capture: {shown}");
                    return Ok(Arc::new(LinkedPcap {
                        inner: port,
                        link: enet_link.clone(),
                    }));
                }
                *l2_label.write() = format!("Npcap could not open {}", iface.name);
                eprintln!();
                eprintln!("  Could not open Npcap on ENET adapter '{}'.", iface.name);
                eprintln!("  Run Client as Administrator / SYSTEM and confirm Npcap is installed.");
                eprintln!();
            } else {
                *l2_label.write() = "waiting for ENET adapter".into();
                eprintln!();
                eprintln!("  Npcap is installed but no ENET adapter was detected yet.");
                eprintln!("  Plug the ENET cable into the car + laptop, then restart Client.");
                eprintln!();
            }
        }
    }
    #[cfg(not(windows))]
    {
        *l2_label.write() = "L2 capture is Windows-only".into();
    }

    let preferred = cfg.enet_interface.clone();
    if let Some(iface) = pick_enet_interface(preferred.as_str()) {
        info!(name = %iface.name, mac = %iface.mac, "selected ENET candidate interface (link monitor only)");
        *enet_name.write() = iface.name.clone();
        let up = adapter_link_up(&iface.name);
        enet_link.store(up, Ordering::Relaxed);
        warn!(
            "L2 capture not active for '{}' — ISTA will not see the car until Npcap opens this NIC",
            iface.name
        );
        return Ok(Arc::new(MonitoredEthernet {
            name: enet_name,
            preferred,
            link: enet_link,
        }));
    }
    let all = detect_candidate_interfaces();
    warn!(
        count = all.len(),
        "no strong ENET candidate — connecting to desktop anyway (vehicle link stays down until ENET is ready)"
    );
    for i in &all {
        info!(name = %i.name, mac = %i.mac, up = i.is_up, "iface");
    }
    eprintln!();
    eprintln!("  NOTE: No BMW ENET adapter detected yet.");
    eprintln!("  The laptop will still connect to the desktop Host.");
    eprintln!("  Plug the ENET cable in when ready — watch http://127.0.0.1:47903/");
    eprintln!();
    *enet_name.write() = if preferred.is_empty() {
        "pending-enet".into()
    } else {
        preferred.clone()
    };
    Ok(Arc::new(MonitoredEthernet {
        name: enet_name,
        preferred,
        link: enet_link,
    }))
}

fn local_ipv4s() -> Vec<IpAddr> {
    let mut out = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(local) = sock.local_addr() {
                if matches!(local.ip(), IpAddr::V4(v4) if !v4.is_loopback()) {
                    out.push(local.ip());
                }
            }
        }
    }
    // Best-effort: also scrape ipconfig-style names via sysinfo interfaces (MAC only);
    // primary outbound IP above is the important self-check.
    out.sort_by_key(|a| a.to_string());
    out.dedup();
    out
}

fn warn_if_peer_is_local(peer: IpAddr) {
    let locals = local_ipv4s();
    if locals.iter().any(|ip| *ip == peer) {
        eprintln!();
        eprintln!("  *** ERROR: --peer {peer} is THIS LAPTOP's own IP ***");
        eprintln!("  That can never reach the desktop Host.");
        eprintln!("  On the DESKTOP open http://127.0.0.1:47901/ and copy the LAN IP shown.");
        eprintln!("  On this laptop open http://127.0.0.1:47903/ and click Connect.");
        eprintln!();
    }
}

fn is_local_ip(peer: IpAddr) -> bool {
    local_ipv4s().iter().any(|ip| *ip == peer)
}

/// Refresh BMW-ENET-Client scheduled task so the next boot keeps --peer.
fn refresh_client_autostart(config_path: &Path, peer: IpAddr, pair_code: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let Some(config_dir) = config_path.parent() else {
            return;
        };
        let Some(install_dir) = config_dir.parent() else {
            return;
        };
        let exe = install_dir.join("enet-agent.exe");
        if !exe.is_file() {
            return;
        }
        let mut args = format!("--config \"{}\" --peer {peer}", config_path.display());
        if !pair_code.trim().is_empty() {
            args.push_str(&format!(" --pair-code {}", pair_code.trim()));
        }
        let script = format!(
            r#"
$ErrorActionPreference = 'Stop'
$taskName = 'BMW-ENET-Client'
$exe = '{exe}'
$args = '{args}'
$wd = '{wd}'
$existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
if (-not $existing) {{ return }}
$action = New-ScheduledTaskAction -Execute $exe -Argument $args -WorkingDirectory $wd
Set-ScheduledTask -TaskName $taskName -Action $action | Out-Null
"#,
            exe = exe.display().to_string().replace('\'', "''"),
            args = args.replace('\'', "''"),
            wd = install_dir.display().to_string().replace('\'', "''"),
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = (config_path, peer, pair_code);
    }
}

async fn resolve_peer(
    cfg: &GatewayConfig,
    args: &Args,
    force_discover: bool,
) -> anyhow::Result<(IpAddr, u16)> {
    let cached = if force_discover {
        None
    } else {
        cfg.peer_addr.or(args.peer)
    };

    if matches!(cfg.network_mode, NetworkMode::Wireguard) && cached.is_none() {
        let ip: IpAddr = cfg
            .wireguard_desktop_ip
            .parse()
            .context("wireguard_desktop_ip invalid")?;
        return Ok((ip, cfg.tunnel_port));
    }

    // Always try LAN discovery when enabled (DHCP IPs change — never rely only on a sticky peer).
    let should_discover = cfg.auto_discover || force_discover || cached.is_none();
    if should_discover && !matches!(cfg.network_mode, NetworkMode::Wireguard) {
        let code = args
            .pair_code
            .clone()
            .unwrap_or_else(|| cfg.pair_code.clone());
        eprintln!(
            "Looking for BMW ENET Gateway on your LAN{}…",
            if force_discover {
                " (re-detecting IP)"
            } else {
                ""
            }
        );
        if !code.is_empty() {
            eprintln!("  (filtering for pair code {code})");
        }
        match discover_gateways(cfg.discovery_port, &code, Duration::from_secs(5)).await {
            Ok(found) if !found.is_empty() => {
                // Prefer a discovery result that matches the cached hint when still valid.
                let gw = found
                    .iter()
                    .find(|g| cached.map(|c| c == g.addr).unwrap_or(false))
                    .or_else(|| found.first())
                    .cloned()
                    .unwrap();
                if gw.password_required && cfg.password.is_empty() {
                    eprintln!(
                        "WARNING: desktop requires a password, but this agent has an empty password."
                    );
                }
                if !gw.password_required && !cfg.password.is_empty() {
                    eprintln!(
                        "WARNING: this agent has a password set, but the desktop does not — clear password on the laptop or set the same password on the Host."
                    );
                }
                eprintln!(
                    "Found desktop “{}” at {} (tunnel {})",
                    gw.hostname, gw.addr, gw.tunnel_port
                );
                warn_if_peer_is_local(gw.addr);
                return Ok((gw.addr, gw.tunnel_port));
            }
            Ok(_) => {
                eprintln!("  No Host beacon heard yet — will fall back to saved IP if any.");
            }
            Err(e) => {
                eprintln!("  Discovery error: {e}");
            }
        }
    }

    if let Some(peer) = cached.or(cfg.peer_addr).or(args.peer) {
        warn_if_peer_is_local(peer);
        eprintln!("Using saved desktop IP {peer} (will auto-redetect if it stops responding)");
        return Ok((peer, cfg.tunnel_port));
    }

    if !cfg.auto_discover {
        anyhow::bail!(
            "No desktop address configured and auto-discover is off.\n\
             Open http://127.0.0.1:47903/ and click Auto-find, or enter a Desktop IP."
        );
    }

    anyhow::bail!(
        "No desktop on this LAN yet.\n\
         Make sure the Host is running on the desktop (http://127.0.0.1:47901/).\n\
         Same home router required (not Guest / client-isolation Wi‑Fi).\n\
         Windows Firewall must allow UDP 47900 and 47902.\n\
         Fallback: open http://127.0.0.1:47903/, enter a Desktop LAN IP, click Connect."
    )
}

fn friendly_line(desktop: bool, enet: bool, awake: bool) -> String {
    if !desktop {
        "Waiting for desktop…".into()
    } else if !enet {
        "Desktop OK — plug ENET into car + laptop".into()
    } else if !awake {
        "ENET link up — turn ignition ON".into()
    } else {
        "Ready — vehicle awake".into()
    }
}

async fn api_status(State(live): State<Arc<LiveStatus>>) -> Json<StatusJson> {
    let (desktop, awake, vehicle_link, rtt_ms, loss_rate, conn_label) = {
        let guard = live.handle.read();
        if let Some(h) = guard.as_ref() {
            let st = h.snapshot_state();
            let (last, _p99, loss) = h.stats.peek_quality();
            // Honest state only — a stale RTT sample is not a connection.
            let desk = matches!(st.connection, ConnectionState::Connected)
                || st.laptop_connected;
            let label = match st.connection {
                ConnectionState::Connected => "connected",
                ConnectionState::Reconnecting => "reconnecting",
                ConnectionState::WaitingForPeer => "waiting",
                ConnectionState::Starting => "starting",
                _ => "down",
            };
            (
                desk,
                st.vehicle.awake,
                st.vehicle.link_up,
                last,
                loss,
                label,
            )
        } else {
            (false, false, false, 0.0, 0.0, "searching")
        }
    };
    // OS carrier alone drives the cable / Vehicle ENET indicator.
    let enet = live.enet_link.load(Ordering::Relaxed);
    let desktop_connected = desktop;
    let friendly = match conn_label {
        "searching" => {
            if live.configured_peer.read().is_some() {
                "Auto-finding / dialing desktop…".into()
            } else {
                "Auto-finding desktop on your LAN…".into()
            }
        }
        "waiting" => "Waiting for desktop reply…".into(),
        "reconnecting" => "Reconnecting — will re-detect IP if needed…".into(),
        _ => friendly_line(desktop_connected, enet, awake),
    };
    Json(StatusJson {
        version: env!("CARGO_PKG_VERSION").into(),
        pair_code: live.pair_code.read().clone(),
        desktop_connected,
        desktop_peer: live.desktop_peer.read().clone(),
        configured_peer: (*live.configured_peer.read()).map(|ip| ip.to_string()),
        enet_interface: live.enet_name.read().clone(),
        enet_link: enet,
        l2_active: live.l2_active.load(Ordering::Relaxed),
        l2_label: live.l2_label.read().clone(),
        vehicle_awake: awake,
        vehicle_link,
        rtt_ms,
        loss_rate,
        friendly,
        update_available: live
            .update_available
            .read()
            .as_ref()
            .map(|u| u.version.clone()),
        auto_update: *live.auto_update.read(),
    })
}

/// Release asset carrying the laptop Client package.
const UPDATE_ASSET: &str = "BMW-ENET-Client-windows-x64.zip";

fn check_update_blocking(
    repo: &str,
    token: &str,
) -> Option<enet_core::updater::UpdateInfo> {
    if !cfg!(windows) || repo.is_empty() {
        return None;
    }
    match enet_core::updater::check_latest(repo, env!("CARGO_PKG_VERSION"), UPDATE_ASSET, token) {
        Ok(u) => u,
        Err(e) => {
            info!(error = %e, "update check skipped");
            None
        }
    }
}

/// Download, swap binaries, restart. Only returns on failure.
fn apply_update_blocking(
    info: &enet_core::updater::UpdateInfo,
    token: &str,
) -> anyhow::Result<()> {
    let dir = enet_core::updater::install_dir()?;
    enet_core::updater::download_and_stage(info, &dir, token)?;
    enet_core::updater::restart_self()
}

async fn api_check_update(State(live): State<Arc<LiveStatus>>) -> Json<ConnectResponse> {
    let repo = live.update_repo.clone();
    let token = live.update_token.clone();
    let found = tokio::task::spawn_blocking(move || check_update_blocking(&repo, &token))
        .await
        .unwrap_or(None);
    match found {
        Some(u) => {
            let version = u.version.clone();
            *live.update_available.write() = Some(u);
            Json(ConnectResponse {
                ok: true,
                message: format!("Update available: v{version}"),
                peer: Some(version),
            })
        }
        None => {
            *live.update_available.write() = None;
            Json(ConnectResponse {
                ok: true,
                message: format!("Already up to date (v{})", env!("CARGO_PKG_VERSION")),
                peer: None,
            })
        }
    }
}

#[derive(Deserialize)]
struct AgentSettingsUpdate {
    auto_update: Option<bool>,
}

async fn api_get_settings(State(live): State<Arc<LiveStatus>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "auto_update": *live.auto_update.read(),
        "version": env!("CARGO_PKG_VERSION"),
        "update_available": live.update_available.read().as_ref().map(|u| u.version.clone()),
    }))
}

async fn api_set_settings(
    State(live): State<Arc<LiveStatus>>,
    Json(update): Json<AgentSettingsUpdate>,
) -> Json<ConnectResponse> {
    if let Some(v) = update.auto_update {
        *live.auto_update.write() = v;
        // Persist into agent.toml next to other settings.
        if let Ok(mut cfg) = GatewayConfig::load(&live.config_path) {
            cfg.auto_update = v;
            let _ = cfg.save(&live.config_path);
        }
    }
    Json(ConnectResponse {
        ok: true,
        message: "Settings saved".into(),
        peer: None,
    })
}

async fn api_update(State(live): State<Arc<LiveStatus>>) -> Json<ConnectResponse> {
    let cached = live.update_available.read().clone();
    let update = match cached {
        Some(u) => Some(u),
        None => {
            let repo = live.update_repo.clone();
            let token = live.update_token.clone();
            tokio::task::spawn_blocking(move || check_update_blocking(&repo, &token))
                .await
                .unwrap_or(None)
        }
    };
    match update {
        Some(u) => {
            let version = u.version.clone();
            let token = live.update_token.clone();
            if let Some(h) = live.handle.write().take() {
                h.stop();
            }
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let _ = tokio::task::spawn_blocking(move || apply_update_blocking(&u, &token)).await;
            });
            Json(ConnectResponse {
                ok: true,
                message: format!("Updating to v{version} — this page will reconnect shortly"),
                peer: None,
            })
        }
        None => Json(ConnectResponse {
            ok: true,
            message: format!("Already up to date (v{})", env!("CARGO_PKG_VERSION")),
            peer: None,
        }),
    }
}

async fn api_connect(
    State(live): State<Arc<LiveStatus>>,
    Json(req): Json<ConnectRequest>,
) -> (StatusCode, Json<ConnectResponse>) {
    let peer_str = req.peer.trim();
    let peer: IpAddr = match peer_str.parse() {
        Ok(ip) => ip,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ConnectResponse {
                    ok: false,
                    message: format!("Invalid IP address: {peer_str}"),
                    peer: None,
                }),
            );
        }
    };
    if is_local_ip(peer) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ConnectResponse {
                ok: false,
                message: format!(
                    "{peer} is this laptop's own IP. Copy a Desktop LAN IP from http://127.0.0.1:47901/"
                ),
                peer: None,
            }),
        );
    }

    let mut cfg = GatewayConfig::load(&live.config_path).unwrap_or_else(|_| {
        let mut c = GatewayConfig::default();
        c.role = Role::Agent;
        c.auto_discover = true;
        c
    });
    cfg.role = Role::Agent;
    cfg.peer_addr = Some(peer);
    // Keep auto-discover ON so DHCP / IP changes still re-learn the Host.
    cfg.auto_discover = true;
    if let Some(code) = req.pair_code.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        cfg.pair_code = code.to_string();
        *live.pair_code.write() = code.to_string();
    }

    if let Err(e) = cfg.save(&live.config_path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ConnectResponse {
                ok: false,
                message: format!("Could not save config: {e}"),
                peer: None,
            }),
        );
    }

    *live.configured_peer.write() = Some(peer);
    *live.desktop_peer.write() = format!("{}:{}", peer, live.tunnel_port);
    live.force_discover.store(false, Ordering::SeqCst);
    refresh_client_autostart(&live.config_path, peer, &cfg.pair_code);

    if let Some(h) = live.handle.write().take() {
        h.stop();
    }
    live.force_reconnect.store(true, Ordering::SeqCst);

    info!(%peer, "desktop peer hint set from status page");
    (
        StatusCode::OK,
        Json(ConnectResponse {
            ok: true,
            message: format!(
                "Saved {peer} as hint — dialing now. Auto-detect stays on if the IP changes later."
            ),
            peer: Some(peer.to_string()),
        }),
    )
}

async fn api_discover(
    State(live): State<Arc<LiveStatus>>,
) -> (StatusCode, Json<ConnectResponse>) {
    // Clear sticky peer so resolve_peer must hear a fresh Host beacon.
    *live.configured_peer.write() = None;
    live.force_discover.store(true, Ordering::SeqCst);

    if let Ok(mut cfg) = GatewayConfig::load(&live.config_path) {
        cfg.peer_addr = None;
        cfg.auto_discover = true;
        let _ = cfg.save(&live.config_path);
    }

    if let Some(h) = live.handle.write().take() {
        h.stop();
    }
    live.force_reconnect.store(true, Ordering::SeqCst);

    (
        StatusCode::OK,
        Json(ConnectResponse {
            ok: true,
            message: "Auto-finding desktop on your LAN…".into(),
            peer: None,
        }),
    )
}

async fn status_page() -> Html<&'static str> {
    Html(include_str!("status.html"))
}

fn spawn_status_server(live: Arc<LiveStatus>, port: u16) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(status_page))
            .route("/api/status", get(api_status))
            .route("/api/connect", post(api_connect))
            .route("/api/discover", post(api_discover))
            .route("/api/update", post(api_update))
            .route("/api/check-update", post(api_check_update))
            .route("/api/settings", get(api_get_settings).post(api_set_settings))
            .with_state(live);
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        // Retry the bind — after an update restart the old process may hold
        // the port for a second or two while it exits.
        for attempt in 0..10u32 {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    info!(%addr, "laptop status page listening");
                    if let Err(e) = axum::serve(listener, app).await {
                        warn!(error = %e, "status server stopped");
                    }
                    return;
                }
                Err(e) => {
                    if attempt == 9 {
                        warn!(error = %e, %port, "could not bind laptop status page");
                    } else {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    });
}

fn spawn_enet_link_refresher(enet_name: Arc<RwLock<String>>, enet_link: Arc<AtomicBool>) {
    tokio::spawn(async move {
        loop {
            let name = enet_name.read().clone();
            if !name.is_empty() && name != "pending-enet" {
                let name_for_job = name.clone();
                let up = tokio::task::spawn_blocking(move || adapter_link_up(&name_for_job))
                    .await
                    .unwrap_or(false);
                enet_link.store(up, Ordering::Relaxed);
            } else if let Some(iface) = pick_enet_interface("") {
                *enet_name.write() = iface.name.clone();
                let n = iface.name;
                let up = tokio::task::spawn_blocking(move || adapter_link_up(&n))
                    .await
                    .unwrap_or(false);
                enet_link.store(up, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn run_until_stop(handle: TunnelHandle, live: Arc<LiveStatus>) {
    *live.handle.write() = Some(handle.clone());
    let mut last_line = String::new();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown requested");
            handle.stop();
        }
        _ = async {
            while handle.is_running() && !live.force_reconnect.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                // Link bit is refreshed by spawn_enet_link_refresher (non-blocking here).
                let st = handle.snapshot_state();
                let (_last, rtt_p99, _loss) = handle.stats.peek_quality();
                let desk = matches!(st.connection, ConnectionState::Connected)
                    || st.laptop_connected;
                let enet = live.enet_link.load(Ordering::Relaxed);
                let line = format!(
                    "[Desktop: {}] [ENET: {}] [Vehicle: {}]  RTT {:.0} ms",
                    if desk { "OK" } else { "…" },
                    if enet { "PLUGGED" } else { "—" },
                    if st.vehicle.awake { "AWAKE" } else { "SLEEP" },
                    rtt_p99
                );
                if line != last_line {
                    eprintln!("  {line}");
                    last_line = line;
                }
                if matches!(st.connection, ConnectionState::Failed) {
                    live.force_discover.store(true, Ordering::SeqCst);
                    break;
                }
            }
        } => {
            if live.force_reconnect.load(Ordering::SeqCst) {
                info!("reconnect requested via status page");
            } else {
                warn!("tunnel stopped; will reconnect");
            }
            handle.stop();
        }
    }
    *live.handle.write() = None;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let _instance = match acquire_single_instance() {
        Some(g) => g,
        None => {
            eprintln!();
            eprintln!("  BMW ENET Agent is already running.");
            eprintln!("  Open status: http://127.0.0.1:47903/");
            eprintln!();
            eprintln!("  If the Host shows Connected but this page says Waiting,");
            eprintln!("  you likely had two Clients fighting. Fix:");
            eprintln!("    Stop-Process -Name enet-agent -Force");
            eprintln!("    Start-ScheduledTask -TaskName BMW-ENET-Client");
            eprintln!();
            std::process::exit(0);
        }
    };
    let mut cfg = GatewayConfig::load(&args.config).unwrap_or_else(|_| {
        let mut c = GatewayConfig::default();
        c.role = Role::Agent;
        c.auto_discover = true;
        c
    });
    cfg.role = Role::Agent;
    if let Some(peer) = args.peer {
        cfg.peer_addr = Some(peer);
    }
    if let Some(code) = &args.pair_code {
        cfg.pair_code = code.clone();
    }
    if let Some(relay) = &args.relay {
        cfg.network_mode = NetworkMode::Relay;
        cfg.relay_url = relay.clone();
        cfg.apply_remote_defaults();
    }
    // Persist --peer / --pair-code so scheduled Client restarts keep working across Wi‑Fi↔LAN.
    if args.peer.is_some() || args.pair_code.is_some() {
        if let Err(e) = cfg.save(&args.config) {
            eprintln!("warning: could not save config: {e}");
        } else {
            eprintln!("Saved peer/pair settings to {}", args.config.display());
        }
    }

    let status_port = args
        .status_port
        .unwrap_or(if cfg.api_port == 47901 {
            DEFAULT_AGENT_API_PORT
        } else {
            cfg.api_port
        });

    let enet_name = Arc::new(RwLock::new(String::new()));
    let enet_link = Arc::new(AtomicBool::new(false));
    let l2_active = Arc::new(AtomicBool::new(false));
    let l2_label = Arc::new(RwLock::new(String::from("starting…")));
    let live = Arc::new(LiveStatus {
        handle: RwLock::new(None),
        pair_code: RwLock::new(cfg.pair_code.clone()),
        desktop_peer: RwLock::new(
            cfg.peer_addr
                .map(|ip| format!("{ip}:{}", cfg.tunnel_port))
                .unwrap_or_default(),
        ),
        configured_peer: RwLock::new(cfg.peer_addr),
        force_discover: AtomicBool::new(false),
        enet_name: enet_name.clone(),
        enet_link: enet_link.clone(),
        l2_active: l2_active.clone(),
        l2_label: l2_label.clone(),
        force_reconnect: AtomicBool::new(false),
        config_path: args.config.clone(),
        tunnel_port: cfg.tunnel_port,
        update_available: RwLock::new(None),
        update_repo: cfg.update_repo.clone(),
        update_token: cfg.update_token.clone(),
        auto_update: RwLock::new(cfg.auto_update),
    });
    spawn_status_server(live.clone(), status_port);
    spawn_enet_link_refresher(enet_name.clone(), enet_link.clone());

    let _guard = init_logging(cfg.log_level, &cfg.log_dir)?;

    // Self-update: clean leftovers, then check GitHub on every start.
    if let Ok(dir) = enet_core::updater::install_dir() {
        enet_core::updater::cleanup_stale(&dir);
        let dir2 = dir.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            enet_core::updater::cleanup_stale(&dir2);
        });
    }
    {
        let repo = cfg.update_repo.clone();
        let token = cfg.update_token.clone();
        let found = tokio::task::spawn_blocking(move || check_update_blocking(&repo, &token))
            .await
            .unwrap_or(None);
        if let Some(update) = found {
            if cfg.auto_update {
                eprintln!("  Update found: v{} — installing…", update.version);
                let token = cfg.update_token.clone();
                let _ = tokio::task::spawn_blocking(move || apply_update_blocking(&update, &token)).await;
                eprintln!("  Update failed — continuing with v{}.", env!("CARGO_PKG_VERSION"));
            } else {
                eprintln!(
                    "  Update available: v{} (auto-update off — use Settings → Check for updates)",
                    update.version
                );
                *live.update_available.write() = Some(update);
            }
        } else {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                "startup update check: already up to date"
            );
        }
    }

    // Periodic update check (every 6 h); auto-install only while the desktop
    // is not connected so a session is never interrupted.
    {
        let live_upd = live.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
                let repo = live_upd.update_repo.clone();
                let token = live_upd.update_token.clone();
                let found = tokio::task::spawn_blocking(move || check_update_blocking(&repo, &token))
                    .await
                    .unwrap_or(None);
                let Some(update) = found else { continue };
                info!(version = %update.version, "update available");
                *live_upd.update_available.write() = Some(update.clone());
                let connected = live_upd
                    .handle
                    .read()
                    .as_ref()
                    .map(|h| {
                        matches!(
                            h.snapshot_state().connection,
                            ConnectionState::Connected
                        )
                    })
                    .unwrap_or(false);
                let auto = *live_upd.auto_update.read();
                if auto && !connected {
                    eprintln!("  Auto-installing v{} — restarting…", update.version);
                    if let Some(h) = live_upd.handle.write().take() {
                        h.stop();
                    }
                    let token = live_upd.update_token.clone();
                    let _ = tokio::task::spawn_blocking(move || apply_update_blocking(&update, &token)).await;
                    eprintln!("  Update install failed");
                }
            }
        });
    }
    info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = ?cfg.network_mode,
        "enet-agent starting"
    );
    eprintln!();
    eprintln!("  BMW ENET Agent (laptop)");
    eprintln!("  Mode: {}", cfg.network_mode.label());
    eprintln!("  Status: http://127.0.0.1:{status_port}/");
    eprintln!("  Auto-detect: ON (works with changing / DHCP desktop IPs)");
    eprintln!("  -----------------------");
    for hint in cfg.setup_hints() {
        eprintln!("  {hint}");
    }
    eprintln!();

    let mut attempt = 0u32;
    // Always keep discovery enabled for LAN so DHCP changes are recovered.
    if cfg.network_mode == NetworkMode::Lan {
        cfg.auto_discover = true;
    }
    loop {
        // Status-page Connect updates configured_peer + agent.toml (hint only).
        if let Some(p) = *live.configured_peer.read() {
            cfg.peer_addr = Some(p);
        }
        if cfg.network_mode == NetworkMode::Lan {
            cfg.auto_discover = true;
        }
        {
            let code = live.pair_code.read().clone();
            if !code.is_empty() {
                cfg.pair_code = code;
            }
        }
        let force_discover = live.force_discover.swap(false, Ordering::SeqCst);
        live.force_reconnect.store(false, Ordering::SeqCst);
        *live.pair_code.write() = cfg.pair_code.clone();
        let eth = match build_ethernet_port(
            &cfg,
            args.simulate,
            enet_name.clone(),
            enet_link.clone(),
            l2_active.clone(),
            l2_label.clone(),
        )
        .await
        {
            Ok(e) => e,
            Err(e) => {
                eprintln!("\n{e}\n");
                attempt = attempt.saturating_add(1);
                sleep_or_reconnect(&live, backoff_delay(
                    cfg.reconnect_delay_ms,
                    cfg.reconnect_delay_max_ms,
                    attempt,
                ))
                .await;
                continue;
            }
        };

        let started = if cfg.network_mode == NetworkMode::Relay {
            if cfg.relay_url.is_empty() {
                eprintln!("relay_url is empty — set --relay host:47910");
                attempt = attempt.saturating_add(1);
                sleep_or_reconnect(&live, Duration::from_secs(2)).await;
                continue;
            }
            if cfg.pair_code.is_empty() {
                eprintln!("pair_code required for relay mode (from desktop dashboard)");
                attempt = attempt.saturating_add(1);
                sleep_or_reconnect(&live, Duration::from_secs(2)).await;
                continue;
            }
            let base = TunnelOptions {
                bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
                peer: None,
                allowed_cidrs: cfg.allowed_cidrs.clone(),
                crypto: None,
                require_crypto: cfg.require_crypto,
                keepalive_interval_ms: cfg.keepalive_interval_ms.clamp(200, 250),
                peer_timeout_ms: cfg.peer_timeout_ms,
                role: "agent".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            }
            .with_password(&cfg.password, cfg.require_crypto);
            let ropts = RelayTunnelOptions {
                base,
                relay_url: cfg.relay_url.clone(),
                pair_code: cfg.pair_code.clone(),
            };
            eprintln!("Dialing relay {} …", cfg.relay_url);
            match RelayTunnelEngine::new(ropts, eth).run().await {
                Ok(h) => {
                    eprintln!("Joined relay. Status: http://127.0.0.1:{status_port}/");
                    Some(h)
                }
                Err(e) => {
                    eprintln!("Relay connect failed: {e}");
                    None
                }
            }
        } else {
            match resolve_peer(&cfg, &args, force_discover).await {
                Ok((peer_ip, tunnel_port)) => {
                    let peer = SocketAddr::new(peer_ip, tunnel_port);
                    *live.desktop_peer.write() = peer.to_string();
                    *live.configured_peer.write() = Some(peer_ip);
                    // Persist last-known good IP as a fast hint (discovery still runs).
                    if cfg.peer_addr != Some(peer_ip) {
                        cfg.peer_addr = Some(peer_ip);
                        cfg.auto_discover = true;
                        let _ = cfg.save(&args.config);
                    }
                    let opts = TunnelOptions {
                        bind: SocketAddr::from((
                            cfg.bind_addr.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
                            0,
                        )),
                        peer: Some(peer),
                        allowed_cidrs: cfg.allowed_cidrs.clone(),
                        crypto: None,
                        require_crypto: cfg.require_crypto,
                        keepalive_interval_ms: cfg.keepalive_interval_ms.clamp(200, 250),
                        peer_timeout_ms: cfg.peer_timeout_ms,
                        role: "agent".into(),
                        version: env!("CARGO_PKG_VERSION").into(),
                    }
                    .with_password(&cfg.password, cfg.require_crypto);
                    match TunnelEngine::new(opts, eth).run().await {
                        Ok(h) => {
                            eprintln!(
                                "Dialing desktop at {peer} … waiting for Host reply.\n\
                            Status: http://127.0.0.1:{status_port}/\n\
                            (IP is auto-detected; if your router reassigns it, Client re-learns automatically.)"
                            );
                            Some(h)
                        }
                        Err(e) => {
                            eprintln!("Tunnel failed: {e}");
                            live.force_discover.store(true, Ordering::SeqCst);
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("\n{e}\n");
                    live.force_discover.store(true, Ordering::SeqCst);
                    None
                }
            }
        };

        if let Some(handle) = started {
            attempt = 0;
            run_until_stop(handle, live.clone()).await;
            if live.force_reconnect.load(Ordering::SeqCst) {
                eprintln!("Applying status-page change…");
                continue;
            }
            // After a drop, re-detect in case DHCP moved the Host.
            live.force_discover.store(true, Ordering::SeqCst);
        }

        attempt = attempt.saturating_add(1);
        let delay = backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
        eprintln!("Reconnecting in {delay:?}…");
        sleep_or_reconnect(&live, delay).await;
    }
}

async fn sleep_or_reconnect(live: &LiveStatus, delay: Duration) {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if live.force_reconnect.load(Ordering::SeqCst) {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let slice = remaining.min(Duration::from_millis(200));
        if slice.is_zero() {
            break;
        }
        tokio::time::sleep(slice).await;
    }
}
