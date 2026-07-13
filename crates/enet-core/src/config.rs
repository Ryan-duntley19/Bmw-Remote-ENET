//! Configuration loading and defaults.

use enet_protocol::magic::{DEFAULT_API_PORT, DEFAULT_DISCOVERY_PORT, DEFAULT_TUNNEL_PORT};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Process role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Laptop connected to the vehicle ENET cable.
    Agent,
    /// Desktop running diagnostic tools.
    Gateway,
}

/// Logging verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Error only.
    Error,
    /// Warnings+.
    Warn,
    /// Informational (default).
    #[default]
    Info,
    /// Debug detail.
    Debug,
    /// Trace everything (including packet meta).
    Trace,
}

impl LogLevel {
    /// Convert to tracing filter string.
    pub fn as_filter(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// Top-level configuration shared by agent, gateway, and GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    /// Role of this process.
    pub role: Role,
    /// UDP tunnel listen/connect port.
    pub tunnel_port: u16,
    /// Optional bind address (default all interfaces for gateway, or peer for agent).
    pub bind_addr: Option<IpAddr>,
    /// Peer address (optional when auto_discover is enabled on the agent).
    pub peer_addr: Option<IpAddr>,
    /// Allowed CIDR strings for peer connections (gateway).
    pub allowed_cidrs: Vec<String>,
    /// Preferred ENET / LAN interface name (empty = auto).
    pub enet_interface: String,
    /// Preferred LAN interface for tunnel (empty = auto).
    pub lan_interface: String,
    /// Virtual TAP/Wintun interface name on gateway.
    pub virtual_interface: String,
    /// Tester IP to assign on gateway virtual NIC.
    pub tester_ip: String,
    /// Tester subnet mask.
    pub tester_mask: String,
    /// Optional PSK password (empty = no encryption).
    pub password: String,
    /// Require encryption.
    pub require_crypto: bool,
    /// Enable TLS for control API (future).
    pub tls_enabled: bool,
    /// Auto-start gateway/agent on boot.
    pub auto_start: bool,
    /// Reconnect delay milliseconds (base).
    pub reconnect_delay_ms: u64,
    /// Max reconnect delay milliseconds.
    pub reconnect_delay_max_ms: u64,
    /// Keepalive interval milliseconds.
    pub keepalive_interval_ms: u64,
    /// Peer timeout milliseconds.
    pub peer_timeout_ms: u64,
    /// Logging level.
    pub log_level: LogLevel,
    /// Directory for log files.
    pub log_dir: PathBuf,
    /// Control API bind port.
    pub api_port: u16,
    /// Enable Windows firewall rule management.
    pub manage_firewall: bool,
    /// Flash safety RTT p99 threshold (ms).
    pub safety_rtt_p99_ms: f64,
    /// Flash safety max loss rate (0.0–1.0).
    pub safety_max_loss_rate: f64,
    /// Flash safety max CPU percent.
    pub safety_max_cpu_pct: f64,
    /// Automatically discover the desktop gateway on the LAN (agent).
    pub auto_discover: bool,
    /// UDP discovery / beacon port.
    pub discovery_port: u16,
    /// Short pair code shown on the desktop; leave empty to auto-generate.
    pub pair_code: String,
    /// True after first-run setup wizard completes.
    pub setup_complete: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            role: Role::Gateway,
            tunnel_port: DEFAULT_TUNNEL_PORT,
            bind_addr: None,
            peer_addr: None,
            allowed_cidrs: vec![
                "192.168.0.0/16".into(),
                "10.0.0.0/8".into(),
                "172.16.0.0/12".into(),
            ],
            enet_interface: String::new(),
            lan_interface: String::new(),
            virtual_interface: "BMW-ENET".into(),
            tester_ip: "169.254.1.1".into(),
            tester_mask: "255.255.0.0".into(),
            password: String::new(),
            require_crypto: false,
            tls_enabled: false,
            auto_start: true,
            reconnect_delay_ms: 500,
            reconnect_delay_max_ms: 10_000,
            keepalive_interval_ms: 1000,
            peer_timeout_ms: 5000,
            log_level: LogLevel::Info,
            log_dir: PathBuf::from("logs"),
            api_port: DEFAULT_API_PORT,
            manage_firewall: true,
            safety_rtt_p99_ms: 20.0,
            safety_max_loss_rate: 0.001,
            safety_max_cpu_pct: 80.0,
            auto_discover: true,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            pair_code: String::new(),
            setup_complete: false,
        }
    }
}

impl GatewayConfig {
    /// Load TOML config from disk, or defaults if missing.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::warn!(?path, "config not found; using defaults");
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Save TOML config to disk.
    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Ensure a pair code exists (generates one if empty).
    pub fn ensure_pair_code(&mut self) -> &str {
        if self.pair_code.trim().is_empty() {
            self.pair_code = crate::lan_discovery::generate_pair_code();
        }
        &self.pair_code
    }

    /// Example config path used by documentation and installer.
    pub fn default_path_for(role: Role) -> PathBuf {
        match role {
            Role::Agent => PathBuf::from("config/agent.toml"),
            Role::Gateway => PathBuf::from("config/gateway.toml"),
        }
    }

    /// Plain-language setup checklist for UIs.
    pub fn setup_hints(&self) -> Vec<String> {
        let mut hints = Vec::new();
        match self.role {
            Role::Gateway => {
                hints.push("1. Keep this desktop on your home Wi‑Fi or Ethernet.".into());
                hints.push(format!(
                    "2. Open http://127.0.0.1:{}/ in a browser for the dashboard.",
                    self.api_port
                ));
                let code = if self.pair_code.is_empty() {
                    "(shown in dashboard)".to_string()
                } else {
                    self.pair_code.clone()
                };
                hints.push(format!(
                    "3. On the laptop, install the Agent and enter pair code {code} (or leave blank to auto-find)."
                ));
                hints.push(
                    "4. Plug the ENET cable into the laptop and the car, then turn ignition ON."
                        .into(),
                );
                hints.push(
                    "5. Launch ISTA / E-Sys on this desktop when the dashboard says Connected."
                        .into(),
                );
            }
            Role::Agent => {
                hints.push(
                    "1. Connect this laptop to the same Wi‑Fi/Ethernet as the desktop.".into(),
                );
                if self.auto_discover {
                    hints.push(
                        "2. Auto-discover is ON — the agent will find the desktop automatically."
                            .into(),
                    );
                } else if let Some(ip) = self.peer_addr {
                    hints.push(format!("2. Connecting to desktop at {ip}."));
                } else {
                    hints.push("2. Set peer_addr or enable auto_discover.".into());
                }
                hints.push("3. Plug ENET into the car OBD port and this laptop.".into());
            }
        }
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cfg.toml");
        let mut cfg = GatewayConfig::default();
        cfg.password = "secret".into();
        cfg.peer_addr = Some("192.168.1.50".parse().unwrap());
        cfg.save(&path).unwrap();
        let loaded = GatewayConfig::load(&path).unwrap();
        assert_eq!(loaded.password, "secret");
        assert_eq!(loaded.tunnel_port, DEFAULT_TUNNEL_PORT);
        assert!(loaded.auto_discover);
    }

    #[test]
    fn ensure_pair_code_generates() {
        let mut cfg = GatewayConfig::default();
        assert!(cfg.pair_code.is_empty());
        let code = cfg.ensure_pair_code().to_string();
        assert!(code.starts_with("BMW-"));
        assert_eq!(cfg.ensure_pair_code(), code);
    }
}
