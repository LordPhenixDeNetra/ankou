//! Tunnel factice pour les tests : enregistre les appels.

use crate::{INTERFACE, PlatformError, Tunnel, TunnelStats};
use ankou_proto::TunnelConfig;
use std::time::SystemTime;

pub struct FakeTunnel {
    pub calls: Vec<String>,
    /// Un handshake est « vu » dès que le tunnel est monté.
    pub handshake_after_up: bool,
    pub fail_up: Option<PlatformError>,
    pub is_up: bool,
}

impl Default for FakeTunnel {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            handshake_after_up: true,
            fail_up: None,
            is_up: false,
        }
    }
}

impl Tunnel for FakeTunnel {
    fn up(&mut self, _cfg: &TunnelConfig) -> Result<(), PlatformError> {
        self.calls.push(format!("up {INTERFACE}"));
        if let Some(e) = &self.fail_up {
            return Err(e.clone());
        }
        self.is_up = true;
        Ok(())
    }

    fn down(&mut self) -> Result<(), PlatformError> {
        self.calls.push("down".into());
        self.is_up = false;
        Ok(())
    }

    fn stats(&self) -> Result<TunnelStats, PlatformError> {
        Ok(TunnelStats {
            last_handshake: (self.is_up && self.handshake_after_up).then(SystemTime::now),
            ..TunnelStats::default()
        })
    }
}
