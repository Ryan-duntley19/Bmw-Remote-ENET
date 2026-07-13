//! Framed TCP transport used for relay / remote mode.
//!
//! Wire format on the stream: `[u32 BE length][TunnelFrame bytes]` repeating.

use anyhow::Context;
use bytes::{BufMut, Bytes, BytesMut};
use enet_protocol::TunnelFrame;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

/// Read one length-prefixed frame from a TCP stream.
pub async fn read_frame(stream: &mut TcpStream) -> anyhow::Result<Bytes> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > enet_protocol::MAX_TUNNEL_PACKET {
        anyhow::bail!("invalid frame length {len}");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Bytes::from(body))
}

/// Write one length-prefixed frame to a TCP stream.
pub async fn write_frame(stream: &mut TcpStream, data: &[u8]) -> anyhow::Result<()> {
    if data.len() > enet_protocol::MAX_TUNNEL_PACKET {
        anyhow::bail!("frame too large");
    }
    let mut hdr = BytesMut::with_capacity(4 + data.len());
    hdr.put_u32(data.len() as u32);
    hdr.extend_from_slice(data);
    stream.write_all(&hdr).await?;
    Ok(())
}

/// Relay handshake roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayRole {
    /// Desktop gateway.
    Gateway,
    /// Laptop agent.
    Agent,
}

impl RelayRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Agent => "agent",
        }
    }
}

/// Connect to a relay and complete the join handshake.
///
/// `relay_addr` formats: `host:port` (TCP).
pub async fn connect_relay(
    relay_addr: &str,
    role: RelayRole,
    pair_code: &str,
    version: &str,
) -> anyhow::Result<TcpStream> {
    let addr = normalize_relay_addr(relay_addr);
    debug!(%addr, role = role.as_str(), "dialing relay");
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("connect relay {addr}"))?;
    stream.set_nodelay(true)?;

    let hello = serde_json::json!({
        "magic": "BMWENETR1",
        "role": role.as_str(),
        "pair_code": pair_code,
        "version": version,
    });
    let payload = serde_json::to_vec(&hello)?;
    write_frame(&mut stream, &payload).await?;

    // Wait for "ok" or "wait" then eventually peer-ready is implicit when data flows;
    // relay replies with a status JSON once.
    let resp = read_frame(&mut stream).await?;
    let v: serde_json::Value = serde_json::from_slice(&resp)?;
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
    if status == "error" {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("relay error");
        anyhow::bail!("{msg}");
    }
    // status "waiting" or "paired" both OK — if waiting, relay will start forwarding when peer joins
    Ok(stream)
}

fn normalize_relay_addr(relay_addr: &str) -> String {
    let trimmed = relay_addr.trim();
    let trimmed = trimmed
        .strip_prefix("tcp://")
        .or_else(|| trimmed.strip_prefix("relay://"))
        .unwrap_or(trimmed);
    if trimmed.contains(':') {
        trimmed.to_string()
    } else {
        format!("{trimmed}:{}", enet_protocol::DEFAULT_RELAY_PORT)
    }
}

/// Encode a tunnel frame for the relay TCP pipe.
pub fn encode_tunnel_frame(
    frame: &TunnelFrame,
    crypto: Option<&enet_protocol::SessionCrypto>,
) -> anyhow::Result<Bytes> {
    Ok(frame.encode(crypto)?)
}

/// Decode a tunnel frame from relay bytes.
pub fn decode_tunnel_frame(
    data: &[u8],
    crypto: Option<&enet_protocol::SessionCrypto>,
) -> anyhow::Result<TunnelFrame> {
    Ok(TunnelFrame::decode(data, crypto)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_port() {
        assert_eq!(normalize_relay_addr("example.com"), "example.com:47910");
        assert_eq!(normalize_relay_addr("example.com:1234"), "example.com:1234");
        assert_eq!(normalize_relay_addr("tcp://a:9"), "a:9");
    }
}
