//! Interface graphique d'aNkou (spec §6.9) : une télécommande du service `ankoud`.

pub mod bridge;
pub mod model;
pub mod prefs;
pub mod screen;
mod views;

use lpdnsoft_i18n::{LanguageIdentifier, Localizer, Resource, langid};

#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("{0}")]
    Display(String),
    #[error("{0}")]
    I18n(#[from] lpdnsoft_i18n::I18nError),
}

/// Traductions embarquées ; `lang` (`"en"`, `"fr"`) sinon la langue du système, repli `en`.
pub fn localizer(lang: Option<&str>) -> Result<Localizer, UiError> {
    let mut loc = Localizer::new(
        vec![
            Resource {
                locale: langid!("en"),
                source: include_str!("../../../locales/en/ankou.ftl"),
            },
            Resource {
                locale: langid!("fr"),
                source: include_str!("../../../locales/fr/ankou.ftl"),
            },
        ],
        langid!("en"),
    )?;
    let requested: Vec<LanguageIdentifier> = match lang {
        Some(l) => l.parse().into_iter().collect(),
        None => lpdnsoft_i18n::system_locales(),
    };
    loc.negotiate(&requested);
    Ok(loc)
}
