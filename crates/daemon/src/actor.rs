//! L'acteur : seul propriétaire de la machine à états et du tunnel. Les commandes des
//! sessions arrivent une par une, donc deux clients ne se marchent jamais dessus.

use crate::store::Store;
use ankou_core::{Action, CoreSettings, Event, Machine, State};
use ankou_platform::{PlatformError, SystemWatch, Tunnel};
use ankou_proto::ipc::{DaemonSettings, DaemonToClient, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::MissedTickBehavior;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_POLL: Duration = Duration::from_millis(200);
/// Sans handshake depuis 3 min (WireGuard le renouvelle toutes les 2 min), le lien est mort.
const LINK_TIMEOUT: Duration = Duration::from_secs(180);
/// Horloge murale en avance de plus que ça sur l'horloge monotone : la machine a dormi.
const SUSPEND_GAP: Duration = Duration::from_secs(15);

pub enum Command {
    Status(oneshot::Sender<Status>),
    /// Statut courant et abonnement, pris ensemble : aucun changement ne se perd entre les deux.
    Subscribe(oneshot::Sender<(Status, broadcast::Receiver<Status>)>),
    Connect(Option<TunnelConfig>, oneshot::Sender<DaemonToClient>),
    Disconnect(oneshot::Sender<DaemonToClient>),
    GetSettings(oneshot::Sender<DaemonSettings>),
    SetSettings(DaemonSettings, oneshot::Sender<DaemonToClient>),
}

pub struct Actor {
    machine: Machine,
    tunnel: Box<dyn Tunnel + Send>,
    watch: Box<dyn SystemWatch>,
    store: Store,
    profile: Option<TunnelConfig>,
    settings: DaemonSettings,
    status: Status,
    changes: broadcast::Sender<Status>,
    /// Détail de la dernière erreur du tunnel, repris par `Status::Error`.
    detail: String,
    tick: Duration,
    retry_at: Option<tokio::time::Instant>,
    handshake_deadline: Option<Instant>,
    /// Prochaine lecture du handshake (instant fixe : le tick ne la repousse pas).
    poll_at: Option<tokio::time::Instant>,
    last_route: Option<String>,
    last_mono: Instant,
    last_wall: SystemTime,
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

/// Attente jusqu'à `at` (gardée par `if at.is_some()` dans le `select!`).
async fn sleep_until(at: Option<tokio::time::Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    }
}

fn poll_later() -> Option<tokio::time::Instant> {
    Some(tokio::time::Instant::now() + HANDSHAKE_POLL)
}

fn failed(code: ErrorCode, detail: String) -> DaemonToClient {
    DaemonToClient::Failed { code, detail }
}

impl Actor {
    pub fn new(
        store: Store,
        tunnel: Box<dyn Tunnel + Send>,
        watch: Box<dyn SystemWatch>,
        tick: Duration,
    ) -> Self {
        let settings = CoreSettings {
            kill_switch: false,
            auto_reconnect: true,
            max_attempts: u32::MAX,
        };
        let (changes, _) = broadcast::channel(64);
        Self {
            machine: Machine::new(settings, seed()),
            profile: store.load_profile(),
            settings: store.load_settings(),
            last_route: watch.default_route(),
            last_wall: watch.wall_clock(),
            last_mono: Instant::now(),
            tunnel,
            watch,
            store,
            status: Status::Disconnected,
            changes,
            detail: String::new(),
            tick,
            retry_at: None,
            handshake_deadline: None,
            poll_at: None,
        }
    }

    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut stop: watch::Receiver<bool>,
    ) {
        // Fail-closed (§8.1) : interface et règles laissées par un crash.
        if let Err(e) = self.tunnel.down() {
            tracing::warn!(error = %e, "nettoyage initial incomplet");
        }
        if let (true, Some(cfg)) = (self.settings.auto_connect, &self.profile) {
            tracing::info!("connexion automatique au démarrage");
            let server = cfg.server_id();
            self.handle(Event::Connect { server });
        }
        let mut tick = tokio::time::interval(self.tick);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let (retry_at, poll_at) = (self.retry_at, self.poll_at);
            tokio::select! {
                _ = stop.changed() => break,
                cmd = commands.recv() => match cmd {
                    Some(cmd) => self.command(cmd),
                    None => break,
                },
                _ = sleep_until(retry_at), if retry_at.is_some() => {
                    self.retry_at = None;
                    self.handle(Event::RetryDue);
                }
                _ = sleep_until(poll_at), if poll_at.is_some() => {
                    self.poll_at = None;
                    self.poll_handshake();
                }
                _ = tick.tick() => self.watch_tick(),
            }
        }
        self.handle(Event::Disconnect);
        if let Err(e) = self.tunnel.down() {
            tracing::warn!(error = %e, "retrait du tunnel incomplet");
        }
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Status(reply) => {
                let _ = reply.send(self.status.clone());
            }
            Command::Subscribe(reply) => {
                let _ = reply.send((self.status.clone(), self.changes.subscribe()));
            }
            Command::Connect(config, reply) => {
                let _ = reply.send(self.connect(config));
            }
            Command::Disconnect(reply) => {
                self.handle(Event::Disconnect);
                let _ = reply.send(DaemonToClient::Done);
            }
            Command::GetSettings(reply) => {
                let _ = reply.send(self.settings.clone());
            }
            Command::SetSettings(settings, reply) => {
                let answer = match self.store.save_settings(&settings) {
                    Ok(()) => {
                        self.settings = settings;
                        DaemonToClient::Done
                    }
                    Err(e) => failed(ErrorCode::Internal, e),
                };
                let _ = reply.send(answer);
            }
        }
    }

    fn connect(&mut self, config: Option<TunnelConfig>) -> DaemonToClient {
        if let Some(cfg) = config {
            if let Err(e) = self.store.save_profile(&cfg) {
                return failed(ErrorCode::Internal, e);
            }
            self.profile = Some(cfg);
        }
        let Some(cfg) = &self.profile else {
            return failed(ErrorCode::ProfileMissing, String::new());
        };
        let server = cfg.server_id();
        self.handle(Event::Connect { server });
        DaemonToClient::Done
    }

    /// Passe l'événement au cœur et exécute ses actions, qui peuvent en produire d'autres.
    fn handle(&mut self, event: Event) {
        let mut events = VecDeque::from([event]);
        while let Some(event) = events.pop_front() {
            for action in self.machine.handle(event, Instant::now()) {
                if let Some(next) = self.execute(action) {
                    events.push_back(next);
                }
            }
        }
    }

    // ponytail: appels Tunnel synchrones dans l'acteur (quelques dizaines de ms) ; spawn_blocking si l'IPC en souffre.
    fn execute(&mut self, action: Action) -> Option<Event> {
        match action {
            Action::BringTunnelUp { .. } => {
                self.retry_at = None;
                let cfg = self.profile.as_ref()?;
                match self.tunnel.up(cfg) {
                    Ok(()) => {
                        self.handshake_deadline = Some(Instant::now() + HANDSHAKE_TIMEOUT);
                        self.poll_at = poll_later();
                        None
                    }
                    Err(e) => {
                        self.detail = e.detail().to_owned();
                        tracing::warn!(error = %e, "échec de la montée du tunnel");
                        Some(match e {
                            // Sans droits ou sans moteur, réessayer ne sert à rien.
                            PlatformError::Permission | PlatformError::Unavailable(_) => {
                                Event::Fatal { code: e.code() }
                            }
                            _ => Event::Failure { code: e.code() },
                        })
                    }
                }
            }
            Action::BringTunnelDown => {
                self.retry_at = None;
                self.handshake_deadline = None;
                self.poll_at = None;
                if let Err(e) = self.tunnel.down() {
                    tracing::warn!(error = %e, "retrait du tunnel incomplet");
                }
                None
            }
            Action::ScheduleRetry(delay) => {
                self.retry_at = Some(tokio::time::Instant::now() + delay);
                None
            }
            Action::ApplyFirewall(_) | Action::ClearFirewall => {
                tracing::warn!("pare-feu demandé : arrive avec le kill switch (étape 3b)");
                None
            }
            Action::Notify(state) => {
                self.notify(state);
                None
            }
        }
    }

    fn notify(&mut self, state: State) {
        let status = match state {
            State::Disconnected => Status::Disconnected,
            State::Connecting { server } => Status::Connecting { server: server.0 },
            State::Connected { server, .. } => Status::Connected {
                server: server.0,
                since: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            },
            State::Reconnecting { server, attempt } => Status::Reconnecting {
                server: server.0,
                attempt,
            },
            State::Blocked { reason } => Status::Blocked { reason },
            State::Error { code } => Status::Error {
                code,
                detail: self.detail.clone(),
            },
        };
        tracing::info!(?status, "état");
        self.status = status.clone();
        let _ = self.changes.send(status);
    }

    /// Le tunnel est neuf : tout handshake compte.
    fn poll_handshake(&mut self) {
        let seen = self
            .tunnel
            .stats()
            .ok()
            .and_then(|s| s.last_handshake)
            .is_some_and(|t| t > UNIX_EPOCH);
        if seen {
            self.handshake_deadline = None;
            self.handle(Event::TunnelUp);
        } else if self.handshake_deadline.is_some_and(|d| Instant::now() >= d) {
            self.handshake_deadline = None;
            self.detail = String::new();
            self.handle(Event::Failure {
                code: ErrorCode::ServerUnreachable,
            });
        } else {
            self.poll_at = poll_later();
        }
    }

    /// Relevés faits à chaque tick (la référence reste à jour), événements seulement connecté.
    fn watch_tick(&mut self) {
        let (mono, wall, route) = (
            Instant::now(),
            self.watch.wall_clock(),
            self.watch.default_route(),
        );
        let wall_elapsed = wall.duration_since(self.last_wall).unwrap_or_default();
        let resumed = wall_elapsed.saturating_sub(mono - self.last_mono) > SUSPEND_GAP;
        let route_changed = route != self.last_route;
        (self.last_mono, self.last_wall, self.last_route) = (mono, wall, route);
        if !matches!(self.status, Status::Connected { .. }) {
            return;
        }
        let lost = self
            .tunnel
            .stats()
            .map(|s| {
                s.last_handshake
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_none_or(|age| age > LINK_TIMEOUT)
            })
            .unwrap_or(false);
        let event = if lost {
            Event::LinkLost
        } else if route_changed {
            Event::NetworkChanged
        } else if resumed {
            Event::Resume
        } else {
            return;
        };
        tracing::info!(?event, "reconnexion");
        self.handle(event);
    }
}
