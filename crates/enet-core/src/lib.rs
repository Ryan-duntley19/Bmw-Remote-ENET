//! Shared core for BMW ENET gateway components.

#![deny(missing_docs)]

pub mod config;
pub mod discovery;
pub mod health;
pub mod lan_discovery;
pub mod logging;
pub mod safety;
pub mod stats;
pub mod state;

pub use config::{GatewayConfig, LogLevel, NetworkMode, Role};
pub use discovery::{InterfaceInfo, detect_candidate_interfaces, looks_like_enet_subnet};
pub use health::HealthMonitor;
pub use lan_discovery::{
    DiscoveredGateway, DiscoveryMessage, discover_gateways, generate_pair_code, run_gateway_beacon,
};
pub use logging::init_logging;
pub use safety::{FlashSafetyChecker, FlashSafetyReport, SafetyThresholds};
pub use stats::PacketStats;
pub use state::{ConnectionState, GatewayState, VehicleState};
