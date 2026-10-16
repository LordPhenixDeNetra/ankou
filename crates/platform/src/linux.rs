//! WireGuard noyau Linux par netlink (ADR 0002).

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

impl Tunnel for KernelTunnel {
    fn up(&mut self, cfg: &TunnelConfig) -> Result<(), PlatformError> {
        if self.exists() {
            self.down()?;
        }
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
            peers: vec![peer.clone()],
            mtu: None,
            fwmark: None,
        };
        if !has_net_admin() {
            return Err(PlatformError::Permission);
        }
        let mut api = self.api()?;
        // La bibliothèque perd le code d'erreur système à la création : avec les droits,
        // un échec veut dire que le module WireGuard manque.
        api.create_interface()
            .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
        let endpoint_route = format!(
            "{}/{}",
            endpoint.ip(),
            if endpoint.is_ipv4() { 32 } else { 128 }
        );
        let route_existed = !ip(&["route", "show", &endpoint_route])
            .unwrap_or_default()
            .is_empty();
        let configured = api
            .configure_interface(&config)
            .and_then(|()| api.configure_peer_routing(&[peer]));
        if let Err(e) = configured {
            // Ne rien laisser derrière en cas d'échec partiel.
            let _ = self.down();
            return Err(map_err(e));
        }
        // `configure_peer_routing` ajoute une route hôte vers l'endpoint dans la table main :
        // inutile avec le routage par fwmark, trou noir sans passerelle, et jamais retirée.
        if !route_existed {
            let _ = ip(&["route", "del", &endpoint_route]);
        }
        Ok(())
    }

    fn down(&mut self) -> Result<(), PlatformError> {
        if !self.exists() {
            return Ok(());
        }
        // `remove_interface` retire les règles fwmark puis la config DNS : sans
        // systemd-resolved ni resolvconf, il échoue avant de supprimer l'interface.
        if let Err(e) = self.api()?.remove_interface() {
            tracing::debug!(error = %e, "remove_interface incomplet, suppression directe");
            ip(&["link", "del", &self.ifname]).map_err(|_| map_err(e))?;
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

/// Lance `ip` (iproute2) ; `Ok(stdout)` si la commande réussit.
// ponytail: deux opérations de nettoyage par `ip` au lieu de messages netlink écrits à la main ;
// à remplacer quand la bibliothèque corrigera ces cas (ou par netlink-packet-route).
fn ip(args: &[&str]) -> Result<String, ()> {
    let out = std::process::Command::new("ip")
        .args(args)
        .output()
        .map_err(|_| ())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(())
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
    use super::cap_eff_has_net_admin;

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
