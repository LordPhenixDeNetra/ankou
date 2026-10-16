//! WireGuard noyau Linux : configuration par netlink, routage par `ip` comme `wg-quick` (ADR 0002).

use crate::{PlatformError, Tunnel, TunnelStats, resolve_endpoint};
use ankou_proto::TunnelConfig;
use defguard_wireguard_rs::error::WireguardInterfaceError;
use defguard_wireguard_rs::key::Key;
use defguard_wireguard_rs::net::IpAddrMask;
use defguard_wireguard_rs::peer::Peer;
use defguard_wireguard_rs::{InterfaceConfiguration, Kernel, WGApi, WireguardInterfaceApi};

pub struct KernelTunnel {
    ifname: String,
}

impl KernelTunnel {
    pub fn new(ifname: &str) -> Self {
        Self {
            ifname: ifname.to_owned(),
        }
    }

    fn api(&self) -> Result<WGApi<Kernel>, PlatformError> {
        WGApi::<Kernel>::new(self.ifname.clone()).map_err(map_err)
    }

    fn exists(&self) -> bool {
        std::path::Path::new("/sys/class/net")
            .join(&self.ifname)
            .exists()
    }
}

fn mask(net: &ipnet::IpNet) -> IpAddrMask {
    IpAddrMask::new(net.addr(), net.prefix_len())
}

/// Traduit les erreurs de la bibliothèque (texte netlink) vers nos catégories.
fn map_err(e: WireguardInterfaceError) -> PlatformError {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if lower.contains("operation not permitted") || lower.contains("permission denied") {
        PlatformError::Permission
    } else if lower.contains("not supported") || lower.contains("kernelnotsupported") {
        PlatformError::Unavailable(text)
    } else {
        PlatformError::Backend(text)
    }
}

/// Table de routage et marque des paquets WireGuard (mêmes valeurs que `wg-quick`).
const FWMARK: u32 = 51820;

impl Tunnel for KernelTunnel {
    fn up(&mut self, cfg: &TunnelConfig) -> Result<(), PlatformError> {
        self.down()?;
        let endpoint = resolve_endpoint(&cfg.peer.endpoint)?;
        let mut peer = Peer::new(Key::new(*cfg.peer.public_key.as_bytes()));
        peer.endpoint = Some(endpoint);
        peer.persistent_keepalive_interval = Some(cfg.peer.persistent_keepalive);
        peer.allowed_ips = cfg.peer.allowed_ips.iter().map(mask).collect();
        let config = InterfaceConfiguration {
            name: self.ifname.clone(),
            prvkey: cfg.interface.private_key.to_base64(),
            addresses: cfg.interface.address.iter().map(mask).collect(),
            port: 0,
            peers: vec![peer],
            mtu: None,
            // Posé dès la configuration : la bibliothèque recréerait sinon le pair plus tard,
            // ce qui perd le handshake en cours.
            fwmark: Some(FWMARK),
        };
        if !has_net_admin() {
            return Err(PlatformError::Permission);
        }
        let mut api = self.api()?;
        // La bibliothèque perd le code d'erreur système à la création : avec les droits,
        // un échec veut dire que le module WireGuard manque.
        api.create_interface()
            .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
        if let Err(e) = api.configure_interface(&config) {
            let _ = self.down();
            return Err(map_err(e));
        }
        // Routage à la `wg-quick` (sa `configure_peer_routing` recrée le pair et laisse une
        // route vers l'endpoint dans la table main).
        for command in routing_commands(&cfg.peer.allowed_ips, &self.ifname) {
            if let Err(detail) = ip(&command.split(' ').collect::<Vec<_>>()) {
                let _ = self.down();
                return Err(PlatformError::Backend(format!("ip {command} : {detail}")));
            }
        }
        // Les paquets marqués doivent passer le filtrage par chemin inverse.
        let _ = std::fs::write("/proc/sys/net/ipv4/conf/all/src_valid_mark", "1");
        Ok(())
    }

    /// Comme `wg-quick down` : règles retirées, puis interface (et ses routes) supprimée.
    fn down(&mut self) -> Result<(), PlatformError> {
        for command in cleanup_commands() {
            let _ = ip(&command.split(' ').collect::<Vec<_>>());
        }
        if self.exists() {
            ip(&["link", "del", &self.ifname])
                .map_err(|detail| PlatformError::Backend(format!("ip link del : {detail}")))?;
        }
        Ok(())
    }

    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        let host = self.api()?.read_interface_data().map_err(map_err)?;
        let mut stats = TunnelStats::default();
        for peer in host.peers.values() {
            stats.rx_bytes += peer.rx_bytes;
            stats.tx_bytes += peer.tx_bytes;
            stats.last_handshake = stats.last_handshake.max(peer.last_handshake);
        }
        Ok(stats)
    }
}

/// Arguments de `ip` pour router `allowed_ips` dans le tunnel. Une route par défaut d'une
/// famille passe par la table `FWMARK` et deux règles, exactement comme `wg-quick`.
fn routing_commands(allowed_ips: &[ipnet::IpNet], ifname: &str) -> Vec<String> {
    let mut commands = Vec::new();
    for net in allowed_ips {
        let family = if net.addr().is_ipv4() { "-4" } else { "-6" };
        if net.prefix_len() == 0 {
            commands.push(format!(
                "{family} route add {net} dev {ifname} table {FWMARK}"
            ));
            commands.push(format!(
                "{family} rule add not fwmark {FWMARK} table {FWMARK}"
            ));
            commands.push(format!(
                "{family} rule add table main suppress_prefixlength 0"
            ));
        } else {
            commands.push(format!("{family} route add {net} dev {ifname}"));
        }
    }
    commands
}

/// Retrait des règles posées par `routing_commands` (erreurs ignorées : règle absente).
fn cleanup_commands() -> Vec<String> {
    ["-4", "-6"]
        .iter()
        .flat_map(|family| {
            [
                format!("{family} rule del not fwmark {FWMARK} table {FWMARK}"),
                format!("{family} rule del table main suppress_prefixlength 0"),
            ]
        })
        .collect()
}

/// Lance `ip` (iproute2), comme `wg-quick` ; `Err(stderr)` si la commande échoue.
fn ip(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("ip")
        .args(args)
        .output()
        .map_err(|e| format!("ip introuvable : {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Bit de `CAP_NET_ADMIN` dans les capacités effectives.
const CAP_NET_ADMIN: u32 = 12;

fn has_net_admin() -> bool {
    std::fs::read_to_string("/proc/self/status").is_ok_and(|s| cap_eff_has_net_admin(&s))
}

fn cap_eff_has_net_admin(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("CapEff:"))
        .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok())
        .is_some_and(|caps| caps & (1 << CAP_NET_ADMIN) != 0)
}

#[cfg(test)]
mod tests {
    use super::{cap_eff_has_net_admin, cleanup_commands, routing_commands};

    fn nets(list: &[&str]) -> Vec<ipnet::IpNet> {
        list.iter().map(|n| n.parse().unwrap()).collect()
    }

    #[test]
    fn full_tunnel_routing_matches_wg_quick() {
        assert_eq!(
            routing_commands(&nets(&["0.0.0.0/0"]), "ankou0"),
            [
                "-4 route add 0.0.0.0/0 dev ankou0 table 51820",
                "-4 rule add not fwmark 51820 table 51820",
                "-4 rule add table main suppress_prefixlength 0",
            ]
        );
    }

    #[test]
    fn split_and_ipv6_routing() {
        assert_eq!(
            routing_commands(&nets(&["10.99.0.0/24", "::/0"]), "ankou0"),
            [
                "-4 route add 10.99.0.0/24 dev ankou0",
                "-6 route add ::/0 dev ankou0 table 51820",
                "-6 rule add not fwmark 51820 table 51820",
                "-6 rule add table main suppress_prefixlength 0",
            ]
        );
    }

    #[test]
    fn cleanup_removes_both_families_rules() {
        assert_eq!(
            cleanup_commands(),
            [
                "-4 rule del not fwmark 51820 table 51820",
                "-4 rule del table main suppress_prefixlength 0",
                "-6 rule del not fwmark 51820 table 51820",
                "-6 rule del table main suppress_prefixlength 0",
            ]
        );
    }

    #[test]
    fn reads_net_admin_capability() {
        assert!(cap_eff_has_net_admin(
            "Name:\tx\nCapEff:\t000001ffffffffff\n"
        ));
        assert!(cap_eff_has_net_admin("CapEff:\t0000000000001000\n"));
        assert!(!cap_eff_has_net_admin("CapEff:\t0000000000000000\n"));
        assert!(!cap_eff_has_net_admin("Name:\tx\n"));
    }
}
