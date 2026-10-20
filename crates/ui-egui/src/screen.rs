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
    // Formulaires du compte. Le mot de passe est vidé dès l'envoi.
    pub api_url: String,
    pub email: String,
    pub password: String,
    pub password_again: String,
    pub invitation: String,
    pub device_name: String,
    pub register_mode: bool,
    pub password_mismatch: bool,
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
            api_url: "https://".into(),
            email: String::new(),
            password: String::new(),
            password_again: String::new(),
            invitation: String::new(),
            device_name: String::new(),
            register_mode: false,
            password_mismatch: false,
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
            Tab::Account => views::account::show(self, ui),
            Tab::Settings => views::settings::show(self, ui),
            Tab::About => views::about::show(self, ui),
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
        self.save_prefs();
    }

    pub(crate) fn save_prefs(&self) {
        if let Some(dir) = &self.prefs_dir {
            crate::prefs::save_prefs(dir, &self.prefs);
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
    use ankou_proto::ipc::{DaemonSettings, Status};
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

    #[test]
    fn login_form_sends_login_and_clears_password() {
        let mut s = ready(Status::Disconnected);
        s.tab = Tab::Account;
        s.api_url.clear();
        let mut h = harness(s);
        type_into(&mut h, "API address", "https://api.example.org");
        type_into(&mut h, "Email", "a@x.org");
        type_into(&mut h, "Password", "correct horse battery");
        h.get_by_label("Sign in").click();
        h.run();
        assert_eq!(
            h.state().outbox,
            [Request::Login {
                api: "https://api.example.org".into(),
                email: "a@x.org".into(),
                password: "correct horse battery".into(),
            }]
        );
        assert!(h.state().password.is_empty());
    }

    #[test]
    fn register_with_mismatched_passwords_sends_nothing() {
        let mut s = ready(Status::Disconnected);
        s.tab = Tab::Account;
        s.register_mode = true;
        let mut h = harness(s);
        type_into(&mut h, "Invitation code", "abc");
        type_into(&mut h, "Email", "a@x.org");
        type_into(&mut h, "Password", "correct horse battery");
        type_into(&mut h, "Password (again)", "something else!");
        h.get_by_label("Create account").click();
        h.run();
        assert!(h.state().outbox.is_empty());
        h.get_by_label_contains("passwords differ");
    }

    #[test]
    fn enroll_form_sends_device_name() {
        let mut s = ready(Status::Disconnected);
        s.tab = Tab::Account;
        s.model.apply(Update::Account {
            session: Some("https://api.example.org".into()),
        });
        let mut h = harness(s);
        type_into(&mut h, "Device name", "Laptop");
        h.get_by_label("Enroll this device").click();
        h.run();
        assert_eq!(
            h.state().outbox,
            [Request::Enroll {
                name: "Laptop".into()
            }]
        );
    }

    #[test]
    fn kill_switch_toggle_sends_settings() {
        let mut s = ready(Status::Disconnected);
        s.tab = Tab::Settings;
        s.model.apply(Update::Settings(DaemonSettings {
            auto_connect: true,
            kill_switch: true,
        }));
        let mut h = harness(s);
        h.get_by_label("Kill switch").click();
        h.run();
        assert_eq!(
            h.state().outbox,
            [Request::SetSettings(DaemonSettings {
                auto_connect: true,
                kill_switch: false,
            })]
        );
    }

    #[test]
    fn language_switch_relabels_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ready(Status::Disconnected);
        s.prefs_dir = Some(dir.path().to_owned());
        s.tab = Tab::Settings;
        let mut h = harness(s);
        h.get_by_label("Français").click();
        h.run();
        h.get_by_label("Réglages");
        let prefs = crate::prefs::load_prefs(dir.path());
        assert_eq!(prefs.language.as_deref(), Some("fr"));
        let loc = crate::localizer(prefs.language.as_deref()).unwrap();
        let h = harness(Screen::new(loc, prefs, Some(dir.path().to_owned())));
        h.get_by_label("Réglages");
    }

    #[test]
    fn favourite_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ready(Status::Disconnected);
        s.prefs_dir = Some(dir.path().to_owned());
        s.model.apply(Update::Servers {
            list: vec![info("a", "FR")],
            cached: false,
        });
        s.tab = Tab::Servers;
        let mut h = harness(s);
        h.get_by_label("☆").click();
        h.run();
        assert_eq!(crate::prefs::load_prefs(dir.path()).favourites, ["a"]);
        h.get_by_label("★");
    }

    #[test]
    fn automatic_server_refresh_counts_as_busy() {
        let mut s = ready(Status::Disconnected);
        s.model.apply(Update::Account {
            session: Some("https://api.example.org".into()),
        });
        s.tab = Tab::Servers;
        let h = harness(s);
        assert_eq!(h.state().outbox, [Request::RefreshServers]);
        assert!(h.state().model.busy);
    }
}
