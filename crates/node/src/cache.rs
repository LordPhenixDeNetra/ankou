use crate::NodeError;
use ankou_proto::node::NodePeer;
use std::path::PathBuf;

/// Dernier état connu des pairs, appliqué au démarrage même sans API (spec §7.5).
pub struct PeerCache {
    path: PathBuf,
}

impl PeerCache {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Absent ou illisible → vide (le prochain `FullSync` le remplacera).
    pub fn load(&self) -> Vec<NodePeer> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(path = %self.path.display(), error = %e, "cache illisible, ignoré");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, peers: &[NodePeer]) -> Result<(), NodeError> {
        let bytes = serde_json::to_vec(peers).map_err(|e| NodeError::Io(e.to_string()))?;
        lpdnsoft_atomic::write_atomic(&self.path, &bytes).map_err(|e| NodeError::Io(e.to_string()))
    }
}
