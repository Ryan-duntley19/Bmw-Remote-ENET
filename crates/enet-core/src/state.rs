//! Shared runtime state models for GUI and services.

use serde::{Deserialize, Serialize};

/// High-level connection state of the tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Not started.
    #[default]
    Stopped,
    /// Starting / binding sockets.
    Starting,
    /// Waiting for peer.
    WaitingForPeer,
    /// Tunnel established.
    Connected,
    /// Transient error; retrying.
    Reconnecting,
    /// Fatal error; requires user action.
    Failed,
}

/// Observed vehicle / ENET side state.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct VehicleState {
    /// Physical/link-level ENET up.
    pub link_up: bool,
    /// Recent diagnostic traffic or discovery response.
    pub awake: bool,
    /// Unix ms of last observed ENET activity.
    pub last_activity_ms: u64,
    /// Discovered vehicle gateway IP if known.
    pub discovered_ip: Option<String>,
    /// VIN if learned from DoIP announcement (optional).
    pub vin: Option<String>,
}

/// Aggregated gateway status published to the GUI / API.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayState {
    /// Tunnel connection state.
    pub connection: ConnectionState,
    /// Vehicle state.
    pub vehicle: VehicleState,
    /// Laptop/agent peer connected (from gateway POV).
    pub laptop_connected: bool,
    /// Gateway service running.
    pub gateway_running: bool,
    /// Human-readable status line.
    pub status_message: String,
    /// Last error string if any.
    pub last_error: Option<String>,
    /// Software version.
    pub version: String,
}

impl GatewayState {
    /// Create with version stamped.
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            status_message: "Stopped".into(),
            ..Default::default()
        }
    }
}
