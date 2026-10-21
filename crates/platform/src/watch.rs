//! Signaux du système que le service surveille : route par défaut, horloge murale.

use std::net::{IpAddr, Ipv4Addr};
use std::time::SystemTime;

pub trait SystemWatch: Send {
    /// Route par défaut de la table main, ex. `"eth0 via 192.168.1.1"` ; `None` sans réseau.
    fn default_route(&self) -> Option<String>;
    /// Horloge murale : elle avance pendant la veille, contrairement à l'horloge monotone.
    fn wall_clock(&self) -> SystemTime;
}

/// Ligne `Destination 00000000` de `/proc/net/route` → `"iface via a.b.c.d"`.
pub fn parse_default_route(proc_net_route: &str) -> Option<String> {
    proc_net_route.lines().skip(1).find_map(|line| {
        let mut cols = line.split_whitespace();
        let iface = cols.next()?;
        if cols.next()? != "00000000" {
            return None;
        }
        // Adresse en hexadécimal, octets dans l'ordre de la machine (petit-boutiste).
        let gateway = u32::from_str_radix(cols.next()?, 16).ok()?;
        Some(format!(
            "{iface} via {}",
            Ipv4Addr::from(gateway.swap_bytes())
        ))
    })
}

/// Même format que Linux : `"eth0 via 192.168.1.1"`, ou le nom seul sans passerelle.
pub fn describe_route(interface: &str, gateway: Option<IpAddr>) -> String {
    match gateway {
        Some(g) => format!("{interface} via {g}"),
        None => interface.to_owned(),
    }
}

/// Première carte physique qui a une passerelle (ordre du système). Les cartes de tunnel
/// (`ankou*` d'aNkou, `utun*` de macOS) en sont exclues : la route par défaut passe par
/// le tunnel une fois connecté, ce qui ferait croire à un changement de réseau.
pub fn physical_route(interfaces: &[(String, Option<IpAddr>)]) -> Option<String> {
    interfaces
        .iter()
        .find(|(name, gw)| gw.is_some() && !name.starts_with("ankou") && !name.starts_with("utun"))
        .map(|(name, gw)| describe_route(name, *gw))
}

struct SysWatch;

impl SystemWatch for SysWatch {
    #[cfg(target_os = "linux")]
    fn default_route(&self) -> Option<String> {
        std::fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|t| parse_default_route(&t))
    }

    #[cfg(not(target_os = "linux"))]
    fn default_route(&self) -> Option<String> {
        let list: Vec<(String, Option<IpAddr>)> = netdev::get_interfaces()
            .into_iter()
            .map(|i| {
                let gateway = i
                    .gateway
                    .and_then(|g| g.ipv4.first().copied())
                    .map(IpAddr::V4);
                (i.friendly_name.unwrap_or(i.name), gateway)
            })
            .collect();
        physical_route(&list)
    }

    fn wall_clock(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Surveillance du système courant.
pub fn default_watch() -> Box<dyn SystemWatch> {
    Box::new(SysWatch)
}

#[cfg(test)]
mod tests {
    use super::{describe_route, physical_route};

    #[test]
    fn tunnel_interfaces_are_not_the_network() {
        let gw = |s: &str| s.parse().ok();
        let list = [
            ("ankou0".to_string(), gw("10.8.0.1")),
            ("utun100".to_string(), gw("10.8.0.1")),
            ("Wi-Fi".to_string(), gw("192.168.1.1")),
            ("Bluetooth".to_string(), None),
        ];
        assert_eq!(physical_route(&list), Some("Wi-Fi via 192.168.1.1".into()));
        assert_eq!(physical_route(&list[..2]), None);
    }

    #[test]
    fn describe_route_formats() {
        let gateway = "192.168.1.1".parse().ok();
        assert_eq!(
            describe_route("Ethernet", gateway),
            "Ethernet via 192.168.1.1"
        );
        assert_eq!(describe_route("Wi-Fi", None), "Wi-Fi");
    }
}
