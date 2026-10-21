//! Pare-feu du kill switch (spec §6.6) : une table nftables dédiée, `inet ankou`.

use crate::PlatformError;
use std::fmt::Write as _;
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq)]
pub struct FirewallPolicy {
    /// IPv4 bloqué hors tunnel ; l'IPv6 l'est toujours.
    pub kill_switch: bool,
    pub interface: String,
    /// Serveurs joignables hors tunnel (paquets WireGuard chiffrés).
    pub endpoints: Vec<SocketAddr>,
}

pub trait Firewall: Send {
    /// Remplace l'ensemble des règles.
    fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError>;
    /// Retire la table. Idempotent.
    fn clear(&mut self) -> Result<(), PlatformError>;
    /// Table présente (par exemple laissée par un arrêt brutal).
    fn is_active(&self) -> bool;
}

const TABLE: &str = "inet ankou";

/// Script pour `nft -f -` : création, suppression puis nouvelle définition dans une
/// seule transaction, donc aucun instant sans règles.
pub fn nft_ruleset(policy: &FirewallPolicy) -> String {
    let iface = &policy.interface;
    let mut s = format!("add table {TABLE}\ndelete table {TABLE}\ntable {TABLE} {{\n");
    s.push_str("  chain output {\n    type filter hook output priority 0; policy accept;\n");
    s.push_str("    oifname \"lo\" accept\n");
    let _ = writeln!(s, "    oifname \"{iface}\" accept");
    // Serveurs : toujours joignables (sinon l'IPv6 bloqué couperait un endpoint IPv6).
    for ep in &policy.endpoints {
        let family = if ep.is_ipv4() { "ip" } else { "ip6" };
        let _ = writeln!(
            s,
            "    {family} daddr {} udp dport {} accept",
            ep.ip(),
            ep.port()
        );
    }
    if policy.endpoints.iter().any(SocketAddr::is_ipv6) {
        // Découverte des voisins, sans laquelle un endpoint IPv6 reste injoignable.
        s.push_str(
            "    icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert, nd-router-solicit } accept\n",
        );
    }
    if policy.kill_switch {
        s.push_str("    udp sport 68 udp dport 67 accept\n");
    }
    s.push_str("    meta nfproto ipv6 drop\n");
    if policy.kill_switch {
        s.push_str("    meta nfproto ipv4 drop\n");
    }
    s.push_str("  }\n");
    // Trafic routé par l'hôte (conteneurs, VM en NAT) : même règle que l'hôte.
    s.push_str("  chain forward {\n    type filter hook forward priority 0; policy accept;\n");
    let _ = writeln!(s, "    iifname \"{iface}\" accept");
    let _ = writeln!(s, "    oifname \"{iface}\" accept");
    s.push_str("    meta nfproto ipv6 drop\n");
    if policy.kill_switch {
        s.push_str("    meta nfproto ipv4 drop\n");
    }
    s.push_str("  }\n}\n");
    s
}

/// Règle WFP (Windows), traduite de la politique sans E/S pour être testée partout.
/// Mêmes effets que `nft_ruleset` : permissions d'abord, blocages ensuite.
#[derive(Debug, Clone, PartialEq)]
pub enum WfpRule {
    /// Carte du tunnel, IPv4 et IPv6 (posée seulement si elle existe déjà).
    PermitInterface {
        name: String,
    },
    /// `127.0.0.0/8` et `::1`.
    PermitLoopback,
    /// UDP vers un serveur, hors tunnel (paquets WireGuard chiffrés).
    PermitEndpoint(SocketAddr),
    /// DHCPv4 (port distant 67) : le bail se renouvelle sous kill switch.
    PermitDhcpV4,
    /// Découverte des voisins, sans laquelle un endpoint IPv6 reste injoignable.
    PermitNeighbourDiscovery,
    BlockAllV6,
    BlockAllV4,
}

pub fn wfp_rules(policy: &FirewallPolicy) -> Vec<WfpRule> {
    let mut rules = vec![
        WfpRule::PermitInterface {
            name: policy.interface.clone(),
        },
        WfpRule::PermitLoopback,
    ];
    rules.extend(policy.endpoints.iter().map(|e| WfpRule::PermitEndpoint(*e)));
    if policy.endpoints.iter().any(SocketAddr::is_ipv6) {
        rules.push(WfpRule::PermitNeighbourDiscovery);
    }
    if policy.kill_switch {
        rules.push(WfpRule::PermitDhcpV4);
    }
    rules.push(WfpRule::BlockAllV6);
    if policy.kill_switch {
        rules.push(WfpRule::BlockAllV4);
    }
    rules
}

/// macOS : pare-feu à l'étape 5b.
#[cfg(not(any(target_os = "linux", windows)))]
struct NoFirewall;

#[cfg(not(any(target_os = "linux", windows)))]
impl Firewall for NoFirewall {
    fn apply(&mut self, _policy: &FirewallPolicy) -> Result<(), PlatformError> {
        Err(PlatformError::Unavailable(
            "pare-feu macOS prévu à l'étape 5b".into(),
        ))
    }
    fn clear(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn is_active(&self) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod nft {
    use super::{Firewall, FirewallPolicy, TABLE, nft_ruleset};
    use crate::PlatformError;
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub struct Nft;

    fn run(args: &[&str], stdin: Option<&str>) -> Result<(), String> {
        let mut child = Command::new("nft")
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("nft introuvable : {e}"))?;
        if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
            input
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }

    impl Firewall for Nft {
        fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError> {
            run(&["-f", "-"], Some(&nft_ruleset(policy))).map_err(PlatformError::Unavailable)
        }

        fn clear(&mut self) -> Result<(), PlatformError> {
            if !self.is_active() {
                return Ok(());
            }
            let table: Vec<&str> = TABLE.split(' ').collect();
            run(&["delete", "table", table[0], table[1]], None).map_err(PlatformError::Backend)
        }

        fn is_active(&self) -> bool {
            let table: Vec<&str> = TABLE.split(' ').collect();
            run(&["list", "table", table[0], table[1]], None).is_ok()
        }
    }
}

/// Pare-feu du système courant.
pub fn default_firewall() -> Box<dyn Firewall> {
    #[cfg(target_os = "linux")]
    {
        Box::new(nft::Nft)
    }
    #[cfg(windows)]
    {
        Box::new(crate::windows_firewall::WfpFirewall)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Box::new(NoFirewall)
    }
}

#[cfg(test)]
mod wfp_tests {
    use super::*;

    #[test]
    fn wfp_rules_cover_policy() {
        let v4: SocketAddr = "1.2.3.4:51820".parse().unwrap();
        let policy = FirewallPolicy {
            kill_switch: true,
            interface: "ankou0".into(),
            endpoints: vec![v4],
        };
        assert_eq!(
            wfp_rules(&policy),
            [
                WfpRule::PermitInterface {
                    name: "ankou0".into()
                },
                WfpRule::PermitLoopback,
                WfpRule::PermitEndpoint(v4),
                WfpRule::PermitDhcpV4,
                WfpRule::BlockAllV6,
                WfpRule::BlockAllV4,
            ]
        );
        let v6: SocketAddr = "[2001:db8::1]:51820".parse().unwrap();
        let rules = wfp_rules(&FirewallPolicy {
            kill_switch: false,
            interface: "ankou0".into(),
            endpoints: vec![v6],
        });
        assert!(rules.contains(&WfpRule::PermitEndpoint(v6)));
        assert!(rules.contains(&WfpRule::PermitNeighbourDiscovery));
        assert!(rules.contains(&WfpRule::BlockAllV6));
        assert!(!rules.contains(&WfpRule::BlockAllV4));
        assert!(!rules.contains(&WfpRule::PermitDhcpV4));
    }
}
