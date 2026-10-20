//! État de l'interface, sans E/S : le pont y verse des `Update`, les écrans le lisent.

use ankou_api_client::ClientError;
use ankou_proto::ErrorCode;
use ankou_proto::api::ServerInfo;
use ankou_proto::ipc::{DaemonSettings, ServerEntry, Status};
use std::time::Instant;

/// Lien avec le service `ankoud`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Starting,
    Ready,
    /// Service absent (socket introuvable ou refusé).
    Missing,
    /// Utilisateur hors du groupe `ankou`.
    Denied,
    /// Le service parle un autre protocole.
    Incompatible(u32),
    Broken(String),
}

/// Ce que l'interface demande au pont.
#[derive(Clone, PartialEq)]
pub enum Request {
    Connect,
    ConnectTo(ServerEntry),
    ConnectFastest,
    Disconnect,
    SetSettings(DaemonSettings),
    RefreshServers,
    Login {
        api: String,
        email: String,
        password: String,
    },
    Register {
        api: String,
        invitation: String,
        email: String,
        password: String,
    },
    Enroll {
        name: String,
    },
    Logout,
}

/// Le mot de passe n'apparaît jamais dans un journal.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect => f.write_str("Connect"),
            Self::ConnectTo(e) => f.debug_tuple("ConnectTo").field(&e.name).finish(),
            Self::ConnectFastest => f.write_str("ConnectFastest"),
            Self::Disconnect => f.write_str("Disconnect"),
            Self::SetSettings(s) => f.debug_tuple("SetSettings").field(s).finish(),
            Self::RefreshServers => f.write_str("RefreshServers"),
            Self::Login { api, email, .. } => f
                .debug_struct("Login")
                .field("api", api)
                .field("email", email)
                .field("password", &"***")
                .finish(),
            Self::Register {
                api,
                invitation,
                email,
                ..
            } => f
                .debug_struct("Register")
                .field("api", api)
                .field("invitation", invitation)
                .field("email", email)
                .field("password", &"***")
                .finish(),
            Self::Enroll { name } => f.debug_struct("Enroll").field("name", name).finish(),
            Self::Logout => f.write_str("Logout"),
        }
    }
}

/// Échec d'une requête, traduit par les écrans.
#[derive(Debug)]
pub enum Problem {
    Daemon(ErrorCode, String),
    Api(ClientError),
}

#[derive(Debug)]
pub enum Update {
    Link(Link),
    Status(Status),
    Settings(DaemonSettings),
    Traffic {
        rx: u64,
        tx: u64,
        at: Instant,
    },
    Servers {
        list: Vec<ServerInfo>,
        cached: bool,
    },
    /// URL de l'API si une session est enregistrée.
    Account {
        session: Option<String>,
    },
    Chosen {
        server: String,
        rtt_ms: u32,
    },
    Failed(Problem),
    /// Fin d'une requête réussie.
    Finished,
}

/// Octets par seconde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rate {
    pub down: u64,
    pub up: u64,
}

/// Dernier message à montrer à l'utilisateur.
#[derive(Debug)]
pub enum Notice {
    Chosen { server: String, rtt_ms: u32 },
    Problem(Problem),
}

#[derive(Debug)]
pub struct Model {
    pub link: Link,
    pub status: Option<Status>,
    pub settings: Option<DaemonSettings>,
    pub rx: u64,
    pub tx: u64,
    pub rate: Rate,
    pub servers: Vec<ServerInfo>,
    pub servers_cached: bool,
    pub session: Option<String>,
    /// Une requête est en cours : les actions attendent sa fin.
    pub busy: bool,
    pub notice: Option<Notice>,
    /// Le service n'a pas de profil : l'appareil doit être enrôlé.
    pub needs_enroll: bool,
    last_sample: Option<(u64, u64, Instant)>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            link: Link::Starting,
            status: None,
            settings: None,
            rx: 0,
            tx: 0,
            rate: Rate::default(),
            servers: Vec::new(),
            servers_cached: false,
            session: None,
            busy: false,
            notice: None,
            needs_enroll: false,
            last_sample: None,
        }
    }
}

impl Model {
    pub fn apply(&mut self, u: Update) {
        match u {
            Update::Link(link) => {
                // Service perdu : son dernier statut ne dit plus rien du tunnel.
                if link != Link::Ready {
                    self.status = None;
                    self.clear_traffic();
                }
                self.link = link;
            }
            Update::Status(status) => {
                if matches!(status, Status::Connected { .. }) {
                    self.needs_enroll = false;
                } else {
                    self.clear_traffic();
                }
                self.status = Some(status);
            }
            Update::Settings(s) => self.settings = Some(s),
            Update::Traffic { rx, tx, at } => {
                if !matches!(self.status, Some(Status::Connected { .. })) {
                    return;
                }
                if let Some(prev) = self.last_sample {
                    self.rate = rate(prev, (rx, tx, at));
                }
                self.last_sample = Some((rx, tx, at));
                self.rx = rx;
                self.tx = tx;
            }
            Update::Servers { list, cached } => {
                self.servers = list;
                self.servers_cached = cached;
            }
            Update::Account { session } => self.session = session,
            Update::Chosen { server, rtt_ms } => {
                self.notice = Some(Notice::Chosen { server, rtt_ms });
            }
            Update::Failed(problem) => {
                self.busy = false;
                if matches!(problem, Problem::Daemon(ErrorCode::ProfileMissing, _)) {
                    self.needs_enroll = true;
                }
                self.notice = Some(Notice::Problem(problem));
            }
            Update::Finished => self.busy = false,
        }
    }

    fn clear_traffic(&mut self) {
        self.rx = 0;
        self.tx = 0;
        self.rate = Rate::default();
        self.last_sample = None;
    }

    /// Le service répond et aucune requête n'est en cours.
    pub fn can_act(&self) -> bool {
        self.link == Link::Ready && !self.busy
    }
}

/// Unité d'une taille (base 1000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    B,
    K,
    M,
    G,
    T,
}

pub fn scale(n: u64) -> (f64, Scale) {
    let mut value = n as f64;
    let mut unit = Scale::B;
    for next in [Scale::K, Scale::M, Scale::G, Scale::T] {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = next;
    }
    (value, unit)
}

/// Durée de connexion : `MM:SS`, puis `H:MM:SS` au-delà d'une heure.
pub fn elapsed(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Débit entre deux relevés ; un compteur qui recule (nouveau tunnel) donne zéro.
pub fn rate(prev: (u64, u64, Instant), cur: (u64, u64, Instant)) -> Rate {
    let dt = cur.2.saturating_duration_since(prev.2).as_secs_f64();
    if dt <= 0.0 {
        return Rate::default();
    }
    let per_sec = |a: u64, b: u64| (b.saturating_sub(a) as f64 / dt) as u64;
    Rate {
        down: per_sec(prev.0, cur.0),
        up: per_sec(prev.1, cur.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankou_proto::ErrorCode;
    use ankou_proto::ipc::Status;
    use std::time::{Duration, Instant};

    #[test]
    fn scale_boundaries() {
        assert_eq!(scale(999), (999.0, Scale::B));
        assert_eq!(scale(1_000), (1.0, Scale::K));
        assert_eq!(scale(1_500_000), (1.5, Scale::M));
    }

    #[test]
    fn elapsed_formats() {
        assert_eq!(elapsed(65), "01:05");
        assert_eq!(elapsed(3_725), "1:02:05");
    }

    #[test]
    fn rate_over_one_second() {
        let t = Instant::now();
        let r = rate((0, 0, t), (2_000, 500, t + Duration::from_secs(1)));
        assert_eq!(
            r,
            Rate {
                down: 2_000,
                up: 500
            }
        );
    }

    #[test]
    fn rate_after_counter_reset_is_zero() {
        let t = Instant::now();
        let r = rate((5_000, 5_000, t), (10, 10, t + Duration::from_secs(1)));
        assert_eq!(r, Rate { down: 0, up: 0 });
        assert_eq!(rate((0, 0, t), (9, 9, t)), Rate { down: 0, up: 0 });
    }

    #[test]
    fn profile_missing_asks_for_enrollment() {
        let mut m = Model {
            busy: true,
            ..Model::default()
        };
        m.apply(Update::Failed(Problem::Daemon(
            ErrorCode::ProfileMissing,
            String::new(),
        )));
        assert!(m.needs_enroll);
        assert!(!m.busy);
    }

    #[test]
    fn leaving_connected_clears_traffic() {
        let mut m = Model::default();
        let t = Instant::now();
        m.apply(Update::Status(Status::Connected {
            server: "vps1".into(),
            since: 1,
        }));
        m.apply(Update::Traffic {
            rx: 0,
            tx: 0,
            at: t,
        });
        m.apply(Update::Traffic {
            rx: 1_000,
            tx: 1_000,
            at: t + Duration::from_secs(1),
        });
        assert_eq!(
            m.rate,
            Rate {
                down: 1_000,
                up: 1_000
            }
        );
        m.apply(Update::Status(Status::Disconnected));
        assert_eq!((m.rx, m.tx), (0, 0));
        assert_eq!(m.rate, Rate::default());
    }

    #[test]
    fn can_act_needs_ready_and_idle() {
        let mut m = Model::default();
        assert!(!m.can_act());
        m.apply(Update::Link(Link::Ready));
        assert!(m.can_act());
        m.busy = true;
        assert!(!m.can_act());
        m.apply(Update::Finished);
        assert!(m.can_act());
    }

    #[test]
    fn request_debug_hides_password() {
        let r = Request::Login {
            api: "https://a".into(),
            email: "a@x.org".into(),
            password: "s3cret".into(),
        };
        let shown = format!("{r:?}");
        assert!(!shown.contains("s3cret"));
        assert!(shown.contains("a@x.org"));
        let r = Request::Register {
            api: "https://a".into(),
            invitation: "i".into(),
            email: "a@x.org".into(),
            password: "s3cret".into(),
        };
        assert!(!format!("{r:?}").contains("s3cret"));
    }

    #[test]
    fn losing_the_daemon_forgets_the_status() {
        let mut m = Model::default();
        m.apply(Update::Link(Link::Ready));
        m.apply(Update::Status(Status::Connected {
            server: "vps1".into(),
            since: 1,
        }));
        m.apply(Update::Traffic {
            rx: 5,
            tx: 5,
            at: Instant::now(),
        });
        m.apply(Update::Link(Link::Missing));
        assert_eq!(m.status, None);
        assert_eq!((m.rx, m.tx), (0, 0));
    }

    #[test]
    fn traffic_outside_connected_is_ignored() {
        let mut m = Model::default();
        m.apply(Update::Status(Status::Disconnected));
        m.apply(Update::Traffic {
            rx: 9,
            tx: 9,
            at: Instant::now(),
        });
        assert_eq!((m.rx, m.tx), (0, 0));
    }
}
