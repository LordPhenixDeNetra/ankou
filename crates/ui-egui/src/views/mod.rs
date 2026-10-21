//! Onglets de la fenêtre et textes partagés (erreurs, tailles).

pub mod about;
pub mod account;
pub mod home;
pub mod servers;
pub mod settings;

use crate::model::{Link, Notice, Problem, Scale, scale};
use crate::screen::{Screen, Tab};
use ankou_api_client::ClientError;
use ankou_proto::ErrorCode;
use ankou_proto::ipc::Status;
use lpdnsoft_i18n::{Localizer, t};

/// Bandeau en tête de chaque onglet : lien avec le service, dernier message.
pub fn banner(s: &mut Screen, ui: &mut egui::Ui) {
    if let Some(text) = link_text(&s.loc, &s.model.link) {
        ui.colored_label(ui.visuals().warn_fg_color, text);
    }
    if s.model.needs_enroll && s.tab != Tab::Account {
        ui.horizontal(|ui| {
            ui.label(t!(&s.loc, "ui-enroll-needed"));
            if ui.button(t!(&s.loc, "ui-go-account")).clicked() {
                s.tab = Tab::Account;
            }
        });
    }
    match &s.model.notice {
        Some(Notice::Chosen { server, rtt_ms }) => {
            ui.label(t!(
                &s.loc,
                "ui-chosen",
                server = server.as_str(),
                rtt = *rtt_ms
            ));
        }
        Some(Notice::Problem(p)) => {
            let server = current_server(s.model.status.as_ref());
            ui.colored_label(
                ui.visuals().error_fg_color,
                problem_text(&s.loc, p, &server),
            );
        }
        None => {}
    }
}

/// Texte du lien avec le service, sauf quand il répond.
pub fn link_text(loc: &Localizer, link: &Link) -> Option<String> {
    Some(match link {
        Link::Ready => return None,
        Link::Starting => t!(loc, "ui-daemon-starting"),
        Link::Missing if cfg!(windows) => t!(loc, "ui-daemon-missing-windows"),
        Link::Missing => t!(loc, "ui-daemon-missing"),
        Link::Denied if cfg!(windows) => t!(loc, "ui-daemon-denied-windows"),
        Link::Denied => t!(loc, "ui-daemon-denied"),
        Link::Incompatible(v) => t!(loc, "ui-daemon-incompatible", version = *v),
        Link::Broken(detail) => t!(loc, "ui-daemon-error", detail = detail.as_str()),
    })
}

pub fn current_server(status: Option<&Status>) -> String {
    match status {
        Some(
            Status::Connecting { server }
            | Status::Connected { server, .. }
            | Status::Reconnecting { server, .. },
        ) => server.clone(),
        _ => String::new(),
    }
}

/// Message traduit d'un code d'erreur du service.
pub fn error_text(loc: &Localizer, code: ErrorCode, server: &str, detail: &str) -> String {
    match code {
        ErrorCode::TunnelPermission => t!(loc, "error-tunnel-permission"),
        ErrorCode::TunnelUnavailable => t!(loc, "error-tunnel-unavailable", detail = detail),
        ErrorCode::ServerUnreachable => t!(loc, "error-server-unreachable", server = server),
        ErrorCode::FirewallUnavailable => t!(loc, "error-firewall-unavailable", detail = detail),
        // L'interface propose l'enrôlement plutôt que la commande de la CLI.
        ErrorCode::ProfileMissing => t!(loc, "ui-enroll-needed"),
        ErrorCode::Internal => t!(loc, "error-internal", detail = detail),
    }
}

pub fn problem_text(loc: &Localizer, p: &Problem, server: &str) -> String {
    match p {
        Problem::Daemon(code, detail) => error_text(loc, *code, server, detail),
        Problem::Api(e) => match e {
            ClientError::SessionExpired => t!(loc, "ui-session-expired"),
            ClientError::Api(code, detail) => t!(
                loc,
                "cli-api-error",
                code = code.as_str(),
                detail = detail.as_str()
            ),
            ClientError::Network(detail) => t!(loc, "cli-network-error", detail = detail.as_str()),
            ClientError::Io(detail) => t!(loc, "cli-local-error", detail = detail.as_str()),
            ClientError::NoServer => t!(loc, "ui-no-server"),
            ClientError::UnknownServer(name) => t!(loc, "ui-unknown-server", name = name.as_str()),
        },
    }
}

/// Taille lisible : « 1.5 MB », « 999 B ».
pub fn size_text(loc: &Localizer, n: u64) -> String {
    let (value, unit) = scale(n);
    let value = if unit == Scale::B {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    let unit = match unit {
        Scale::B => t!(loc, "ui-unit-b"),
        Scale::K => t!(loc, "ui-unit-kb"),
        Scale::M => t!(loc, "ui-unit-mb"),
        Scale::G => t!(loc, "ui-unit-gb"),
        Scale::T => t!(loc, "ui-unit-tb"),
    };
    t!(loc, "ui-size", value = value.as_str(), unit = unit.as_str())
}
