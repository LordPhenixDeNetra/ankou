//! Faux tunnel et fausse surveillance pour les tests : poignées partagées, pilotables
//! depuis le test pendant que le code testé les possède.

use crate::{
    Dns, Firewall, FirewallPolicy, INTERFACE, PlatformError, SystemWatch, Tunnel, TunnelStats,
};
use ankou_proto::TunnelConfig;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

struct TunnelState {
    calls: Vec<String>,
    handshakes: bool,
    fail_up: Option<PlatformError>,
    handshake: Option<SystemTime>,
    fail_stats: bool,
}

#[derive(Clone)]
pub struct FakeTunnel(Arc<Mutex<TunnelState>>);

impl Default for FakeTunnel {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(TunnelState {
            calls: Vec::new(),
            handshakes: true,
            fail_up: None,
            handshake: None,
            fail_stats: false,
        })))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl FakeTunnel {
    pub fn calls(&self) -> Vec<String> {
        lock(&self.0).calls.clone()
    }
    /// `false` : aucun handshake après `up` (serveur muet).
    pub fn set_handshakes(&self, on: bool) {
        lock(&self.0).handshakes = on;
    }
    pub fn fail_up(&self, e: Option<PlatformError>) {
        lock(&self.0).fail_up = e;
    }
    /// Recule le dernier handshake de `by` (lien mort).
    pub fn age_handshake(&self, by: Duration) {
        let mut s = lock(&self.0);
        s.handshake = s.handshake.map(|t| t - by);
    }
    /// `true` : `stats()` échoue, comme une interface supprimée par un autre outil.
    pub fn fail_stats(&self, on: bool) {
        lock(&self.0).fail_stats = on;
    }
    pub fn is_up(&self) -> bool {
        lock(&self.0)
            .calls
            .last()
            .is_some_and(|c| c.starts_with("up"))
    }
}

impl Tunnel for FakeTunnel {
    fn up(&mut self, _cfg: &TunnelConfig) -> Result<(), PlatformError> {
        let mut s = lock(&self.0);
        s.calls.push(format!("up {INTERFACE}"));
        if let Some(e) = &s.fail_up {
            return Err(e.clone());
        }
        s.handshake = s.handshakes.then(SystemTime::now);
        Ok(())
    }

    fn down(&mut self) -> Result<(), PlatformError> {
        let mut s = lock(&self.0);
        s.calls.push("down".into());
        s.handshake = None;
        Ok(())
    }

    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        let s = lock(&self.0);
        if s.fail_stats {
            return Err(PlatformError::Backend("interface absente".into()));
        }
        Ok(TunnelStats {
            last_handshake: s.handshake,
            ..TunnelStats::default()
        })
    }
}

struct WatchState {
    route: Option<String>,
    offset: Duration,
}

#[derive(Clone)]
pub struct FakeWatch(Arc<Mutex<WatchState>>);

impl Default for FakeWatch {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(WatchState {
            route: Some("eth0 via 10.0.0.1".into()),
            offset: Duration::ZERO,
        })))
    }
}

impl FakeWatch {
    pub fn set_route(&self, r: Option<&str>) {
        lock(&self.0).route = r.map(str::to_owned);
    }
    /// Avance l'horloge murale seule, comme une sortie de veille.
    pub fn jump_clock(&self, by: Duration) {
        lock(&self.0).offset += by;
    }
}

impl SystemWatch for FakeWatch {
    fn default_route(&self) -> Option<String> {
        lock(&self.0).route.clone()
    }
    fn wall_clock(&self) -> SystemTime {
        SystemTime::now() + lock(&self.0).offset
    }
}

#[derive(Default)]
struct FirewallState {
    active: Option<FirewallPolicy>,
    fail: bool,
}

#[derive(Clone, Default)]
pub struct FakeFirewall(Arc<Mutex<FirewallState>>);

impl FakeFirewall {
    pub fn active(&self) -> Option<FirewallPolicy> {
        lock(&self.0).active.clone()
    }
    /// `true` : `apply` échoue, comme sans `nft`.
    pub fn fail(&self, on: bool) {
        lock(&self.0).fail = on;
    }
    /// Règles laissées par un arrêt brutal du service.
    pub fn set_leftover(&self, p: FirewallPolicy) {
        lock(&self.0).active = Some(p);
    }
}

impl Firewall for FakeFirewall {
    fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError> {
        let mut s = lock(&self.0);
        if s.fail {
            return Err(PlatformError::Unavailable("nft introuvable".into()));
        }
        s.active = Some(policy.clone());
        Ok(())
    }
    fn clear(&mut self) -> Result<(), PlatformError> {
        lock(&self.0).active = None;
        Ok(())
    }
    fn is_active(&self) -> bool {
        lock(&self.0).active.is_some()
    }
}

#[derive(Default)]
struct DnsState {
    current: Option<(String, Vec<IpAddr>)>,
    fail: bool,
}

#[derive(Clone, Default)]
pub struct FakeDns(Arc<Mutex<DnsState>>);

impl FakeDns {
    pub fn current(&self) -> Option<(String, Vec<IpAddr>)> {
        lock(&self.0).current.clone()
    }
    /// `true` : `set` échoue, comme sans systemd-resolved.
    pub fn fail(&self, on: bool) {
        lock(&self.0).fail = on;
    }
}

impl Dns for FakeDns {
    fn set(&mut self, interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError> {
        let mut s = lock(&self.0);
        if s.fail {
            return Err(PlatformError::Unavailable("resolvectl introuvable".into()));
        }
        s.current = Some((interface.to_owned(), servers.to_vec()));
        Ok(())
    }
    fn revert(&mut self, _interface: &str) {
        lock(&self.0).current = None;
    }
}
