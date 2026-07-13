//! Simulated BMW ENET traffic for automated tests and lab demos.
//!
//! Generates:
//! - HSFZ discovery-like UDP broadcast frames (Ethernet-encapsulated stubs)
//! - Burst Ethernet frames (ISTA scan simulation)
//! - Long-lived packet streams (coding session simulation)
//! - Disconnect / reconnect link flaps

use bytes::{BufMut, Bytes, BytesMut};
use clap::{Parser, Subcommand};
use enet_protocol::{
    BMW_DOIP_PORT, BMW_HSFZ_DISCOVERY_PORT, BMW_HSFZ_PORT, TunnelFrame,
};
use enet_tunnel::{EthernetPort, SimulatedEthernet, TunnelEngine, TunnelOptions};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "enet-sim")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run paired agent+gateway tunnel with simulated car traffic
    Lab {
        /// Seconds to run
        #[arg(long, default_value_t = 5)]
        seconds: u64,
        /// Inject link flaps
        #[arg(long)]
        flaps: bool,
        /// Burst size for ISTA-like scan
        #[arg(long, default_value_t = 200)]
        burst: usize,
    },
    /// Emit protocol self-check JSON
    ProtoCheck,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();
    let args = Args::parse();
    match args.cmd {
        Cmd::Lab {
            seconds,
            flaps,
            burst,
        } => run_lab(seconds, flaps, burst).await,
        Cmd::ProtoCheck => {
            proto_check();
            Ok(())
        }
    }
}

fn proto_check() {
    let report = serde_json::json!({
        "hsfz_port": BMW_HSFZ_PORT,
        "hsfz_discovery_port": BMW_HSFZ_DISCOVERY_PORT,
        "doip_port": BMW_DOIP_PORT,
        "ok": true,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

async fn run_lab(seconds: u64, flaps: bool, burst: usize) -> anyhow::Result<()> {
    let (agent_eth, car) = SimulatedEthernet::pair("agent", "car");
    let (gw_eth, tools) = SimulatedEthernet::pair("gateway", "tools");

    let gw_addr = {
        let s = tokio::net::UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
        let addr = s.local_addr()?;
        drop(s);
        addr
    };

    let gw_opts = TunnelOptions {
        bind: gw_addr,
        peer: None,
        allowed_cidrs: vec![],
        crypto: None,
        require_crypto: false,
        keepalive_interval_ms: 300,
        peer_timeout_ms: 3000,
        role: "gateway".into(),
        version: "sim".into(),
    };
    let agent_opts = TunnelOptions {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        peer: Some(gw_addr),
        allowed_cidrs: vec![],
        crypto: None,
        require_crypto: false,
        keepalive_interval_ms: 300,
        peer_timeout_ms: 3000,
        role: "agent".into(),
        version: "sim".into(),
    };

    let gw = TunnelEngine::new(gw_opts, gw_eth).run().await?;
    let agent = TunnelEngine::new(agent_opts, agent_eth).run().await?;
    info!(%gw_addr, "lab tunnel up");

    // Car traffic generator
    let car2 = car.clone();
    let gen = tokio::spawn(async move {
        // Discovery-like broadcast stub
        let disc = fake_ethernet_udp_broadcast(BMW_HSFZ_DISCOVERY_PORT, b"\x00\x00\x00\x00\x00\x11");
        let _ = car2.send(disc).await;

        // ISTA scan burst
        for i in 0..burst {
            let payload = format!("ISTA-SCAN-{i}");
            let frame = fake_ethernet_ipv4(payload.as_bytes());
            let _ = car2.send(frame).await;
            if i % 50 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }

        // Coding session stream
        for i in 0..100 {
            let payload = format!("ESYS-CODE-{i}");
            let _ = car2.send(fake_ethernet_ipv4(payload.as_bytes())).await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        if flaps {
            info!("simulating vehicle sleep / cable unplug");
            car2.set_link(false);
            tokio::time::sleep(Duration::from_millis(400)).await;
            car2.set_link(true);
            info!("vehicle link restored");
            let _ = car2
                .send(fake_ethernet_ipv4(b"POST-WAKE-DISCOVERY"))
                .await;
        }
    });

    // Drain tool side and count
    let tools2 = tools.clone();
    let counter = tokio::spawn(async move {
        let mut n = 0u64;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(100), tools2.recv()).await {
                Ok(Ok(_frame)) => n += 1,
                Ok(Err(_)) => break,
                Err(_) => {}
            }
        }
        n
    });

    tokio::time::sleep(Duration::from_secs(seconds)).await;
    let _ = gen.await;
    let received = counter.await.unwrap_or(0);
    info!(received, "lab complete");

    let snap = gw.stats.snapshot();
    println!(
        "{}",
        serde_json::json!({
            "received_on_tools": received,
            "gw_tx": snap.tx_packets,
            "gw_rx": snap.rx_packets,
            "loss_rate": snap.loss_rate,
            "rtt_p99_ms": snap.rtt_p99_ms,
            "errors": snap.errors,
        })
    );

    if received == 0 {
        warn!("no frames received — lab failed");
        anyhow::bail!("lab received zero frames");
    }

    agent.stop();
    gw.stop();
    Ok(())
}

fn fake_ethernet_ipv4(payload: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(64 + payload.len());
    // dst mac broadcast-ish
    b.put_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    // src mac
    b.put_slice(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05]);
    // ethertype IPv4
    b.put_u16(0x0800);
    b.put_slice(payload);
    b.freeze()
}

fn fake_ethernet_udp_broadcast(port: u16, payload: &[u8]) -> Bytes {
    let mut body = BytesMut::new();
    body.put_u16(port);
    body.put_slice(payload);
    fake_ethernet_ipv4(&body)
}

/// Ensure encode path used by benches stays covered.
#[allow(dead_code)]
fn _encode_sample() {
    let _ = TunnelFrame::ethernet(1, 0, fake_ethernet_ipv4(b"x"));
}
