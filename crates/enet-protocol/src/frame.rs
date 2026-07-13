//! Tunnel frame codec.

use crate::{ProtocolError, Result};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};

/// Current wire protocol version.
pub const PROTOCOL_VERSION: u8 = 1;

/// Fixed header length in bytes.
pub const HEADER_LEN: usize = 24;

/// Maximum Ethernet frame we will tunnel (jumbo not required for ENET).
pub const MAX_ETHERNET_FRAME: usize = 1518;

/// Maximum UDP payload including header + optional AEAD tag overhead.
pub const MAX_TUNNEL_PACKET: usize = HEADER_LEN + MAX_ETHERNET_FRAME + 16;

/// Discriminant for tunnel frame kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FrameType {
    /// Raw Ethernet frame from ENET ↔ TAP.
    Ethernet = 1,
    /// Keepalive / RTT probe.
    Keepalive = 2,
    /// Peer hello / capability exchange.
    Hello = 3,
    /// Peer goodbye / graceful shutdown.
    Goodbye = 4,
    /// Status / health advertisement.
    Status = 5,
    /// Explicit ack for reliability experiments (optional).
    Ack = 6,
    /// Flash-safety probe result.
    SafetyProbe = 7,
}

impl FrameType {
    fn from_u8(v: u8) -> Result<Self> {
        match v {
            1 => Ok(Self::Ethernet),
            2 => Ok(Self::Keepalive),
            3 => Ok(Self::Hello),
            4 => Ok(Self::Goodbye),
            5 => Ok(Self::Status),
            6 => Ok(Self::Ack),
            7 => Ok(Self::SafetyProbe),
            other => Err(ProtocolError::BadFrameType(other)),
        }
    }
}

/// Fixed 24-byte tunnel header.
///
/// Layout:
/// ```text
/// 0      magic "ENET" (4)
/// 4      version u8
/// 5      frame_type u8
/// 6      flags u16 BE
/// 8      sequence u64 BE
/// 16     payload_len u16 BE
/// 18     reserved u16 BE
/// 20     timestamp_ms_lo u32 BE  (low 32 bits of unix ms for RTT)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    /// Protocol version.
    pub version: u8,
    /// Frame type.
    pub frame_type: FrameType,
    /// Bit flags (bit0 = encrypted, bit1 = compressed reserved).
    pub flags: u16,
    /// Monotonic sequence number per direction.
    pub sequence: u64,
    /// Payload length in bytes.
    pub payload_len: u16,
    /// Low 32 bits of sender unix timestamp in milliseconds (RTT).
    pub timestamp_ms_lo: u32,
}

/// Flag: payload is AEAD-encrypted.
pub const FLAG_ENCRYPTED: u16 = 0x0001;

impl FrameHeader {
    /// Encode header into a buffer.
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_slice(b"ENET");
        buf.put_u8(self.version);
        buf.put_u8(self.frame_type as u8);
        buf.put_u16(self.flags);
        buf.put_u64(self.sequence);
        buf.put_u16(self.payload_len);
        buf.put_u16(0); // reserved
        buf.put_u32(self.timestamp_ms_lo);
    }

    /// Decode header from the front of `buf`.
    pub fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < HEADER_LEN {
            return Err(ProtocolError::TooShort(buf.remaining()));
        }
        let mut magic = [0u8; 4];
        buf.copy_to_slice(&mut magic);
        if &magic != b"ENET" {
            return Err(ProtocolError::BadControl(format!(
                "bad magic: {:02x?}",
                magic
            )));
        }
        let version = buf.get_u8();
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::BadVersion(version));
        }
        let frame_type = FrameType::from_u8(buf.get_u8())?;
        let flags = buf.get_u16();
        let sequence = buf.get_u64();
        let payload_len = buf.get_u16();
        let _reserved = buf.get_u16();
        let timestamp_ms_lo = buf.get_u32();
        Ok(Self {
            version,
            frame_type,
            flags,
            sequence,
            payload_len,
            timestamp_ms_lo,
        })
    }
}

/// A complete tunnel frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelFrame {
    /// Header.
    pub header: FrameHeader,
    /// Payload bytes (Ethernet frame or control blob).
    pub payload: Bytes,
}

impl TunnelFrame {
    /// Construct an Ethernet tunnel frame.
    pub fn ethernet(sequence: u64, timestamp_ms_lo: u32, frame: Bytes) -> Result<Self> {
        if frame.len() > MAX_ETHERNET_FRAME {
            return Err(ProtocolError::PayloadTooLarge(frame.len()));
        }
        Ok(Self {
            header: FrameHeader {
                version: PROTOCOL_VERSION,
                frame_type: FrameType::Ethernet,
                flags: 0,
                sequence,
                payload_len: frame.len() as u16,
                timestamp_ms_lo,
            },
            payload: frame,
        })
    }

    /// Construct a keepalive frame carrying an echo cookie.
    pub fn keepalive(sequence: u64, timestamp_ms_lo: u32, cookie: u64) -> Self {
        let mut payload = BytesMut::with_capacity(8);
        payload.put_u64(cookie);
        Self {
            header: FrameHeader {
                version: PROTOCOL_VERSION,
                frame_type: FrameType::Keepalive,
                flags: 0,
                sequence,
                payload_len: 8,
                timestamp_ms_lo,
            },
            payload: payload.freeze(),
        }
    }

    /// Encode to bytes (optionally encrypting payload).
    pub fn encode(&self, crypto: Option<&crate::SessionCrypto>) -> Result<Bytes> {
        let mut body = self.payload.clone();
        let mut flags = self.header.flags;
        if let Some(c) = crypto {
            body = Bytes::from(c.encrypt(self.header.sequence, &body)?);
            flags |= FLAG_ENCRYPTED;
        }
        let header = FrameHeader {
            flags,
            payload_len: body.len() as u16,
            ..self.header.clone()
        };
        let mut out = BytesMut::with_capacity(HEADER_LEN + body.len());
        header.encode(&mut out);
        out.extend_from_slice(&body);
        Ok(out.freeze())
    }

    /// Decode from UDP datagram bytes.
    pub fn decode(data: &[u8], crypto: Option<&crate::SessionCrypto>) -> Result<Self> {
        let mut buf = data;
        let header = FrameHeader::decode(&mut buf)?;
        if buf.remaining() < header.payload_len as usize {
            return Err(ProtocolError::TooShort(data.len()));
        }
        let mut payload = Bytes::copy_from_slice(&buf[..header.payload_len as usize]);
        let mut header = header;
        if header.flags & FLAG_ENCRYPTED != 0 {
            let c = crypto.ok_or(ProtocolError::CryptoFailed)?;
            payload = Bytes::from(c.decrypt(header.sequence, &payload)?);
            header.flags &= !FLAG_ENCRYPTED;
            header.payload_len = payload.len() as u16;
        }
        Ok(Self { header, payload })
    }
}

/// Structured control payloads for Hello/Status/Safety.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlPayload {
    /// Initial peer handshake.
    Hello {
        /// Human-readable role: "agent" or "gateway".
        role: String,
        /// Software version.
        version: String,
        /// Hostname.
        hostname: String,
        /// Whether encryption is required.
        require_crypto: bool,
    },
    /// Periodic status.
    Status {
        /// ENET / vehicle link up.
        vehicle_link: bool,
        /// Vehicle appears awake (recent traffic or discovery reply).
        vehicle_awake: bool,
        /// Peer tunnel connected.
        peer_connected: bool,
        /// Packets forwarded lifetime.
        packets_tx: u64,
        /// Packets received lifetime.
        packets_rx: u64,
        /// Estimated loss rate 0.0–1.0.
        loss_rate: f64,
        /// Last RTT milliseconds.
        rtt_ms: f64,
    },
    /// Flash safety probe summary.
    SafetyProbe {
        /// Whether flashing is considered safe.
        safe: bool,
        /// Human-readable reasons if not safe.
        reasons: Vec<String>,
        /// Measured RTT p99 ms.
        rtt_p99_ms: f64,
        /// Measured loss rate.
        loss_rate: f64,
    },
}

impl ControlPayload {
    /// Serialize to JSON bytes.
    pub fn to_bytes(&self) -> Result<Bytes> {
        let v = serde_json::to_vec(self)
            .map_err(|e| ProtocolError::BadControl(e.to_string()))?;
        Ok(Bytes::from(v))
    }

    /// Deserialize from JSON bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data).map_err(|e| ProtocolError::BadControl(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ethernet() {
        let payload = Bytes::from_static(b"\xff\xff\xff\xff\xff\xff\x00\x11\x22\x33\x44\x55\x08\x00hello");
        let frame = TunnelFrame::ethernet(42, 123456, payload.clone()).unwrap();
        let encoded = frame.encode(None).unwrap();
        let decoded = TunnelFrame::decode(&encoded, None).unwrap();
        assert_eq!(decoded.header.sequence, 42);
        assert_eq!(decoded.header.frame_type, FrameType::Ethernet);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn reject_oversized() {
        let big = Bytes::from(vec![0u8; MAX_ETHERNET_FRAME + 1]);
        assert!(TunnelFrame::ethernet(1, 0, big).is_err());
    }
}
