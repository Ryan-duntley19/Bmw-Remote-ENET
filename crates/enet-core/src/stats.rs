//! Packet and connection statistics.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Thread-safe packet counters and RTT/loss estimators.
#[derive(Debug, Default)]
pub struct PacketStats {
    tx_packets: AtomicU64,
    rx_packets: AtomicU64,
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
    dropped: AtomicU64,
    reconnects: AtomicU64,
    errors: AtomicU64,
    seq_gaps: AtomicU64,
    inner: Mutex<StatsInner>,
}

#[derive(Debug)]
struct StatsInner {
    rtt_samples_ms: VecDeque<f64>,
    window_start: Instant,
    window_tx: u64,
    window_rx: u64,
    last_rx_seq: Option<u64>,
    expected_loss: u64,
}

impl Default for StatsInner {
    fn default() -> Self {
        Self {
            rtt_samples_ms: VecDeque::with_capacity(256),
            window_start: Instant::now(),
            window_tx: 0,
            window_rx: 0,
            last_rx_seq: None,
            expected_loss: 0,
        }
    }
}

/// Serializable snapshot for GUI / API / logs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatsSnapshot {
    /// Lifetime TX packets.
    pub tx_packets: u64,
    /// Lifetime RX packets.
    pub rx_packets: u64,
    /// Lifetime TX bytes.
    pub tx_bytes: u64,
    /// Lifetime RX bytes.
    pub rx_bytes: u64,
    /// Dropped packets.
    pub dropped: u64,
    /// Reconnect count.
    pub reconnects: u64,
    /// Error count.
    pub errors: u64,
    /// Sequence gaps observed.
    pub seq_gaps: u64,
    /// Packets/sec TX over recent window.
    pub tx_pps: f64,
    /// Packets/sec RX over recent window.
    pub rx_pps: f64,
    /// Bytes/sec TX.
    pub tx_bps: f64,
    /// Bytes/sec RX.
    pub rx_bps: f64,
    /// Last RTT ms.
    pub rtt_ms: f64,
    /// RTT p50 ms.
    pub rtt_p50_ms: f64,
    /// RTT p99 ms.
    pub rtt_p99_ms: f64,
    /// Estimated loss rate 0.0–1.0.
    pub loss_rate: f64,
}

impl PacketStats {
    /// Create a new stats collector.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record an outgoing packet.
    pub fn record_tx(&self, bytes: usize) {
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        let mut inner = self.inner.lock();
        inner.window_tx = inner.window_tx.saturating_add(1);
    }

    /// Record an incoming packet and optional sequence for loss detection.
    pub fn record_rx(&self, bytes: usize, sequence: Option<u64>) {
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        let mut inner = self.inner.lock();
        inner.window_rx = inner.window_rx.saturating_add(1);
        if let Some(seq) = sequence {
            if let Some(prev) = inner.last_rx_seq {
                if seq > prev + 1 {
                    let gap = seq - prev - 1;
                    self.seq_gaps.fetch_add(gap, Ordering::Relaxed);
                    inner.expected_loss = inner.expected_loss.saturating_add(gap);
                }
            }
            inner.last_rx_seq = Some(seq);
        }
    }

    /// Record RTT sample in milliseconds.
    pub fn record_rtt_ms(&self, rtt_ms: f64) {
        let mut inner = self.inner.lock();
        if inner.rtt_samples_ms.len() >= 256 {
            inner.rtt_samples_ms.pop_front();
        }
        inner.rtt_samples_ms.push_back(rtt_ms.max(0.0));
    }

    /// Increment dropped counter.
    pub fn record_drop(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment reconnect counter.
    pub fn record_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment error counter.
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Produce a snapshot, resetting the rate window.
    pub fn snapshot(&self) -> StatsSnapshot {
        let mut inner = self.inner.lock();
        let elapsed = inner.window_start.elapsed().as_secs_f64().max(0.001);
        let tx_pps = inner.window_tx as f64 / elapsed;
        let rx_pps = inner.window_rx as f64 / elapsed;
        let tx_bytes = self.tx_bytes.load(Ordering::Relaxed);
        let rx_bytes = self.rx_bytes.load(Ordering::Relaxed);
        // Approximate bps from lifetime / uptime of window is coarse; good enough for UI.
        let tx_bps = (inner.window_tx as f64 * 800.0) / elapsed; // assume ~100B avg if unknown
        let rx_bps = (inner.window_rx as f64 * 800.0) / elapsed;

        let mut samples: Vec<f64> = inner.rtt_samples_ms.iter().copied().collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rtt_ms = samples.last().copied().unwrap_or(0.0);
        let rtt_p50 = percentile(&samples, 0.50);
        let rtt_p99 = percentile(&samples, 0.99);
        let rx = self.rx_packets.load(Ordering::Relaxed).max(1);
        let loss_rate = inner.expected_loss as f64 / (rx as f64 + inner.expected_loss as f64);

        inner.window_start = Instant::now();
        inner.window_tx = 0;
        inner.window_rx = 0;

        StatsSnapshot {
            tx_packets: self.tx_packets.load(Ordering::Relaxed),
            rx_packets: self.rx_packets.load(Ordering::Relaxed),
            tx_bytes,
            rx_bytes,
            dropped: self.dropped.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            seq_gaps: self.seq_gaps.load(Ordering::Relaxed),
            tx_pps,
            rx_pps,
            tx_bps,
            rx_bps,
            rtt_ms,
            rtt_p50_ms: rtt_p50,
            rtt_p99_ms: rtt_p99,
            loss_rate,
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Helper to measure elapsed wall time.
pub fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Duration helper for reconnect backoff.
pub fn backoff_delay(base_ms: u64, max_ms: u64, attempt: u32) -> Duration {
    let mult = 1u64 << attempt.min(10);
    Duration::from_millis((base_ms.saturating_mul(mult)).min(max_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_and_rtt() {
        let s = PacketStats::new();
        s.record_rx(100, Some(1));
        s.record_rx(100, Some(2));
        s.record_rx(100, Some(5)); // gap of 2
        s.record_rtt_ms(1.5);
        s.record_rtt_ms(2.0);
        let snap = s.snapshot();
        assert_eq!(snap.seq_gaps, 2);
        assert!(snap.loss_rate > 0.0);
        assert!(snap.rtt_p50_ms > 0.0);
    }

    #[test]
    fn backoff_grows() {
        assert!(backoff_delay(500, 10_000, 0) < backoff_delay(500, 10_000, 3));
        assert_eq!(backoff_delay(500, 1000, 20), Duration::from_millis(1000));
    }
}
