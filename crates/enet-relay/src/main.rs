//! BMW ENET relay — both laptop and desktop dial out; no shared LAN required.
//!
//! Rooms are keyed by pair code. When both roles join, bytes are piped
//! bidirectionally (opaque length-prefixed tunnel frames end-to-end).

use anyhow::Context;
use bytes::{BufMut, BytesMut};
use clap::Parser;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "enet-relay",
    about = "Relay for BMW ENET when desktop and laptop are on different networks"
)]
struct Args {
    /// Listen address
    #[arg(long, default_value = "0.0.0.0:47910")]
    listen: SocketAddr,
}

struct Waiting {
    notify: oneshot::Sender<TcpStream>,
}

#[derive(Default)]
struct Room {
    gateway: Option<Waiting>,
    agent: Option<Waiting>,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

enum JoinOutcome {
    Wait(oneshot::Receiver<TcpStream>),
    HandedOff,
    Duplicate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();
    let args = Args::parse();
    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("bind {}", args.listen))?;
    info!(%args.listen, "enet-relay listening");
    eprintln!();
    eprintln!("  BMW ENET Relay");
    eprintln!("  --------------");
    eprintln!("  Listening on {}", args.listen);
    eprintln!("  Desktop + laptop: network_mode = \"relay\", same relay_url + pair code");
    eprintln!("  Firewall: allow TCP {} inbound on this host.", args.listen.port());
    eprintln!();

    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (socket, peer) = listener.accept().await?;
        let rooms = rooms.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, peer, rooms).await {
                warn!(%peer, error = %e, "session ended");
            }
        });
    }
}

async fn handle_client(mut mine: TcpStream, peer: SocketAddr, rooms: Rooms) -> anyhow::Result<()> {
    mine.set_nodelay(true)?;
    let hello = read_frame(&mut mine).await?;
    let v: serde_json::Value = serde_json::from_slice(&hello)?;
    if v.get("magic").and_then(|m| m.as_str()) != Some("BMWENETR1") {
        write_json(
            &mut mine,
            &serde_json::json!({"status":"error","message":"bad magic"}),
        )
        .await?;
        anyhow::bail!("bad magic");
    }
    let role = v
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let pair_code = v
        .get("pair_code")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_uppercase();
    if pair_code.is_empty() || (role != "gateway" && role != "agent") {
        write_json(
            &mut mine,
            &serde_json::json!({"status":"error","message":"need role+pair_code"}),
        )
        .await?;
        anyhow::bail!("invalid hello");
    }
    info!(%peer, %role, %pair_code, "join");

    let (tx, rx) = oneshot::channel::<TcpStream>();
    let mut mine_opt = Some(mine);
    let outcome = {
        let mut map = rooms.lock();
        let room = map.entry(pair_code.clone()).or_default();
        match role.as_str() {
            "gateway" => {
                if room.gateway.is_some() {
                    JoinOutcome::Duplicate
                } else if let Some(waiting_agent) = room.agent.take() {
                    let _ = waiting_agent.notify.send(mine_opt.take().unwrap());
                    JoinOutcome::HandedOff
                } else {
                    room.gateway = Some(Waiting { notify: tx });
                    JoinOutcome::Wait(rx)
                }
            }
            "agent" => {
                if room.agent.is_some() {
                    JoinOutcome::Duplicate
                } else if let Some(waiting_gw) = room.gateway.take() {
                    let _ = waiting_gw.notify.send(mine_opt.take().unwrap());
                    JoinOutcome::HandedOff
                } else {
                    room.agent = Some(Waiting { notify: tx });
                    JoinOutcome::Wait(rx)
                }
            }
            _ => JoinOutcome::Duplicate,
        }
    };

    match outcome {
        JoinOutcome::HandedOff => Ok(()),
        JoinOutcome::Duplicate => {
            let mut mine = mine_opt.take().unwrap();
            write_json(
                &mut mine,
                &serde_json::json!({"status":"error","message":"role already connected for this pair code"}),
            )
            .await?;
            anyhow::bail!("duplicate role");
        }
        JoinOutcome::Wait(rx) => {
            let mut mine = mine_opt.take().unwrap();
            // First status for connect_relay(); do NOT send another status later —
            // the client switches to tunnel frames immediately after this.
            write_json(
                &mut mine,
                &serde_json::json!({"status":"waiting","message":"waiting for peer"}),
            )
            .await?;
            let mut other = rx
                .await
                .map_err(|_| anyhow::anyhow!("peer disconnected before pairing"))?;
            other.set_nodelay(true)?;
            // Second peer is still blocked in connect_relay() awaiting one status.
            write_json(
                &mut other,
                &serde_json::json!({"status":"paired","message":"peer connected"}),
            )
            .await?;
            info!(%pair_code, "paired — piping");
            match tokio::io::copy_bidirectional(&mut mine, &mut other).await {
                Ok((a, b)) => info!(%pair_code, a, b, "pipe done"),
                Err(e) => warn!(%pair_code, error = %e, "pipe error"),
            }
            Ok(())
        }
    }
}

async fn read_frame(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > enet_protocol::MAX_TUNNEL_PACKET {
        anyhow::bail!("bad length {len}");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

async fn write_frame(stream: &mut TcpStream, data: &[u8]) -> anyhow::Result<()> {
    let mut buf = BytesMut::with_capacity(4 + data.len());
    buf.put_u32(data.len() as u32);
    buf.extend_from_slice(data);
    stream.write_all(&buf).await?;
    Ok(())
}

async fn write_json(stream: &mut TcpStream, v: &serde_json::Value) -> anyhow::Result<()> {
    write_frame(stream, &serde_json::to_vec(v)?).await
}
