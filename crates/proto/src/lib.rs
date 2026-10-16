//! Constantes et types partagés entre les binaires aNkou.

/// Identifiant technique (dossiers système, logs).
pub const APP_ID: &str = "ankou";
/// Nom affiché.
pub const APP_NAME: &str = "aNkou";
/// Version du protocole client ↔ service ↔ serveur.
pub const PROTOCOL_VERSION: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
