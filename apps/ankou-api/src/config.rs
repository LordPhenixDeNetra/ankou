use anyhow::Context;
use ipnet::Ipv4Net;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Configuration de l'API (`/etc/ankou/api.toml`) ; `DATABASE_URL` l'emporte sur `database_url`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    #[serde(default)]
    pub database_url: String,
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    pub token_key_file: PathBuf,
    #[serde(default = "default_pool")]
    pub ip_pool: Ipv4Net,
    #[serde(default)]
    pub registration_open: bool,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8080))
}

fn default_pool() -> Ipv4Net {
    Ipv4Net::new(std::net::Ipv4Addr::new(10, 8, 0, 0), 16).unwrap_or_default()
}

impl ApiConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("configuration {}", path.display()))?;
        if let Ok(url) = std::env::var("DATABASE_URL") {
            config.database_url = url;
        }
        anyhow::ensure!(!config.database_url.is_empty(), "database_url manquant");
        Ok(config)
    }

    /// Valeurs par défaut sans fichier (tests).
    pub fn for_tests() -> Self {
        Self {
            database_url: String::new(),
            listen: default_listen(),
            token_key_file: PathBuf::new(),
            ip_pool: default_pool(),
            registration_open: false,
        }
    }
}
