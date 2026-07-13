//! Asynchronous Layer-2 Ethernet-over-UDP / relay tunnel.
//!
//! The tunnel is transport-only: it does not interpret HSFZ/DoIP/UDS. Raw Ethernet
//! frames are forwarded bidirectionally between a local [`EthernetPort`] and a peer
//! (LAN UDP or remote TCP relay).

#![deny(missing_docs)]

mod engine;
mod ethernet;
mod peer;
mod relay_client;
mod relay_engine;

pub use engine::{TunnelEngine, TunnelHandle};
pub use ethernet::{EthernetPort, LoopbackEthernet, SimulatedEthernet};
pub use peer::PeerAddr;
pub use relay_client::RelayRole;
pub use relay_engine::{RelayTunnelEngine, RelayTunnelOptions};

use enet_core::stats::PacketStats;
use enet_protocol::SessionCrypto;
use std::net::SocketAddr;
use std::sync::Arc;

/// Runtime options for a tunnel endpoint.
#[derive(Debug, Clone)]
pub struct TunnelOptions {
    /// Local UDP bind address.
    pub bind: SocketAddr,
    /// Optional fixed peer (agent mode). If `None`, accept first allowed peer (gateway).
    pub peer: Option<SocketAddr>,
    /// Allowed peer IP prefixes as strings (e.g. "192.168.0.0/16"). Empty = allow any.
    pub allowed_cidrs: Vec<String>,
    /// Optional session crypto.
    pub crypto: Option<SessionCrypto>,
    /// Require encrypted frames.
    pub require_crypto: bool,
    /// Keepalive interval ms.
    pub keepalive_interval_ms: u64,
    /// Peer timeout ms.
    pub peer_timeout_ms: u64,
    /// Role label for hello.
    pub role: String,
    /// Software version string.
    pub version: String,
}

impl TunnelOptions {
    /// Build crypto from password if non-empty.
    pub fn with_password(mut self, password: &str, require: bool) -> Self {
        if !password.is_empty() {
            let key = enet_protocol::derive_key_from_password(password);
            self.crypto = Some(SessionCrypto::from_key(key));
        }
        self.require_crypto = require;
        self
    }
}

/// Shared tunnel metrics.
pub type SharedStats = Arc<PacketStats>;
