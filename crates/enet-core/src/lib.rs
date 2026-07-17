//! Shared core for BMW ENET gateway components.

#![deny(missing_docs)]

pub mod config;
pub mod discovery;
pub mod health;
pub mod lan_discovery;
pub mod logging;
pub mod npcap;
pub mod safety;
pub mod stats;
pub mod state;
pub mod updater;

pub use config::{GatewayConfig, LogLevel, NetworkMode, Role};
pub use discovery::{
    InterfaceInfo, adapter_link_up, detect_candidate_interfaces, looks_like_enet_subnet,
    pick_enet_interface, score_enet_candidate,
};
pub use health::HealthMonitor;
pub use lan_discovery::{
    DiscoveredGateway, DiscoveryMessage, discover_gateways, generate_pair_code, list_lan_ipv4s,
    pick_reachable_host_ip, run_gateway_beacon,
};
pub use logging::init_logging;
pub use npcap::{ensure_npcap_installed, npcap_installed, NPCAP_INSTALLER_URL};
pub use safety::{FlashSafetyChecker, FlashSafetyReport, SafetyThresholds};
pub use stats::PacketStats;
pub use state::{ConnectionState, GatewayState, VehicleState};
