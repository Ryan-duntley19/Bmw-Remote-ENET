//! Desktop ENET gateway — Windows service-compatible tunnel server + friendly dashboard.

use anyhow::Context;
use async_trait::async_trait;
use axum::extract::State;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
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
    /// Use simulated TAP (no Wintun)
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

struct VirtualNic {
    name: String,
    inner: Arc<SimulatedEthernet>,
}

#[async_trait]
impl EthernetPort for VirtualNic {
    fn name(&self) -> &str {
        &self.name
    }
    async fn link_up(&self) -> bool {
        self.inner.link_up().await
    }
    async fn recv(&self) -> anyhow::Result<Bytes> {
        self.inner.recv().await
    }
    async fn send(&self, frame: Bytes) -> anyhow::Result<()> {
        self.inner.send(frame).await
    }
}

#[derive(Clone)]
struct AppState {
    cfg: Arc<RwLock<GatewayConfig>>,
    handle: Arc<RwLock<Option<TunnelHandle>>>,
    health: Arc<RwLock<HealthMonitor>>,
    config_path: PathBuf,
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
            eprintln!("  Laptop: install Agent → auto-finds this PC on the LAN.");
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

    let (tap, _tool_peer) = SimulatedEthernet::pair(&cfg.virtual_interface, "tool-stack");
    tap.set_link(true);
    std::mem::forget(_tool_peer);

    let eth: Arc<dyn EthernetPort> = Arc::new(VirtualNic {
        name: cfg.virtual_interface.clone(),
        inner: tap,
    });

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
            pair_code: pair_code.clone(),
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

    let app_state = AppState {
        cfg: Arc::new(RwLock::new(cfg.clone())),
        handle: Arc::new(RwLock::new(Some(handle.clone()))),
        health: Arc::new(RwLock::new(HealthMonitor::new())),
        config_path: args.config.clone(),
    };

    let api = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/status", get(api_status))
        .route("/api/start", post(api_start))
        .route("/api/stop", post(api_stop))
        .route("/api/restart", post(api_restart))
        .route("/api/settings", get(api_get_settings).post(api_set_settings))
        .route("/api/safety", get(api_safety))
        .route("/api/export-logs", post(api_export_logs))
        .route("/api/complete-setup", post(api_complete_setup))
        .layer(CorsLayer::permissive())
        .with_state(app_state.clone());

    let api_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, cfg.api_port));
    info!(%api_addr, "dashboard + control API listening");
    let server = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(api_addr).await.expect("api bind");
        axum::serve(listener, api).await.expect("api serve");
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

async fn dashboard_html(State(state): State<AppState>) -> Html<String> {
    let status = api_status(State(state)).await.0;
    let connected = matches!(status.state.connection, ConnectionState::Connected)
        || status.state.laptop_connected;
    let vehicle = status.state.vehicle.link_up;
    let awake = status.state.vehicle.awake;
    let safe = status.flash_safety.safe;
    let pair = html_escape(&status.pair_code);
    let msg = html_escape(&status.friendly_status);
    let mode = html_escape(&status.network_mode_label);
    let relay = if status.relay_url.is_empty() {
        String::new()
    } else {
        format!(" · relay {}", html_escape(&status.relay_url))
    };
    let hints: String = status
        .setup_hints
        .iter()
        .map(|h| format!("<li>{}</li>", html_escape(h)))
        .collect();

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<meta http-equiv="refresh" content="3"/>
<title>BMW ENET Gateway</title>
<style>
  :root {{
    --bg: #12161c; --card: #1a222c; --text: #e6eaf0; --muted: #8fa0b0;
    --ok: #3cb478; --warn: #dc783c; --accent: #008ca0;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; font-family: "Segoe UI", system-ui, sans-serif;
    background: radial-gradient(1200px 600px at 10% -10%, #1c3038, var(--bg));
    color: var(--text); min-height: 100vh; padding: 2rem;
  }}
  h1 {{ font-weight: 650; letter-spacing: 0.02em; margin: 0 0 0.25rem; }}
  .sub {{ color: var(--muted); margin-bottom: 1.5rem; }}
  .grid {{ display: grid; gap: 1rem; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); }}
  .pill {{
    background: var(--card); border-radius: 12px; padding: 1rem 1.1rem;
    border: 1px solid #2a3542;
  }}
  .dot {{ display:inline-block; width:0.7rem; height:0.7rem; border-radius:50%; margin-right:0.5rem; }}
  .on {{ background: var(--ok); }} .off {{ background: #5a6570; }}
  .pair {{
    font-size: 2rem; font-weight: 700; letter-spacing: 0.12em;
    color: var(--accent); margin: 0.4rem 0 1rem;
  }}
  .hint {{ background: var(--card); border-radius: 12px; padding: 1.2rem 1.4rem; border: 1px solid #2a3542; }}
  .hint ol {{ margin: 0.4rem 0 0 1.1rem; color: var(--muted); }}
  .safe-ok {{ color: var(--ok); }} .safe-no {{ color: var(--warn); }}
  a {{ color: var(--accent); }}
  .actions button {{
    background: var(--accent); color: white; border: 0; border-radius: 8px;
    padding: 0.55rem 1rem; margin-right: 0.5rem; cursor: pointer; font-weight: 600;
  }}
  .actions button.secondary {{ background: #2a3542; }}
</style>
</head>
<body>
  <h1>BMW ENET Gateway</h1>
  <div class="sub">F-Series remote diagnostics · {msg}</div>

  <div class="pair">Pair code: {pair}</div>
  <p class="sub">Network mode: {mode}{relay}</p>

  <div class="grid">
    <div class="pill"><span class="dot {gw_cls}"></span>Gateway running</div>
    <div class="pill"><span class="dot {lap_cls}"></span>Laptop connected</div>
    <div class="pill"><span class="dot {veh_cls}"></span>Vehicle link</div>
    <div class="pill"><span class="dot {awk_cls}"></span>Vehicle awake</div>
  </div>

  <p style="margin-top:1.25rem">
    Flash safety:
    <strong class="{safe_cls}">{safe_txt}</strong>
  </p>
  <p class="sub">RTT p99 {rtt:.1} ms · Loss {loss:.3}% · CPU {cpu:.0}%</p>

  <div class="hint">
    <strong>Get connected in 5 steps</strong>
    <ol>{hints}</ol>
  </div>

  <p class="sub" style="margin-top:1.5rem">
    Different networks? Use a relay or WireGuard — see docs/REMOTE.md.
  </p>
  <div class="actions">
    <button onclick="fetch('/api/complete-setup',{{method:'POST'}})">Mark setup complete</button>
    <button class="secondary" onclick="fetch('/api/export-logs',{{method:'POST'}}).then(r=>r.json()).then(j=>alert(j.path||j.error||'done'))">Export logs</button>
  </div>
</body>
</html>"#,
        gw_cls = if status.state.gateway_running { "on" } else { "off" },
        lap_cls = if connected { "on" } else { "off" },
        veh_cls = if vehicle { "on" } else { "off" },
        awk_cls = if awake { "on" } else { "off" },
        safe_cls = if safe { "safe-ok" } else { "safe-no" },
        safe_txt = if safe { "OK to consider flashing" } else { "Not safe — do not flash yet" },
        rtt = status.stats.rtt_p99_ms,
        loss = status.stats.loss_rate * 100.0,
        cpu = status.cpu_pct,
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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

    let (cpu_pct, memory_used, memory_total) = {
        let mut health = state.health.write();
        (
            health.cpu_pct(),
            health.memory_used_bytes(),
            health.memory_total_bytes(),
        )
    };

    let checker = FlashSafetyChecker::new(SafetyThresholds::from(&cfg));
    let flash_safety = checker.evaluate(
        &stats,
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
    Json(StatusResponse {
        state: gateway_state,
        stats,
        cpu_pct,
        memory_used,
        memory_total,
        flash_safety,
        pair_code: cfg.pair_code.clone(),
        setup_hints: cfg.setup_hints(),
        setup_complete: cfg.setup_complete,
        friendly_status: friendly,
        network_mode: format!("{:?}", cfg.network_mode).to_lowercase(),
        network_mode_label: cfg.network_mode.label().into(),
        is_remote: cfg.network_mode.is_remote(),
        relay_url: cfg.relay_url.clone(),
    })
}

async fn api_start(State(state): State<AppState>) -> Json<serde_json::Value> {
    if state.handle.read().is_some() {
        return Json(serde_json::json!({"ok": true, "message": "already running"}));
    }
    Json(serde_json::json!({
        "ok": false,
        "message": "Restart the BMW ENET Gateway app/service to start again"
    }))
}

async fn api_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    if let Some(h) = state.handle.write().take() {
        h.stop();
        Json(serde_json::json!({"ok": true}))
    } else {
        Json(serde_json::json!({"ok": true, "message": "already stopped"}))
    }
}

async fn api_restart(State(state): State<AppState>) -> Json<serde_json::Value> {
    if let Some(h) = state.handle.read().as_ref() {
        h.stop();
    }
    Json(serde_json::json!({
        "ok": true,
        "message": "Stop signaled — if installed as a service, Windows will restart it"
    }))
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
    Json(serde_json::json!({"ok": true}))
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
