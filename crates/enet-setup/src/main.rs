//! First-run setup wizard — LAN, relay (different networks), or WireGuard.

use clap::{Parser, Subcommand, ValueEnum};
use enet_core::config::{GatewayConfig, NetworkMode, Role};
use enet_core::discover_gateways;
use enet_core::discovery::detect_candidate_interfaces;
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "enet-setup",
    about = "BMW ENET setup — same network OR different networks"
)]
struct Args {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Configure this PC as the desktop gateway (runs ISTA / E-Sys)
    Gateway {
        #[arg(long, default_value = "config/gateway.toml")]
        config: PathBuf,
        #[arg(long, default_value = "")]
        password: String,
        /// Different networks: relay host:port (both PCs dial out)
        #[arg(long)]
        remote_relay: Option<String>,
        /// Different networks via WireGuard overlay
        #[arg(long)]
        wireguard: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Configure this PC as the laptop agent (ENET cable)
    Agent {
        #[arg(long, default_value = "config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "")]
        pair_code: String,
        #[arg(long)]
        peer: Option<IpAddr>,
        #[arg(long, default_value = "")]
        password: String,
        #[arg(long)]
        remote_relay: Option<String>,
        #[arg(long)]
        wireguard: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Scan the LAN for a running desktop gateway
    Find {
        #[arg(long, default_value = "")]
        pair_code: String,
        #[arg(long, default_value_t = 47902)]
        discovery_port: u16,
    },
    /// Generate WireGuard configs for desktop + laptop
    Wireguard {
        /// Public endpoint for the desktop (or VPS), e.g. 1.2.3.4:51820
        #[arg(long, default_value = "YOUR_PUBLIC_IP:51820")]
        desktop_endpoint: String,
        #[arg(long, default_value = "config")]
        out_dir: PathBuf,
    },
    /// Print a health / readiness checklist
    Doctor {
        #[arg(long, value_enum, default_value = "gateway")]
        role: DoctorRole,
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum DoctorRole {
    Gateway,
    Agent,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    match args.cmd {
        Command::Gateway {
            config,
            password,
            remote_relay,
            wireguard,
            yes,
        } => setup_gateway(config, password, remote_relay, wireguard, yes),
        Command::Agent {
            config,
            pair_code,
            peer,
            password,
            remote_relay,
            wireguard,
            yes,
        } => setup_agent(config, pair_code, peer, password, remote_relay, wireguard, yes).await,
        Command::Find {
            pair_code,
            discovery_port,
        } => find_gateways(pair_code, discovery_port).await,
        Command::Wireguard {
            desktop_endpoint,
            out_dir,
        } => gen_wireguard(desktop_endpoint, out_dir),
        Command::Doctor { role, config } => doctor(role, config),
    }
}

fn setup_gateway(
    config: PathBuf,
    password: String,
    remote_relay: Option<String>,
    wireguard: bool,
    yes: bool,
) -> anyhow::Result<()> {
    banner("Desktop Gateway setup");
    let mut cfg = GatewayConfig::default();
    cfg.role = Role::Gateway;
    cfg.password = password;

    if let Some(relay) = remote_relay {
        println!("Mode: Different networks via RELAY ({relay})");
        println!("Both PCs dial out — no port-forward on home routers.\n");
        cfg.network_mode = NetworkMode::Relay;
        cfg.relay_url = relay;
        cfg.apply_remote_defaults();
        if cfg.password.is_empty() {
            cfg.password = "change-me".into();
            println!("NOTE: set a real password (placeholder written as change-me).");
        }
    } else if wireguard {
        println!("Mode: Different networks via WIREGUARD\n");
        cfg.network_mode = NetworkMode::Wireguard;
        cfg.apply_remote_defaults();
        if cfg.password.is_empty() {
            cfg.password = "change-me".into();
        }
    } else {
        println!("Mode: Same network (LAN auto-discover)\n");
        cfg.network_mode = NetworkMode::Lan;
        cfg.auto_discover = true;
        cfg.require_crypto = !cfg.password.is_empty();
    }

    let code = cfg.ensure_pair_code().to_string();
    if !yes {
        for iface in detect_candidate_interfaces().into_iter().take(6) {
            println!("  • {} ({})", iface.name, iface.mac);
        }
        if !prompt_yes("Write config?", true)? {
            println!("Cancelled.");
            return Ok(());
        }
    }
    cfg.setup_complete = true;
    cfg.save(&config)?;
    println!("\nSaved {}", config.display());
    println!("Pair code: {code}");
    match cfg.network_mode {
        NetworkMode::Relay => {
            println!(
                "\nNext:\n  1. On a VPS: enet-relay --listen 0.0.0.0:47910\n  \
                 2. Here: enet-gateway --config {}\n  \
                 3. Laptop: enet-setup agent --remote-relay {} --pair-code {code} --yes\n  \
                 See docs/REMOTE.md\n",
                config.display(),
                cfg.relay_url
            );
        }
        NetworkMode::Wireguard => {
            println!(
                "\nNext:\n  1. enet-setup wireguard --desktop-endpoint YOUR_IP:51820\n  \
                 2. Import WireGuard confs on both PCs\n  \
                 3. enet-gateway --config {}\n",
                config.display()
            );
        }
        NetworkMode::Lan => {
            println!(
                "\nNext:\n  1. enet-gateway --config {}\n  \
                 2. Open http://127.0.0.1:{}/\n  \
                 3. Laptop: enet-setup agent && enet-agent\n",
                config.display(),
                cfg.api_port
            );
        }
    }
    Ok(())
}

async fn setup_agent(
    config: PathBuf,
    pair_code: String,
    peer: Option<IpAddr>,
    password: String,
    remote_relay: Option<String>,
    wireguard: bool,
    yes: bool,
) -> anyhow::Result<()> {
    banner("Laptop Agent setup");
    let mut cfg = GatewayConfig::default();
    cfg.role = Role::Agent;
    cfg.password = password;
    cfg.pair_code = pair_code;
    cfg.peer_addr = peer;

    if let Some(relay) = remote_relay {
        println!("Mode: Different networks via RELAY ({relay})\n");
        cfg.network_mode = NetworkMode::Relay;
        cfg.relay_url = relay;
        cfg.apply_remote_defaults();
        if cfg.password.is_empty() {
            cfg.password = "change-me".into();
        }
    } else if wireguard {
        println!("Mode: WireGuard\n");
        cfg.network_mode = NetworkMode::Wireguard;
        cfg.apply_remote_defaults();
        if cfg.password.is_empty() {
            cfg.password = "change-me".into();
        }
    } else {
        println!("Mode: Same network (auto-discover)\n");
        cfg.network_mode = NetworkMode::Lan;
        cfg.auto_discover = peer.is_none();
        cfg.require_crypto = !cfg.password.is_empty();
    }

    if cfg.pair_code.is_empty() && !yes {
        print!("Pair code from desktop (Enter to skip/auto): ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        cfg.pair_code = line.trim().to_string();
    }

    if cfg.network_mode == NetworkMode::Lan && cfg.auto_discover {
        println!("Searching LAN…");
        match discover_gateways(cfg.discovery_port, &cfg.pair_code, Duration::from_secs(3)).await {
            Ok(found) if !found.is_empty() => {
                let g = &found[0];
                println!("✓ Found {} at {}", g.hostname, g.addr);
                if cfg.pair_code.is_empty() {
                    cfg.pair_code = g.pair_code.clone();
                }
            }
            Ok(_) => println!("✗ None yet (start desktop first, or use --remote-relay)."),
            Err(e) => println!("Discovery error: {e}"),
        }
    }

    if !yes && !prompt_yes("Save laptop config?", true)? {
        println!("Cancelled.");
        return Ok(());
    }
    cfg.setup_complete = true;
    cfg.save(&config)?;
    println!("\nSaved {}", config.display());
    println!(
        "\nNext:\n  1. Plug ENET into car + laptop\n  2. enet-agent --config {}\n",
        config.display()
    );
    Ok(())
}

fn gen_wireguard(desktop_endpoint: String, out_dir: PathBuf) -> anyhow::Result<()> {
    banner("WireGuard config generator");
    std::fs::create_dir_all(&out_dir)?;
    // Demo keys — REPLACE before production use. Generated as placeholders with instructions.
    let desktop_priv = "REPLACE_DESKTOP_PRIVATE_KEY________________=";
    let desktop_pub = "REPLACE_DESKTOP_PUBLIC_KEY_________________=";
    let laptop_priv = "REPLACE_LAPTOP_PRIVATE_KEY_________________=";
    let laptop_pub = "REPLACE_LAPTOP_PUBLIC_KEY__________________=";

    let desktop = format!(
        r#"# BMW ENET — Desktop WireGuard
# 1) Install WireGuard  2) Generate real keys: wg genkey | tee private.key | wg pubkey
# 3) Replace REPLACE_* keys below  4) Activate this tunnel  5) Start enet-gateway

[Interface]
PrivateKey = {desktop_priv}
Address = 10.66.0.1/24
ListenPort = 51820

[Peer]
PublicKey = {laptop_pub}
AllowedIPs = 10.66.0.2/32
"#
    );
    let laptop = format!(
        r#"# BMW ENET — Laptop WireGuard
# Import into WireGuard app, activate, then start enet-agent (network_mode=wireguard)

[Interface]
PrivateKey = {laptop_priv}
Address = 10.66.0.2/24

[Peer]
PublicKey = {desktop_pub}
Endpoint = {desktop_endpoint}
AllowedIPs = 10.66.0.1/32
PersistentKeepalive = 25
"#
    );
    let dpath = out_dir.join("wireguard-desktop.conf");
    let lpath = out_dir.join("wireguard-laptop.conf");
    std::fs::write(&dpath, desktop)?;
    std::fs::write(&lpath, laptop)?;
    println!("Wrote {}", dpath.display());
    println!("Wrote {}", lpath.display());
    println!(
        "\nIMPORTANT: Replace placeholder keys with real `wg genkey` / `wg pubkey` output.\n\
         Then:\n  enet-setup gateway --wireguard --yes\n  enet-setup agent --wireguard --yes\n\
         See docs/REMOTE.md\n"
    );
    Ok(())
}

async fn find_gateways(pair_code: String, discovery_port: u16) -> anyhow::Result<()> {
    banner("Find desktop gateway (LAN only)");
    let found = discover_gateways(discovery_port, &pair_code, Duration::from_secs(4)).await?;
    if found.is_empty() {
        println!("No LAN gateways found.");
        println!("Different network? Use: enet-setup gateway --remote-relay HOST:47910");
        return Ok(());
    }
    for g in found {
        println!(
            "• {} — {}  tunnel:{}  code:{}",
            g.hostname, g.addr, g.tunnel_port, g.pair_code
        );
    }
    Ok(())
}

fn doctor(role: DoctorRole, config: Option<PathBuf>) -> anyhow::Result<()> {
    banner("Doctor");
    let path = config.unwrap_or_else(|| match role {
        DoctorRole::Gateway => GatewayConfig::default_path_for(Role::Gateway),
        DoctorRole::Agent => GatewayConfig::default_path_for(Role::Agent),
    });
    let cfg = GatewayConfig::load(&path).unwrap_or_default();
    println!("Config: {}", path.display());
    println!("Mode:   {}", cfg.network_mode.label());
    println!("Pair:   {}", if cfg.pair_code.is_empty() { "(none)" } else { &cfg.pair_code });
    println!("Relay:  {}", if cfg.relay_url.is_empty() { "(n/a)" } else { &cfg.relay_url });
    println!("Crypto: {}", cfg.require_crypto);
    println!();
    for hint in cfg.setup_hints() {
        println!("{hint}");
    }
    if cfg.network_mode.is_remote() {
        println!("\nRemote tip: docs/REMOTE.md — prefer WireGuard for flashing.");
    }
    Ok(())
}

fn banner(title: &str) {
    println!();
    println!("══ {title} ══");
    println!();
}

fn prompt_yes(question: &str, default_yes: bool) -> anyhow::Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let t = line.trim().to_lowercase();
    if t.is_empty() {
        return Ok(default_yes);
    }
    Ok(t == "y" || t == "yes")
}
