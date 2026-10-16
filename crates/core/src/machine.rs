use crate::backoff;
use ankou_proto::{ErrorCode, ServerId};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct CoreSettings {
    pub kill_switch: bool,
    pub auto_reconnect: bool,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Disconnected,
    Connecting { server: ServerId },
    Connected { server: ServerId, since: Instant },
    Reconnecting { server: ServerId, attempt: u32 },
    Blocked { reason: ErrorCode },
    Error { code: ErrorCode },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connect {
        server: ServerId,
    },
    Disconnect,
    TunnelUp,
    Failure {
        code: ErrorCode,
    },
    LinkLost,
    Resume,
    NetworkChanged,
    RetryDue,
    /// Erreur sans issue (droits, moteur absent) : réessayer ne sert à rien.
    Fatal {
        code: ErrorCode,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Policy {
    /// Tout bloquer sauf le tunnel et l'endpoint du serveur.
    KillSwitch { server: ServerId },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    BringTunnelUp { server: ServerId },
    BringTunnelDown,
    ApplyFirewall(Policy),
    ClearFirewall,
    ScheduleRetry(Duration),
    Notify(State),
}

/// Machine à états de connexion (spec §6.4). Aucune E/S : le temps arrive par
/// `now`, l'aléa de la gigue par la graine.
pub struct Machine {
    settings: CoreSettings,
    state: State,
    rng: u64,
}

impl Machine {
    pub fn new(settings: CoreSettings, seed: u64) -> Self {
        Self {
            settings,
            state: State::Disconnected,
            rng: seed,
        }
    }

    /// Vaut pour les transitions suivantes ; l'état courant ne change pas.
    pub fn set_settings(&mut self, settings: CoreSettings) {
        self.settings = settings;
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        let next = match (&self.state, event) {
            (state, Event::Connect { server }) => {
                if *state != State::Disconnected {
                    actions.push(Action::BringTunnelDown);
                }
                if self.settings.kill_switch {
                    actions.push(Action::ApplyFirewall(Policy::KillSwitch {
                        server: server.clone(),
                    }));
                }
                actions.push(Action::BringTunnelUp {
                    server: server.clone(),
                });
                Some(State::Connecting { server })
            }
            (State::Disconnected, Event::Disconnect | Event::Fatal { .. }) => None,
            (_, Event::Fatal { code }) => Some(self.give_up(code, &mut actions)),
            (_, Event::Disconnect) => {
                actions.push(Action::BringTunnelDown);
                if self.settings.kill_switch {
                    actions.push(Action::ClearFirewall);
                }
                Some(State::Disconnected)
            }
            (
                State::Connecting { server } | State::Reconnecting { server, .. },
                Event::TunnelUp,
            ) => Some(State::Connected {
                server: server.clone(),
                since: now,
            }),
            (State::Connecting { server }, Event::Failure { code }) => {
                let server = server.clone();
                if self.settings.auto_reconnect && self.settings.max_attempts >= 1 {
                    Some(self.retry(server, 1, &mut actions))
                } else {
                    Some(self.give_up(code, &mut actions))
                }
            }
            (
                State::Connected { server, .. },
                Event::LinkLost | Event::Resume | Event::NetworkChanged,
            ) => {
                let server = server.clone();
                actions.push(Action::BringTunnelDown);
                Some(self.retry(server, 1, &mut actions))
            }
            (State::Reconnecting { server, .. }, Event::RetryDue) => {
                actions.push(Action::BringTunnelUp {
                    server: server.clone(),
                });
                None
            }
            (State::Reconnecting { server, attempt }, Event::Failure { code }) => {
                let (server, attempt) = (server.clone(), *attempt);
                if attempt < self.settings.max_attempts {
                    Some(self.retry(server, attempt + 1, &mut actions))
                } else {
                    Some(self.give_up(code, &mut actions))
                }
            }
            _ => None,
        };
        if let Some(state) = next {
            self.state = state.clone();
            actions.push(Action::Notify(state));
        }
        actions
    }

    fn retry(&mut self, server: ServerId, attempt: u32, actions: &mut Vec<Action>) -> State {
        actions.push(Action::ScheduleRetry(backoff(attempt, &mut self.rng)));
        State::Reconnecting { server, attempt }
    }

    /// Abandon : tunnel retiré ; avec kill switch, le pare-feu reste et l'état est `Blocked`.
    fn give_up(&self, code: ErrorCode, actions: &mut Vec<Action>) -> State {
        actions.push(Action::BringTunnelDown);
        if self.settings.kill_switch {
            State::Blocked { reason: code }
        } else {
            State::Error { code }
        }
    }
}
