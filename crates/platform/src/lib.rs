//! Accès système d'aNkou derrière des traits (spec §4.1.3, §6.2).

#[cfg(feature = "fake")]
pub mod fake;
#[cfg(target_os = "linux")]
mod linux;

use ankou_proto::{Endpoint, ErrorCode, TunnelConfig};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::SystemTime;

/// Nom de l'interface WireGuard du client.
pub const INTERFACE: &str = "ankou0";

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
pub fn default_tunnel() -> Box<dyn Tunnel> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::KernelTunnel::new(INTERFACE))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(Unsupported)
    }
}

/// Systèmes dont le moteur arrive à l'étape 5.
#[cfg(not(target_os = "linux"))]
struct Unsupported;

#[cfg(not(target_os = "linux"))]
impl Tunnel for Unsupported {
    fn up(&mut self, _cfg: &TunnelConfig) -> Result<(), PlatformError> {
        Err(PlatformError::Unavailable(
            "moteur prévu à l'étape 5 (Linux seulement pour l'instant)".into(),
        ))
    }
    fn down(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        Ok(TunnelStats::default())
    }
}
