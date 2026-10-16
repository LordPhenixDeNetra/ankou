//! Constantes et types partagés entre les binaires aNkou.

/// Identifiant technique (dossiers système, logs).
pub const APP_ID: &str = "ankou";
/// Nom affiché.
pub const APP_NAME: &str = "aNkou";
/// Version du protocole client ↔ service ↔ serveur.
pub const PROTOCOL_VERSION: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod api;
mod config;
mod error;
pub mod ipc;
mod keys;
pub mod node;

pub use config::{
    DEFAULT_DNS, DEFAULT_KEEPALIVE, Endpoint, EndpointError, InterfaceSection, PeerSection,
    ServerId, TunnelConfig,
};
pub use error::ErrorCode;
pub use keys::{KeyError, PrivateKey, PublicKey};
