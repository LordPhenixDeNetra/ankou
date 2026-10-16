//! Exécuteur provisoire des actions du cœur (déménage dans `ankou-daemon` à l'étape 3).

use crate::messages::error_message;
use ankou_core::{Action, CoreSettings, Event, Machine, State};
use ankou_platform::{PlatformError, Tunnel};
use ankou_proto::{ErrorCode, TunnelConfig};
use lpdnsoft_i18n::{Localizer, t};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

enum Wait {
    Handshake,
    Timeout,
    Stopped,
}

/// Connecte, garde le tunnel jusqu'à un message sur `stop` (Ctrl+C), puis nettoie.
// ponytail: pas de détection de perte de lien une fois connecté ; arrive avec ankoud (étape 3).
pub fn run(
    loc: &Localizer,
    cfg: &TunnelConfig,
    tunnel: &mut dyn Tunnel,
    retries: u32,
    stop: Receiver<()>,
) -> Result<(), ErrorCode> {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let settings = CoreSettings {
        kill_switch: false,
        auto_reconnect: retries > 0,
        max_attempts: retries,
    };
    let mut machine = Machine::new(settings, seed);
    let server = cfg.server_id();
    let mut detail = String::new();
    let mut events = VecDeque::from([Event::Connect {
        server: server.clone(),
    }]);
    loop {
        let event = match events.pop_front() {
            Some(e) => e,
            // Connecté : rien à faire jusqu'à Ctrl+C.
            None => {
                let _ = stop.recv();
                Event::Disconnect
            }
        };
        for action in machine.handle(event, Instant::now()) {
            match action {
                Action::BringTunnelUp { .. } => match tunnel.up(cfg) {
                    Ok(()) => events.push_back(match wait_handshake(tunnel, &stop) {
                        Wait::Handshake => Event::TunnelUp,
                        Wait::Timeout => Event::Failure {
                            code: ErrorCode::ServerUnreachable,
                        },
                        Wait::Stopped => Event::Disconnect,
                    }),
                    // Sans droits ou sans moteur, réessayer ne sert à rien.
                    Err(e @ (PlatformError::Permission | PlatformError::Unavailable(_))) => {
                        let _ = tunnel.down();
                        let detail = e.detail();
                        eprintln!("{}", error_message(loc, e.code(), &server.0, detail));
                        return Err(e.code());
                    }
                    Err(e) => {
                        detail = e.detail().to_owned();
                        events.push_back(Event::Failure { code: e.code() });
                    }
                },
                Action::BringTunnelDown => {
                    if let Err(e) = tunnel.down() {
                        tracing::warn!(error = %e, "retrait du tunnel incomplet");
                    }
                }
                Action::ApplyFirewall(_) | Action::ClearFirewall => {
                    // Le kill switch arrive à l'étape 3 ; il n'est jamais demandé ici.
                    let _ = tunnel.down();
                    return Err(ErrorCode::Internal);
                }
                Action::ScheduleRetry(delay) => match stop.recv_timeout(delay) {
                    Ok(()) => events.push_back(Event::Disconnect),
                    Err(RecvTimeoutError::Timeout) => events.push_back(Event::RetryDue),
                    Err(RecvTimeoutError::Disconnected) => {
                        std::thread::sleep(delay);
                        events.push_back(Event::RetryDue);
                    }
                },
                Action::Notify(state) => match state {
                    State::Connecting { server } => {
                        println!(
                            "{}",
                            t!(loc, "cli-status-connecting", server = server.0.as_str())
                        );
                    }
                    State::Connected { server, .. } => {
                        println!(
                            "{}",
                            t!(loc, "cli-status-connected", server = server.0.as_str())
                        );
                    }
                    State::Reconnecting { attempt, .. } => {
                        println!("{}", t!(loc, "cli-status-reconnecting", attempt = attempt));
                    }
                    State::Disconnected => {
                        println!("{}", t!(loc, "cli-status-disconnected"));
                        return Ok(());
                    }
                    State::Error { code } | State::Blocked { reason: code } => {
                        eprintln!("{}", error_message(loc, code, &server.0, &detail));
                        return Err(code);
                    }
                },
            }
        }
    }
}

/// Attend un premier handshake (l'interface est neuve : tout handshake compte).
fn wait_handshake(tunnel: &dyn Tunnel, stop: &Receiver<()>) -> Wait {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    while Instant::now() < deadline {
        let seen = tunnel
            .stats()
            .ok()
            .and_then(|s| s.last_handshake)
            .is_some_and(|t| t > UNIX_EPOCH);
        if seen {
            return Wait::Handshake;
        }
        match stop.recv_timeout(POLL) {
            Ok(()) => return Wait::Stopped,
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    Wait::Timeout
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankou_platform::fake::FakeTunnel;
    use ankou_proto::{ErrorCode, TunnelConfig};
    use std::sync::mpsc;
    use std::time::Duration;

    fn config() -> TunnelConfig {
        toml::from_str(include_str!("../tests/fixtures/client.toml")).unwrap()
    }

    fn loc() -> lpdnsoft_i18n::Localizer {
        crate::localizer().unwrap()
    }

    #[test]
    fn run_connects_then_stops_on_signal() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = tx.send(());
        });
        let mut t = FakeTunnel::default();
        assert_eq!(run(&loc(), &config(), &mut t, 3, rx), Ok(()));
        assert_eq!(t.calls, ["up ankou0", "down"]);
    }

    #[test]
    fn run_gives_up_without_handshake() {
        let (_tx, rx) = mpsc::channel();
        let mut t = FakeTunnel {
            handshake_after_up: false,
            ..FakeTunnel::default()
        };
        assert_eq!(
            run(&loc(), &config(), &mut t, 0, rx),
            Err(ErrorCode::ServerUnreachable)
        );
        assert_eq!(t.calls.last().map(String::as_str), Some("down"));
    }
}
