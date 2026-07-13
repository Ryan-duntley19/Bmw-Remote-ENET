//! Tunnel engine: bidirectional Ethernet ↔ UDP forwarding.

use crate::ethernet::EthernetPort;
use crate::peer::ip_allowed;
use crate::{SharedStats, TunnelOptions};
use enet_core::stats::PacketStats;
use enet_core::state::{ConnectionState, GatewayState};
use enet_protocol::{ControlPayload, FrameType, SessionCrypto, TunnelFrame};
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, warn};

/// Cloneable handle to observe and control a running tunnel.
#[derive(Clone)]
pub struct TunnelHandle {
    /// Shared stats.
    pub stats: SharedStats,
    /// Shared gateway state.
    pub state: Arc<RwLock<GatewayState>>,
    running: Arc<AtomicBool>,
    stop_tx: broadcast::Sender<()>,
}

impl TunnelHandle {
    /// Construct a handle for an already-spawned tunnel.
    pub fn new(
        stats: SharedStats,
        state: Arc<RwLock<GatewayState>>,
        running: Arc<AtomicBool>,
        stop_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            stats,
            state,
            running,
            stop_tx,
        }
    }

    /// Request graceful stop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.stop_tx.send(());
    }

    /// Whether the engine is marked running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Snapshot state.
    pub fn snapshot_state(&self) -> GatewayState {
        self.state.read().clone()
    }
}

/// Owns the forwarding tasks.
pub struct TunnelEngine {
    opts: TunnelOptions,
    eth: Arc<dyn EthernetPort>,
    stats: SharedStats,
    state: Arc<RwLock<GatewayState>>,
}

impl TunnelEngine {
    /// Create an engine bound to an Ethernet port.
    pub fn new(opts: TunnelOptions, eth: Arc<dyn EthernetPort>) -> Self {
        let stats = PacketStats::new();
        let state = Arc::new(RwLock::new(GatewayState::new(opts.version.clone())));
        Self {
            opts,
            eth,
            stats,
            state,
        }
    }

    /// Access stats before start.
    pub fn stats(&self) -> SharedStats {
        self.stats.clone()
    }

    /// Access state before start.
    pub fn state(&self) -> Arc<RwLock<GatewayState>> {
        self.state.clone()
    }

    /// Bind UDP and run until stopped.
    pub async fn run(self) -> anyhow::Result<TunnelHandle> {
        let socket = Arc::new(UdpSocket::bind(self.opts.bind).await?);
        let _ = socket.set_broadcast(true);
        info!(bind = %self.opts.bind, role = %self.opts.role, "tunnel UDP bound");

        let running = Arc::new(AtomicBool::new(true));
        let (stop_tx, _) = broadcast::channel::<()>(4);
        let handle = TunnelHandle {
            stats: self.stats.clone(),
            state: self.state.clone(),
            running: running.clone(),
            stop_tx: stop_tx.clone(),
        };

        {
            let mut st = self.state.write();
            st.connection = ConnectionState::Starting;
            st.gateway_running = true;
            st.status_message = "Starting".into();
        }

        let peer_slot: Arc<RwLock<Option<SocketAddr>>> = Arc::new(RwLock::new(self.opts.peer));
        let tx_seq = Arc::new(AtomicU64::new(1));
        let last_peer_rx = Arc::new(RwLock::new(Instant::now()));
        let (peer_watch_tx, _peer_watch_rx) = watch::channel::<Option<SocketAddr>>(self.opts.peer);

        if let Some(peer) = self.opts.peer {
            send_hello(&socket, peer, &self.opts, &tx_seq, self.opts.crypto.as_ref()).await?;
            let mut st = self.state.write();
            st.connection = ConnectionState::WaitingForPeer;
            st.status_message = format!("Waiting for peer at {peer}");
        } else {
            let mut st = self.state.write();
            st.connection = ConnectionState::WaitingForPeer;
            st.status_message = "Waiting for agent connection".into();
        }

        let eth_to_udp = {
            let socket = socket.clone();
            let eth = self.eth.clone();
            let opts = self.opts.clone();
            let stats = self.stats.clone();
            let peer_slot = peer_slot.clone();
            let tx_seq = tx_seq.clone();
            let state = self.state.clone();
            let running = running.clone();
            tokio::spawn(async move {
                while running.load(Ordering::SeqCst) {
                    match eth.recv().await {
                        Ok(frame) => {
                            let peer = { *peer_slot.read() };
                            let Some(peer) = peer else { continue };
                            let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                            match TunnelFrame::ethernet(seq, now_ms_lo(), frame) {
                                Ok(tf) => match tf.encode(opts.crypto.as_ref()) {
                                    Ok(pkt) => {
                                        if let Err(e) = socket.send_to(&pkt, peer).await {
                                            stats.record_error();
                                            warn!(error = %e, "udp send failed");
                                        } else {
                                            stats.record_tx(pkt.len());
                                            let mut st = state.write();
                                            st.vehicle.last_activity_ms = now_ms();
                                            st.vehicle.awake = true;
                                            st.vehicle.link_up = true;
                                        }
                                    }
                                    Err(e) => {
                                        stats.record_error();
                                        warn!(error = %e, "encode failed");
                                    }
                                },
                                Err(e) => {
                                    stats.record_drop();
                                    warn!(error = %e, "ethernet frame rejected");
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

        let udp_to_eth = {
            let socket = socket.clone();
            let eth = self.eth.clone();
            let opts = self.opts.clone();
            let stats = self.stats.clone();
            let peer_slot = peer_slot.clone();
            let state = self.state.clone();
            let running = running.clone();
            let last_peer_rx = last_peer_rx.clone();
            let peer_watch_tx = peer_watch_tx.clone();
            let tx_seq = tx_seq.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                while running.load(Ordering::SeqCst) {
                    match socket.recv_from(&mut buf).await {
                        Ok((n, src)) => {
                            if !ip_allowed(src.ip(), &opts.allowed_cidrs) {
                                warn!(%src, "rejected packet from non-allowed peer");
                                stats.record_drop();
                                continue;
                            }
                            {
                                let mut slot = peer_slot.write();
                                if slot.is_none() {
                                    info!(%src, "learned tunnel peer");
                                    *slot = Some(src);
                                    let _ = peer_watch_tx.send(Some(src));
                                } else if let Some(existing) = *slot {
                                    if existing.ip() != src.ip() && opts.peer.is_some() {
                                        warn!(%src, expected = %existing, "unexpected peer ip");
                                        stats.record_drop();
                                        continue;
                                    }
                                    *slot = Some(src);
                                }
                            }
                            *last_peer_rx.write() = Instant::now();

                            let crypto = opts.crypto.as_ref();
                            match TunnelFrame::decode(&buf[..n], crypto) {
                                Ok(frame) => {
                                    stats.record_rx(n, Some(frame.header.sequence));
                                    match frame.header.frame_type {
                                        FrameType::Ethernet => {
                                            if let Err(e) = eth.send(frame.payload).await {
                                                stats.record_error();
                                                warn!(error = %e, "ethernet inject failed");
                                            } else {
                                                let link = eth.link_up().await;
                                                let mut st = state.write();
                                                st.connection = ConnectionState::Connected;
                                                st.laptop_connected = true;
                                                st.status_message = "Connected".into();
                                                st.vehicle.link_up = link;
                                            }
                                        }
                                        FrameType::Keepalive => {
                                            // Payload cookie: 0 = probe (reply), 1 = reply (do not re-reply)
                                            let is_reply = frame.payload.len() >= 8
                                                && frame.payload.as_ref()[7] == 1;
                                            let rtt = rtt_from_ts(frame.header.timestamp_ms_lo);
                                            if rtt > 0.0 && is_reply {
                                                stats.record_rtt_ms(rtt);
                                            }
                                            if !is_reply {
                                                let peer = *peer_slot.read();
                                                if let Some(peer) = peer {
                                                    let seq =
                                                        tx_seq.fetch_add(1, Ordering::Relaxed);
                                                    let reply = TunnelFrame::keepalive(
                                                        seq,
                                                        frame.header.timestamp_ms_lo,
                                                        1,
                                                    );
                                                    if let Ok(pkt) = reply.encode(crypto) {
                                                        let _ = socket.send_to(&pkt, peer).await;
                                                        stats.record_tx(pkt.len());
                                                    }
                                                }
                                            }
                                            let mut st = state.write();
                                            st.connection = ConnectionState::Connected;
                                            st.laptop_connected = true;
                                        }
                                        FrameType::Hello
                                        | FrameType::Status
                                        | FrameType::SafetyProbe => {
                                            if let Ok(ctrl) =
                                                ControlPayload::from_bytes(&frame.payload)
                                            {
                                                debug!(?ctrl, "control payload");
                                                apply_control(&state, ctrl);
                                            }
                                            let mut st = state.write();
                                            st.connection = ConnectionState::Connected;
                                            st.laptop_connected = true;
                                            st.status_message = "Connected".into();
                                        }
                                        FrameType::Goodbye => {
                                            info!("peer goodbye");
                                            let mut st = state.write();
                                            st.connection = ConnectionState::Reconnecting;
                                            st.laptop_connected = false;
                                            st.status_message = "Peer disconnected".into();
                                        }
                                        FrameType::Ack => {}
                                    }
                                }
                                Err(e) => {
                                    stats.record_error();
                                    debug!(error = %e, "failed to decode tunnel frame");
                                }
                            }
                        }
                        Err(e) => {
                            stats.record_error();
                            warn!(error = %e, "udp recv failed");
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                }
            })
        };

        let keepalive = {
            let socket = socket.clone();
            let opts = self.opts.clone();
            let stats = self.stats.clone();
            let peer_slot = peer_slot.clone();
            let tx_seq = tx_seq.clone();
            let running = running.clone();
            let last_peer_rx = last_peer_rx.clone();
            let state = self.state.clone();
            let eth = self.eth.clone();
            tokio::spawn(async move {
                let interval = Duration::from_millis(opts.keepalive_interval_ms.max(200));
                let timeout = Duration::from_millis(opts.peer_timeout_ms.max(1000));
                while running.load(Ordering::SeqCst) {
                    tokio::time::sleep(interval).await;
                    let link = eth.link_up().await;
                    {
                        let mut st = state.write();
                        st.vehicle.link_up = link;
                        if !link {
                            st.vehicle.awake = false;
                        }
                    }
                    let peer = *peer_slot.read();
                    if let Some(peer) = peer {
                        let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                        let frame = TunnelFrame::keepalive(seq, now_ms_lo(), 0);
                        if let Ok(pkt) = frame.encode(opts.crypto.as_ref()) {
                            let _ = socket.send_to(&pkt, peer).await;
                            stats.record_tx(pkt.len());
                        }
                        let snap = stats.snapshot();
                        let st = state.read().clone();
                        let status = ControlPayload::Status {
                            vehicle_link: st.vehicle.link_up,
                            vehicle_awake: st.vehicle.awake,
                            peer_connected: matches!(st.connection, ConnectionState::Connected),
                            packets_tx: snap.tx_packets,
                            packets_rx: snap.rx_packets,
                            loss_rate: snap.loss_rate,
                            rtt_ms: snap.rtt_ms,
                        };
                        if let Ok(payload) = status.to_bytes() {
                            let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                            let mut tf = TunnelFrame::keepalive(seq, now_ms_lo(), 0);
                            tf.header.frame_type = FrameType::Status;
                            tf.header.payload_len = payload.len() as u16;
                            tf.payload = payload;
                            if let Ok(pkt) = tf.encode(opts.crypto.as_ref()) {
                                let _ = socket.send_to(&pkt, peer).await;
                            }
                        }
                    }
                    let timed_out = peer_slot.read().is_some()
                        && last_peer_rx.read().elapsed() > timeout;
                    if timed_out {
                        warn!("peer timeout");
                        stats.record_reconnect();
                        let mut st = state.write();
                        st.connection = ConnectionState::Reconnecting;
                        st.laptop_connected = false;
                        st.status_message = "Peer timeout — reconnecting".into();
                        drop(st);
                        if opts.peer.is_none() {
                            *peer_slot.write() = None;
                        }
                    }
                }
            })
        };

        let running_flag = running.clone();
        let mut stop_rx = stop_tx.subscribe();
        tokio::spawn(async move {
            let _ = stop_rx.recv().await;
            running_flag.store(false, Ordering::SeqCst);
            eth_to_udp.abort();
            udp_to_eth.abort();
            keepalive.abort();
        });

        Ok(handle)
    }
}

async fn send_hello(
    socket: &UdpSocket,
    peer: SocketAddr,
    opts: &TunnelOptions,
    tx_seq: &AtomicU64,
    crypto: Option<&SessionCrypto>,
) -> anyhow::Result<()> {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".into());
    let hello = ControlPayload::Hello {
        role: opts.role.clone(),
        version: opts.version.clone(),
        hostname: host,
        require_crypto: opts.require_crypto,
    };
    let payload = hello.to_bytes()?;
    let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
    let mut frame = TunnelFrame::keepalive(seq, now_ms_lo(), 0);
    frame.header.frame_type = FrameType::Hello;
    frame.header.payload_len = payload.len() as u16;
    frame.payload = payload;
    let pkt = frame.encode(crypto)?;
    socket.send_to(&pkt, peer).await?;
    Ok(())
}

fn apply_control(state: &Arc<RwLock<GatewayState>>, ctrl: ControlPayload) {
    let mut st = state.write();
    match ctrl {
        ControlPayload::Status {
            vehicle_link,
            vehicle_awake,
            peer_connected,
            ..
        } => {
            st.vehicle.link_up = vehicle_link;
            st.vehicle.awake = vehicle_awake;
            st.laptop_connected = peer_connected;
        }
        ControlPayload::Hello { hostname, role, .. } => {
            st.status_message = format!("Hello from {role}@{hostname}");
        }
        ControlPayload::SafetyProbe { safe, reasons, .. } => {
            if !safe {
                st.status_message = format!("Safety: {}", reasons.join("; "));
            }
        }
    }
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

fn rtt_from_ts(timestamp_ms_lo: u32) -> f64 {
    let now = now_ms_lo();
    let delta = now.wrapping_sub(timestamp_ms_lo);
    if delta > 60_000 {
        0.0
    } else {
        delta as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimulatedEthernet;
    use bytes::Bytes;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn tunnel_forwards_ethernet_frame() {
        let (eth_a, car_side) = SimulatedEthernet::pair("agent-eth", "car");
        let (eth_b, tool_side) = SimulatedEthernet::pair("gw-eth", "tools");

        let gw_sock = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let gw_addr = gw_sock.local_addr().unwrap();
        drop(gw_sock);

        let gw_opts = TunnelOptions {
            bind: gw_addr,
            peer: None,
            allowed_cidrs: vec![],
            crypto: None,
            require_crypto: false,
            keepalive_interval_ms: 500,
            peer_timeout_ms: 5000,
            role: "gateway".into(),
            version: "test".into(),
        };

        let agent_opts = TunnelOptions {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            peer: Some(gw_addr),
            allowed_cidrs: vec![],
            crypto: None,
            require_crypto: false,
            keepalive_interval_ms: 500,
            peer_timeout_ms: 5000,
            role: "agent".into(),
            version: "test".into(),
        };

        let gw_handle = TunnelEngine::new(gw_opts, eth_b).run().await.unwrap();
        let agent_handle = TunnelEngine::new(agent_opts, eth_a).run().await.unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        let frame = Bytes::from_static(
            b"\xff\xff\xff\xff\xff\xff\x11\x22\x33\x44\x55\x66\x08\x00fake-ip-packet",
        );
        car_side.send(frame.clone()).await.unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), tool_side.recv())
            .await
            .expect("timeout waiting for forwarded frame")
            .unwrap();
        assert_eq!(received, frame);

        agent_handle.stop();
        gw_handle.stop();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
