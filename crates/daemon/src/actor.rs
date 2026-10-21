//! L'acteur : seul propriétaire de la machine à états et du tunnel. Les commandes des
//! sessions arrivent une par une, donc deux clients ne se marchent jamais dessus.

use crate::System;
use crate::store::Store;
use ankou_core::{Action, CoreSettings, Event, Machine, State};
use ankou_platform::{
    Dns, Firewall, FirewallPolicy, INTERFACE, PlatformError, SystemWatch, Tunnel,
};
use ankou_proto::ipc::{DaemonSettings, DaemonToClient, MAX_PROBED, ServerEntry, Status};
use ankou_proto::{
    DEFAULT_DNS, DEFAULT_KEEPALIVE, Endpoint, ErrorCode, InterfaceSection, PeerSection,
    TunnelConfig,
};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::MissedTickBehavior;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_POLL: Duration = Duration::from_millis(200);
/// Sans handshake depuis 3 min (WireGuard le renouvelle toutes les 2 min), le lien est mort.
const LINK_TIMEOUT: Duration = Duration::from_secs(180);
/// Horloge murale en avance de plus que ça sur l'horloge monotone : la machine a dormi.
const SUSPEND_GAP: Duration = Duration::from_secs(15);
/// Mesure d'un serveur (§6.5) : 3 handshakes de 1 s au plus.
const PROBE_TRIES: u32 = 3;
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

pub enum Command {
    Status(oneshot::Sender<Status>),
    /// Statut courant et abonnement, pris ensemble : aucun changement ne se perd entre les deux.
    Subscribe(oneshot::Sender<(Status, broadcast::Receiver<Status>)>),
    Connect(Option<TunnelConfig>, oneshot::Sender<DaemonToClient>),
    ConnectFastest(
        Option<TunnelConfig>,
        Vec<ServerEntry>,
        oneshot::Sender<DaemonToClient>,
    ),
    Disconnect(oneshot::Sender<DaemonToClient>),
    GetSettings(oneshot::Sender<DaemonSettings>),
    SetSettings(DaemonSettings, oneshot::Sender<DaemonToClient>),
    Stats(oneshot::Sender<DaemonToClient>),
}

pub struct Actor {
    machine: Machine,
    tunnel: Box<dyn Tunnel + Send>,
    firewall: Box<dyn Firewall>,
    dns: Box<dyn Dns>,
    watch: Box<dyn SystemWatch>,
    resolve: crate::Resolver,
    prober: Box<dyn ankou_platform::Prober>,
    /// Dernière politique appliquée ; `None` : pas de table.
    firewall_policy: Option<FirewallPolicy>,
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

fn core_settings(settings: &DaemonSettings) -> CoreSettings {
    CoreSettings {
        kill_switch: settings.kill_switch,
        auto_reconnect: true,
        max_attempts: u32::MAX,
    }
}

/// Adresse résolue, sinon celle du cache (DNS bloqué par le kill switch).
fn pick_endpoint(
    resolved: Result<SocketAddr, PlatformError>,
    cached: Option<SocketAddr>,
) -> Option<SocketAddr> {
    resolved.ok().or(cached)
}

fn failed(code: ErrorCode, detail: String) -> DaemonToClient {
    DaemonToClient::Failed { code, detail }
}

impl Actor {
    pub fn new(store: Store, system: System, tick: Duration) -> Self {
        let settings = store.load_settings();
        let (changes, _) = broadcast::channel(64);
        Self {
            machine: Machine::new(core_settings(&settings), seed()),
            profile: store.load_profile(),
            settings,
            last_route: system.watch.default_route(),
            last_wall: system.watch.wall_clock(),
            last_mono: Instant::now(),
            tunnel: system.tunnel,
            firewall: system.firewall,
            dns: system.dns,
            watch: system.watch,
            resolve: system.resolve,
            prober: system.prober,
            firewall_policy: None,
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
        self.prober.cleanup();
        // Règles laissées par un arrêt brutal : la protection continue, puis le service
        // se reconnecte au profil (§8.1).
        let leftover = self.firewall.is_active();
        match (&self.profile, leftover || self.settings.auto_connect) {
            (Some(cfg), true) => {
                if leftover {
                    tracing::warn!("reprise après arrêt brutal : kill switch toujours en place");
                } else {
                    tracing::info!("connexion automatique au démarrage");
                }
                let server = cfg.server_id();
                self.handle(Event::Connect { server });
            }
            _ if leftover => self.clear_firewall(),
            _ => {}
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
        // Arrêt du service (mise à jour, redémarrage, extinction) : seul `Disconnect` de
        // l'utilisateur lève la protection (§6.4). La table reste ; au démarrage suivant,
        // le service la retrouve et se reconnecte.
        self.dns.revert(INTERFACE);
        if let Err(e) = self.tunnel.down() {
            tracing::warn!(error = %e, "retrait du tunnel incomplet");
        }
        if self.status == Status::Disconnected {
            self.clear_firewall();
        } else {
            tracing::info!("arrêt pendant une connexion : protection gardée jusqu'au redémarrage");
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
            Command::ConnectFastest(config, servers, reply) => {
                let _ = reply.send(self.connect_fastest(config, servers));
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
                        self.machine.set_settings(core_settings(&settings));
                        self.settings = settings;
                        self.refresh_firewall();
                        DaemonToClient::Done
                    }
                    Err(e) => failed(ErrorCode::Internal, e),
                };
                let _ = reply.send(answer);
            }
            Command::Stats(reply) => {
                let _ = reply.send(self.traffic());
            }
        }
    }

    /// Compteurs du tunnel quand il est établi, sinon zéro. Illisibles : erreur, pas un
    /// faux zéro (le client en tirerait un débit énorme au relevé suivant).
    fn traffic(&self) -> DaemonToClient {
        let stats = match self.status {
            Status::Connected { .. } => match self.tunnel.stats() {
                Ok(stats) => stats,
                Err(e) => return failed(ErrorCode::Internal, e.to_string()),
            },
            _ => ankou_platform::TunnelStats::default(),
        };
        DaemonToClient::Stats {
            rx_bytes: stats.rx_bytes,
            tx_bytes: stats.tx_bytes,
        }
    }

    fn connect(&mut self, config: Option<TunnelConfig>) -> DaemonToClient {
        if let Some(cfg) = config {
            if let Err(e) = self.store.save_profile(&cfg) {
                return failed(ErrorCode::Internal, e);
            }
            // Résolu (et mis en cache) tant que l'ancien tunnel porte encore le DNS : une
            // fois coupé, le kill switch bloquerait la résolution.
            let _ = self.endpoint(&cfg.peer.endpoint);
            self.profile = Some(cfg);
        }
        let Some(cfg) = &self.profile else {
            return failed(ErrorCode::ProfileMissing, String::new());
        };
        let server = cfg.server_id();
        self.handle(Event::Connect { server });
        DaemonToClient::Done
    }

    /// Mesure les candidats (en parallèle), garde le plus rapide et s'y connecte (§6.5).
    // ponytail: mesure synchrone dans l'acteur (≤ 3 s) ; tâche à part si l'IPC en souffre.
    fn connect_fastest(
        &mut self,
        config: Option<TunnelConfig>,
        servers: Vec<ServerEntry>,
    ) -> DaemonToClient {
        let Some(base) = config.or_else(|| self.profile.clone()) else {
            return failed(ErrorCode::ProfileMissing, String::new());
        };
        let interface: InterfaceSection = base.interface;
        let allowed_ips = base.peer.allowed_ips;
        let candidates: Vec<(ServerEntry, SocketAddr)> = servers
            .into_iter()
            .take(MAX_PROBED)
            .filter_map(|s| match (self.resolve)(&s.endpoint) {
                Ok(addr) => Some((s, addr)),
                Err(e) => {
                    tracing::warn!(server = %s.name, error = %e, "candidat écarté");
                    None
                }
            })
            .collect();
        // Table en place : ouverte aux candidats le temps de la mesure, rien d'autre.
        let previous = self.firewall_policy.clone();
        if let Some(prev) = &previous {
            let mut policy = prev.clone();
            for (_, addr) in &candidates {
                if !policy.endpoints.contains(addr) {
                    policy.endpoints.push(*addr);
                }
            }
            if let Err(e) = self.firewall.apply(&policy) {
                return failed(ErrorCode::FirewallUnavailable, e.to_string());
            }
            self.firewall_policy = Some(policy);
        }
        let peer = |(s, addr): &(ServerEntry, SocketAddr)| PeerSection {
            public_key: s.public_key.clone(),
            endpoint: Endpoint {
                host: addr.ip().to_string(),
                port: addr.port(),
            },
            allowed_ips: allowed_ips.clone(),
            persistent_keepalive: DEFAULT_KEEPALIVE,
        };
        let peers: Vec<PeerSection> = candidates.iter().map(peer).collect();
        let prober = &*self.prober;
        let results: Vec<Vec<Duration>> = std::thread::scope(|scope| {
            let handles: Vec<_> = peers
                .iter()
                .enumerate()
                .map(|(slot, p)| {
                    let interface = &interface;
                    scope
                        .spawn(move || prober.probe(slot, interface, p, PROBE_TRIES, PROBE_TIMEOUT))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        });
        let Some(best) = ankou_core::fastest(&results) else {
            if let Some(prev) = previous {
                if let Err(e) = self.firewall.apply(&prev) {
                    tracing::warn!(error = %e, "pare-feu non rétabli après la mesure");
                }
                self.firewall_policy = Some(prev);
            }
            return failed(ErrorCode::ServerUnreachable, String::new());
        };
        let rtt = ankou_core::median(&results[best]).unwrap_or_default();
        let (server, addr) = &candidates[best];
        tracing::info!(server = %server.name, ?rtt, "serveur le plus rapide");
        if let Err(e) = self.store.save_endpoint(&server.endpoint, *addr) {
            tracing::warn!(error = %e, "cache de l'endpoint non écrit");
        }
        let profile = TunnelConfig {
            interface,
            peer: PeerSection {
                endpoint: server.endpoint.clone(),
                ..peer(&candidates[best])
            },
        };
        match self.connect(Some(profile)) {
            DaemonToClient::Done => DaemonToClient::Chosen {
                server: server.name.clone(),
                rtt_ms: u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX),
            },
            other => other,
        }
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
                let mut cfg = self.profile.clone()?;
                let Some(addr) = self.endpoint(&cfg.peer.endpoint) else {
                    self.detail = cfg.peer.endpoint.to_string();
                    return Some(Event::Failure {
                        code: ErrorCode::ServerUnreachable,
                    });
                };
                // Pare-feu avant le tunnel : aucun instant sans protection.
                if let Err(e) = self.apply_firewall(addr) {
                    if self.settings.kill_switch {
                        self.detail = e.to_string();
                        tracing::error!(error = %e, "kill switch impossible : pas de connexion");
                        return Some(Event::Fatal {
                            code: ErrorCode::FirewallUnavailable,
                        });
                    }
                    tracing::warn!(error = %e, "pare-feu indisponible : IPv6 non bloqué");
                }
                cfg.peer.endpoint = Endpoint {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                };
                match self.tunnel.up(&cfg) {
                    Ok(()) => {
                        self.reapply_firewall();
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
                self.dns.revert(INTERFACE);
                if let Err(e) = self.tunnel.down() {
                    tracing::warn!(error = %e, "retrait du tunnel incomplet");
                }
                None
            }
            Action::ScheduleRetry(delay) => {
                self.retry_at = Some(tokio::time::Instant::now() + delay);
                None
            }
            // Le pare-feu suit l'état (montée du tunnel, `notify`), y compris sans kill switch
            // pour l'IPv6 : ces actions du cœur sont déjà couvertes.
            Action::ApplyFirewall(_) | Action::ClearFirewall => None,
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
        match &status {
            Status::Disconnected | Status::Error { .. } => self.clear_firewall(),
            Status::Connected { .. } => self.set_dns(),
            _ => {}
        }
        self.status = status.clone();
        let _ = self.changes.send(status);
    }

    /// Résout l'endpoint et met le cache à jour ; sinon, l'adresse du cache.
    fn endpoint(&self, endpoint: &Endpoint) -> Option<SocketAddr> {
        let resolved = (self.resolve)(endpoint);
        if let Ok(addr) = resolved
            && let Err(e) = self.store.save_endpoint(endpoint, addr)
        {
            tracing::warn!(error = %e, "cache de l'endpoint non écrit");
        }
        pick_endpoint(resolved, self.store.load_endpoint(endpoint))
    }

    fn apply_firewall(&mut self, endpoint: SocketAddr) -> Result<(), PlatformError> {
        let policy = FirewallPolicy {
            kill_switch: self.settings.kill_switch,
            interface: INTERFACE.into(),
            endpoints: vec![endpoint],
        };
        if self.firewall_policy.as_ref() != Some(&policy) {
            self.firewall.apply(&policy)?;
            self.firewall_policy = Some(policy);
        }
        Ok(())
    }

    /// Même politique, posée de nouveau une fois le tunnel monté : sous Windows, la règle
    /// qui autorise le tunnel vise la carte, qui n'existait pas encore (ADR 0009).
    fn reapply_firewall(&mut self) {
        if let Some(policy) = &self.firewall_policy
            && let Err(e) = self.firewall.apply(policy)
        {
            tracing::warn!(error = %e, "pare-feu non réappliqué après la montée du tunnel");
        }
    }

    /// Réglage du kill switch changé : la table en place prend la nouvelle politique.
    fn refresh_firewall(&mut self) {
        let Some(endpoint) = self
            .firewall_policy
            .as_ref()
            .and_then(|p| p.endpoints.first().copied())
        else {
            return;
        };
        if let Err(e) = self.apply_firewall(endpoint) {
            tracing::warn!(error = %e, "pare-feu non mis à jour");
        }
    }

    fn clear_firewall(&mut self) {
        if let Err(e) = self.firewall.clear() {
            tracing::warn!(error = %e, "retrait du pare-feu incomplet");
        }
        self.firewall_policy = None;
    }

    fn set_dns(&mut self) {
        let servers = self
            .profile
            .as_ref()
            .map(|p| p.interface.dns.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| DEFAULT_DNS.to_vec());
        if let Err(e) = self.dns.set(INTERFACE, &servers) {
            tracing::warn!(error = %e, "DNS du tunnel non configuré (systemd-resolved ?)");
        }
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
            // Tunnel muet retiré pendant l'attente : sans kill switch, la machine garde Internet.
            self.execute(Action::BringTunnelDown);
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
            // Statistiques illisibles (interface supprimée) : lien tenu pour perdu.
            .unwrap_or(true);
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

#[cfg(test)]
mod tests {
    use super::pick_endpoint;
    use ankou_platform::PlatformError;

    #[test]
    fn cached_endpoint_used_when_resolution_fails() {
        let (r, c) = (
            "203.0.113.7:51820".parse().unwrap(),
            "198.51.100.9:51820".parse().unwrap(),
        );
        let err = || Err(PlatformError::Endpoint("nom.invalid".into()));
        assert_eq!(pick_endpoint(err(), Some(c)), Some(c));
        assert_eq!(pick_endpoint(Ok(r), Some(c)), Some(r));
        assert_eq!(pick_endpoint(err(), None), None);
    }
}
