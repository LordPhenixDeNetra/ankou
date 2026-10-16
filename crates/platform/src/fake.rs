//! Faux tunnel et fausse surveillance pour les tests : poignées partagées, pilotables
//! depuis le test pendant que le code testé les possède.

use crate::{INTERFACE, PlatformError, SystemWatch, Tunnel, TunnelStats};
use ankou_proto::TunnelConfig;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

struct TunnelState {
    calls: Vec<String>,
    handshakes: bool,
    fail_up: Option<PlatformError>,
    handshake: Option<SystemTime>,
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
        Ok(TunnelStats {
            last_handshake: lock(&self.0).handshake,
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
