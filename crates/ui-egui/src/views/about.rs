//! À propos : version, licence, sources.

use crate::screen::Screen;
use lpdnsoft_i18n::t;
use lpdnsoft_ui::card;

const SOURCE: &str = "https://github.com/LordPhenixDeNetra/ankou";

pub fn show(s: &mut Screen, ui: &mut egui::Ui) {
    card(ui, |ui| {
        ui.heading(t!(&s.loc, "ui-window-title"));
        ui.label(t!(&s.loc, "ui-about-tagline"));
        ui.label(t!(
            &s.loc,
            "ui-about-version",
            version = ankou_proto::VERSION
        ));
        ui.label(t!(&s.loc, "ui-about-license"));
        ui.hyperlink_to(t!(&s.loc, "ui-about-source"), SOURCE);
    });
}
