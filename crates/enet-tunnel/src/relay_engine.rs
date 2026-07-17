//! Tunnel engine over a relay TCP stream (remote / different-network mode).

use crate::ethernet::EthernetPort;
use crate::relay_client::{
    connect_relay, decode_tunnel_frame, encode_tunnel_frame, RelayRole,
};
use crate::{SharedStats, TunnelHandle, TunnelOptions};
use enet_core::stats::PacketStats;
use enet_core::state::{ConnectionState, GatewayState};
use enet_protocol::{ControlPayload, FrameType, TunnelFrame};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

/// Options for relay-backed tunnels.
#[derive(Debug, Clone)]
pub struct RelayTunnelOptions {
    /// Shared tunnel options (crypto, keepalive, role, …).
    pub base: TunnelOptions,
    /// Relay host:port.
    pub relay_url: String,
    /// Pair code room.
    pub pair_code: String,
}

/// Run the L2 tunnel over an outbound TCP relay connection.
pub struct RelayTunnelEngine {
    opts: RelayTunnelOptions,
    eth: Arc<dyn EthernetPort>,
    stats: SharedStats,
    state: Arc<RwLock<GatewayState>>,
}

impl RelayTunnelEngine {
    /// Create a relay tunnel engine.
    pub fn new(opts: RelayTunnelOptions, eth: Arc<dyn EthernetPort>) -> Self {
        let stats = PacketStats::new();
        let state = Arc::new(RwLock::new(GatewayState::new(opts.base.version.clone())));
        Self {
            opts,
            eth,
            stats,
            state,
        }
    }

    /// Shared stats.
    pub fn stats(&self) -> SharedStats {
        self.stats.clone()
    }

    /// Shared state.
    pub fn state(&self) -> Arc<RwLock<GatewayState>> {
        self.state.clone()
    }

    /// Connect to relay and forward until stopped.
    pub async fn run(self) -> anyhow::Result<TunnelHandle> {
        if self.opts.base.require_crypto && self.opts.base.crypto.is_none() {
            anyhow::bail!(
                "require_crypto is set but no password is configured — set the same password on both PCs"
            );
        }
        let role = if self.opts.base.role == "gateway" {
            RelayRole::Gateway
        } else {
            RelayRole::Agent
        };
        info!(
            relay = %self.opts.relay_url,
            pair = %self.opts.pair_code,
            role = self.opts.base.role,
            "connecting to relay"
        );
        let stream = connect_relay(
            &self.opts.relay_url,
            role,
            &self.opts.pair_code,
            &self.opts.base.version,
        )
        .await?;
        info!("relay joined — waiting for peer / forwarding");
        let (reader, writer) = stream.into_split();
        let reader = Arc::new(Mutex::new(reader));
        let writer = Arc::new(Mutex::new(writer));

        let running = Arc::new(AtomicBool::new(true));
        let (stop_tx, _) = broadcast::channel::<()>(4);
        let handle = TunnelHandle::new(
            self.stats.clone(),
            self.state.clone(),
            running.clone(),
            stop_tx.clone(),
        );

        {
            let mut st = self.state.write();
            st.connection = ConnectionState::WaitingForPeer;
            st.gateway_running = true;
            st.status_message = format!("Joined relay {}", self.opts.relay_url);
        }

        let tx_seq = Arc::new(AtomicU64::new(1));
        let last_peer_rx = Arc::new(RwLock::new(Instant::now()));

        let eth_to_relay = {
            let writer = writer.clone();
            let eth = self.eth.clone();
            let opts = self.opts.base.clone();
            let stats = self.stats.clone();
            let state = self.state.clone();
            let running = running.clone();
            let tx_seq = tx_seq.clone();
            let is_agent = self.opts.base.role == "agent";
            tokio::spawn(async move {
                while running.load(Ordering::SeqCst) {
                    match eth.recv().await {
                        Ok(frame) => {
                            let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                            match TunnelFrame::ethernet(seq, now_ms_lo(), frame) {
                                Ok(tf) => match encode_tunnel_frame(&tf, opts.crypto.as_ref()) {
                                    Ok(pkt) => {
                                        let mut w = writer.lock().await;
                                        if let Err(e) = write_frame_half(&mut w, &pkt).await {
                                            stats.record_error();
                                            warn!(error = %e, "relay write failed");
                                            break;
                                        }
                                        stats.record_tx(pkt.len());
                                        if is_agent {
                                            let mut st = state.write();
                                            st.vehicle.last_activity_ms = now_ms();
                                            if st.vehicle.link_up {
                                                st.vehicle.awake = true;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        stats.record_error();
                                        warn!(error = %e, "encode failed");
                                    }
                                },
                                Err(e) => {
                                    stats.record_drop();
                                    warn!(error = %e, "bad ethernet frame");
                                }
                            }
                        }
                        Err(e) => {
                            stats.record_error();
                            warn!(error = %e, "ethernet recv failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            })
        };

        let relay_to_eth = {
            let reader = reader.clone();
            let writer = writer.clone();
            let eth = self.eth.clone();
            let opts = self.opts.base.clone();
            let relay_url = self.opts.relay_url.clone();
            let stats = self.stats.clone();
            let state = self.state.clone();
            let running = running.clone();
            let last_peer_rx = last_peer_rx.clone();
            let tx_seq = tx_seq.clone();
            tokio::spawn(async move {
                while running.load(Ordering::SeqCst) {
                    let pkt = {
                        let mut r = reader.lock().await;
                        match read_frame_half(&mut r).await {
                            Ok(p) => p,
                            Err(e) => {
                                stats.record_error();
                                warn!(error = %e, "relay read failed");
                                break;
                            }
                        }
                    };
                    // Enforce encryption on the relay path too.
                    if opts.require_crypto
                        && !enet_protocol::TunnelFrame::is_encrypted_raw(&pkt).unwrap_or(false)
                    {
                        stats.record_drop();
                        warn!("dropping plaintext relay frame (require_crypto)");
                        continue;
                    }
                    match decode_tunnel_frame(&pkt, opts.crypto.as_ref()) {
                        Ok(frame) => {
                            // Only Ethernet frames share one sequence space; control
                            // frames would fake loss gaps.
                            let seq = if frame.header.frame_type == FrameType::Ethernet {
                                Some(frame.header.sequence)
                            } else {
                                None
                            };
                            stats.record_rx(pkt.len(), seq);
                            *last_peer_rx.write() = Instant::now();
                            match frame.header.frame_type {
                                FrameType::Ethernet => {
                                    if let Err(e) = eth.send(frame.payload).await {
                                        stats.record_error();
                                        warn!(error = %e, "ethernet inject failed");
                                    } else {
                                        let mut st = state.write();
                                        st.connection = ConnectionState::Connected;
                                        st.laptop_connected = true;
                                        st.status_message = "Connected via relay".into();
                                        st.peer_endpoint = Some(format!("relay:{relay_url}"));
                                        // Tunnel Ethernet on the gateway ⇒ awake; link from Status only.
                                        if opts.role == "gateway" {
                                            st.vehicle.last_activity_ms = now_ms();
                                            if st.vehicle.link_up {
                                                st.vehicle.awake = true;
                                            }
                                        }
                                    }
                                }
                                FrameType::Keepalive => {
                                    // Payload cookie: 0 = probe (echo it back), 1 = reply.
                                    let is_reply = frame.payload.len() >= 8
                                        && frame.payload.as_ref()[7] == 1;
                                    if !is_reply {
                                        let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                                        let reply = TunnelFrame::keepalive(
                                            seq,
                                            frame.header.timestamp_ms_lo,
                                            1,
                                        );
                                        if let Ok(pkt) =
                                            encode_tunnel_frame(&reply, opts.crypto.as_ref())
                                        {
                                            let mut w = writer.lock().await;
                                            let _ = write_frame_half(&mut w, &pkt).await;
                                        }
                                    } else {
                                        let rtt = rtt_from_ts(frame.header.timestamp_ms_lo);
                                        if rtt > 0.0 {
                                            stats.record_rtt_ms(rtt);
                                            state.write().rtt_local_ms = rtt;
                                        }
                                    }
                                    let mut st = state.write();
                                    st.connection = ConnectionState::Connected;
                                    st.laptop_connected = true;
                                    st.status_message = "Connected via relay".into();
                                    st.peer_endpoint = Some(format!("relay:{relay_url}"));
                                }
                                FrameType::Hello | FrameType::Status | FrameType::SafetyProbe => {
                                    if let Ok(ctrl) = ControlPayload::from_bytes(&frame.payload) {
                                        let mut st = state.write();
                                        if let ControlPayload::Status {
                                            vehicle_link,
                                            vehicle_awake,
                                            ..
                                        } = ctrl
                                        {
                                            if opts.role == "gateway" {
                                                st.vehicle.link_up = vehicle_link;
                                                st.vehicle.awake = vehicle_awake;
                                            }
                                            st.laptop_connected = true;
                                        }
                                        st.connection = ConnectionState::Connected;
                                        st.status_message = "Connected via relay".into();
                                        st.peer_endpoint = Some(format!("relay:{relay_url}"));
                                    }
                                }
                                FrameType::Goodbye => {
                                    let mut st = state.write();
                                    st.connection = ConnectionState::Reconnecting;
                                    st.laptop_connected = false;
                                    st.peer_endpoint = None;
                                    if opts.role == "gateway" {
                                        st.vehicle.link_up = false;
                                        st.vehicle.awake = false;
                                    }
                                }
                                FrameType::Ack => {}
                            }
                        }
                        Err(e) => {
                            stats.record_error();
                            warn!(error = %e, "decode failed");
                        }
                    }
                }
            })
        };

        let keepalive = {
            let writer = writer.clone();
            let opts = self.opts.base.clone();
            let stats = self.stats.clone();
            let running = running.clone();
            let tx_seq = tx_seq.clone();
            let state = self.state.clone();
            let eth = self.eth.clone();
            let last_peer_rx = last_peer_rx.clone();
            tokio::spawn(async move {
                let interval = Duration::from_millis(opts.keepalive_interval_ms.max(500));
                // Relay paths ride TCP through the Internet — be generous but finite.
                let timeout = Duration::from_millis(opts.peer_timeout_ms.max(15_000));
                while running.load(Ordering::SeqCst) {
                    tokio::time::sleep(interval).await;
                    if opts.role == "agent" {
                        let link = eth.link_up().await;
                        let mut st = state.write();
                        st.vehicle.link_up = link;
                        if !link {
                            st.vehicle.awake = false;
                        } else if st.vehicle.awake {
                            let age = now_ms().saturating_sub(st.vehicle.last_activity_ms);
                            if st.vehicle.last_activity_ms > 0 && age > 10_000 {
                                st.vehicle.awake = false;
                            }
                        }
                    }
                    let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                    let frame = TunnelFrame::keepalive(seq, now_ms_lo(), 0);
                    if let Ok(pkt) = encode_tunnel_frame(&frame, opts.crypto.as_ref()) {
                        let mut w = writer.lock().await;
                        if write_frame_half(&mut w, &pkt).await.is_err() {
                            break;
                        }
                        stats.record_tx(pkt.len());
                    }
                    // Peer silence → stop claiming Connected.
                    if last_peer_rx.read().elapsed() > timeout {
                        let mut st = state.write();
                        if matches!(st.connection, ConnectionState::Connected) {
                            warn!("relay peer timeout");
                            stats.record_reconnect();
                            st.connection = ConnectionState::Reconnecting;
                            st.laptop_connected = false;
                            st.status_message = "Relay peer timeout — waiting".into();
                            if opts.role == "gateway" {
                                st.vehicle.link_up = false;
                                st.vehicle.awake = false;
                            }
                        }
                    }
                }
                // Keepalive writer failed → relay TCP is dead.
                let mut st = state.write();
                st.connection = ConnectionState::Failed;
                st.laptop_connected = false;
                st.status_message = "Relay connection lost".into();
            })
        };

        let running_flag = running.clone();
        let mut stop_rx = stop_tx.subscribe();
        let writer_shutdown = writer.clone();
        tokio::spawn(async move {
            let _ = stop_rx.recv().await;
            running_flag.store(false, Ordering::SeqCst);
            eth_to_relay.abort();
            relay_to_eth.abort();
            keepalive.abort();
            if let Ok(mut w) = writer_shutdown.try_lock() {
                let _ = w.shutdown().await;
            }
        });

        Ok(handle)
    }
}

async fn read_frame_half(stream: &mut OwnedReadHalf) -> anyhow::Result<bytes::Bytes> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > enet_protocol::MAX_TUNNEL_PACKET {
        anyhow::bail!("invalid frame length {len}");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(bytes::Bytes::from(body))
}

async fn write_frame_half(stream: &mut OwnedWriteHalf, data: &[u8]) -> anyhow::Result<()> {
    use bytes::{BufMut, BytesMut};
    if data.len() > enet_protocol::MAX_TUNNEL_PACKET {
        anyhow::bail!("frame too large");
    }
    let mut hdr = BytesMut::with_capacity(4 + data.len());
    hdr.put_u32(data.len() as u32);
    hdr.extend_from_slice(data);
    stream.write_all(&hdr).await?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_ms_lo() -> u32 {
    (now_ms() & 0xffff_ffff) as u32
}

/// RTT from an echoed low-32 timestamp; 0.0 when wrapped/implausible.
fn rtt_from_ts(ts_lo: u32) -> f64 {
    let now = now_ms_lo();
    let delta = now.wrapping_sub(ts_lo);
    if delta == 0 || delta > 60_000 {
        return 0.0;
    }
    delta as f64
}
