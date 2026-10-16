//! Mesure de latence par handshake WireGuard réel (spec §6.5).

use ankou_proto::{InterfaceSection, PeerSection};
use std::time::Duration;

pub trait Prober: Send + Sync {
    /// `tries` handshakes vers `peer`, chacun sur une interface neuve `ankoup<slot>` (sans
    /// adresse ni route) ; renvoie la durée des essais réussis.
    fn probe(
        &self,
        slot: usize,
        interface: &InterfaceSection,
        peer: &PeerSection,
        tries: u32,
        timeout: Duration,
    ) -> Vec<Duration>;
    /// Supprime toute interface `ankoup*` restée (arrêt brutal pendant une mesure).
    fn cleanup(&self);
}

#[cfg(not(target_os = "linux"))]
struct NoProber;

#[cfg(not(target_os = "linux"))]
impl Prober for NoProber {
    fn probe(
        &self,
        _slot: usize,
        _interface: &InterfaceSection,
        _peer: &PeerSection,
        _tries: u32,
        _timeout: Duration,
    ) -> Vec<Duration> {
        Vec::new()
    }
    fn cleanup(&self) {}
}

/// Mesure du système courant (sans effet hors Linux pour l'instant).
pub fn default_prober() -> Box<dyn Prober> {
    #[cfg(target_os = "linux")]
    {
        Box::new(crate::linux::KernelProber)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoProber)
    }
}
