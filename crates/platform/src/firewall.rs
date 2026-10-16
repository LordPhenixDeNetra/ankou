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
    let mut s = format!("add table {TABLE}\ndelete table {TABLE}\ntable {TABLE} {{\n");
    s.push_str("  chain output {\n    type filter hook output priority 0; policy accept;\n");
    s.push_str("    oifname \"lo\" accept\n");
    let _ = writeln!(s, "    oifname \"{}\" accept", policy.interface);
    if policy.kill_switch {
        for ep in &policy.endpoints {
            let family = if ep.is_ipv4() { "ip" } else { "ip6" };
            let _ = writeln!(
                s,
                "    {family} daddr {} udp dport {} accept",
                ep.ip(),
                ep.port()
            );
        }
        s.push_str("    udp sport 68 udp dport 67 accept\n");
    }
    s.push_str("    meta nfproto ipv6 drop\n");
    if policy.kill_switch {
        s.push_str("    meta nfproto ipv4 drop\n");
    }
    s.push_str("  }\n}\n");
    s
}

/// Systèmes sans pare-feu géré (avant l'étape 5).
#[cfg(not(target_os = "linux"))]
struct NoFirewall;

#[cfg(not(target_os = "linux"))]
impl Firewall for NoFirewall {
    fn apply(&mut self, _policy: &FirewallPolicy) -> Result<(), PlatformError> {
        Err(PlatformError::Unavailable(
            "pare-feu prévu à l'étape 5 (Linux seulement pour l'instant)".into(),
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
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoFirewall)
    }
}
