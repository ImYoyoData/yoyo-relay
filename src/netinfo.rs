//! Local endpoint discovery for the UI.

use std::net::Ipv4Addr;

#[derive(Clone, Debug)]
pub struct LanAddress {
    pub name: String,
    pub ip: Ipv4Addr,
}

/// Non-loopback IPv4 addresses of physical/virtual adapters.
pub fn lan_addresses() -> Vec<LanAddress> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for interface in interfaces {
        if interface.is_loopback() {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = interface.addr else {
            continue;
        };
        let ip = v4.ip;
        if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
            continue;
        }
        out.push(LanAddress {
            name: interface.name,
            ip,
        });
    }
    out.sort_by_key(|address| address.ip);
    out.dedup_by_key(|address| address.ip);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_loopback_and_link_local() {
        for address in lan_addresses() {
            assert!(!address.ip.is_loopback());
            assert!(!address.ip.is_link_local());
            assert!(!address.ip.is_unspecified());
        }
    }
}
