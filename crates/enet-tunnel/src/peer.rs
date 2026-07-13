//! Peer allowlisting helpers.

use std::net::{IpAddr, SocketAddr};

/// Peer address with optional CIDR allowlist checks.
#[derive(Debug, Clone)]
pub struct PeerAddr {
    /// Socket address of the peer.
    pub addr: SocketAddr,
}

/// Parse simple IPv4 CIDR like "192.168.0.0/16". Returns (network, prefix_len).
pub fn parse_cidr(cidr: &str) -> Option<(IpAddr, u8)> {
    let (ip, prefix) = cidr.split_once('/')?;
    let ip: IpAddr = ip.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    Some((ip, prefix))
}

/// Return true if `ip` is contained in any of the CIDR strings.
pub fn ip_allowed(ip: IpAddr, cidrs: &[String]) -> bool {
    if cidrs.is_empty() {
        return true;
    }
    for c in cidrs {
        if let Some((network, prefix)) = parse_cidr(c) {
            if ip_in_cidr(ip, network, prefix) {
                return true;
            }
        } else if let Ok(single) = c.parse::<IpAddr>() {
            if single == ip {
                return true;
            }
        }
    }
    false
}

fn ip_in_cidr(ip: IpAddr, network: IpAddr, prefix: u8) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(net)) => {
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 {
                0u32
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(ip) & mask) == (u32::from(net) & mask)
        }
        (IpAddr::V6(ip), IpAddr::V6(net)) => {
            if prefix > 128 {
                return false;
            }
            let ip = u128::from(ip);
            let net = u128::from(net);
            let mask = if prefix == 0 {
                0u128
            } else {
                u128::MAX << (128 - prefix)
            };
            (ip & mask) == (net & mask)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn cidr_match() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50));
        assert!(ip_allowed(ip, &["192.168.0.0/16".into()]));
        assert!(!ip_allowed(ip, &["10.0.0.0/8".into()]));
        assert!(ip_allowed(ip, &["192.168.1.50".into()]));
    }
}
