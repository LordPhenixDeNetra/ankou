//! État persistant du service : profil (dernier `TunnelConfig`) et réglages.

use ankou_proto::TunnelConfig;
use ankou_proto::ipc::DaemonSettings;
use lpdnsoft_settings::{Loaded, Settings, SettingsStore};
use serde::{Deserialize, Serialize};
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
