//! Réglages : kill switch, connexion au démarrage (service), langue (interface).

use crate::model::Request;
use crate::screen::Screen;
use lpdnsoft_i18n::{LanguageIdentifier, t};
use lpdnsoft_ui::card;

pub fn show(s: &mut Screen, ui: &mut egui::Ui) {
    card(ui, |ui| {
        let current = s.model.settings.clone();
        let enabled = current.is_some() && s.model.can_act();
        let mut next = current.clone().unwrap_or_default();
        ui.add_enabled(
            enabled,
            egui::Checkbox::new(&mut next.kill_switch, t!(&s.loc, "ui-kill-switch")),
        );
        ui.label(t!(&s.loc, "ui-kill-switch-hint"));
        ui.add_enabled(
            enabled,
            egui::Checkbox::new(&mut next.auto_connect, t!(&s.loc, "ui-auto-connect")),
        );
        if current.is_some_and(|c| c != next) {
            s.send(Request::SetSettings(next));
        }
    });
    card(ui, |ui| {
        ui.label(t!(&s.loc, "ui-language"));
        let mut choice = s.prefs.language.clone();
        ui.horizontal(|ui| {
            ui.radio_value(&mut choice, None, t!(&s.loc, "ui-language-system"));
            // Noms des langues dans leur propre langue : pas de traduction.
            ui.radio_value(&mut choice, Some("en".to_owned()), "English");
            ui.radio_value(&mut choice, Some("fr".to_owned()), "Français");
        });
        if choice != s.prefs.language {
            let requested: Vec<LanguageIdentifier> = match &choice {
                Some(l) => l.parse().into_iter().collect(),
                None => lpdnsoft_i18n::system_locales(),
            };
            s.loc.negotiate(&requested);
            s.prefs.language = choice;
            s.save_prefs();
            ui.ctx().request_repaint();
        }
    });
}
