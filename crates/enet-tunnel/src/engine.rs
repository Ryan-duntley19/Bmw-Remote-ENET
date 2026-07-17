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
        if self.opts.require_crypto && self.opts.crypto.is_none() {
            anyhow::bail!(
                "require_crypto is set but no password is configured — set the same password on both PCs"
            );
        }
        let socket = Arc::new(UdpSocket::bind(self.opts.bind).await?);
        let _ = socket.set_broadcast(true);
        // Larger buffers help Wi‑Fi↔LAN jitter absorb bursts (ISTA/coding traffic).
        {
            let sock_ref = socket2::SockRef::from(socket.as_ref());
            let _ = sock_ref.set_recv_buffer_size(4 * 1024 * 1024);
            let _ = sock_ref.set_send_buffer_size(4 * 1024 * 1024);
        }
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
            let is_agent = self.opts.role == "agent";
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
                                            // Car-side evidence only on the agent. Gateway local TAP is tools.
                                            if is_agent {
                                                let mut st = state.write();
                                                st.vehicle.last_activity_ms = now_ms();
                                                // Awake = recent car traffic; link_up is OS carrier only.
                                                if st.vehicle.link_up {
                                                    st.vehicle.awake = true;
                                                }
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
                                    stats.reset_rx_sequence();
                                    let mut st = state.write();
                                    st.peer_endpoint = Some(src.to_string());
                                } else if let Some(existing) = *slot {
                                    if existing.ip() != src.ip() {
                                        let tunnel_port = opts
                                            .peer
                                            .map(|p| p.port())
                                            .unwrap_or(existing.port());
                                        // Laptop --peer can be a different NIC than the one the
                                        // desktop replies from (multi-homed Host). Learn it.
                                        // Ignore other Clients (ephemeral ports ≠ tunnel port).
                                        if opts.role == "agent"
                                            && opts.peer.is_some()
                                            && src.port() == tunnel_port
                                        {
                                            let learned = SocketAddr::new(src.ip(), tunnel_port);
                                            info!(
                                                %learned,
                                                from = %src,
                                                was = %existing,
                                                "desktop reply IP differs from --peer; switching"
                                            );
                                            *slot = Some(learned);
                                            let _ = peer_watch_tx.send(Some(learned));
                                            stats.reset_rx_sequence();
                                            let mut st = state.write();
                                            st.peer_endpoint = Some(learned.to_string());
                                        } else if opts.peer.is_some() {
                                            debug!(
                                                %src,
                                                expected = %existing,
                                                "ignoring packet from non-peer IP"
                                            );
                                            stats.record_drop();
                                            continue;
                                        } else if opts.role == "gateway" {
                                            // Another laptop appeared — take the new peer.
                                            info!(%src, previous = %existing, "peer IP changed");
                                            *slot = Some(src);
                                            let _ = peer_watch_tx.send(Some(src));
                                            stats.reset_rx_sequence();
                                            let mut st = state.write();
                                            st.peer_endpoint = Some(src.to_string());
                                        }
                                    } else {
                                        if existing != src {
                                            // NAT port change or second Client — resync loss tracking.
                                            stats.reset_rx_sequence();
                                        }
                                        *slot = Some(src);
                                        let mut st = state.write();
                                        st.peer_endpoint = Some(src.to_string());
                                    }
                                }
                            }
                            *last_peer_rx.write() = Instant::now();

                            // Enforce encryption: with require_crypto, plaintext
                            // frames are dropped instead of silently accepted.
                            if opts.require_crypto
                                && !TunnelFrame::is_encrypted_raw(&buf[..n]).unwrap_or(false)
                            {
                                warn!(%src, "dropping plaintext frame (require_crypto)");
                                stats.record_drop();
                                continue;
                            }

                            let crypto = opts.crypto.as_ref();
                            match TunnelFrame::decode(&buf[..n], crypto) {
                                Ok(frame) => {
                                    // Loss % tracks Ethernet data-plane only — control/keepalive
                                    // flaps were inventing 90%+ "loss" on an otherwise fine tunnel.
                                    let seq_for_loss =
                                        if frame.header.frame_type == FrameType::Ethernet {
                                            Some(frame.header.sequence)
                                        } else {
                                            None
                                        };
                                    stats.record_rx(n, seq_for_loss);
                                    match frame.header.frame_type {
                                        FrameType::Ethernet => {
                                            if let Err(e) = eth.send(frame.payload).await {
                                                stats.record_error();
                                                warn!(error = %e, "ethernet inject failed");
                                            } else {
                                                let mut st = state.write();
                                                st.connection = ConnectionState::Connected;
                                                st.laptop_connected = true;
                                                st.status_message = "Connected".into();
                                                // Gateway: car traffic via laptop ⇒ awake; link comes from Status.
                                                if opts.role == "gateway" {
                                                    st.vehicle.last_activity_ms = now_ms();
                                                    if st.vehicle.link_up {
                                                        st.vehicle.awake = true;
                                                    }
                                                }
                                            }
                                        }
                                        FrameType::Keepalive => {
                                            // Payload cookie: 0 = probe (reply), 1 = reply (do not re-reply)
                                            let is_reply = frame.payload.len() >= 8
                                                && frame.payload.as_ref()[7] == 1;
                                            // Reply ASAP — Wi‑Fi sleep makes delayed replies look like 100ms+ RTT.
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
                                            let rtt = rtt_from_ts(frame.header.timestamp_ms_lo);
                                            if rtt > 0.0 && is_reply {
                                                stats.record_rtt_ms(rtt);
                                                let mut st = state.write();
                                                st.rtt_local_ms = rtt;
                                                st.connection = ConnectionState::Connected;
                                                st.laptop_connected = true;
                                            } else {
                                                let mut st = state.write();
                                                st.connection = ConnectionState::Connected;
                                                st.laptop_connected = true;
                                            }
                                        }
                                        FrameType::Hello
                                        | FrameType::Status
                                        | FrameType::SafetyProbe => {
                                            if let Ok(ctrl) =
                                                ControlPayload::from_bytes(&frame.payload)
                                            {
                                                debug!(?ctrl, "control payload");
                                                apply_control(&state, &opts.role, ctrl);
                                            }
                                            // Any control plane RX proves the path is live both ways.
                                            {
                                                let mut st = state.write();
                                                st.connection = ConnectionState::Connected;
                                                st.laptop_connected = true;
                                                st.status_message = "Connected".into();
                                            }
                                            // ACK Hello immediately so the laptop leaves "waiting"
                                            // even before the next keepalive tick (Wi‑Fi / firewall).
                                            if frame.header.frame_type == FrameType::Hello {
                                                let peer = *peer_slot.read();
                                                if let Some(peer) = peer {
                                                    let seq =
                                                        tx_seq.fetch_add(1, Ordering::Relaxed);
                                                    let reply = TunnelFrame::keepalive(
                                                        seq,
                                                        now_ms_lo(),
                                                        1,
                                                    );
                                                    if let Ok(pkt) = reply.encode(crypto) {
                                                        let _ = socket.send_to(&pkt, peer).await;
                                                        stats.record_tx(pkt.len());
                                                    }
                                                }
                                            }
                                        }
                                        FrameType::Goodbye => {
                                            info!("peer goodbye");
                                            let mut st = state.write();
                                            st.connection = ConnectionState::Reconnecting;
                                            st.laptop_connected = false;
                                            st.status_message = "Peer disconnected".into();
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
                // Agents probe often so the laptop Wi‑Fi radio stays awake; Status is less frequent.
                let probe_ms = if opts.role == "agent" {
                    opts.keepalive_interval_ms.clamp(200, 250)
                } else {
                    opts.keepalive_interval_ms.max(200)
                };
                let interval = Duration::from_millis(probe_ms);
                let timeout = Duration::from_millis(if opts.role == "agent" {
                    // Wi‑Fi sleep can starve RX for several seconds; don't flap to "waiting".
                    opts.peer_timeout_ms.max(20_000)
                } else {
                    opts.peer_timeout_ms.max(1000)
                });
                let mut tick: u32 = 0;
                while running.load(Ordering::SeqCst) {
                    tokio::time::sleep(interval).await;
                    tick = tick.wrapping_add(1);
                    let peer = *peer_slot.read();
                    if let Some(peer) = peer {
                        // Send keepalive FIRST so RTT isn't delayed by link checks.
                        let seq = tx_seq.fetch_add(1, Ordering::Relaxed);
                        let frame = TunnelFrame::keepalive(seq, now_ms_lo(), 0);
                        if let Ok(pkt) = frame.encode(opts.crypto.as_ref()) {
                            let _ = socket.send_to(&pkt, peer).await;
                            stats.record_tx(pkt.len());
                        }
                        // While still waiting, resend Hello so the desktop ACKs us.
                        if opts.role == "agent"
                            && tick % 5 == 1
                            && !matches!(
                                state.read().connection,
                                ConnectionState::Connected
                            )
                        {
                            let _ = send_hello(
                                &socket,
                                peer,
                                &opts,
                                &tx_seq,
                                opts.crypto.as_ref(),
                            )
                            .await;
                        }
                    }
                    // Only the laptop agent owns local ENET link state (cached / non-blocking).
                    if opts.role == "agent" {
                        let link = eth.link_up().await;
                        let mut st = state.write();
                        st.vehicle.link_up = link;
                        if !link {
                            st.vehicle.awake = false;
                        } else if st.vehicle.awake {
                            // Expire awake without recent frames (noise / unplugged car).
                            let age = now_ms().saturating_sub(st.vehicle.last_activity_ms);
                            if st.vehicle.last_activity_ms > 0 && age > 10_000 {
                                st.vehicle.awake = false;
                            }
                        }
                    }
                    let send_status = opts.role != "agent" || tick % 4 == 0;
                    let peer = *peer_slot.read();
                    if send_status {
                        if let Some(peer) = peer {
                            let (rtt_ms, _rtt_p99, loss_rate) = stats.peek_quality();
                            let st = state.read().clone();
                            let status = ControlPayload::Status {
                                vehicle_link: if opts.role == "agent" {
                                    st.vehicle.link_up
                                } else {
                                    false
                                },
                                vehicle_awake: if opts.role == "agent" {
                                    st.vehicle.awake
                                } else {
                                    false
                                },
                                peer_connected: matches!(st.connection, ConnectionState::Connected),
                                packets_tx: stats.tx_packets(),
                                packets_rx: stats.rx_packets(),
                                loss_rate,
                                rtt_ms,
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
                    }
                    let timed_out = peer_slot.read().is_some()
                        && last_peer_rx.read().elapsed() > timeout;
                    if timed_out {
                        warn!("peer timeout");
                        stats.record_reconnect();
                        let mut st = state.write();
                        st.laptop_connected = false;
                        st.peer_endpoint = None;
                        if opts.role == "gateway" {
                            st.connection = ConnectionState::Reconnecting;
                            st.status_message = "Peer timeout — reconnecting".into();
                            st.vehicle.link_up = false;
                            st.vehicle.awake = false;
                        } else {
                            // Agent: Host DHCP may have changed — fail out so outer loop re-discovers.
                            st.connection = ConnectionState::Failed;
                            st.status_message = "Peer timeout — re-detecting desktop IP".into();
                        }
                        drop(st);
                        *peer_slot.write() = None;
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

fn apply_control(state: &Arc<RwLock<GatewayState>>, role: &str, ctrl: ControlPayload) {
    let mut st = state.write();
    match ctrl {
        ControlPayload::Status {
            vehicle_link,
            vehicle_awake,
            peer_connected: _,
            rtt_ms,
            ..
        } => {
            // Laptop agent is the authority for vehicle ENET / awake.
            // Gateway accepts those fields; agent ignores the desktop's (often false) vehicle flags.
            if role == "gateway" {
                st.vehicle.link_up = vehicle_link;
                st.vehicle.awake = vehicle_awake;
                // Laptop's view of RTT (usually lower — it initiates TX and wakes Wi‑Fi).
                if rtt_ms.is_finite() && rtt_ms > 0.0 {
                    st.rtt_peer_ms = rtt_ms;
                }
            }
            // peer_connected in Status is "what the sender thinks"; for gateway the
            // presence of Status from the agent already means the laptop is up.
            if role == "gateway" {
                st.laptop_connected = true;
            } else {
                // Receiving Status from the desktop means the tunnel is up — don't
                // require peer_connected (gateway may still be catching up).
                st.laptop_connected = true;
                st.connection = ConnectionState::Connected;
                st.status_message = "Connected".into();
            }
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
