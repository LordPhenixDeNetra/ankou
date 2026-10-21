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

#[cfg(not(any(target_os = "linux", windows)))]
struct NoProber;

#[cfg(not(any(target_os = "linux", windows)))]
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

/// Mesure du système courant (sans effet sous macOS jusqu'à l'étape 5b).
pub fn default_prober() -> Box<dyn Prober> {
    #[cfg(target_os = "linux")]
    {
        Box::new(crate::linux::KernelProber)
    }
    #[cfg(windows)]
    {
        Box::new(crate::windows::WindowsProber)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Box::new(NoProber)
    }
}
