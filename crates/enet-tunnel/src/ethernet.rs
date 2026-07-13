//! Ethernet port abstraction (real NIC, TAP, or simulator).

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Asynchronous Ethernet frame source/sink.
#[async_trait]
pub trait EthernetPort: Send + Sync {
    /// Human-readable port name.
    fn name(&self) -> &str;
    /// Whether the underlying link is up.
    async fn link_up(&self) -> bool;
    /// Receive next Ethernet frame (blocks until available).
    async fn recv(&self) -> anyhow::Result<Bytes>;
    /// Send an Ethernet frame.
    async fn send(&self, frame: Bytes) -> anyhow::Result<()>;
}

/// Simulated Ethernet port backed by Tokio channels (primary test double).
pub struct SimulatedEthernet {
    name: String,
    tx: mpsc::UnboundedSender<Bytes>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Bytes>>,
    link: Mutex<bool>,
}

impl SimulatedEthernet {
    /// Create a connected pair of ports.
    pub fn pair(name_a: &str, name_b: &str) -> (Arc<Self>, Arc<Self>) {
        let (tx_a, rx_a) = mpsc::unbounded_channel();
        let (tx_b, rx_b) = mpsc::unbounded_channel();
        let a = Arc::new(Self {
            name: name_a.into(),
            tx: tx_b,
            rx: tokio::sync::Mutex::new(rx_a),
            link: Mutex::new(true),
        });
        let b = Arc::new(Self {
            name: name_b.into(),
            tx: tx_a,
            rx: tokio::sync::Mutex::new(rx_b),
            link: Mutex::new(true),
        });
        (a, b)
    }

    /// Set link up/down.
    pub fn set_link(&self, up: bool) {
        *self.link.lock() = up;
    }
}

#[async_trait]
impl EthernetPort for SimulatedEthernet {
    fn name(&self) -> &str {
        &self.name
    }

    async fn link_up(&self) -> bool {
        *self.link.lock()
    }

    async fn recv(&self) -> anyhow::Result<Bytes> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("ethernet channel closed"))
    }

    async fn send(&self, frame: Bytes) -> anyhow::Result<()> {
        self.tx
            .send(frame)
            .map_err(|_| anyhow::anyhow!("ethernet channel closed"))?;
        Ok(())
    }
}

/// Alias kept for docs/examples that refer to loopback Ethernet.
pub type LoopbackEthernet = SimulatedEthernet;
