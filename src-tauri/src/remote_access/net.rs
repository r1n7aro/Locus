//! Interface discovery and the per-request network checks.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ListenerKind {
    /// 127.0.0.1, plain HTTP, for local tunnel/proxy processes.
    Loopback,
    /// Private LAN address, HTTPS with the pinned certificate.
    Lan,
    /// Tailscale adapter address, HTTPS with the pinned certificate.
    Tailscale,
}

impl ListenerKind {
    pub fn secure(self) -> bool {
        !matches!(self, ListenerKind::Loopback)
    }

    /// Whether the peer address identifies one client. Behind the loopback
    /// listener every request comes from the local tunnel process, so
    /// per-peer caps and bans would throttle or lock out all tunnel users at
    /// once (and let anyone on the internet lock the owner out).
    pub fn per_peer_limits(self) -> bool {
        !matches!(self, ListenerKind::Loopback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceKind {
    Lan,
    Tailscale,
    /// Public or otherwise unsuitable addresses; shown but not selectable.
    Other,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    pub name: String,
    pub kind: InterfaceKind,
    pub addresses: Vec<String>,
}

/// IPv4 addresses grouped by adapter. Loopback and APIPA (169.254/16)
/// addresses are skipped: neither is reachable from a phone.
pub fn discover() -> Vec<NetworkInterface> {
    let interfaces = match if_addrs::get_if_addrs() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            eprintln!("[RemoteAccess] interface discovery failed: {error}");
            return Vec::new();
        }
    };
    let mut grouped: BTreeMap<String, Vec<Ipv4Addr>> = BTreeMap::new();
    for interface in interfaces {
        if interface.is_loopback() {
            continue;
        }
        let IpAddr::V4(ip) = interface.ip() else {
            continue;
        };
        if ip.is_link_local() || ip.is_unspecified() {
            continue;
        }
        let addresses = grouped.entry(interface.name.clone()).or_default();
        if !addresses.contains(&ip) {
            addresses.push(ip);
        }
    }
    grouped
        .into_iter()
        .map(|(name, addresses)| NetworkInterface {
            kind: classify(&name, &addresses),
            addresses: addresses.iter().map(ToString::to_string).collect(),
            name,
        })
        .collect()
}

pub fn classify(name: &str, addresses: &[Ipv4Addr]) -> InterfaceKind {
    if name.to_ascii_lowercase().contains("tailscale")
        && addresses.iter().all(|ip| is_tailscale_v4(*ip))
    {
        return InterfaceKind::Tailscale;
    }
    if !addresses.is_empty() && addresses.iter().all(|ip| ip.is_private()) {
        return InterfaceKind::Lan;
    }
    InterfaceKind::Other
}

/// Socket addresses to listen on for the given settings.
pub fn desired_binds(
    port: u16,
    loopback: bool,
    selected: &[String],
    interfaces: &[NetworkInterface],
) -> Vec<(SocketAddr, ListenerKind)> {
    let mut binds = Vec::new();
    if loopback {
        binds.push((SocketAddr::from((Ipv4Addr::LOCALHOST, port)), ListenerKind::Loopback));
    }
    for interface in interfaces.iter().filter(|interface| selected.contains(&interface.name)) {
        let kind = match interface.kind {
            InterfaceKind::Lan => ListenerKind::Lan,
            InterfaceKind::Tailscale => ListenerKind::Tailscale,
            InterfaceKind::Other => continue,
        };
        for address in &interface.addresses {
            if let Ok(ip) = address.parse::<Ipv4Addr>() {
                binds.push((SocketAddr::from((ip, port)), kind));
            }
        }
    }
    binds.sort();
    binds.dedup();
    binds
}

fn is_tailscale_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..128).contains(&b)
}

fn is_tailscale_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
}

fn is_private_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Which peers each listener accepts. A router forwarding the port from the
/// internet therefore still cannot reach a LAN listener.
pub fn peer_allowed(kind: ListenerKind, peer: IpAddr) -> bool {
    let peer = match peer {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(peer),
        other => other,
    };
    match kind {
        ListenerKind::Loopback => peer.is_loopback(),
        ListenerKind::Lan => is_private_peer(peer),
        ListenerKind::Tailscale => match peer {
            IpAddr::V4(ip) => is_tailscale_v4(ip),
            IpAddr::V6(ip) => is_tailscale_v6(ip),
        },
    }
}

/// Host must be an IP literal (any port: tunnels such as UU map the service
/// to arbitrary local ports) or a registered hostname. Anything else is a
/// DNS-rebinding attempt or a misrouted request.
pub fn host_allowed(host_header: Option<&str>, hostnames: &[String]) -> bool {
    let Some(host) = host_header.map(str::trim).filter(|host| !host.is_empty()) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let bare = if let Some(rest) = host.strip_prefix('[') {
        let Some((inside, _)) = rest.split_once(']') else {
            return false;
        };
        return inside.parse::<Ipv6Addr>().is_ok();
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) if port.parse::<u16>().is_ok() => name.to_string(),
            Some(_) => return false,
            None => host.clone(),
        }
    };
    if bare.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    hostnames.iter().any(|registered| {
        let registered_host = registered.split(':').next().unwrap_or(registered);
        registered_host == bare
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_uses_adapter_name_for_tailscale() {
        let ts: Ipv4Addr = "100.101.102.103".parse().unwrap();
        let lan: Ipv4Addr = "192.168.1.20".parse().unwrap();
        let public: Ipv4Addr = "8.8.8.8".parse().unwrap();
        assert_eq!(classify("Tailscale", &[ts]), InterfaceKind::Tailscale);
        // Carrier-grade NAT shares the range; without the adapter name it is not Tailscale.
        assert_eq!(classify("Ethernet", &[ts]), InterfaceKind::Other);
        assert_eq!(classify("Wi-Fi", &[lan]), InterfaceKind::Lan);
        assert_eq!(classify("WAN", &[public]), InterfaceKind::Other);
    }

    #[test]
    fn binds_follow_selection_and_skip_other_interfaces() {
        let interfaces = vec![
            NetworkInterface { name: "Wi-Fi".into(), kind: InterfaceKind::Lan, addresses: vec!["192.168.1.20".into()] },
            NetworkInterface { name: "Tailscale".into(), kind: InterfaceKind::Tailscale, addresses: vec!["100.90.1.2".into()] },
            NetworkInterface { name: "WAN".into(), kind: InterfaceKind::Other, addresses: vec!["8.8.4.4".into()] },
        ];
        let binds = desired_binds(47831, true, &["Wi-Fi".into(), "Tailscale".into(), "WAN".into()], &interfaces);
        let kinds: Vec<_> = binds.iter().map(|(addr, kind)| (addr.to_string(), *kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("100.90.1.2:47831".to_string(), ListenerKind::Tailscale),
                ("127.0.0.1:47831".to_string(), ListenerKind::Loopback),
                ("192.168.1.20:47831".to_string(), ListenerKind::Lan),
            ]
        );
        assert!(desired_binds(47831, false, &[], &interfaces).is_empty());
    }

    #[test]
    fn peers_are_filtered_per_listener() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(peer_allowed(ListenerKind::Loopback, ip("127.0.0.1")));
        assert!(!peer_allowed(ListenerKind::Loopback, ip("192.168.1.5")));
        assert!(peer_allowed(ListenerKind::Lan, ip("192.168.1.5")));
        assert!(peer_allowed(ListenerKind::Lan, ip("10.1.2.3")));
        assert!(peer_allowed(ListenerKind::Lan, ip("::ffff:172.16.0.9")));
        assert!(!peer_allowed(ListenerKind::Lan, ip("203.0.113.7")));
        assert!(!peer_allowed(ListenerKind::Lan, ip("100.64.0.1")));
        assert!(peer_allowed(ListenerKind::Tailscale, ip("100.64.0.1")));
        assert!(peer_allowed(ListenerKind::Tailscale, ip("fd7a:115c:a1e0::1")));
        assert!(!peer_allowed(ListenerKind::Tailscale, ip("192.168.1.5")));
    }

    #[test]
    fn hosts_must_be_ip_literals_or_registered() {
        let names = vec!["pc.tail1.ts.net".to_string(), "box.example.net:8443".to_string()];
        assert!(host_allowed(Some("192.168.1.20:47831"), &names));
        assert!(host_allowed(Some("127.0.0.1:13306"), &names));
        assert!(host_allowed(Some("[::1]:47831"), &names));
        assert!(host_allowed(Some("PC.tail1.ts.net"), &names));
        assert!(host_allowed(Some("box.example.net:8443"), &names));
        assert!(!host_allowed(Some("evil.example.com"), &names));
        assert!(!host_allowed(Some("localhost:47831"), &names));
        assert!(!host_allowed(Some("192.168.1.20:bad"), &names));
        assert!(!host_allowed(None, &names));
    }
}
