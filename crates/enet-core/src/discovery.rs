//! Interface discovery helpers for ENET and LAN adapters.

use enet_protocol::magic::LINK_LOCAL_PREFIX;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Snapshot of a network interface useful for auto-detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// OS interface name (e.g. "eth0", "Ethernet 2").
    pub name: String,
    /// Optional friendly description.
    pub description: String,
    /// MAC address string.
    pub mac: String,
    /// Assigned IPv4 addresses.
    pub ipv4: Vec<IpAddr>,
    /// Whether the link is reported up.
    pub is_up: bool,
    /// Whether any address is in 169.254.0.0/16.
    pub has_link_local: bool,
}

/// Return true if `ip` is in 169.254.0.0/16.
pub fn looks_like_enet_subnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == LINK_LOCAL_PREFIX[0] && o[1] == LINK_LOCAL_PREFIX[1]
        }
        IpAddr::V6(_) => false,
    }
}

/// Detect candidate interfaces using `sysinfo` / OS enumeration.
///
/// On CI/Linux without real ENET hardware this still returns available NICs so
/// auto-detection logic and the simulator can be exercised.
pub fn detect_candidate_interfaces() -> Vec<InterfaceInfo> {
    // sysinfo 0.33 Networks API
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut out = Vec::new();
    for (name, data) in networks.list() {
        let mac = format!("{}", data.mac_address());
        // sysinfo does not always expose IP list uniformly across versions;
        // we fill what we can and rely on OS-specific enrichment later.
        let ipv4: Vec<IpAddr> = Vec::new();
        let has_link_local = ipv4.iter().copied().any(looks_like_enet_subnet);
        out.push(InterfaceInfo {
            name: name.clone(),
            description: String::new(),
            mac,
            ipv4,
            is_up: data.total_received() > 0 || data.total_transmitted() > 0,
            has_link_local,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Score an interface for likelihood of being the BMW ENET NIC.
pub fn score_enet_candidate(iface: &InterfaceInfo, preferred_name: &str) -> i32 {
    let mut score = 0;
    if !preferred_name.is_empty() && iface.name.eq_ignore_ascii_case(preferred_name) {
        score += 100;
    }
    if iface.has_link_local {
        score += 50;
    }
    let desc = iface.description.to_lowercase();
    let name = iface.name.to_lowercase();
    for needle in ["enet", "realtek", "usb", "ethernet"] {
        if desc.contains(needle) || name.contains(needle) {
            score += 5;
        }
    }
    // Deprioritize known virtual / tunnel adapters
    for needle in ["wintun", "tap", "vpn", "hyper-v", "vethernet", "docker", "wsll"] {
        if desc.contains(needle) || name.contains(needle) {
            score -= 40;
        }
    }
    if iface.is_up {
        score += 10;
    }
    score
}

/// Pick the best ENET candidate, if any.
pub fn pick_enet_interface(preferred: &str) -> Option<InterfaceInfo> {
    let mut ifaces = detect_candidate_interfaces();
    ifaces.sort_by_key(|i| std::cmp::Reverse(score_enet_candidate(i, preferred)));
    ifaces.into_iter().next().filter(|i| score_enet_candidate(i, preferred) > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn link_local_detection() {
        assert!(looks_like_enet_subnet(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(!looks_like_enet_subnet(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn scoring_prefers_name() {
        let a = InterfaceInfo {
            name: "Ethernet 3".into(),
            description: "USB ENET".into(),
            mac: "00:11:22:33:44:55".into(),
            ipv4: vec![IpAddr::V4(Ipv4Addr::new(169, 254, 5, 77))],
            is_up: true,
            has_link_local: true,
        };
        let b = InterfaceInfo {
            name: "Wi-Fi".into(),
            description: "Wireless".into(),
            mac: "aa:bb:cc:dd:ee:ff".into(),
            ipv4: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))],
            is_up: true,
            has_link_local: false,
        };
        assert!(score_enet_candidate(&a, "Ethernet 3") > score_enet_candidate(&b, "Ethernet 3"));
    }
}
