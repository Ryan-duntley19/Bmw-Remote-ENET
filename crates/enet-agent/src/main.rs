//! Laptop ENET agent — auto-discovers the desktop gateway and tunnels ENET frames.

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use clap::Parser;
use enet_core::config::{GatewayConfig, Role};
use enet_core::discover_gateways;
use enet_core::discovery::{detect_candidate_interfaces, pick_enet_interface};
use enet_core::logging::init_logging;
use enet_core::stats::backoff_delay;
use enet_tunnel::{EthernetPort, SimulatedEthernet, TunnelEngine, TunnelOptions};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "enet-agent",
    about = "BMW ENET laptop agent — finds your desktop automatically"
)]
struct Args {
    /// Path to agent.toml
    #[arg(short, long, default_value = "config/agent.toml")]
    config: PathBuf,
    /// Override gateway peer host (skips discovery)
    #[arg(long)]
    peer: Option<IpAddr>,
    /// Pair code shown on the desktop dashboard (optional filter)
    #[arg(long)]
    pair_code: Option<String>,
    /// Run with simulated ENET (no hardware)
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
             running in monitor-only mode for interface '{}'. Use --simulate for lab tests.",
            iface.name
        );
        return Ok(Arc::new(NullEthernet { name: iface.name }));
    }

    let all = detect_candidate_interfaces();
    warn!(count = all.len(), "no strong ENET candidate; listing interfaces");
    for i in &all {
        info!(name = %i.name, mac = %i.mac, up = i.is_up, "iface");
    }
    anyhow::bail!("no ENET interface detected; pass --simulate or set enet_interface in config")
}

async fn resolve_peer(cfg: &GatewayConfig, args: &Args) -> anyhow::Result<(IpAddr, u16)> {
    if let Some(peer) = args.peer.or(cfg.peer_addr) {
        return Ok((peer, cfg.tunnel_port));
    }
    if !cfg.auto_discover {
        anyhow::bail!(
            "No desktop IP configured.\n\n\
             Easy fix: enable auto_discover (default) OR run:\n  \
             enet-setup agent\n  \
             enet-agent --peer <desktop-ip>"
        );
    }

    let code = args
        .pair_code
        .clone()
        .unwrap_or_else(|| cfg.pair_code.clone());
    eprintln!("Looking for BMW ENET Gateway on your network…");
    if !code.is_empty() {
        eprintln!("Using pair code filter: {code}");
    } else {
        eprintln!("(No pair code set — will accept the first gateway found)");
    }

    let found = discover_gateways(cfg.discovery_port, &code, Duration::from_secs(3)).await?;
    let gw = found.into_iter().next().context(
        "No desktop gateway found.\n\n\
         Check:\n  • Desktop gateway is running\n  \
         • Both PCs are on the same Wi‑Fi/Ethernet\n  \
         • Pair code matches (see desktop dashboard)\n  \
         • Firewall allows UDP 47902",
    )?;

    eprintln!(
        "Found desktop “{}” at {} (tunnel port {})",
        gw.hostname, gw.addr, gw.tunnel_port
    );
    Ok((gw.addr, gw.tunnel_port))
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

    let _guard = init_logging(cfg.log_level, &cfg.log_dir)?;
    info!(version = env!("CARGO_PKG_VERSION"), "enet-agent starting");
    eprintln!();
    eprintln!("  BMW ENET Agent (laptop)");
    eprintln!("  -----------------------");
    for hint in cfg.setup_hints() {
        eprintln!("  {hint}");
    }
    eprintln!();

    let mut attempt = 0u32;
    loop {
        let (peer_ip, tunnel_port) = match resolve_peer(&cfg, &args).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "peer resolve failed");
                eprintln!("\n{e}\n");
                attempt = attempt.saturating_add(1);
                let delay =
                    backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
                tokio::time::sleep(delay).await;
                continue;
            }
        };
        let peer = SocketAddr::new(peer_ip, tunnel_port);

        let eth = match build_ethernet_port(&cfg, args.simulate).await {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "ethernet setup failed");
                eprintln!("\n{e}\n");
                attempt = attempt.saturating_add(1);
                let delay =
                    backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let bind = SocketAddr::from((
            cfg.bind_addr.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            0,
        ));
        let opts = TunnelOptions {
            bind,
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
            Ok(handle) => {
                info!(%peer, "agent tunnel running");
                eprintln!("Connected to desktop at {peer}. Leave this window open.");
                attempt = 0;
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        info!("shutdown requested");
                        handle.stop();
                        break;
                    }
                    _ = async {
                        while handle.is_running() {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let st = handle.snapshot_state();
                            if matches!(
                                st.connection,
                                enet_core::state::ConnectionState::Failed
                            ) {
                                break;
                            }
                        }
                    } => {
                        warn!("tunnel stopped; will reconnect");
                        handle.stop();
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to start tunnel");
                eprintln!("Could not start tunnel: {e}");
            }
        }

        attempt = attempt.saturating_add(1);
        let delay = backoff_delay(cfg.reconnect_delay_ms, cfg.reconnect_delay_max_ms, attempt);
        info!(?delay, attempt, "reconnecting");
        eprintln!("Reconnecting in {:?}…", delay);
        tokio::time::sleep(delay).await;
    }

    Ok(())
}
