//! Configuration loading and defaults.

use enet_protocol::magic::{
    DEFAULT_API_PORT, DEFAULT_DISCOVERY_PORT, DEFAULT_RELAY_PORT, DEFAULT_TUNNEL_PORT,
};
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

/// How the laptop and desktop reach each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// Same Wi‑Fi / Ethernet (UDP + LAN discovery).
    #[default]
    Lan,
    /// Different networks via outbound TCP relay (easiest remote).
    Relay,
    /// Different networks via WireGuard / Tailscale-style VPN overlay.
    Wireguard,
}

impl NetworkMode {
    /// Human label for UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Lan => "Same network (LAN)",
            Self::Relay => "Different networks (Relay)",
            Self::Wireguard => "Different networks (WireGuard / VPN)",
        }
    }

    /// Whether this mode typically traverses the Internet.
    pub fn is_remote(self) -> bool {
        !matches!(self, Self::Lan)
    }
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
    /// How peers reach each other.
    pub network_mode: NetworkMode,
    /// UDP tunnel listen/connect port (LAN / WireGuard modes).
    pub tunnel_port: u16,
    /// Optional bind address.
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
    /// Relay host:port for remote mode (both sides dial out).
    pub relay_url: String,
    /// Expected desktop WireGuard IP.
    pub wireguard_desktop_ip: String,
    /// Expected laptop WireGuard IP.
    pub wireguard_laptop_ip: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            role: Role::Gateway,
            network_mode: NetworkMode::Lan,
            tunnel_port: DEFAULT_TUNNEL_PORT,
            bind_addr: None,
            peer_addr: None,
            allowed_cidrs: vec![
                "192.168.0.0/16".into(),
                "10.0.0.0/8".into(),
                "172.16.0.0/12".into(),
                "10.66.0.0/24".into(),
                "100.64.0.0/10".into(),
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
            relay_url: String::new(),
            wireguard_desktop_ip: "10.66.0.1".into(),
            wireguard_laptop_ip: "10.66.0.2".into(),
        }
    }
}

impl GatewayConfig {
    /// Apply safer defaults when switching into a remote network mode.
    pub fn apply_remote_defaults(&mut self) {
        match self.network_mode {
            NetworkMode::Lan => {
                self.safety_rtt_p99_ms = 20.0;
                self.peer_timeout_ms = 5000;
            }
            NetworkMode::Relay => {
                self.auto_discover = false;
                self.require_crypto = true;
                self.safety_rtt_p99_ms = 80.0;
                self.peer_timeout_ms = 15_000;
                if self.relay_url.is_empty() {
                    self.relay_url = format!("127.0.0.1:{DEFAULT_RELAY_PORT}");
                }
            }
            NetworkMode::Wireguard => {
                self.auto_discover = false;
                self.require_crypto = true;
                self.safety_rtt_p99_ms = 40.0;
                self.peer_timeout_ms = 10_000;
                if self.role == Role::Agent && self.peer_addr.is_none() {
                    if let Ok(ip) = self.wireguard_desktop_ip.parse() {
                        self.peer_addr = Some(ip);
                    }
                }
            }
        }
    }

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
        match (self.role, self.network_mode) {
            (Role::Gateway, NetworkMode::Lan) => {
                hints.push("Mode: Same network.".into());
                hints.push(format!(
                    "1. Open http://127.0.0.1:{}/ and note the pair code.",
                    self.api_port
                ));
                hints.push(format!(
                    "2. On the laptop run the Agent (pair code {}).",
                    if self.pair_code.is_empty() {
                        "from dashboard"
                    } else {
                        &self.pair_code
                    }
                ));
                hints.push("3. Plug ENET into car + laptop, ignition ON.".into());
                hints.push("4. Open ISTA/E-Sys when the dashboard is green.".into());
            }
            (Role::Gateway, NetworkMode::Relay) => {
                hints.push("Mode: Different networks via Relay.".into());
                hints.push(format!(
                    "1. Run a relay: enet-relay --listen 0.0.0.0:{DEFAULT_RELAY_PORT} (on a VPS)."
                ));
                hints.push(format!("2. This PC dials relay: {}", self.relay_url));
                hints.push(format!(
                    "3. Laptop uses the same relay + pair code {}.",
                    if self.pair_code.is_empty() {
                        "(dashboard)"
                    } else {
                        &self.pair_code
                    }
                ));
                hints.push("4. Set a password — Internet paths require encryption.".into());
                hints.push("5. Prefer WireGuard for ECU flashing if latency is high.".into());
            }
            (Role::Gateway, NetworkMode::Wireguard) => {
                hints.push("Mode: WireGuard / VPN overlay.".into());
                hints.push(
                    "1. Import config/wireguard-desktop.conf in WireGuard and activate.".into(),
                );
                hints.push(format!(
                    "2. Desktop WG IP should be {}.",
                    self.wireguard_desktop_ip
                ));
                hints.push("3. Laptop dials that WG IP after its tunnel is up.".into());
            }
            (Role::Agent, NetworkMode::Lan) => {
                hints.push("Mode: Same network — auto-discover desktop.".into());
                hints.push("1. Same Wi‑Fi/Ethernet as the desktop.".into());
                hints.push("2. Start enet-agent (optional pair code).".into());
                hints.push("3. Plug ENET into car + this laptop.".into());
            }
            (Role::Agent, NetworkMode::Relay) => {
                hints.push("Mode: Different networks via Relay.".into());
                hints.push(format!("1. Relay: {}", self.relay_url));
                hints.push(format!(
                    "2. Pair code must match desktop ({})",
                    if self.pair_code.is_empty() {
                        "required"
                    } else {
                        &self.pair_code
                    }
                ));
                hints.push("3. Plug ENET; leave agent running.".into());
            }
            (Role::Agent, NetworkMode::Wireguard) => {
                hints.push("Mode: WireGuard / VPN.".into());
                hints.push("1. Import config/wireguard-laptop.conf and activate.".into());
                hints.push(format!(
                    "2. Agent connects to desktop at {:?}.",
                    self.peer_addr
                ));
                hints.push("3. Plug ENET; start enet-agent.".into());
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
        cfg.network_mode = NetworkMode::Relay;
        cfg.relay_url = "vps:47910".into();
        cfg.save(&path).unwrap();
        let loaded = GatewayConfig::load(&path).unwrap();
        assert_eq!(loaded.password, "secret");
        assert_eq!(loaded.network_mode, NetworkMode::Relay);
        assert_eq!(loaded.relay_url, "vps:47910");
    }

    #[test]
    fn remote_defaults_require_crypto() {
        let mut cfg = GatewayConfig::default();
        cfg.network_mode = NetworkMode::Relay;
        cfg.apply_remote_defaults();
        assert!(cfg.require_crypto);
        assert!(!cfg.auto_discover);
        assert!(cfg.safety_rtt_p99_ms >= 80.0);
    }
}
