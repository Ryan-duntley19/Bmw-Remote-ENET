//! Desktop ENET gateway — Windows service-compatible tunnel server + friendly dashboard.

use anyhow::Context;
use axum::extract::State;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use enet_core::config::{GatewayConfig, NetworkMode, Role};
use enet_core::health::HealthMonitor;
use enet_core::logging::init_logging;
use enet_core::run_gateway_beacon;
use enet_core::safety::{FlashSafetyChecker, SafetyThresholds};
use enet_core::state::{ConnectionState, GatewayState};
use enet_tunnel::{
    EthernetPort, RelayTunnelEngine, RelayTunnelOptions, SimulatedEthernet, TunnelEngine,
    TunnelHandle, TunnelOptions,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "enet-gateway", about = "BMW ENET desktop gateway")]
struct Args {
    /// Path to gateway.toml
    #[arg(short, long, default_value = "config/gateway.toml")]
    config: PathBuf,
    /// Force in-memory TAP (no Npcap / BMW-ENET) — for tests only
    #[arg(long)]
    simulate: bool,
    /// Run once and exit after N seconds (for tests)
    #[arg(long)]
    run_seconds: Option<u64>,
    /// Skip opening hints in logs
    #[arg(long)]
    quiet: bool,
    /// Force relay URL (enables remote relay mode)
    #[arg(long)]
    relay: Option<String>,
}

/// Host L2 port + whether ISTA forwarding is actually active + adapter label.
type HostEthernet = (Arc<dyn EthernetPort>, bool, String);

fn build_host_ethernet(cfg: &GatewayConfig, simulate: bool) -> HostEthernet {
    let fallback = |label: &str| -> HostEthernet {
        let (tap, _tool_peer) = SimulatedEthernet::pair(&cfg.virtual_interface, "tool-stack");
        tap.set_link(true);
        std::mem::forget(_tool_peer);
        (tap, false, label.to_string())
    };

    if simulate {
        warn!("Host running in --simulate mode — ISTA cannot see the car");
        return fallback("simulated (test mode)");
    }

    #[cfg(windows)]
    {
        if !enet_tunnel::PcapEthernet::npcap_available() {
            eprintln!();
            eprintln!("  *** Npcap not found — launching installer ***");
            eprintln!("  Enable “WinPcap API-compatible Mode”, then Finish.");
            eprintln!();
            let installed = enet_core::ensure_npcap_installed(|msg| {
                eprintln!("  {msg}");
            })
            .unwrap_or(false);
            if !installed || !enet_tunnel::PcapEthernet::npcap_available() {
                eprintln!();
                eprintln!("  *** ISTA will NOT see the car yet ***");
                eprintln!("  Install Npcap: https://npcap.com  (enable WinPcap API compatibility)");
                eprintln!("  Re-run BMW-ENET-Setup (Host) to create BMW-ENET at {}", cfg.tester_ip);
                eprintln!("  Tunnel stays up for connection testing; L2/ISTA path is inactive.");
                eprintln!();
                warn!("Npcap missing — Host L2 disabled");
                return fallback("Npcap not installed");
            }
        }
        let candidates = [
            cfg.virtual_interface.as_str(),
            "BMW-ENET",
            "KM-TEST Loopback",
            "Loopback",
        ];
        let mut last_err = None;
        for want in candidates {
            if want.is_empty() {
                continue;
            }
            match enet_tunnel::PcapEthernet::open(want) {
                Ok(port) => {
                    let label = port.display_name().to_string();
                    info!(
                        adapter = %port.name(),
                        display = %label,
                        tester_ip = %cfg.tester_ip,
                        "Host L2 port ready for ISTA"
                    );
                    eprintln!();
                    eprintln!("  ISTA / E-Sys: select adapter “{}”", cfg.virtual_interface);
                    eprintln!("  Tester IP on that adapter should be {}", cfg.tester_ip);
                    eprintln!();
                    return (port, true, label);
                }
                Err(e) => last_err = Some(e),
            }
        }
        eprintln!();
        eprintln!("  *** BMW-ENET adapter not open — ISTA cannot see the car ***");
        eprintln!(
            "  Re-run BMW-ENET-Setup as Host (creates loopback BMW-ENET at {}).",
            cfg.tester_ip
        );
        if let Some(e) = last_err {
            eprintln!("  Detail: {e:#}");
        }
        eprintln!("  Tunnel stays up; fix the adapter then restart Host.");
        eprintln!();
        warn!("BMW-ENET pcap open failed — Host L2 disabled");
        fallback("BMW-ENET adapter missing")
    }

    #[cfg(not(windows))]
    {
        warn!("non-Windows Host uses SimulatedEthernet — ISTA path is Windows-only");
        fallback("simulated (non-Windows)")
    }
}

/// Build the tunnel engine (LAN or relay) from the current config.
async fn start_tunnel(
    cfg: &GatewayConfig,
    pair_code: &str,
    simulate: bool,
) -> anyhow::Result<(TunnelHandle, bool, String)> {
    let (eth, l2_active, l2_label) = build_host_ethernet(cfg, simulate);

    let base_opts = TunnelOptions {
        bind: SocketAddr::from((
            cfg.bind_addr.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            cfg.tunnel_port,
        )),
        peer: cfg.peer_addr.map(|ip| SocketAddr::new(ip, 0)),
        allowed_cidrs: cfg.allowed_cidrs.clone(),
        crypto: None,
        require_crypto: cfg.require_crypto,
        keepalive_interval_ms: cfg.keepalive_interval_ms,
        peer_timeout_ms: cfg.peer_timeout_ms,
        role: "gateway".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
    .with_password(&cfg.password, cfg.require_crypto);

    let handle = if cfg.network_mode == NetworkMode::Relay {
        anyhow::ensure!(
            !cfg.relay_url.is_empty(),
            "relay mode needs relay_url (or --relay host:47910)"
        );
        let ropts = RelayTunnelOptions {
            base: base_opts,
            relay_url: cfg.relay_url.clone(),
            pair_code: pair_code.to_string(),
        };
        RelayTunnelEngine::new(ropts, eth)
            .run()
            .await
            .context("failed to join relay")?
    } else {
        let bind = base_opts.bind;
        let handle = TunnelEngine::new(base_opts, eth)
            .run()
            .await
            .context("failed to bind gateway tunnel")?;
        info!(%bind, "gateway tunnel listening");
        handle
    };
    Ok((handle, l2_active, l2_label))
}

/// Dashboard tunnel lifecycle commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TunnelCmd {
    Start,
    Stop,
    Restart,
}

/// Release asset carrying the desktop Host package.
const UPDATE_ASSET: &str = "BMW-ENET-Host-windows-x64.zip";

fn check_update_blocking(cfg: &GatewayConfig) -> Option<enet_core::updater::UpdateInfo> {
    if !cfg!(windows) || cfg.update_repo.is_empty() {
        return None;
    }
    match enet_core::updater::check_latest(
        &cfg.update_repo,
        env!("CARGO_PKG_VERSION"),
        UPDATE_ASSET,
        &cfg.update_token,
    ) {
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

#[derive(Clone)]
struct AppState {
    cfg: Arc<RwLock<GatewayConfig>>,
    handle: Arc<RwLock<Option<TunnelHandle>>>,
    health: Arc<RwLock<HealthMonitor>>,
    config_path: PathBuf,
    activity: Arc<RwLock<ActivityLog>>,
    /// (ISTA L2 forwarding active, adapter label)
    l2: Arc<RwLock<(bool, String)>>,
    tunnel_tx: tokio::sync::mpsc::UnboundedSender<TunnelCmd>,
    /// Newer release found by the periodic check.
    update_available: Arc<RwLock<Option<enet_core::updater::UpdateInfo>>>,
}

#[derive(Clone, Serialize)]
struct ActivityEntry {
    ts: f64,
    level: String,
    message: String,
}

struct ActivityLog {
    entries: Vec<ActivityEntry>,
}

impl ActivityLog {
    fn new() -> Self {
        Self {
            entries: vec![ActivityEntry {
                ts: now_secs_f64(),
                level: "info".into(),
                message: "Dashboard ready — waiting for laptop agent".into(),
            }],
        }
    }

    fn push(&mut self, level: &str, message: impl Into<String>) {
        self.entries.push(ActivityEntry {
            ts: now_secs_f64(),
            level: level.into(),
            message: message.into(),
        });
        const MAX: usize = 400;
        if self.entries.len() > MAX {
            let drain = self.entries.len() - MAX;
            self.entries.drain(0..drain);
        }
    }
}

#[derive(Serialize)]
struct StatusResponse {
    state: GatewayState,
    stats: enet_core::stats::StatsSnapshot,
    cpu_pct: f64,
    memory_used: u64,
    memory_total: u64,
    flash_safety: enet_core::safety::FlashSafetyReport,
    pair_code: String,
    setup_hints: Vec<String>,
    setup_complete: bool,
    friendly_status: String,
    network_mode: String,
    network_mode_label: String,
    is_remote: bool,
    relay_url: String,
    /// This PC's LAN IPv4 addresses (for Wi‑Fi laptop → wired desktop connect).
    lan_ips: Vec<String>,
    /// Ready-to-run laptop command when auto-discovery fails.
    connect_command: String,
    /// Desktop-measured RTT to the laptop (desktop→laptop direction).
    rtt_to_laptop_ms: f64,
    /// Laptop-reported RTT (laptop→desktop direction).
    rtt_from_laptop_ms: f64,
    /// True when real L2 frames can reach ISTA (Npcap + BMW-ENET open).
    l2_active: bool,
    /// Adapter label or reason the ISTA bridge is inactive.
    l2_adapter: String,
    /// Newer release version available for install (e.g. "0.1.20").
    update_available: Option<String>,
    /// Whether new releases auto-install when idle.
    auto_update: bool,
}

#[derive(Deserialize)]
struct SettingsUpdate {
    tunnel_port: Option<u16>,
    password: Option<String>,
    log_level: Option<String>,
    reconnect_delay_ms: Option<u64>,
    peer_timeout_ms: Option<u64>,
    require_crypto: Option<bool>,
    auto_start: Option<bool>,
    pair_code: Option<String>,
    setup_complete: Option<bool>,
    auto_discover: Option<bool>,
    auto_update: Option<bool>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut cfg = GatewayConfig::load(&args.config).unwrap_or_default();
    cfg.role = Role::Gateway;
    if let Some(relay) = &args.relay {
        cfg.network_mode = NetworkMode::Relay;
        cfg.relay_url = relay.clone();
        cfg.apply_remote_defaults();
    }
    let pair_code = cfg.ensure_pair_code().to_string();
    let _ = cfg.save(&args.config);
    let _guard = init_logging(cfg.log_level, &cfg.log_dir)?;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        %pair_code,
        mode = ?cfg.network_mode,
        "enet-gateway starting"
    );

    if !args.quiet {
        eprintln!();
        eprintln!("  BMW ENET Gateway");
        eprintln!("  Mode: {}", cfg.network_mode.label());
        eprintln!("  ----------------");
        eprintln!("  Pair code:  {pair_code}");
        eprintln!("  Dashboard:  http://127.0.0.1:{}/", cfg.api_port);
        if cfg.network_mode == NetworkMode::Relay {
            eprintln!("  Relay:      {}", cfg.relay_url);
            eprintln!("  Laptop must use the same relay + pair code.");
        } else {
            eprintln!("  Laptop: auto-detects this PC by pair code (DHCP OK)");
            eprintln!("    Pair {pair_code} · fallback status http://127.0.0.1:47903/");
        }
        eprintln!();
        for hint in cfg.setup_hints() {
            eprintln!("  {hint}");
        }
        eprintln!();
    }

    if cfg.manage_firewall && cfg.network_mode == NetworkMode::Lan {
        info!(
            "firewall: allow UDP {} (tunnel) and UDP {} (discovery) from LAN",
            cfg.tunnel_port, cfg.discovery_port
        );
    }

    // Self-update: clean leftovers, then check GitHub on every start.
    // Auto-install only when auto_update is enabled (safe — nothing connected yet).
    if let Ok(dir) = enet_core::updater::install_dir() {
        enet_core::updater::cleanup_stale(&dir);
        let dir2 = dir.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            enet_core::updater::cleanup_stale(&dir2);
        });
    }
    let mut startup_update: Option<enet_core::updater::UpdateInfo> = None;
    {
        let cfg_upd = cfg.clone();
        let found = tokio::task::spawn_blocking(move || check_update_blocking(&cfg_upd))
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
                startup_update = Some(update);
            }
        } else {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                "startup update check: already up to date"
            );
        }
    }

    // Retry the initial bind — after an update restart the old process may
    // hold UDP 47900 for a second or two while it exits.
    let (handle, l2_active, l2_label) = {
        let mut result = None;
        for attempt in 0..5u32 {
            match start_tunnel(&cfg, &pair_code, args.simulate).await {
                Ok(t) => {
                    result = Some(t);
                    break;
                }
                Err(e) if attempt < 4 => {
                    warn!(error = format!("{e:#}"), attempt, "tunnel start failed — retrying in 2s");
                    eprintln!("  Tunnel start failed: {e:#}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e),
            }
        }
        result.expect("tunnel start loop")
    };

    // LAN beacon only for same-network mode.
    let beacon = if cfg.network_mode == NetworkMode::Lan {
        let discovery_port = cfg.discovery_port;
        let tunnel_port = cfg.tunnel_port;
        let api_port = cfg.api_port;
        let pair_code = pair_code.clone();
        let password_required = !cfg.password.is_empty() || cfg.require_crypto;
        Some(tokio::spawn(async move {
            if let Err(e) = run_gateway_beacon(
                discovery_port,
                tunnel_port,
                api_port,
                pair_code,
                password_required,
                env!("CARGO_PKG_VERSION").into(),
            )
            .await
            {
                warn!(error = %e, "discovery beacon stopped");
            }
        }))
    } else {
        None
    };

    let (tunnel_tx, mut tunnel_rx) = tokio::sync::mpsc::unbounded_channel::<TunnelCmd>();
    let app_state = AppState {
        cfg: Arc::new(RwLock::new(cfg.clone())),
        handle: Arc::new(RwLock::new(Some(handle.clone()))),
        health: Arc::new(RwLock::new(HealthMonitor::new())),
        config_path: args.config.clone(),
        activity: Arc::new(RwLock::new(ActivityLog::new())),
        l2: Arc::new(RwLock::new((l2_active, l2_label.clone()))),
        tunnel_tx,
        update_available: Arc::new(RwLock::new(None)),
    };

    {
        let mut log = app_state.activity.write();
        log.push("info", format!("Gateway started · pair {}", pair_code));
        log.push("info", format!("Network mode: {}", cfg.network_mode.label()));
        if l2_active {
            log.push("info", format!("ISTA bridge ready · {l2_label}"));
        } else {
            log.push("warn", format!("ISTA bridge inactive · {l2_label}"));
        }
        if cfg.network_mode == NetworkMode::Relay {
            log.push("info", format!("Relay: {}", cfg.relay_url));
        }
        if let Some(ref u) = startup_update {
            log.push("info", format!("Update available: v{}", u.version));
        }
    }
    if let Some(u) = startup_update {
        *app_state.update_available.write() = Some(u);
    }

    // Periodic update check (every 6 h). Auto-installs only while no laptop
    // session is active so an update can never interrupt diagnostics/flashing.
    {
        let st = app_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
                let cfg_now = st.cfg.read().clone();
                if !cfg_now.auto_update && st.update_available.read().is_some() {
                    continue;
                }
                let cfg_chk = cfg_now.clone();
                let found = tokio::task::spawn_blocking(move || check_update_blocking(&cfg_chk))
                    .await
                    .unwrap_or(None);
                let Some(update) = found else { continue };
                st.activity
                    .write()
                    .push("info", format!("Update available: v{}", update.version));
                *st.update_available.write() = Some(update.clone());
                let connected = st
                    .handle
                    .read()
                    .as_ref()
                    .map(|h| h.snapshot_state().laptop_connected)
                    .unwrap_or(false);
                if cfg_now.auto_update && !connected {
                    st.activity
                        .write()
                        .push("info", format!("Auto-installing v{} — restarting…", update.version));
                    let token = cfg_now.update_token.clone();
                    let _ = tokio::task::spawn_blocking(move || apply_update_blocking(&update, &token)).await;
                    st.activity.write().push("error", "Update install failed");
                }
            }
        });
    }

    // Tunnel lifecycle supervisor — lets the dashboard Stop / Start / Restart in-process.
    {
        let sup = app_state.clone();
        let sup_pair = pair_code.clone();
        let simulate = args.simulate;
        tokio::spawn(async move {
            while let Some(cmd) = tunnel_rx.recv().await {
                if let Some(h) = sup.handle.write().take() {
                    h.stop();
                    sup.activity.write().push("info", "Tunnel stopped");
                }
                if cmd == TunnelCmd::Stop {
                    continue;
                }
                // Give the old socket a moment to release the port.
                tokio::time::sleep(Duration::from_millis(400)).await;
                let cfg_now = sup.cfg.read().clone();
                match start_tunnel(&cfg_now, &sup_pair, simulate).await {
                    Ok((h, l2a, l2l)) => {
                        *sup.handle.write() = Some(h);
                        *sup.l2.write() = (l2a, l2l.clone());
                        let mut log = sup.activity.write();
                        log.push("info", "Tunnel restarted");
                        if l2a {
                            log.push("info", format!("ISTA bridge ready · {l2l}"));
                        } else {
                            log.push("warn", format!("ISTA bridge inactive · {l2l}"));
                        }
                    }
                    Err(e) => {
                        sup.activity
                            .write()
                            .push("error", format!("Tunnel start failed: {e:#}"));
                    }
                }
            }
        });
    }

    // Derive activity-log lines from status transitions.
    {
        let watch_state = app_state.clone();
        tokio::spawn(async move {
            let mut prev_lap = false;
            let mut prev_link = false;
            let mut prev_awake = false;
            let mut prev_peer = String::new();
            let mut prev_conn = ConnectionState::Starting;
            loop {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let snap = {
                    let guard = watch_state.handle.read();
                    guard.as_ref().map(|h| h.snapshot_state())
                };
                let Some(st) = snap else { continue };
                let mut log = watch_state.activity.write();
                if st.connection != prev_conn {
                    log.push(
                        "info",
                        format!("Tunnel state → {:?}", st.connection),
                    );
                    prev_conn = st.connection;
                }
                let peer = st.peer_endpoint.clone().unwrap_or_default();
                if st.laptop_connected && !prev_lap {
                    log.push(
                        "info",
                        if peer.is_empty() {
                            "Laptop connected".into()
                        } else {
                            format!("Laptop connected · {peer}")
                        },
                    );
                } else if !st.laptop_connected && prev_lap {
                    log.push("warn", "Laptop disconnected");
                }
                if peer != prev_peer && !peer.is_empty() {
                    log.push("info", format!("Peer endpoint · {peer}"));
                    prev_peer = peer;
                }
                if st.vehicle.link_up && !prev_link {
                    let extra = st
                        .vehicle
                        .discovered_ip
                        .as_deref()
                        .unwrap_or("link up");
                    log.push("info", format!("Vehicle ENET · {extra}"));
                } else if !st.vehicle.link_up && prev_link {
                    log.push("warn", "Vehicle ENET link down");
                }
                if st.vehicle.awake && !prev_awake {
                    log.push("info", "Vehicle state → AWAKE");
                } else if !st.vehicle.awake && prev_awake {
                    log.push("info", "Vehicle state → SLEEP");
                }
                prev_lap = st.laptop_connected;
                prev_link = st.vehicle.link_up;
                prev_awake = st.vehicle.awake;
            }
        });
    }

    let api = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/status", get(api_status))
        .route("/api/logs", get(api_logs))
        .route("/api/start", post(api_start))
        .route("/api/stop", post(api_stop))
        .route("/api/restart", post(api_restart))
        .route("/api/settings", get(api_get_settings).post(api_set_settings))
        .route("/api/safety", get(api_safety))
        .route("/api/export-logs", post(api_export_logs))
        .route("/api/complete-setup", post(api_complete_setup))
        .route("/api/update", post(api_update))
        .route("/api/check-update", post(api_check_update))
        .layer(CorsLayer::permissive())
        .with_state(app_state.clone());

    let api_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, cfg.api_port));
    info!(%api_addr, "dashboard + control API listening");
    let server = tokio::spawn(async move {
        // Retry — the previous instance may hold the port briefly after an update.
        let mut listener = None;
        for attempt in 0..10u32 {
            match tokio::net::TcpListener::bind(api_addr).await {
                Ok(l) => {
                    listener = Some(l);
                    break;
                }
                Err(e) => {
                    warn!(error = %e, attempt, "api bind failed — retrying in 1s");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        let Some(listener) = listener else {
            warn!("dashboard API could not bind — running headless");
            return;
        };
        if let Err(e) = axum::serve(listener, api).await {
            warn!(error = %e, "api server stopped");
        }
    });

    if let Some(secs) = args.run_seconds {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        handle.stop();
        if let Some(b) = beacon {
            b.abort();
        }
        server.abort();
        return Ok(());
    }

    tokio::signal::ctrl_c().await.ok();
    info!("shutdown");
    handle.stop();
    if let Some(b) = beacon {
        b.abort();
    }
    server.abort();
    Ok(())
}

async fn dashboard_html() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn api_logs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let entries = state.activity.read().entries.clone();
    Json(serde_json::json!({ "entries": entries }))
}

fn friendly_status(state: &GatewayState) -> String {
    if !state.gateway_running {
        return "Gateway is stopped".into();
    }
    if state.laptop_connected || matches!(state.connection, ConnectionState::Connected) {
        if state.vehicle.awake {
            "Ready — vehicle is awake".into()
        } else if state.vehicle.link_up {
            "Laptop connected — waiting for vehicle wake / ignition".into()
        } else {
            "Laptop connected — plug in ENET and wake the car".into()
        }
    } else {
        match state.connection {
            ConnectionState::WaitingForPeer | ConnectionState::Starting => {
                "Waiting for laptop… open the Agent on the laptop".into()
            }
            ConnectionState::Reconnecting => "Laptop disconnected — reconnecting…".into(),
            ConnectionState::Failed => "Something went wrong — check logs".into(),
            ConnectionState::Stopped => "Stopped".into(),
            ConnectionState::Connected => "Connected".into(),
        }
    }
}

async fn api_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let cfg = state.cfg.read().clone();
    let (mut gateway_state, stats) = {
        let guard = state.handle.read();
        if let Some(h) = guard.as_ref() {
            (h.snapshot_state(), h.stats.snapshot())
        } else {
            (
                GatewayState {
                    connection: ConnectionState::Stopped,
                    gateway_running: false,
                    status_message: "Stopped".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    ..Default::default()
                },
                enet_core::stats::PacketStats::new().snapshot(),
            )
        }
    };
    gateway_state.gateway_running = state.handle.read().is_some();
    // Vehicle indicators require a connected laptop; never show car link from the desktop TAP alone.
    if !gateway_state.laptop_connected
        && !matches!(gateway_state.connection, ConnectionState::Connected)
    {
        gateway_state.vehicle.link_up = false;
        gateway_state.vehicle.awake = false;
    }

    let (cpu_pct, memory_used, memory_total) = {
        let mut health = state.health.write();
        (
            health.cpu_pct(),
            health.memory_used_bytes(),
            health.memory_total_bytes(),
        )
    };

    let checker = FlashSafetyChecker::new(SafetyThresholds::from(&cfg));
    // Tools traffic is desktop→laptop; use the worse of both directions for flash safety.
    let mut stats_for_safety = stats.clone();
    let to_laptop = gateway_state.rtt_local_ms.max(stats.rtt_p99_ms);
    let from_laptop = gateway_state.rtt_peer_ms;
    if to_laptop > stats_for_safety.rtt_p99_ms {
        stats_for_safety.rtt_p99_ms = to_laptop;
    }
    if from_laptop > stats_for_safety.rtt_p99_ms {
        stats_for_safety.rtt_p99_ms = from_laptop;
    }
    let flash_safety = checker.evaluate(
        &stats_for_safety,
        &gateway_state.vehicle,
        cpu_pct,
        gateway_state.laptop_connected
            || matches!(gateway_state.connection, ConnectionState::Connected),
    );

    let friendly = friendly_status(&gateway_state);
    let mut flash_safety = flash_safety;
    if cfg.network_mode.is_remote() && flash_safety.safe {
        flash_safety.warning.push_str(
            " Remote link: prefer WireGuard or same-LAN for ECU flashing whenever possible.",
        );
    } else if cfg.network_mode.is_remote() {
        flash_safety.warning.push_str(
            " You are on a remote path (relay/VPN). Expect higher latency than LAN.",
        );
    }
    let lan_ips = local_lan_ipv4s();
    let primary_ip = lan_ips
        .first()
        .cloned()
        .unwrap_or_else(|| "DESKTOP_IP".into());
    let connect_command = primary_ip.clone();
    let mut setup_hints = cfg.setup_hints();
    if !cfg.network_mode.is_remote()
        && !gateway_state.laptop_connected
        && !matches!(gateway_state.connection, ConnectionState::Connected)
    {
        setup_hints.push(
            "Laptop Client auto-detects this PC by pair code (DHCP / changing IPs OK)."
                .into(),
        );
        if lan_ips.len() > 1 {
            setup_hints.push(format!(
                "This PC LAN IPs (fallback hint only): {}",
                lan_ips.join(", ")
            ));
        }
        setup_hints.push(
            "If Client stays Waiting: open http://127.0.0.1:47903/ → Auto-find desktop. Same router required (not Guest Wi‑Fi)."
                .into(),
        );
    }
    if gateway_state.laptop_connected
        && gateway_state.rtt_local_ms > 40.0
        && gateway_state.rtt_peer_ms > 0.0
        && gateway_state.rtt_local_ms > gateway_state.rtt_peer_ms * 2.0
    {
        setup_hints.push(
            "Desktop→laptop RTT is much higher than laptop→desktop — usually laptop Wi‑Fi power saving. Disable Wi‑Fi power save / use High performance / 5 GHz."
                .into(),
        );
    }
    let rtt_to_laptop = gateway_state.rtt_local_ms.max(stats.rtt_ms);
    let rtt_from_laptop = gateway_state.rtt_peer_ms;
    let (l2_active, l2_adapter) = state.l2.read().clone();
    Json(StatusResponse {
        state: gateway_state,
        stats,
        cpu_pct,
        memory_used,
        memory_total,
        flash_safety,
        pair_code: cfg.pair_code.clone(),
        setup_hints,
        setup_complete: cfg.setup_complete,
        friendly_status: friendly,
        network_mode: format!("{:?}", cfg.network_mode).to_lowercase(),
        network_mode_label: cfg.network_mode.label().into(),
        is_remote: cfg.network_mode.is_remote(),
        relay_url: cfg.relay_url.clone(),
        lan_ips,
        connect_command,
        rtt_to_laptop_ms: rtt_to_laptop,
        rtt_from_laptop_ms: rtt_from_laptop,
        l2_active,
        l2_adapter,
        update_available: state
            .update_available
            .read()
            .as_ref()
            .map(|u| u.version.clone()),
        auto_update: cfg.auto_update,
    })
}

async fn api_check_update(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg_now = state.cfg.read().clone();
    state
        .activity
        .write()
        .push("info", "Checking for updates…");
    let cfg_chk = cfg_now.clone();
    let found = tokio::task::spawn_blocking(move || check_update_blocking(&cfg_chk))
        .await
        .unwrap_or(None);
    match found {
        Some(u) => {
            let version = u.version.clone();
            *state.update_available.write() = Some(u);
            state
                .activity
                .write()
                .push("info", format!("Update available: v{version}"));
            Json(serde_json::json!({
                "ok": true,
                "current": env!("CARGO_PKG_VERSION"),
                "update_available": version,
                "message": format!("Update available: v{version}")
            }))
        }
        None => {
            *state.update_available.write() = None;
            state.activity.write().push(
                "info",
                format!("Already up to date (v{})", env!("CARGO_PKG_VERSION")),
            );
            Json(serde_json::json!({
                "ok": true,
                "current": env!("CARGO_PKG_VERSION"),
                "update_available": serde_json::Value::Null,
                "message": format!("Already up to date (v{})", env!("CARGO_PKG_VERSION"))
            }))
        }
    }
}

async fn api_update(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg_now = state.cfg.read().clone();
    let cached = state.update_available.read().clone();
    let update = match cached {
        Some(u) => Some(u),
        None => {
            let cfg_chk = cfg_now.clone();
            tokio::task::spawn_blocking(move || check_update_blocking(&cfg_chk))
                .await
                .unwrap_or(None)
        }
    };
    match update {
        Some(u) => {
            let version = u.version.clone();
            state
                .activity
                .write()
                .push("info", format!("Installing v{version} — restarting…"));
            let token = cfg_now.update_token.clone();
            tokio::spawn(async move {
                // Let the HTTP response flush before the process restarts.
                tokio::time::sleep(Duration::from_millis(800)).await;
                let _ = tokio::task::spawn_blocking(move || apply_update_blocking(&u, &token)).await;
            });
            Json(serde_json::json!({
                "ok": true,
                "message": format!("Updating to v{version} — this page will reconnect shortly")
            }))
        }
        None => Json(serde_json::json!({
            "ok": true,
            "message": format!("Already up to date (v{})", env!("CARGO_PKG_VERSION"))
        })),
    }
}

/// Best-effort local IPv4 list for dashboard hints (all LAN NICs).
fn local_lan_ipv4s() -> Vec<String> {
    enet_core::list_lan_ipv4s()
        .into_iter()
        .map(|ip| ip.to_string())
        .collect()
}

async fn api_start(State(state): State<AppState>) -> Json<serde_json::Value> {
    if state.handle.read().is_some() {
        return Json(serde_json::json!({"ok": true, "message": "Tunnel already running"}));
    }
    let _ = state.tunnel_tx.send(TunnelCmd::Start);
    Json(serde_json::json!({"ok": true, "message": "Starting tunnel…"}))
}

async fn api_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    if state.handle.read().is_none() {
        return Json(serde_json::json!({"ok": true, "message": "Tunnel already stopped"}));
    }
    let _ = state.tunnel_tx.send(TunnelCmd::Stop);
    Json(serde_json::json!({"ok": true, "message": "Stopping tunnel…"}))
}

async fn api_restart(State(state): State<AppState>) -> Json<serde_json::Value> {
    let _ = state.tunnel_tx.send(TunnelCmd::Restart);
    Json(serde_json::json!({"ok": true, "message": "Restarting tunnel…"}))
}

async fn api_get_settings(State(state): State<AppState>) -> Json<GatewayConfig> {
    Json(state.cfg.read().clone())
}

async fn api_set_settings(
    State(state): State<AppState>,
    Json(update): Json<SettingsUpdate>,
) -> Json<serde_json::Value> {
    let mut cfg = state.cfg.write();
    if let Some(p) = update.tunnel_port {
        cfg.tunnel_port = p;
    }
    if let Some(p) = update.password {
        cfg.password = p;
    }
    if let Some(ms) = update.reconnect_delay_ms {
        cfg.reconnect_delay_ms = ms;
    }
    if let Some(ms) = update.peer_timeout_ms {
        cfg.peer_timeout_ms = ms;
    }
    if let Some(v) = update.require_crypto {
        cfg.require_crypto = v;
    }
    if let Some(v) = update.auto_start {
        cfg.auto_start = v;
    }
    if let Some(v) = update.pair_code {
        cfg.pair_code = v;
    }
    if let Some(v) = update.setup_complete {
        cfg.setup_complete = v;
    }
    if let Some(v) = update.auto_discover {
        cfg.auto_discover = v;
    }
    if let Some(v) = update.auto_update {
        cfg.auto_update = v;
    }
    if let Some(level) = update.log_level {
        cfg.log_level = match level.to_lowercase().as_str() {
            "error" => enet_core::config::LogLevel::Error,
            "warn" => enet_core::config::LogLevel::Warn,
            "debug" => enet_core::config::LogLevel::Debug,
            "trace" => enet_core::config::LogLevel::Trace,
            _ => enet_core::config::LogLevel::Info,
        };
    }
    let _ = cfg.save(&state.config_path);
    drop(cfg);
    state
        .activity
        .write()
        .push("info", "Settings saved — Restart tunnel to apply");
    Json(serde_json::json!({"ok": true, "message": "Saved — click Restart tunnel to apply"}))
}

async fn api_complete_setup(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut cfg = state.cfg.write();
    cfg.setup_complete = true;
    let _ = cfg.save(&state.config_path);
    Json(serde_json::json!({"ok": true}))
}

async fn api_safety(State(state): State<AppState>) -> Json<enet_core::safety::FlashSafetyReport> {
    let status = api_status(State(state)).await;
    Json(status.0.flash_safety)
}

async fn api_export_logs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = state.cfg.read().clone();
    let src = cfg.log_dir.clone();
    let dest = std::env::temp_dir().join(format!("enet-logs-{}.txt", now_secs()));
    let mut bundle = String::new();
    if src.exists() {
        if let Ok(entries) = std::fs::read_dir(&src) {
            for entry in entries.flatten() {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    bundle.push_str(&format!("===== {} =====\n", entry.path().display()));
                    bundle.push_str(&text);
                    bundle.push('\n');
                }
            }
        }
    }
    match std::fs::write(&dest, bundle) {
        Ok(()) => Json(serde_json::json!({"ok": true, "path": dest})),
        Err(e) => Json(serde_json::json!({"ok": false, "error": e.to_string()})),
    }
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
