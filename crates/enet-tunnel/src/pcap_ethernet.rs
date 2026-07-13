//! Windows Npcap/WinPcap Ethernet port for real L2 capture/inject.
//!
//! Used by:
//! - Client: physical ENET (OBD) adapter toward the car
//! - Host: virtual `BMW-ENET` loopback adapter toward ISTA / E-Sys

#![cfg(windows)]

use crate::ethernet::EthernetPort;
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Live Npcap/WinPcap Ethernet port backed by a capture thread.
pub struct PcapEthernet {
    name: String,
    display: String,
    link: AtomicBool,
    /// Frames captured from the wire → tunnel.
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Bytes>>,
    /// Frames from tunnel → wire (blocking send on worker).
    tx_wire: Mutex<Option<mpsc::UnboundedSender<Bytes>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for PcapEthernet {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        *self.tx_wire.lock() = None;
    }
}

impl PcapEthernet {
    /// Open a device by Windows adapter name or description substring.
    ///
    /// `want` examples: `"Ethernet 2"`, `"BMW-ENET"`, `"Realtek"`.
    pub fn open(want: &str) -> anyhow::Result<Arc<Self>> {
        let devices = pcap::Device::list().map_err(|e| {
            anyhow::anyhow!(
                "Npcap/WinPcap not available ({e}). Install Npcap from https://npcap.com \
                 (check “WinPcap API compatibility”)."
            )
        })?;
        let want_l = want.to_lowercase();
        let dev = devices
            .into_iter()
            .find(|d| {
                let name_l = d.name.to_lowercase();
                let desc_l = d
                    .desc
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase();
                name_l == want_l
                    || desc_l == want_l
                    || name_l.contains(&want_l)
                    || desc_l.contains(&want_l)
                    || want_l.contains(&name_l)
            })
            .ok_or_else(|| {
                let listed = pcap::Device::list()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|d| {
                        format!(
                            "{} ({})",
                            d.name,
                            d.desc.unwrap_or_else(|| "no desc".into())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                anyhow::anyhow!(
                    "No Npcap device matching '{want}'. Available: {listed}"
                )
            })?;

        let if_name = dev.name.clone();
        let display = dev
            .desc
            .clone()
            .unwrap_or_else(|| if_name.clone());

        let cap = pcap::Capture::from_device(dev)
            .map_err(|e| anyhow::anyhow!("open device: {e}"))?
            .promisc(true)
            .immediate_mode(true)
            .timeout(50)
            .open()
            .map_err(|e| anyhow::anyhow!("pcap open: {e}"))?;

        // Separate capture handle for inject (pcap Capture isn't Sync).
        let inject = pcap::Capture::from_device(
            pcap::Device::list()
                .ok()
                .and_then(|list| {
                    list.into_iter().find(|d| d.name == if_name)
                })
                .ok_or_else(|| anyhow::anyhow!("device vanished after open"))?,
        )
        .map_err(|e| anyhow::anyhow!("reopen for inject: {e}"))?
        .promisc(true)
        .immediate_mode(true)
        .timeout(50)
        .open()
        .map_err(|e| anyhow::anyhow!("pcap inject open: {e}"))?;

        let (tx_cap, rx_cap) = mpsc::unbounded_channel::<Bytes>();
        let (tx_inj, mut rx_inj) = mpsc::unbounded_channel::<Bytes>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_cap = stop.clone();
        let stop_inj = stop.clone();

        // Capture thread: wire → channel.
        thread::Builder::new()
            .name(format!("pcap-rx-{if_name}"))
            .spawn(move || {
                let mut cap = cap;
                while !stop_cap.load(Ordering::Relaxed) {
                    match cap.next_packet() {
                        Ok(pkt) => {
                            if pkt.data.len() >= 14 {
                                let _ = tx_cap.send(Bytes::copy_from_slice(pkt.data));
                            }
                        }
                        Err(pcap::Error::TimeoutExpired) => continue,
                        Err(e) => {
                            if stop_cap.load(Ordering::Relaxed) {
                                break;
                            }
                            warn!(error = %e, "pcap capture error");
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
            })
            .map_err(|e| anyhow::anyhow!("spawn capture thread: {e}"))?;

        // Inject thread: channel → wire.
        thread::Builder::new()
            .name(format!("pcap-tx-{if_name}"))
            .spawn(move || {
                let mut inj = inject;
                while !stop_inj.load(Ordering::Relaxed) {
                    match rx_inj.blocking_recv() {
                        Some(frame) => {
                            if let Err(e) = inj.sendpacket(frame.as_ref()) {
                                warn!(error = %e, "pcap inject failed");
                            }
                        }
                        None => break,
                    }
                }
            })
            .map_err(|e| anyhow::anyhow!("spawn inject thread: {e}"))?;

        info!(%if_name, %display, "Npcap Ethernet port open");
        Ok(Arc::new(Self {
            name: if_name,
            display,
            link: AtomicBool::new(true),
            rx: tokio::sync::Mutex::new(rx_cap),
            tx_wire: Mutex::new(Some(tx_inj)),
            stop,
        }))
    }

    /// Human description (Npcap desc).
    pub fn display_name(&self) -> &str {
        &self.display
    }

    /// List Npcap devices as `"name|description"` lines (for diagnostics).
    pub fn list_devices() -> anyhow::Result<Vec<String>> {
        let devices = pcap::Device::list().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(devices
            .into_iter()
            .map(|d| {
                format!(
                    "{}|{}",
                    d.name,
                    d.desc.unwrap_or_default()
                )
            })
            .collect())
    }

    /// True if Npcap/WinPcap appears loadable.
    pub fn npcap_available() -> bool {
        pcap::Device::list().is_ok()
    }
}

#[async_trait]
impl EthernetPort for PcapEthernet {
    fn name(&self) -> &str {
        &self.name
    }

    async fn link_up(&self) -> bool {
        self.link.load(Ordering::Relaxed)
    }

    async fn recv(&self) -> anyhow::Result<Bytes> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("pcap capture closed"))
    }

    async fn send(&self, frame: Bytes) -> anyhow::Result<()> {
        let guard = self.tx_wire.lock();
        let tx = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("pcap inject closed"))?;
        tx.send(frame)
            .map_err(|_| anyhow::anyhow!("pcap inject closed"))?;
        Ok(())
    }
}
