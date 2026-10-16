//! Préférences propres à l'interface (langue, favoris), à côté de la session.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    /// `None` : langue du système.
    #[serde(default)]
    pub language: Option<String>,
    /// Noms des serveurs favoris, affichés en tête.
    #[serde(default)]
    pub favourites: Vec<String>,
}
