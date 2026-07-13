//! Wire protocol for the BMW ENET Layer-2 Ethernet-over-UDP tunnel.
//!
//! Frames carry raw Ethernet payloads between the laptop agent (ENET NIC)
//! and the desktop gateway (virtual TAP/Wintun NIC). Discovery broadcasts,
//! ARP, HSFZ (TCP 6801), and DoIP (TCP/UDP 13400) traverse unchanged.

#![deny(missing_docs)]

pub mod crypto;
pub mod frame;
pub mod magic;

pub use crypto::{SessionCrypto, derive_key_from_password};
pub use frame::{
    ControlPayload, FrameHeader, FrameType, TunnelFrame, HEADER_LEN, MAX_ETHERNET_FRAME,
    MAX_TUNNEL_PACKET, PROTOCOL_VERSION,
};
pub use magic::{
    BMW_DOIP_PORT, BMW_HSFZ_DISCOVERY_PORT, BMW_HSFZ_PORT, DEFAULT_AGENT_API_PORT, DEFAULT_API_PORT,
    DEFAULT_DISCOVERY_PORT, DEFAULT_RELAY_PORT, DEFAULT_TESTER_IP, DEFAULT_TESTER_MASK,
    DEFAULT_TUNNEL_PORT, LINK_LOCAL_PREFIX,
};

/// Result alias for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Protocol-level errors.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Packet too short to contain a header.
    #[error("packet too short: {0} bytes")]
    TooShort(usize),
    /// Unsupported protocol version.
    #[error("unsupported protocol version: {0}")]
    BadVersion(u8),
    /// Invalid frame type discriminant.
    #[error("invalid frame type: {0}")]
    BadFrameType(u8),
    /// AEAD authentication failed.
    #[error("decryption / authentication failed")]
    CryptoFailed,
    /// Payload exceeds maximum Ethernet frame size.
    #[error("payload too large: {0} bytes")]
    PayloadTooLarge(usize),
    /// Malformed control payload.
    #[error("malformed control payload: {0}")]
    BadControl(String),
    /// I/O error while encoding/decoding.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
