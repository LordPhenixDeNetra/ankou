//! Fenêtre de l'interface : onglets et boîte d'envoi des requêtes.

use crate::model::{Model, Request};
use crate::prefs::Prefs;
use crate::views;
use ankou_proto::ipc::Status;
use lpdnsoft_i18n::{Localizer, t};
use lpdnsoft_ui::Tone;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Servers,
    Account,
    Settings,
    About,
}

/// État de la fenêtre ; `show` dessine, les requêtes s'accumulent dans `outbox`.
pub struct Screen {
    pub loc: Localizer,
    pub model: Model,
    pub tab: Tab,
    pub prefs: Prefs,
    /// Dossier où `prefs` est enregistré ; `None` : rien n'est écrit (tests).
    pub prefs_dir: Option<PathBuf>,
    pub outbox: Vec<Request>,
    pub search: String,
    /// Étiquette retenue dans la liste des serveurs.
    pub tag: Option<String>,
    /// Liste des serveurs déjà demandée une fois.
    pub servers_requested: bool,
}

impl Screen {
    pub fn new(loc: Localizer, prefs: Prefs, prefs_dir: Option<PathBuf>) -> Self {
        Self {
            loc,
            model: Model::default(),
            tab: Tab::Home,
            prefs,
            prefs_dir,
            outbox: Vec::new(),
            search: String::new(),
            tag: None,
            servers_requested: false,
        }
    }

    /// Barre d'onglets, bandeau d'alerte, puis l'onglet courant.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Home, t!(&self.loc, "ui-tab-home")),
                (Tab::Servers, t!(&self.loc, "ui-tab-servers")),
                (Tab::Account, t!(&self.loc, "ui-tab-account")),
                (Tab::Settings, t!(&self.loc, "ui-tab-settings")),
                (Tab::About, t!(&self.loc, "ui-tab-about")),
            ] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.separator();
        views::banner(self, ui);
        match self.tab {
            Tab::Home => views::home::show(self, ui),
            Tab::Servers => views::servers::show(self, ui),
            Tab::Account | Tab::Settings | Tab::About => {}
        }
    }

    /// Ajoute ou retire un serveur des favoris.
    pub(crate) fn toggle_favourite(&mut self, name: &str) {
        let favs = &mut self.prefs.favourites;
        match favs.iter().position(|f| f == name) {
            Some(i) => {
                favs.remove(i);
            }
            None => favs.push(name.to_owned()),
        }
    }

    /// Met la requête en file ; les actions attendent sa fin (`model.busy`).
    pub(crate) fn send(&mut self, r: Request) {
        self.model.notice = None;
        self.model.busy = true;
        self.outbox.push(r);
    }
}

/// Couleur de la pastille d'état.
pub fn tone(status: Option<&Status>) -> Tone {
    match status {
        Some(Status::Connected { .. }) => Tone::On,
        Some(Status::Connecting { .. } | Status::Reconnecting { .. }) => Tone::Busy,
        Some(Status::Blocked { .. } | Status::Error { .. }) => Tone::Alert,
        Some(Status::Disconnected) | None => Tone::Off,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Link, Request, Update};
    use crate::prefs::Prefs;
    use ankou_proto::PrivateKey;
    use ankou_proto::api::ServerInfo;
    use ankou_proto::ipc::Status;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::time::{Duration, Instant};

    pub fn english() -> Localizer {
        crate::localizer(Some("en")).unwrap()
    }

    pub fn harness(screen: Screen) -> Harness<'static, Screen> {
        let mut h = Harness::new_ui_state(|ui, s: &mut Screen| s.show(ui), screen);
        h.run();
        h
    }

    /// Donne le focus au champ puis tape le texte.
    pub fn type_into(h: &mut Harness<'static, Screen>, label: &str, text: &str) {
        h.get_by_label(label).focus();
        h.run();
        h.get_by_label(label).type_text(text);
        h.run();
    }

    pub fn ready(status: Status) -> Screen {
        let mut s = Screen::new(english(), Prefs::default(), None);
        s.model.apply(Update::Link(Link::Ready));
        s.model.apply(Update::Status(status));
        s
    }

    pub fn info(name: &str, country: &str) -> ServerInfo {
        ServerInfo {
            id: uuid::Uuid::nil(),
            name: name.into(),
            country: country.into(),
            city: format!("{name}-city"),
            endpoint: "1.2.3.4:51820".parse().unwrap(),
            public_key: PrivateKey::generate().public_key(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn connect_button_sends_connect() {
        let mut h = harness(ready(Status::Disconnected));
        h.get_by_label("Connect").click();
        h.run();
        assert_eq!(h.state().outbox, [Request::Connect]);
        assert!(h.state().model.busy);
    }

    #[test]
    fn busy_disables_connect_button() {
        let mut h = harness(ready(Status::Disconnected));
        h.get_by_label("Connect").click();
        h.run();
        h.get_by_label("Connect").click();
        h.run();
        assert_eq!(h.state().outbox.len(), 1);
    }

    #[test]
    fn connected_shows_server_and_traffic() {
        let mut s = ready(Status::Connected {
            server: "vps1".into(),
            since: 0,
        });
        let t = Instant::now();
        s.model.apply(Update::Traffic {
            rx: 0,
            tx: 0,
            at: t,
        });
        s.model.apply(Update::Traffic {
            rx: 1_500_000,
            tx: 2_000,
            at: t + Duration::from_secs(1),
        });
        let mut h = harness(s);
        h.get_by_label_contains("vps1");
        h.get_by_label_contains("1.5 MB");
        h.get_by_label("Disconnect").click();
        h.run();
        assert_eq!(h.state().outbox, [Request::Disconnect]);
    }

    #[test]
    fn daemon_missing_shows_banner_and_disables_actions() {
        let mut s = ready(Status::Disconnected);
        s.model.apply(Update::Link(Link::Missing));
        let mut h = harness(s);
        h.get_by_label_contains("ankoud service is not running");
        h.get_by_label("Connect").click();
        h.run();
        assert!(h.state().outbox.is_empty());
    }

    #[test]
    fn server_search_and_connect() {
        let mut s = ready(Status::Disconnected);
        s.model.apply(Update::Servers {
            list: vec![info("a", "FR"), info("b", "DE"), info("c", "FR")],
            cached: false,
        });
        s.tab = Tab::Servers;
        let mut h = harness(s);
        type_into(&mut h, "Search", "de");
        assert!(h.query_by_label_contains("a-city").is_none());
        h.get_by_label_contains("b-city");
        h.get_by_label("Connect").click();
        h.run();
        assert!(matches!(
            h.state().outbox.as_slice(),
            [Request::ConnectTo(e)] if e.name == "b"
        ));
    }

    #[test]
    fn tone_mapping() {
        use lpdnsoft_ui::Tone;
        assert_eq!(tone(None), Tone::Off);
        assert_eq!(tone(Some(&Status::Disconnected)), Tone::Off);
        let server = || "x".to_string();
        assert_eq!(
            tone(Some(&Status::Connecting { server: server() })),
            Tone::Busy
        );
        assert_eq!(
            tone(Some(&Status::Reconnecting {
                server: server(),
                attempt: 1
            })),
            Tone::Busy
        );
        assert_eq!(
            tone(Some(&Status::Connected {
                server: server(),
                since: 0
            })),
            Tone::On
        );
        assert_eq!(
            tone(Some(&Status::Blocked {
                reason: ankou_proto::ErrorCode::ServerUnreachable
            })),
            Tone::Alert
        );
    }
}
