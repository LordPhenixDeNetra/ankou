//! Accès système d'aNkou derrière des traits (spec §4.1.3, §6.2).

mod dns;
#[cfg(feature = "fake")]
pub mod fake;
mod firewall;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_firewall;
mod probe;
mod watch;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_firewall;

use ankou_proto::{Endpoint, ErrorCode, TunnelConfig};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::SystemTime;

pub use dns::{Dns, default_dns, resolvectl_commands};
pub use firewall::{
    Firewall, FirewallPolicy, PF_ANCHOR, WfpRule, default_firewall, nft_ruleset, pf_ruleset,
    wfp_rules,
};
pub use probe::{Prober, default_prober};
pub use watch::{SystemWatch, default_watch, describe_route, parse_default_route, physical_route};
#[cfg(windows)]
pub use windows::{WIREGUARD_DLL, dll_present};

/// Préfixe des interfaces temporaires de mesure (`ankoup<n>`).
#[cfg(any(target_os = "linux", windows))]
pub(crate) const PROBE_PREFIX: &str = "ankoup";

/// Mesures sous macOS : `utun<PROBE_BASE + n>` (seuls les noms `utun<n>` sont admis).
#[cfg(target_os = "macos")]
pub(crate) const PROBE_BASE: usize = 110;

/// Nom de l'interface WireGuard du client ; macOS n'admet que `utun<n>` (ADR 0010).
#[cfg(not(target_os = "macos"))]
pub const INTERFACE: &str = "ankou0";
#[cfg(target_os = "macos")]
pub const INTERFACE: &str = "utun100";

pub trait Tunnel {
    /// Monte le tunnel ; supprime d'abord une interface restée d'une exécution précédente.
    fn up(&mut self, cfg: &TunnelConfig) -> Result<(), PlatformError>;
    /// Retire le tunnel et ses routes. Idempotent.
    fn down(&mut self) -> Result<(), PlatformError>;
    fn stats(&self) -> Result<TunnelStats, PlatformError>;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TunnelStats {
    pub last_handshake: Option<SystemTime>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlatformError {
    #[error("droits insuffisants")]
    Permission,
    #[error("moteur WireGuard indisponible : {0}")]
    Unavailable(String),
    #[error("endpoint injoignable : {0}")]
    Endpoint(String),
    #[error("erreur système : {0}")]
    Backend(String),
}

impl PlatformError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Permission => ErrorCode::TunnelPermission,
            Self::Unavailable(_) => ErrorCode::TunnelUnavailable,
            Self::Endpoint(_) => ErrorCode::ServerUnreachable,
            Self::Backend(_) => ErrorCode::Internal,
        }
    }

    /// Détail technique (vide pour `Permission`).
    pub fn detail(&self) -> &str {
        match self {
            Self::Permission => "",
            Self::Unavailable(d) | Self::Endpoint(d) | Self::Backend(d) => d,
        }
    }
}

/// Résout l'endpoint : première adresse IPv4, sinon la première adresse.
pub fn resolve_endpoint(ep: &Endpoint) -> Result<SocketAddr, PlatformError> {
    let addrs: Vec<SocketAddr> = (ep.host.as_str(), ep.port)
        .to_socket_addrs()
        .map_err(|e| PlatformError::Endpoint(format!("{ep}: {e}")))?
        .collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| PlatformError::Endpoint(ep.to_string()))
}

/// Moteur de tunnel du système courant.
pub fn default_tunnel() -> Box<dyn Tunnel + Send> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::KernelTunnel::new(INTERFACE))
    }
    #[cfg(windows)]
    {
        Box::new(windows::WindowsTunnel::new(INTERFACE))
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacTunnel::new(INTERFACE))
    }
}
