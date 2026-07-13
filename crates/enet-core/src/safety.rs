//! Flash-safety evaluation — never auto-modify vehicle data.

use crate::config::GatewayConfig;
use crate::stats::StatsSnapshot;
use crate::state::VehicleState;
use serde::{Deserialize, Serialize};

/// Thresholds for declaring a connection flash-safe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyThresholds {
    /// Maximum acceptable RTT p99 in milliseconds.
    pub max_rtt_p99_ms: f64,
    /// Maximum acceptable loss rate (0.0–1.0).
    pub max_loss_rate: f64,
    /// Maximum host CPU percent.
    pub max_cpu_pct: f64,
    /// Minimum samples before trusting metrics.
    pub min_rtt_samples_hint: u32,
}

impl From<&GatewayConfig> for SafetyThresholds {
    fn from(cfg: &GatewayConfig) -> Self {
        Self {
            max_rtt_p99_ms: cfg.safety_rtt_p99_ms,
            max_loss_rate: cfg.safety_max_loss_rate,
            max_cpu_pct: cfg.safety_max_cpu_pct,
            min_rtt_samples_hint: 20,
        }
    }
}

/// Result of a flash-safety check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlashSafetyReport {
    /// True only when all checks pass.
    pub safe: bool,
    /// Human-readable failure reasons (empty when safe).
    pub reasons: Vec<String>,
    /// Echo of measured RTT p99.
    pub rtt_p99_ms: f64,
    /// Echo of measured loss rate.
    pub loss_rate: f64,
    /// Echo of CPU percent.
    pub cpu_pct: f64,
    /// Whether vehicle link is up.
    pub vehicle_link: bool,
    /// Whether vehicle appears awake.
    pub vehicle_awake: bool,
    /// Strong warning text for UI.
    pub warning: String,
}

impl Default for FlashSafetyReport {
    fn default() -> Self {
        Self {
            safe: false,
            reasons: vec!["No status yet".into()],
            rtt_p99_ms: 0.0,
            loss_rate: 0.0,
            cpu_pct: 0.0,
            vehicle_link: false,
            vehicle_awake: false,
            warning: "Waiting for gateway API".into(),
        }
    }
}

/// Evaluates whether ECU flashing should be allowed.
#[derive(Debug, Clone)]
pub struct FlashSafetyChecker {
    thresholds: SafetyThresholds,
}

impl FlashSafetyChecker {
    /// Create with explicit thresholds.
    pub fn new(thresholds: SafetyThresholds) -> Self {
        Self { thresholds }
    }

    /// Evaluate current stats + vehicle state + host CPU.
    pub fn evaluate(
        &self,
        stats: &StatsSnapshot,
        vehicle: &VehicleState,
        cpu_pct: f64,
        peer_connected: bool,
    ) -> FlashSafetyReport {
        let mut reasons = Vec::new();

        if !peer_connected {
            reasons.push("Tunnel peer is not connected".into());
        }
        if !vehicle.link_up {
            reasons.push("Vehicle ENET link is down".into());
        }
        if !vehicle.awake {
            reasons.push("Vehicle appears asleep (no recent ENET activity)".into());
        }
        if stats.rtt_p99_ms > self.thresholds.max_rtt_p99_ms {
            reasons.push(format!(
                "RTT p99 {:.2} ms exceeds limit {:.2} ms",
                stats.rtt_p99_ms, self.thresholds.max_rtt_p99_ms
            ));
        }
        if stats.loss_rate > self.thresholds.max_loss_rate {
            reasons.push(format!(
                "Packet loss {:.4}% exceeds limit {:.4}%",
                stats.loss_rate * 100.0,
                self.thresholds.max_loss_rate * 100.0
            ));
        }
        if cpu_pct > self.thresholds.max_cpu_pct {
            reasons.push(format!(
                "Host CPU {:.1}% exceeds limit {:.1}%",
                cpu_pct, self.thresholds.max_cpu_pct
            ));
        }
        if stats.rx_packets < self.thresholds.min_rtt_samples_hint as u64 {
            reasons.push("Insufficient traffic samples to certify link quality".into());
        }

        let safe = reasons.is_empty();
        let warning = if safe {
            "Connection quality is within flash-safe thresholds. Proceed only if you accept the risk of remote flashing.".into()
        } else {
            format!(
                "FLASHING NOT RECOMMENDED: {}. Do not start ECU programming until these are resolved.",
                reasons.join("; ")
            )
        };

        FlashSafetyReport {
            safe,
            reasons,
            rtt_p99_ms: stats.rtt_p99_ms,
            loss_rate: stats.loss_rate,
            cpu_pct,
            vehicle_link: vehicle.link_up,
            vehicle_awake: vehicle.awake,
            warning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::StatsSnapshot;

    fn good_stats() -> StatsSnapshot {
        StatsSnapshot {
            tx_packets: 1000,
            rx_packets: 1000,
            tx_bytes: 100_000,
            rx_bytes: 100_000,
            dropped: 0,
            reconnects: 0,
            errors: 0,
            seq_gaps: 0,
            tx_pps: 100.0,
            rx_pps: 100.0,
            tx_bps: 80_000.0,
            rx_bps: 80_000.0,
            rtt_ms: 1.0,
            rtt_p50_ms: 1.0,
            rtt_p99_ms: 2.0,
            loss_rate: 0.0,
        }
    }

    #[test]
    fn safe_when_healthy() {
        let checker = FlashSafetyChecker::new(SafetyThresholds {
            max_rtt_p99_ms: 20.0,
            max_loss_rate: 0.001,
            max_cpu_pct: 80.0,
            min_rtt_samples_hint: 20,
        });
        let vehicle = VehicleState {
            link_up: true,
            awake: true,
            last_activity_ms: 0,
            discovered_ip: Some("169.254.5.77".into()),
            vin: None,
        };
        let report = checker.evaluate(&good_stats(), &vehicle, 10.0, true);
        assert!(report.safe);
        assert!(report.reasons.is_empty());
    }

    #[test]
    fn unsafe_on_loss() {
        let checker = FlashSafetyChecker::new(SafetyThresholds {
            max_rtt_p99_ms: 20.0,
            max_loss_rate: 0.001,
            max_cpu_pct: 80.0,
            min_rtt_samples_hint: 20,
        });
        let mut stats = good_stats();
        stats.loss_rate = 0.05;
        let vehicle = VehicleState {
            link_up: true,
            awake: true,
            last_activity_ms: 0,
            discovered_ip: None,
            vin: None,
        };
        let report = checker.evaluate(&stats, &vehicle, 10.0, true);
        assert!(!report.safe);
        assert!(!report.reasons.is_empty());
    }
}
