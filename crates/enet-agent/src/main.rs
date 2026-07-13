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
    force_reconnect: AtomicBool,
    config_path: PathBuf,
    tunnel_port: u16,
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
    vehicle_awake: bool,
    vehicle_link: bool,
    rtt_ms: f64,
    loss_rate: f64,
    friendly: String,
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
) -> anyhow::Result<Arc<dyn EthernetPort>> {
    if simulate {
        let (port, _peer) = SimulatedEthernet::pair("sim-enet", "sim-car");
        info!("using simulated ENET interface");
        *enet_name.write() = "sim-enet".into();
        enet_link.store(true, Ordering::Relaxed);
        std::mem::forget(_peer);
        return Ok(port);
    }

    #[cfg(windows)]
    {
        if !enet_tunnel::PcapEthernet::npcap_available() {
            eprintln!();
            eprintln!("  *** Npcap required for ISTA ***");
            eprintln!("  Install from https://npcap.com (enable WinPcap API compatibility),");
            eprintln!("  then restart BMW ENET Client.");
            eprintln!("  Tunnel can still connect, but car frames will NOT reach the desktop.");
            eprintln!();
        } else if let Some(iface) = pick_enet_interface(cfg.enet_interface.as_str()) {
            // Only open the scored ENET candidate — never the Wi‑Fi / LAN tunnel NIC.
            match enet_tunnel::PcapEthernet::open(&iface.name) {
                Ok(port) => {
                    let shown = port.display_name().to_string();
                    info!(adapter = %port.name(), %shown, "Client L2 ENET capture ready");
                    *enet_name.write() = shown.clone();
                    enet_link.store(true, Ordering::Relaxed);
                    eprintln!("  ENET capture: {shown}");
                    return Ok(port);
                }
                Err(e) => {
                    // Fall back: match Npcap description against the OS adapter name.
                    warn!(error = %e, name = %iface.name, "direct pcap open failed; trying description match");
                    if let Ok(list) = enet_tunnel::PcapEthernet::list_devices() {
                        let want = iface.name.to_lowercase();
                        for line in list {
                            let lower = line.to_lowercase();
                            if lower.contains(&want)
                                || (want.len() >= 4 && lower.contains(want.trim()))
                            {
                                let npf = line.split('|').next().unwrap_or(&line);
                                if let Ok(port) = enet_tunnel::PcapEthernet::open(npf) {
                                    let shown = port.display_name().to_string();
                                    *enet_name.write() = shown.clone();
                                    enet_link.store(true, Ordering::Relaxed);
                                    eprintln!("  ENET capture: {shown}");
                                    return Ok(port);
                                }
                            }
                        }
                    }
                    eprintln!();
                    eprintln!("  Could not open Npcap on ENET adapter '{}'.", iface.name);
                    eprintln!("  Run Client as Administrator / SYSTEM and confirm Npcap is installed.");
                    eprintln!();
                }
            }
        } else {
            eprintln!();
            eprintln!("  Npcap is installed but no ENET adapter was detected yet.");
            eprintln!("  Plug the ENET cable into the car + laptop, then restart Client.");
            eprintln!();
        }
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
            let desk = matches!(
                st.connection,
                ConnectionState::Connected | ConnectionState::Reconnecting
            ) || st.laptop_connected
                || last > 0.0;
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
    // Prefer OS carrier for cable indicator; fall back to tunnel vehicle_link.
    let enet = live.enet_link.load(Ordering::Relaxed) || vehicle_link;
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
        vehicle_awake: awake,
        vehicle_link: enet,
        rtt_ms,
        loss_rate,
        friendly,
    })
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
            .with_state(live);
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                info!(%addr, "laptop status page listening");
                if let Err(e) = axum::serve(listener, app).await {
                    warn!(error = %e, "status server stopped");
                }
            }
            Err(e) => warn!(error = %e, %port, "could not bind laptop status page"),
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
                let enet = live.enet_link.load(Ordering::Relaxed) || st.vehicle.link_up;
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
        force_reconnect: AtomicBool::new(false),
        config_path: args.config.clone(),
        tunnel_port: cfg.tunnel_port,
    });
    spawn_status_server(live.clone(), status_port);
    spawn_enet_link_refresher(enet_name.clone(), enet_link.clone());

    let _guard = init_logging(cfg.log_level, &cfg.log_dir)?;
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
        let eth = match build_ethernet_port(&cfg, args.simulate, enet_name.clone(), enet_link.clone())
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
