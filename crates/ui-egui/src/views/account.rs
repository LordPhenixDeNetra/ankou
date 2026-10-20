//! Compte : connexion, inscription par invitation, enrôlement de l'appareil.

use crate::model::Request;
use crate::screen::Screen;
use lpdnsoft_i18n::t;
use lpdnsoft_ui::card;

/// Champ texte précédé de son libellé (lu aussi par les lecteurs d'écran).
fn field(ui: &mut egui::Ui, label: String, value: &mut String, secret: bool) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::TextEdit::singleline(value).password(secret))
            .labelled_by(label.id);
    });
}

pub fn show(s: &mut Screen, ui: &mut egui::Ui) {
    let enabled = !s.model.busy;
    match s.model.session.clone() {
        Some(api) => card(ui, |ui| {
            ui.label(t!(&s.loc, "ui-signed-in", api = api.as_str()));
            field(ui, t!(&s.loc, "ui-device-name"), &mut s.device_name, false);
            let ok = enabled
                && s.model.link == crate::model::Link::Ready
                && !s.device_name.trim().is_empty();
            if ui
                .add_enabled(ok, egui::Button::new(t!(&s.loc, "ui-enroll")))
                .clicked()
            {
                let name = s.device_name.trim().to_owned();
                s.send(Request::Enroll { name });
            }
            ui.separator();
            if ui
                .add_enabled(enabled, egui::Button::new(t!(&s.loc, "ui-logout")))
                .clicked()
            {
                s.send(Request::Logout);
            }
        }),
        None => card(ui, |ui| {
            field(ui, t!(&s.loc, "ui-api-url"), &mut s.api_url, false);
            if s.register_mode {
                field(ui, t!(&s.loc, "ui-invitation"), &mut s.invitation, false);
            }
            field(ui, t!(&s.loc, "ui-email"), &mut s.email, false);
            field(ui, t!(&s.loc, "ui-password"), &mut s.password, true);
            if s.register_mode {
                field(
                    ui,
                    t!(&s.loc, "ui-password-again"),
                    &mut s.password_again,
                    true,
                );
                if s.password_mismatch {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        t!(&s.loc, "ui-password-mismatch"),
                    );
                }
                if ui
                    .add_enabled(enabled, egui::Button::new(t!(&s.loc, "ui-register")))
                    .clicked()
                {
                    register(s);
                }
                if ui.link(t!(&s.loc, "ui-have-account")).clicked() {
                    s.register_mode = false;
                }
            } else {
                if ui
                    .add_enabled(enabled, egui::Button::new(t!(&s.loc, "ui-login")))
                    .clicked()
                {
                    let password = std::mem::take(&mut s.password);
                    s.send(Request::Login {
                        api: s.api_url.trim().to_owned(),
                        email: s.email.trim().to_owned(),
                        password,
                    });
                }
                if ui.link(t!(&s.loc, "ui-need-account")).clicked() {
                    s.register_mode = true;
                }
            }
        }),
    };
}

fn register(s: &mut Screen) {
    s.password_mismatch = s.password != s.password_again;
    if s.password_mismatch {
        return;
    }
    let password = std::mem::take(&mut s.password);
    s.password_again.clear();
    s.send(Request::Register {
        api: s.api_url.trim().to_owned(),
        invitation: s.invitation.trim().to_owned(),
        email: s.email.trim().to_owned(),
        password,
    });
}
