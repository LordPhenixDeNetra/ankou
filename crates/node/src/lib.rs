//! Logique d'`ankou-node` (spec §7.1, §7.5, ADR 0004).

mod backend;
mod cache;
mod reconcile;

#[cfg(feature = "fake")]
pub use backend::FakeBackend;
pub use backend::PeerBackend;
#[cfg(target_os = "linux")]
pub use backend::WgBackend;
pub use cache::PeerCache;
pub use reconcile::{PeerOp, reconcile};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NodeError {
    #[error("WireGuard : {0}")]
    Backend(String),
    #[error("fichier : {0}")]
    Io(String),
}
