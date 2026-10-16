use crate::ClientError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Ce qui survit entre deux commandes : l'API et le jeton de rafraîchissement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub api_url: String,
    pub refresh_token: String,
}

impl Session {
    /// `Ok(None)` si le fichier n'existe pas.
    pub fn load(path: &Path) -> Result<Option<Self>, ClientError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ClientError::Io(format!("{} : {e}", path.display()))),
        };
        toml::from_str(&text)
            .map(Some)
            .map_err(|e| ClientError::Io(format!("{} : {e}", path.display())))
    }

    /// Écriture atomique, fichier en 0600 (le jeton vaut un mot de passe).
    pub fn save(&self, path: &Path) -> Result<(), ClientError> {
        let io = |e: String| ClientError::Io(format!("{} : {e}", path.display()));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(e.to_string()))?;
        }
        let text = toml::to_string(self).map_err(|e| io(e.to_string()))?;
        lpdnsoft_atomic::write_atomic(path, text.as_bytes()).map_err(|e| io(e.to_string()))
    }
}

/// `ANKOU_SESSION_FILE`, sinon `<dossier de config>/ankou/session.toml`.
pub fn session_path() -> Option<PathBuf> {
    std::env::var_os("ANKOU_SESSION_FILE")
        .map(PathBuf::from)
        .or_else(|| lpdnsoft_settings::config_dir("ankou").map(|d| d.join("session.toml")))
}
