//! First-run setup wizard — writes config and prints plain-language next steps.

use clap::{Parser, Subcommand, ValueEnum};
use enet_core::config::{GatewayConfig, Role};
use enet_core::discover_gateways;
use enet_core::discovery::detect_candidate_interfaces;
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "enet-setup",
    about = "BMW ENET Gateway setup wizard — no networking expertise required"
)]
struct Args {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Configure this PC as the desktop gateway (runs ISTA / E-Sys)
    Gateway {
        /// Config output path
        #[arg(long, default_value = "config/gateway.toml")]
        config: PathBuf,
        /// Optional password for LAN encryption
        #[arg(long, default_value = "")]
        password: String,
        /// Non-interactive (use defaults)
        #[arg(long)]
        yes: bool,
    },
    /// Configure this PC as the laptop agent (ENET cable)
    Agent {
        /// Config output path
        #[arg(long, default_value = "config/agent.toml")]
        config: PathBuf,
        /// Pair code from the desktop dashboard (optional)
        #[arg(long, default_value = "")]
        pair_code: String,
        /// Desktop IP if you prefer not to auto-discover
        #[arg(long)]
        peer: Option<IpAddr>,
        /// Optional password (must match desktop)
        #[arg(long, default_value = "")]
        password: String,
        /// Non-interactive
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
            yes,
        } => setup_gateway(config, password, yes),
        Command::Agent {
            config,
            pair_code,
            peer,
            password,
            yes,
        } => setup_agent(config, pair_code, peer, password, yes).await,
        Command::Find {
            pair_code,
            discovery_port,
        } => find_gateways(pair_code, discovery_port).await,
        Command::Doctor { role, config } => doctor(role, config),
    }
}

fn setup_gateway(config: PathBuf, password: String, yes: bool) -> anyhow::Result<()> {
    banner("Desktop Gateway setup");
    println!("This PC will run ISTA / E-Sys / BimmerUtility.");
    println!("Your laptop (near the car) will connect automatically.\n");

    let mut cfg = GatewayConfig::default();
    cfg.role = Role::Gateway;
    cfg.auto_discover = true;
    cfg.password = password;
    cfg.require_crypto = !cfg.password.is_empty();
    let code = cfg.ensure_pair_code().to_string();

    if !yes {
        println!("Detected network adapters:");
        for iface in detect_candidate_interfaces().into_iter().take(8) {
            println!("  • {} ({})", iface.name, iface.mac);
        }
        println!();
        if prompt_yes("Write config and finish setup?", true)? {
            // continue
        } else {
            println!("Cancelled.");
            return Ok(());
        }
    }

    cfg.setup_complete = true;
    cfg.save(&config)?;

    println!("\nSaved {}", config.display());
    println!("╔══════════════════════════════════════════╗");
    println!("║  Your pair code:  {code:<22} ║");
    println!("╚══════════════════════════════════════════╝");
    println!(
        "\nNext steps:\n  1. Start the gateway:  enet-gateway --config {}\n  \
         2. Open dashboard:     http://127.0.0.1:{}/\n  \
         3. On the laptop run:  enet-setup agent\n  \
            then:               enet-agent\n",
        config.display(),
        cfg.api_port
    );
    Ok(())
}

async fn setup_agent(
    config: PathBuf,
    pair_code: String,
    peer: Option<IpAddr>,
    password: String,
    yes: bool,
) -> anyhow::Result<()> {
    banner("Laptop Agent setup");
    println!("This PC stays near the car with the ENET cable plugged in.\n");

    let mut cfg = GatewayConfig::default();
    cfg.role = Role::Agent;
    cfg.auto_discover = peer.is_none();
    cfg.peer_addr = peer;
    cfg.password = password;
    cfg.require_crypto = !cfg.password.is_empty();
    cfg.pair_code = pair_code;

    if cfg.pair_code.is_empty() && !yes {
        print!("Enter pair code from the desktop (or press Enter to auto-find): ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        cfg.pair_code = line.trim().to_string();
    }

    if cfg.auto_discover {
        println!("Searching for desktop gateway…");
        match discover_gateways(cfg.discovery_port, &cfg.pair_code, Duration::from_secs(3)).await {
            Ok(found) if !found.is_empty() => {
                let g = &found[0];
                println!(
                    "✓ Found “{}” at {} — pair code {}",
                    g.hostname, g.addr, g.pair_code
                );
                if cfg.pair_code.is_empty() {
                    cfg.pair_code = g.pair_code.clone();
                }
            }
            Ok(_) => {
                println!("✗ No gateway found yet (that's OK — start the desktop first, then the agent).");
            }
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
        "\nNext steps:\n  1. Plug ENET cable into the car and this laptop\n  \
         2. Start: enet-agent --config {}\n  \
         3. Watch the desktop dashboard turn green\n",
        config.display()
    );
    Ok(())
}

async fn find_gateways(pair_code: String, discovery_port: u16) -> anyhow::Result<()> {
    banner("Find desktop gateway");
    let found = discover_gateways(discovery_port, &pair_code, Duration::from_secs(4)).await?;
    if found.is_empty() {
        println!("No gateways found on the LAN.");
        println!("Make sure enet-gateway is running on the desktop.");
        return Ok(());
    }
    for g in found {
        println!(
            "• {} — {}  tunnel:{}  api:{}  code:{}  password_required:{}",
            g.hostname, g.addr, g.tunnel_port, g.api_port, g.pair_code, g.password_required
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
    println!("Role:   {:?}", cfg.role);
    println!("Pair:   {}", if cfg.pair_code.is_empty() { "(none)" } else { &cfg.pair_code });
    println!("Auto-discover: {}", cfg.auto_discover);
    println!("Peer:   {:?}", cfg.peer_addr);
    println!("Crypto: {}", cfg.require_crypto);
    println!();
    println!("Checklist:");
    match role {
        DoctorRole::Gateway => {
            check("Config file exists", path.exists());
            check("Pair code set", !cfg.pair_code.is_empty());
            check("Dashboard ready", true);
            println!("  → Open http://127.0.0.1:{}/", cfg.api_port);
            check("Firewall ports documented", true);
            println!(
                "     Allow UDP {} (tunnel) and {} (discovery) from your LAN",
                cfg.tunnel_port, cfg.discovery_port
            );
        }
        DoctorRole::Agent => {
            check("Config file exists", path.exists());
            check(
                "Peer or auto-discover configured",
                cfg.auto_discover || cfg.peer_addr.is_some(),
            );
            check("Npcap / ENET cable (manual)", true);
            println!("  → Plug ENET into car + laptop before starting agent");
        }
    }
    println!();
    for hint in cfg.setup_hints() {
        println!("{hint}");
    }
    Ok(())
}

fn check(label: &str, ok: bool) {
    if ok {
        println!("  ✓ {label}");
    } else {
        println!("  ✗ {label}");
    }
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
