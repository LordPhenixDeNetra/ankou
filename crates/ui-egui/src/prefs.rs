//! Préférences propres à l'interface (langue, favoris), à côté de la session.

use lpdnsoft_settings::{Loaded, SettingsStore};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    /// `None` : langue du système.
    #[serde(default)]
    pub language: Option<String>,
    /// Noms des serveurs favoris, affichés en tête.
    #[serde(default)]
    pub favourites: Vec<String>,
}

impl lpdnsoft_settings::Settings for Prefs {
    const FILE_NAME: &'static str = "ui.toml";
    const SCHEMA_VERSION: u32 = 1;
}

/// Valeurs par défaut si le fichier manque ou se lit mal.
pub fn load_prefs(dir: &Path) -> Prefs {
    match SettingsStore::<Prefs>::new(dir.to_owned()).load() {
        Ok(loaded) => loaded.settings,
        Err(e) => {
            tracing::warn!(error = %e, "préférences de l'interface illisibles");
            Prefs::default()
        }
    }
}

/// Un échec est journalisé : les préférences ne sont qu'un confort.
pub fn save_prefs(dir: &Path, prefs: &Prefs) {
    let loaded = Loaded {
        settings: prefs.clone(),
        recovery: None,
        read_only: false,
    };
    if let Err(e) = SettingsStore::<Prefs>::new(dir.to_owned()).save(&loaded) {
        tracing::warn!(error = %e, "préférences de l'interface non enregistrées");
    }
}
