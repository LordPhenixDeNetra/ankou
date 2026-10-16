//! Signaux du système que le service surveille : route par défaut, horloge murale.

use std::net::Ipv4Addr;
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

struct ProcWatch;

impl SystemWatch for ProcWatch {
    fn default_route(&self) -> Option<String> {
        std::fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|t| parse_default_route(&t))
    }

    fn wall_clock(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Surveillance du système courant (sans `/proc/net/route`, la route vaut `None`).
pub fn default_watch() -> Box<dyn SystemWatch> {
    Box::new(ProcWatch)
}
