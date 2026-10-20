//! Accueil : état, gros bouton, statistiques en direct.

use super::{error_text, size_text};
use crate::model::{Request, elapsed};
use crate::screen::{Screen, tone};
use ankou_proto::ipc::Status;
use lpdnsoft_i18n::t;
use lpdnsoft_ui::{card, status_color};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn show(s: &mut Screen, ui: &mut egui::Ui) {
    let status = s.model.status.clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            // Disque dessiné : la police par défaut n'a pas le caractère « ● ».
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 7.0, status_color(tone(status.as_ref())));
            ui.heading(state_text(s, status.as_ref()));
        });
        if let Some(Status::Blocked { reason: code } | Status::Error { code, .. }) = &status {
            let detail = match &status {
                Some(Status::Error { detail, .. }) => detail.as_str(),
                _ => "",
            };
            ui.label(error_text(&s.loc, *code, "", detail));
        }
    });
    let active = !matches!(
        status,
        None | Some(Status::Disconnected | Status::Error { .. })
    );
    let enabled = s.model.can_act();
    ui.add_space(8.0);
    let (label, request) = if active {
        (t!(&s.loc, "ui-disconnect"), Request::Disconnect)
    } else {
        (t!(&s.loc, "ui-connect"), Request::Connect)
    };
    let big =
        egui::Button::new(egui::RichText::new(label).size(22.0)).min_size(egui::vec2(220.0, 56.0));
    if ui.add_enabled(enabled, big).clicked() {
        s.send(request);
    }
    if ui
        .add_enabled(
            enabled && !active,
            egui::Button::new(t!(&s.loc, "ui-fastest")),
        )
        .clicked()
    {
        s.send(Request::ConnectFastest);
    }
    if let Some(Status::Connected { since, .. }) = status {
        ui.add_space(8.0);
        card(ui, |ui| traffic(s, ui, since));
        // Durée affichée à la seconde ; aucun redessin hors connexion.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
}

pub(crate) fn state_text(s: &Screen, status: Option<&Status>) -> String {
    let loc = &s.loc;
    match status {
        None => t!(loc, "ui-state-unknown"),
        Some(Status::Disconnected) => t!(loc, "ui-state-disconnected"),
        Some(Status::Connecting { server }) => {
            t!(loc, "ui-state-connecting", server = server.as_str())
        }
        Some(Status::Connected { server, .. }) => {
            t!(loc, "ui-state-connected", server = server.as_str())
        }
        Some(Status::Reconnecting { server, attempt }) => t!(
            loc,
            "ui-state-reconnecting",
            server = server.as_str(),
            attempt = *attempt
        ),
        Some(Status::Blocked { .. }) => t!(loc, "ui-state-blocked"),
        Some(Status::Error { .. }) => t!(loc, "ui-state-error"),
    }
}

fn traffic(s: &Screen, ui: &mut egui::Ui, since: u64) {
    let loc = &s.loc;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let time = elapsed(now.saturating_sub(since));
    ui.label(t!(loc, "ui-duration", time = time.as_str()));
    let rate = |n: u64| {
        let size = size_text(loc, n);
        t!(loc, "ui-rate", size = size.as_str())
    };
    let (down, rx) = (rate(s.model.rate.down), size_text(loc, s.model.rx));
    let (up, tx) = (rate(s.model.rate.up), size_text(loc, s.model.tx));
    ui.label(t!(
        loc,
        "ui-download",
        rate = down.as_str(),
        total = rx.as_str()
    ));
    ui.label(t!(
        loc,
        "ui-upload",
        rate = up.as_str(),
        total = tx.as_str()
    ));
}
