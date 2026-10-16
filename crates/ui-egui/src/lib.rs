//! Interface graphique d'aNkou (spec §6.9) : une télécommande du service `ankoud`.

pub mod model;

#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("{0}")]
    Display(String),
}
