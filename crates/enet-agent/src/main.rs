//! Laptop ENET agent — LAN auto-discover or remote relay / WireGuard.

use anyhow::Context;
use async_trait::async_trait;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
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
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

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
    enet_name: Arc<RwLock<String>>,
    enet_link: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct StatusJson {
    version: String,
    pair_code: String,
    desktop_connected: bool,
    desktop_peer: String,
    enet_interface: String,
    enet_link: bool,
    vehicle_awake: bool,
    vehicle_link: bool,
    rtt_ms: f64,
    loss_rate: f64,
    friendly: String,
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
    let preferred = cfg.enet_interface.clone();
    if let Some(iface) = pick_enet_interface(preferred.as_str()) {
        info!(name = %iface.name, mac = %iface.mac, "selected ENET candidate interface");
        *enet_name.write() = iface.name.clone();
        let up = adapter_link_up(&iface.name);
        enet_link.store(up, Ordering::Relaxed);
        warn!(
            "raw ENET capture requires Npcap (Windows) or CAP_NET_RAW (Linux); \
             link indicator works now for '{}'. Tunnel to desktop will still connect.",
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

async fn resolve_peer(cfg: &GatewayConfig, args: &Args) -> anyhow::Result<(IpAddr, u16)> {
    if let Some(peer) = args.peer.or(cfg.peer_addr) {
        return Ok((peer, cfg.tunnel_port));
    }
    if matches!(cfg.network_mode, NetworkMode::Wireguard) {
        let ip: IpAddr = cfg
            .wireguard_desktop_ip
            .parse()
            .context("wireguard_desktop_ip invalid")?;
        return Ok((ip, cfg.tunnel_port));
    }
    if !cfg.auto_discover {
        anyhow::bail!(
            "No desktop address configured.\n\
             For different networks use: enet-setup agent --remote-relay HOST:47910\n\
             Or WireGuard: enet-setup wireguard"
        );
    }
    let code = args
        .pair_code
        .clone()
        .unwrap_or_else(|| cfg.pair_code.clone());
    eprintln!("Looking for BMW ENET Gateway on your LAN…");
    if !code.is_empty() {
        eprintln!("  (filtering for pair code {code})");
    }
    let found = discover_gateways(cfg.discovery_port, &code, Duration::from_secs(5)).await?;
    let gw = found.into_iter().next().context(
        "No desktop on this LAN.\n\
         If the desktop is on Ethernet and this laptop is on Wi‑Fi, broadcast discovery usually fails.\n\
         Fix (recommended):\n\
           1) On the desktop, open http://127.0.0.1:47901 and copy the pair code + LAN IP.\n\
           2) Match passwords on both PCs (or clear password on both).\n\
           3) On the laptop (Admin PowerShell):\n\
              Stop-Process -Name enet-agent -Force -ErrorAction SilentlyContinue\n\
              cd C:\\BMW-ENET\\Client\n\
              .\\enet-agent.exe --config config\\agent.toml --pair-code BMW-XXXX --peer DESKTOP_LAN_IP\n\
         Also check: same router (not Guest / client-isolation), Windows Firewall allows UDP 47900/47902.",
    )?;
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
    Ok((gw.addr, gw.tunnel_port))
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
    let (desktop, awake, vehicle_link, rtt_ms, loss_rate) = {
        let guard = live.handle.read();
        if let Some(h) = guard.as_ref() {
            let st = h.snapshot_state();
            let snap = h.stats.snapshot();
            let desk = matches!(st.connection, ConnectionState::Connected) || st.laptop_connected;
            (
                desk,
                st.vehicle.awake,
                st.vehicle.link_up,
                snap.rtt_ms,
                snap.loss_rate,
            )
        } else {
            (false, false, false, 0.0, 0.0)
        }
    };
    // Prefer OS carrier for cable indicator; fall back to tunnel vehicle_link.
    let enet = live.enet_link.load(Ordering::Relaxed) || vehicle_link;
    let desktop_connected = desktop;
    Json(StatusJson {
        version: env!("CARGO_PKG_VERSION").into(),
        pair_code: live.pair_code.read().clone(),
        desktop_connected,
        desktop_peer: live.desktop_peer.read().clone(),
        enet_interface: live.enet_name.read().clone(),
        enet_link: enet,
        vehicle_awake: awake,
        vehicle_link: enet,
        rtt_ms,
        loss_rate,
        friendly: friendly_line(desktop_connected, enet, awake),
    })
}

async fn status_page() -> Html<&'static str> {
    Html(include_str!("status.html"))
}

fn spawn_status_server(live: Arc<LiveStatus>, port: u16) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(status_page))
            .route("/api/status", get(api_status))
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
            tokio::time::sleep(Duration::from_secs(2)).await;
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
            while handle.is_running() {
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
                    break;
                }
            }
        } => {
            warn!("tunnel stopped; will reconnect");
            handle.stop();
        }
    }
    *live.handle.write() = None;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
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
        enet_name: enet_name.clone(),
        enet_link: enet_link.clone(),
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
    eprintln!("  -----------------------");
    for hint in cfg.setup_hints() {
        eprintln!("  {hint}");
    }
    eprintln!();

    let mut attempt = 0u32;
    loop {
        *live.pair_code.write() = cfg.pair_code.clone();
        let eth = match build_ethernet_port(&cfg, args.simulate, enet_name.clone(), enet_link.clone())
            .await
        {
            Ok(e) => e,
            Err(e) => {
                eprintln!("\n{e}\n");
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(backoff_delay(
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
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            if cfg.pair_code.is_empty() {
                eprintln!("pair_code required for relay mode (from desktop dashboard)");
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(Duration::from_secs(2)).await;
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
            match resolve_peer(&cfg, &args).await {
                Ok((peer_ip, tunnel_port)) => {
                    let peer = SocketAddr::new(peer_ip, tunnel_port);
                    *live.desktop_peer.write() = peer.to_string();
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
                                "Connected to desktop at {peer}. Status: http://127.0.0.1:{status_port}/"
                            );
                            Some(h)
                        }
                        Err(e) => {
                            eprintln!("Tunnel failed: {e}");
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("\n{e}\n");
                    None
                }
            }
        };

        if let Some(handle) = started {
            attempt = 0;
            run_until_stop(handle, live.clone()).await;
        }

        attempt = attempt.saturating_add(1);
        let delay = backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
        eprintln!("Reconnecting in {delay:?}…");
        tokio::time::sleep(delay).await;
    }
}
