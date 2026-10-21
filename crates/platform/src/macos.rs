//! WireGuard sous macOS : boringtun dans le processus par `defguard_wireguard_rs`
//! (`WGApi<Userspace>`, carte `utun`), DNS par `networksetup` (ADR 0010).
//!
//! La carte vit tant que sa poignée vit : elle disparaît avec `ankoud` (les règles `pf`,
//! elles, restent).

use crate::{Dns, PROBE_BASE, PlatformError, Prober, Tunnel, TunnelStats, resolve_endpoint};
use ankou_proto::{InterfaceSection, PeerSection, TunnelConfig};
use defguard_wireguard_rs::error::WireguardInterfaceError;
use defguard_wireguard_rs::key::Key;
use defguard_wireguard_rs::net::IpAddrMask;
use defguard_wireguard_rs::peer::Peer;
use defguard_wireguard_rs::{InterfaceConfiguration, Userspace, WGApi, WireguardInterfaceApi};
use std::net::IpAddr;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, UNIX_EPOCH};

fn map_err(e: WireguardInterfaceError) -> PlatformError {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if lower.contains("operation not permitted") || lower.contains("permission denied") {
        PlatformError::Permission
    } else {
        PlatformError::Backend(text)
    }
}

fn mask(net: &ipnet::IpNet) -> IpAddrMask {
    IpAddrMask::new(net.addr(), net.prefix_len())
}

fn peer(section: &PeerSection, keepalive: u16) -> Result<Peer, PlatformError> {
    let mut p = Peer::new(Key::new(*section.public_key.as_bytes()));
    p.endpoint = Some(resolve_endpoint(&section.endpoint)?);
    p.persistent_keepalive_interval = Some(keepalive);
    p.allowed_ips = section.allowed_ips.iter().map(mask).collect();
    Ok(p)
}

/// Carte créée et configurée (adresses, clé, pair), sans routes.
fn open(
    name: &str,
    interface: &InterfaceSection,
    peers: Vec<Peer>,
    addresses: bool,
) -> Result<WGApi<Userspace>, PlatformError> {
    let config = InterfaceConfiguration {
        name: name.to_owned(),
        prvkey: interface.private_key.to_base64(),
        addresses: if addresses {
            interface.address.iter().map(mask).collect()
        } else {
            Vec::new()
        },
        port: 0,
        peers,
        mtu: None,
        fwmark: None,
    };
    let mut api = WGApi::<Userspace>::new(name.to_owned()).map_err(map_err)?;
    api.create_interface().map_err(map_err)?;
    api.configure_interface(&config).map_err(map_err)?;
    Ok(api)
}

fn read_stats(api: &WGApi<Userspace>) -> Result<TunnelStats, PlatformError> {
    let host = api.read_interface_data().map_err(map_err)?;
    let mut stats = TunnelStats::default();
    for peer in host.peers.values() {
        stats.rx_bytes += peer.rx_bytes;
        stats.tx_bytes += peer.tx_bytes;
        stats.last_handshake = stats.last_handshake.max(peer.last_handshake);
    }
    Ok(stats)
}

pub struct MacTunnel {
    ifname: String,
    /// Carte et endpoint (`ip:port`), dont la route vers la passerelle est à retirer.
    up: Option<(WGApi<Userspace>, String)>,
}

impl MacTunnel {
    pub fn new(ifname: &str) -> Self {
        Self {
            ifname: ifname.to_owned(),
            up: None,
        }
    }
}

impl Tunnel for MacTunnel {
    fn up(&mut self, cfg: &TunnelConfig) -> Result<(), PlatformError> {
        self.down()?;
        let p = peer(&cfg.peer, cfg.peer.persistent_keepalive)?;
        let endpoint = p.endpoint.map(|e| e.to_string()).unwrap_or_default();
        let api = open(&self.ifname, &cfg.interface, vec![p.clone()], true)?;
        // Routes 0/1 et 128/1 (comme `wg-quick`), endpoint par la passerelle physique.
        api.configure_peer_routing(&[p]).map_err(map_err)?;
        self.up = Some((api, endpoint));
        Ok(())
    }

    /// Route de l'endpoint retirée, puis poignée lâchée : boringtun s'arrête et la carte
    /// disparaît avec ses routes. Pas de `remove_interface` : sous macOS, il vide le DNS de
    /// tous les services réseau (ADR 0010). Idempotent.
    fn down(&mut self) -> Result<(), PlatformError> {
        if let Some((api, endpoint)) = self.up.take()
            && let Err(e) = api.remove_endpoint_routing(&endpoint)
        {
            tracing::warn!(error = %e, "route de l'endpoint restée");
        }
        Ok(())
    }

    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        let (api, _) = self
            .up
            .as_ref()
            .ok_or_else(|| PlatformError::Backend("carte absente".into()))?;
        read_stats(api)
    }
}

/// Mesure par handshake sur une carte `utun<110+n>` sans adresse ni route.
// ponytail: comme sous Windows (ADR 0009), connecté, la mesure passe par le tunnel courant.
pub struct MacProber;

impl MacProber {
    fn once(
        name: &str,
        interface: &InterfaceSection,
        section: &PeerSection,
        timeout: Duration,
    ) -> Option<Duration> {
        let mut p = peer(section, 1).ok()?;
        p.allowed_ips.clear();
        // Poignée lâchée en fin de fonction, même après une erreur : la carte disparaît.
        let api = open(name, interface, vec![p], false).ok()?;
        let start = Instant::now();
        while start.elapsed() < timeout {
            let seen = read_stats(&api)
                .ok()
                .and_then(|s| s.last_handshake)
                .is_some_and(|t| t > UNIX_EPOCH);
            if seen {
                return Some(start.elapsed());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Prober for MacProber {
    fn probe(
        &self,
        slot: usize,
        interface: &InterfaceSection,
        peer: &PeerSection,
        tries: u32,
        timeout: Duration,
    ) -> Vec<Duration> {
        let name = format!("utun{}", PROBE_BASE + slot);
        (0..tries)
            .filter_map(|_| Self::once(&name, interface, peer, timeout))
            .collect()
    }

    /// Les cartes disparaissent avec le processus qui les a créées : rien à nettoyer.
    fn cleanup(&self) {}
}

/// Services réseau actifs, d'après `networksetup -listallnetworkservices` : la première
/// ligne est une explication, `*` marque un service désactivé.
fn parse_services(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1)
        .filter(|l| !l.is_empty() && !l.starts_with('*'))
        .map(str::to_owned)
        .collect()
}

/// Serveurs de `networksetup -getdnsservers` ; aucun (phrase « There aren't any… ») → vide.
fn parse_dns_servers(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|l| l.parse::<IpAddr>().is_ok())
        .map(str::to_owned)
        .collect()
}

/// Arguments de `networksetup` : liste vide → `Empty` (DNS fourni par le DHCP).
fn setdns_args(service: &str, servers: &[String]) -> Vec<String> {
    let mut args = vec!["-setdnsservers".to_owned(), service.to_owned()];
    if servers.is_empty() {
        args.push("Empty".into());
    } else {
        args.extend(servers.iter().cloned());
    }
    args
}

/// Sauvegarde des réglages d'origine : une ligne `service\tserveurs séparés par des espaces`.
fn format_backup(saved: &[(String, Vec<String>)]) -> String {
    saved
        .iter()
        .map(|(service, servers)| format!("{service}\t{}\n", servers.join(" ")))
        .collect()
}

fn parse_backup(text: &str) -> Vec<(String, Vec<String>)> {
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(service, servers)| {
            let servers = servers.split_whitespace().map(str::to_owned).collect();
            (service.to_owned(), servers)
        })
        .collect()
}

fn networksetup(args: &[String]) -> Result<String, String> {
    let out = Command::new("networksetup")
        .args(args)
        .output()
        .map_err(|e| format!("networksetup introuvable : {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn flush_cache() {
    let _ = Command::new("dscacheutil").arg("-flushcache").status();
    let _ = Command::new("killall")
        .args(["-HUP", "mDNSResponder"])
        .status();
}

/// DNS du tunnel sur chaque service réseau. Les réglages d'origine sont gardés dans un
/// fichier, pour être rendus même après un plantage d'`ankoud` (ADR 0010).
pub struct MacDns;

const BACKUP: &str = "/var/run/ankou/dns-backup";

impl Dns for MacDns {
    fn set(&mut self, _interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError> {
        let list = networksetup(&["-listallnetworkservices".into()])
            .map_err(PlatformError::Unavailable)?;
        let services = parse_services(&list);
        // Une sauvegarde déjà là vient d'une connexion précédente : c'est la bonne.
        let backup = Path::new(BACKUP);
        if !backup.exists() {
            let saved: Vec<_> = services
                .iter()
                .map(|s| {
                    let out = networksetup(&["-getdnsservers".into(), s.clone()]);
                    (s.clone(), parse_dns_servers(&out.unwrap_or_default()))
                })
                .collect();
            let written = backup
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(backup, format_backup(&saved)));
            written.map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        let servers: Vec<String> = servers.iter().map(ToString::to_string).collect();
        for service in &services {
            networksetup(&setdns_args(service, &servers)).map_err(PlatformError::Unavailable)?;
        }
        flush_cache();
        Ok(())
    }

    /// Réglages d'origine rendus ; sans sauvegarde (DNS jamais posé), rien n'est touché.
    fn revert(&mut self, _interface: &str) {
        let Ok(text) = std::fs::read_to_string(BACKUP) else {
            return;
        };
        for (service, servers) in parse_backup(&text) {
            if let Err(e) = networksetup(&setdns_args(&service, &servers)) {
                tracing::warn!(service, error = %e, "DNS d'origine non rendu");
            }
        }
        let _ = std::fs::remove_file(BACKUP);
        flush_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networksetup_parsing() {
        let list = "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\n*Bluetooth PAN\nUSB 10/100 LAN\n";
        assert_eq!(parse_services(list), ["Wi-Fi", "USB 10/100 LAN"]);
        assert!(parse_dns_servers("There aren't any DNS Servers set on Wi-Fi.\n").is_empty());
        assert_eq!(
            parse_dns_servers("1.1.1.1\n2606:4700::1111\n"),
            ["1.1.1.1", "2606:4700::1111"]
        );
    }

    #[test]
    fn networksetup_restore_commands() {
        let saved = vec![
            ("Wi-Fi".to_owned(), vec![]),
            ("USB 10/100 LAN".to_owned(), vec!["9.9.9.9".to_owned()]),
        ];
        let back = parse_backup(&format_backup(&saved));
        assert_eq!(back, saved);
        assert_eq!(
            setdns_args(&back[0].0, &back[0].1),
            ["-setdnsservers", "Wi-Fi", "Empty"]
        );
        assert_eq!(
            setdns_args(&back[1].0, &back[1].1),
            ["-setdnsservers", "USB 10/100 LAN", "9.9.9.9"]
        );
    }
}
