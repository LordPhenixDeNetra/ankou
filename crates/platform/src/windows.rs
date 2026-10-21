//! WireGuard sous Windows : pilote wireguard-nt par `defguard_wireguard_rs` (ADR 0009).
//!
//! La carte vit tant que sa poignée vit : `WindowsTunnel` la garde, et Windows la retire
//! avec le processus si le service s'arrête brutalement (le pare-feu WFP, lui, reste).

use crate::{Dns, PlatformError, Prober, Tunnel, TunnelStats, resolve_endpoint};
use ankou_proto::{InterfaceSection, PeerSection, TunnelConfig};
use defguard_wireguard_rs::dns::DnsConfig;
use defguard_wireguard_rs::error::WireguardInterfaceError;
use defguard_wireguard_rs::key::Key;
use defguard_wireguard_rs::net::IpAddrMask;
use defguard_wireguard_rs::peer::Peer;
use defguard_wireguard_rs::{InterfaceConfiguration, Kernel, WGApi, WireguardInterfaceApi};
use std::net::IpAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Chemin où `defguard_wireguard_rs` charge `wireguard.dll`, relatif au dossier courant
/// (`ankoud` se place dans le dossier de son exécutable au démarrage).
pub const WIREGUARD_DLL: &str = r"resources-windows\binaries\wireguard-amd64.dll";

pub fn dll_present() -> bool {
    std::path::Path::new(WIREGUARD_DLL).is_file()
}

fn missing_dll() -> PlatformError {
    let dir = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    PlatformError::Unavailable(format!(
        "wireguard.dll introuvable : {dir}\\{WIREGUARD_DLL}"
    ))
}

fn map_err(e: WireguardInterfaceError) -> PlatformError {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if lower.contains("access is denied") || lower.contains("accès refusé") {
        PlatformError::Permission
    } else {
        PlatformError::Backend(text)
    }
}

fn mask(net: &ipnet::IpNet) -> IpAddrMask {
    IpAddrMask::new(net.addr(), net.prefix_len())
}

/// Crée (ou rouvre) la carte. La bibliothèque panique si la DLL ne se charge pas (fichier
/// abîmé, mauvaise architecture) : rattrapé ici, le service survit et le dit.
fn create(api: &mut WGApi<Kernel>) -> Result<(), PlatformError> {
    if !dll_present() {
        return Err(missing_dll());
    }
    match catch_unwind(AssertUnwindSafe(|| api.create_interface())) {
        Ok(result) => result.map_err(map_err),
        Err(_) => Err(PlatformError::Unavailable(
            "wireguard.dll ne se charge pas".into(),
        )),
    }
}

pub struct WindowsTunnel {
    ifname: String,
    api: Option<WGApi<Kernel>>,
}

impl WindowsTunnel {
    pub fn new(ifname: &str) -> Self {
        Self {
            ifname: ifname.to_owned(),
            api: None,
        }
    }
}

impl Tunnel for WindowsTunnel {
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
            fwmark: None,
        };
        let mut api = WGApi::<Kernel>::new(self.ifname.clone()).map_err(map_err)?;
        create(&mut api)?;
        // Adresses, routes (métrique 0 pour une route par défaut) et carte montée.
        api.configure_interface(&config).map_err(map_err)?;
        self.api = Some(api);
        Ok(())
    }

    /// Règles NRPT retirées, puis carte supprimée avec sa poignée. Idempotent.
    fn down(&mut self) -> Result<(), PlatformError> {
        if let Some(mut api) = self.api.take()
            && let Err(e) = api.remove_interface()
        {
            tracing::warn!(error = %e, "retrait de la carte incomplet");
        }
        Ok(())
    }

    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| PlatformError::Backend("carte absente".into()))?;
        let host = api.read_interface_data().map_err(map_err)?;
        let mut stats = TunnelStats::default();
        for peer in host.peers.values() {
            stats.rx_bytes += peer.rx_bytes;
            stats.tx_bytes += peer.tx_bytes;
            stats.last_handshake = stats.last_handshake.max(peer.last_handshake);
        }
        Ok(stats)
    }
}

/// Mesure par handshake sur une carte `ankoup<n>` sans adresse ni route.
// ponytail: pas de fwmark sous Windows ; connecté, la mesure passe par le tunnel courant
// (ADR 0009). Lier la socket du pilote à la carte physique si la précision compte.
pub struct WindowsProber;

impl WindowsProber {
    fn once(
        name: &str,
        interface: &InterfaceSection,
        peer: &PeerSection,
        timeout: Duration,
    ) -> Option<Duration> {
        let mut p = Peer::new(Key::new(*peer.public_key.as_bytes()));
        p.endpoint = Some(resolve_endpoint(&peer.endpoint).ok()?);
        p.persistent_keepalive_interval = Some(1);
        let config = InterfaceConfiguration {
            name: name.to_owned(),
            prvkey: interface.private_key.to_base64(),
            addresses: Vec::new(),
            port: 0,
            peers: vec![p],
            mtu: None,
            fwmark: None,
        };
        let mut api = WGApi::<Kernel>::new(name.to_owned()).ok()?;
        create(&mut api).ok()?;
        let measured = (|| {
            api.configure_interface(&config).ok()?;
            let start = Instant::now();
            while start.elapsed() < timeout {
                let seen = api.read_interface_data().ok().is_some_and(|host| {
                    host.peers
                        .values()
                        .any(|p| p.last_handshake.is_some_and(|t| t > UNIX_EPOCH))
                });
                if seen {
                    return Some(start.elapsed());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            None
        })();
        // Toujours retirée, même après une erreur.
        let _ = api.remove_interface();
        measured
    }
}

impl Prober for WindowsProber {
    fn probe(
        &self,
        slot: usize,
        interface: &InterfaceSection,
        peer: &PeerSection,
        tries: u32,
        timeout: Duration,
    ) -> Vec<Duration> {
        let name = format!("{}{slot}", crate::PROBE_PREFIX);
        (0..tries)
            .filter_map(|_| Self::once(&name, interface, peer, timeout))
            .collect()
    }

    /// Les cartes disparaissent avec le processus qui les a créées : rien à nettoyer.
    fn cleanup(&self) {}
}

/// DNS du tunnel pour tous les noms : règle NRPT « . » et serveurs de la carte.
pub struct WindowsDns;

impl Dns for WindowsDns {
    fn set(&mut self, interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError> {
        let api = WGApi::<Kernel>::new(interface.to_owned()).map_err(map_err)?;
        api.set_dns(&DnsConfig::full_tunnel(servers))
            .map_err(|e| PlatformError::Unavailable(e.to_string()))
    }

    fn revert(&mut self, interface: &str) {
        if let Ok(api) = WGApi::<Kernel>::new(interface.to_owned()) {
            let _ = api.set_dns(&DnsConfig::split_dns(&[], &[], &[]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TunnelConfig {
        toml::from_str(
            r#"[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]
[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "127.0.0.1:51820"
allowed_ips = ["0.0.0.0/0"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn missing_dll_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let err = WindowsTunnel::new("ankou0").up(&config()).unwrap_err();
        assert!(matches!(err, PlatformError::Unavailable(_)), "{err:?}");
    }
}
