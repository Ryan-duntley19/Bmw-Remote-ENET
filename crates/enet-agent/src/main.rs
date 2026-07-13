//! Laptop ENET agent — LAN auto-discover or remote relay / WireGuard.

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use clap::Parser;
use enet_core::config::{GatewayConfig, NetworkMode, Role};
use enet_core::discover_gateways;
use enet_core::discovery::{detect_candidate_interfaces, pick_enet_interface};
use enet_core::logging::init_logging;
use enet_core::stats::backoff_delay;
use enet_tunnel::{
    EthernetPort, RelayTunnelEngine, RelayTunnelOptions, SimulatedEthernet, TunnelEngine,
    TunnelHandle, TunnelOptions,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
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
}

struct NullEthernet {
    name: String,
}

#[async_trait]
impl EthernetPort for NullEthernet {
    fn name(&self) -> &str {
        &self.name
    }
    async fn link_up(&self) -> bool {
        false
    }
    async fn recv(&self) -> anyhow::Result<Bytes> {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Err(anyhow::anyhow!("null ethernet closed"))
    }
    async fn send(&self, _frame: Bytes) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn build_ethernet_port(cfg: &GatewayConfig, simulate: bool) -> anyhow::Result<Arc<dyn EthernetPort>> {
    if simulate {
        let (port, _peer) = SimulatedEthernet::pair("sim-enet", "sim-car");
        info!("using simulated ENET interface");
        std::mem::forget(_peer);
        return Ok(port);
    }
    let preferred = cfg.enet_interface.as_str();
    if let Some(iface) = pick_enet_interface(preferred) {
        info!(name = %iface.name, mac = %iface.mac, "selected ENET candidate interface");
        warn!(
            "raw ENET capture requires Npcap (Windows) or CAP_NET_RAW (Linux); \
             monitor-only for '{}'. Use --simulate for lab tests.",
            iface.name
        );
        return Ok(Arc::new(NullEthernet { name: iface.name }));
    }
    let all = detect_candidate_interfaces();
    warn!(count = all.len(), "no strong ENET candidate");
    for i in &all {
        info!(name = %i.name, mac = %i.mac, up = i.is_up, "iface");
    }
    anyhow::bail!("no ENET interface detected; pass --simulate or set enet_interface")
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
    let found = discover_gateways(cfg.discovery_port, &code, Duration::from_secs(3)).await?;
    let gw = found.into_iter().next().context(
        "No desktop on this LAN.\n\
         If you're on a different network, set up Relay or WireGuard — see docs/REMOTE.md",
    )?;
    eprintln!(
        "Found desktop “{}” at {} (tunnel {})",
        gw.hostname, gw.addr, gw.tunnel_port
    );
    Ok((gw.addr, gw.tunnel_port))
}

async fn run_until_stop(handle: TunnelHandle) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown requested");
            handle.stop();
        }
        _ = async {
            while handle.is_running() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let st = handle.snapshot_state();
                if matches!(st.connection, enet_core::state::ConnectionState::Failed) {
                    break;
                }
            }
        } => {
            warn!("tunnel stopped; will reconnect");
            handle.stop();
        }
    }
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

    let _guard = init_logging(cfg.log_level, &cfg.log_dir)?;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = ?cfg.network_mode,
        "enet-agent starting"
    );
    eprintln!();
    eprintln!("  BMW ENET Agent (laptop)");
    eprintln!("  Mode: {}", cfg.network_mode.label());
    eprintln!("  -----------------------");
    for hint in cfg.setup_hints() {
        eprintln!("  {hint}");
    }
    eprintln!();

    let mut attempt = 0u32;
    loop {
        let eth = match build_ethernet_port(&cfg, args.simulate).await {
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
                keepalive_interval_ms: cfg.keepalive_interval_ms,
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
                    eprintln!("Joined relay. Leave this window open.");
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
                    let opts = TunnelOptions {
                        bind: SocketAddr::from((
                            cfg.bind_addr.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
                            0,
                        )),
                        peer: Some(peer),
                        allowed_cidrs: cfg.allowed_cidrs.clone(),
                        crypto: None,
                        require_crypto: cfg.require_crypto,
                        keepalive_interval_ms: cfg.keepalive_interval_ms,
                        peer_timeout_ms: cfg.peer_timeout_ms,
                        role: "agent".into(),
                        version: env!("CARGO_PKG_VERSION").into(),
                    }
                    .with_password(&cfg.password, cfg.require_crypto);
                    match TunnelEngine::new(opts, eth).run().await {
                        Ok(h) => {
                            eprintln!("Connected to desktop at {peer}. Leave this window open.");
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
            run_until_stop(handle).await;
        }

        attempt = attempt.saturating_add(1);
        let delay = backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
        eprintln!("Reconnecting in {delay:?}…");
        tokio::time::sleep(delay).await;
    }
}
