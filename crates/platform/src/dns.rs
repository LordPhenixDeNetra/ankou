//! DNS dans le tunnel (spec §6.6) : `resolvectl` route tous les domaines vers l'interface.

use crate::PlatformError;
use std::net::IpAddr;

pub trait Dns: Send {
    fn set(&mut self, interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError>;
    /// Au mieux : l'interface supprimée emporte déjà sa configuration.
    fn revert(&mut self, interface: &str);
}

/// `resolvectl dns`, `domain "~."` (tous les domaines), `default-route yes`.
pub fn resolvectl_commands(interface: &str, servers: &[IpAddr]) -> Vec<Vec<String>> {
    let mut dns = vec!["resolvectl".into(), "dns".into(), interface.into()];
    dns.extend(servers.iter().map(ToString::to_string));
    vec![
        dns,
        ["resolvectl", "domain", interface, "~."]
            .map(String::from)
            .to_vec(),
        ["resolvectl", "default-route", interface, "yes"]
            .map(String::from)
            .to_vec(),
    ]
}

#[cfg(not(target_os = "linux"))]
struct NoDns;

#[cfg(not(target_os = "linux"))]
impl Dns for NoDns {
    fn set(&mut self, _interface: &str, _servers: &[IpAddr]) -> Result<(), PlatformError> {
        Ok(())
    }
    fn revert(&mut self, _interface: &str) {}
}

#[cfg(target_os = "linux")]
struct Resolvectl;

#[cfg(target_os = "linux")]
fn run(args: &[String]) -> Result<(), String> {
    let out = std::process::Command::new(&args[0])
        .args(&args[1..])
        .output()
        .map_err(|e| format!("{} introuvable : {e}", args[0]))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(target_os = "linux")]
impl Dns for Resolvectl {
    fn set(&mut self, interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError> {
        for cmd in resolvectl_commands(interface, servers) {
            run(&cmd).map_err(PlatformError::Unavailable)?;
        }
        Ok(())
    }

    fn revert(&mut self, interface: &str) {
        let _ = run(&["resolvectl", "revert", interface].map(String::from));
    }
}

/// DNS du système courant (sans effet hors Linux pour l'instant).
pub fn default_dns() -> Box<dyn Dns> {
    #[cfg(target_os = "linux")]
    {
        Box::new(Resolvectl)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoDns)
    }
}
