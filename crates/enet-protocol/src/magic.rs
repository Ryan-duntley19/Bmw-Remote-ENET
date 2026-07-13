//! BMW ENET well-known constants.

/// HSFZ diagnostic TCP port (F-Series primary).
pub const BMW_HSFZ_PORT: u16 = 6801;

/// HSFZ vehicle discovery UDP port.
pub const BMW_HSFZ_DISCOVERY_PORT: u16 = 6811;

/// DoIP (ISO 13400) TCP/UDP port.
pub const BMW_DOIP_PORT: u16 = 13400;

/// IPv4 link-local prefix used by BMW ENET (169.254.0.0/16).
pub const LINK_LOCAL_PREFIX: [u8; 2] = [169, 254];

/// Default recommended tester IPv4 on the ENET side.
pub const DEFAULT_TESTER_IP: [u8; 4] = [169, 254, 1, 1];

/// Default subnet mask for ENET link-local.
pub const DEFAULT_TESTER_MASK: [u8; 4] = [255, 255, 0, 0];

/// Default gateway tunnel UDP port on the desktop.
pub const DEFAULT_TUNNEL_PORT: u16 = 47900;

/// Default control / status HTTP API port on the desktop.
pub const DEFAULT_API_PORT: u16 = 47901;

/// Default laptop Client status page port (avoids colliding with Host :47901).
pub const DEFAULT_AGENT_API_PORT: u16 = 47903;

/// UDP port for LAN gateway auto-discovery beacons.
pub const DEFAULT_DISCOVERY_PORT: u16 = 47902;

/// Default TCP port for the Internet relay (both peers dial out).
pub const DEFAULT_RELAY_PORT: u16 = 47910;
