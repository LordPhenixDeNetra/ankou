//! État persistant du service : profil (dernier `TunnelConfig`) et réglages.

use ankou_proto::ipc::DaemonSettings;
use ankou_proto::{Endpoint, TunnelConfig};
use lpdnsoft_settings::{Loaded, Settings, SettingsStore};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

/// `DaemonSettings` vit dans `ankou-proto` : enveloppe locale pour le trait de stockage.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Stored(DaemonSettings);

impl Settings for Stored {
    const FILE_NAME: &'static str = "settings.toml";
    const SCHEMA_VERSION: u32 = 1;
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn profile_path(&self) -> PathBuf {
        self.dir.join("profile.toml")
    }

    /// Profil illisible : journalisé et ignoré (le service démarre quand même).
    pub fn load_profile(&self) -> Option<TunnelConfig> {
        let text = match std::fs::read_to_string(self.profile_path()) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!(error = %e, "profil illisible");
                return None;
            }
        };
        toml::from_str(&text)
            .inspect_err(|e| tracing::warn!(error = %e, "profil invalide"))
            .ok()
    }

    /// Écriture atomique en `0600` : le profil contient la clé privée.
    pub fn save_profile(&self, cfg: &TunnelConfig) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let text = toml::to_string(cfg).map_err(|e| e.to_string())?;
        lpdnsoft_atomic::write_atomic(&self.profile_path(), text.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Dernière adresse résolue de `endpoint` : le kill switch bloque le DNS hors
    /// tunnel pendant une reconnexion.
    pub fn save_endpoint(&self, endpoint: &Endpoint, addr: SocketAddr) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let text = format!("{endpoint} {addr}\n");
        lpdnsoft_atomic::write_atomic(&self.dir.join("endpoint"), text.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Seulement si le cache concerne ce même endpoint.
    pub fn load_endpoint(&self, endpoint: &Endpoint) -> Option<SocketAddr> {
        let text = std::fs::read_to_string(self.dir.join("endpoint")).ok()?;
        let (name, addr) = text.trim().split_once(' ')?;
        (name == endpoint.to_string()).then(|| addr.parse().ok())?
    }

    pub fn load_settings(&self) -> DaemonSettings {
        match SettingsStore::<Stored>::new(self.dir.clone()).load() {
            Ok(loaded) => loaded.settings.0,
            Err(e) => {
                tracing::warn!(error = %e, "réglages illisibles, valeurs par défaut");
                DaemonSettings::default()
            }
        }
    }

    pub fn save_settings(&self, settings: &DaemonSettings) -> Result<(), String> {
        let loaded = Loaded {
            settings: Stored(settings.clone()),
            recovery: None,
            read_only: false,
        };
        SettingsStore::<Stored>::new(self.dir.clone())
            .save(&loaded)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::Store;
    use ankou_proto::Endpoint;

    #[test]
    fn endpoint_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state"));
        let ep: Endpoint = "vps1.example.org:51820".parse().unwrap();
        assert_eq!(store.load_endpoint(&ep), None);
        let addr = "203.0.113.7:51820".parse().unwrap();
        store.save_endpoint(&ep, addr).unwrap();
        assert_eq!(store.load_endpoint(&ep), Some(addr));
        let other: Endpoint = "vps2.example.org:51820".parse().unwrap();
        assert_eq!(store.load_endpoint(&other), None);
    }
}
