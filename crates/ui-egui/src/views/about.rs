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
        if let Some(v) = &s.model.new_version {
            ui.hyperlink_to(
                t!(&s.loc, "ui-about-update", version = v.as_str()),
                ankou_api_client::RELEASES_PAGE,
            );
        }
        ui.label(t!(&s.loc, "ui-about-license"));
        ui.hyperlink_to(t!(&s.loc, "ui-about-source"), SOURCE);
    });
}
