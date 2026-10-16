use ankou_core::{Action, CoreSettings, Event, Machine, Policy, State, backoff};
use ankou_proto::{ErrorCode, ServerId};
use std::time::{Duration, Instant};

fn srv() -> ServerId {
    ServerId("vps1:51820".into())
}

fn settings(kill_switch: bool) -> CoreSettings {
    CoreSettings {
        kill_switch,
        auto_reconnect: true,
        max_attempts: 3,
    }
}

fn connected(kill_switch: bool, now: Instant) -> Machine {
    let mut m = Machine::new(settings(kill_switch), 7);
    m.handle(Event::Connect { server: srv() }, now);
    m.handle(Event::TunnelUp, now);
    m
}

#[test]
fn connect_then_tunnel_up() {
    let now = Instant::now();
    let mut m = Machine::new(settings(false), 7);
    assert_eq!(m.state(), &State::Disconnected);
    let a = m.handle(Event::Connect { server: srv() }, now);
    assert_eq!(
        a,
        [
            Action::BringTunnelUp { server: srv() },
            Action::Notify(State::Connecting { server: srv() })
        ]
    );
    m.handle(Event::TunnelUp, now);
    assert_eq!(
        m.state(),
        &State::Connected {
            server: srv(),
            since: now
        }
    );
}

#[test]
fn kill_switch_applies_firewall_before_tunnel() {
    let mut m = Machine::new(settings(true), 7);
    let a = m.handle(Event::Connect { server: srv() }, Instant::now());
    assert_eq!(
        a,
        [
            Action::ApplyFirewall(Policy::KillSwitch { server: srv() }),
            Action::BringTunnelUp { server: srv() },
            Action::Notify(State::Connecting { server: srv() })
        ]
    );
}

fn fail_until_give_up(m: &mut Machine, now: Instant) -> Vec<Action> {
    m.handle(Event::Connect { server: srv() }, now);
    for attempt in 1..=3 {
        let a = m.handle(
            Event::Failure {
                code: ErrorCode::ServerUnreachable,
            },
            now,
        );
        assert!(matches!(a[0], Action::ScheduleRetry(_)), "{a:?}");
        assert_eq!(
            m.state(),
            &State::Reconnecting {
                server: srv(),
                attempt
            }
        );
        assert_eq!(
            m.handle(Event::RetryDue, now),
            [Action::BringTunnelUp { server: srv() }]
        );
    }
    m.handle(
        Event::Failure {
            code: ErrorCode::ServerUnreachable,
        },
        now,
    )
}

#[test]
fn failure_schedules_retry_then_gives_up() {
    let mut m = Machine::new(settings(false), 7);
    let a = fail_until_give_up(&mut m, Instant::now());
    let err = State::Error {
        code: ErrorCode::ServerUnreachable,
    };
    assert_eq!(a, [Action::BringTunnelDown, Action::Notify(err.clone())]);
    assert_eq!(m.state(), &err);
}

#[test]
fn give_up_with_kill_switch_blocks() {
    let mut m = Machine::new(settings(true), 7);
    let a = fail_until_give_up(&mut m, Instant::now());
    assert!(!a.contains(&Action::ClearFirewall));
    assert_eq!(
        m.state(),
        &State::Blocked {
            reason: ErrorCode::ServerUnreachable
        }
    );
}

#[test]
fn no_auto_reconnect_goes_to_error() {
    let mut m = Machine::new(
        CoreSettings {
            kill_switch: false,
            auto_reconnect: false,
            max_attempts: 3,
        },
        7,
    );
    let now = Instant::now();
    m.handle(Event::Connect { server: srv() }, now);
    let a = m.handle(
        Event::Failure {
            code: ErrorCode::TunnelPermission,
        },
        now,
    );
    assert_eq!(a[0], Action::BringTunnelDown);
    assert_eq!(
        m.state(),
        &State::Error {
            code: ErrorCode::TunnelPermission
        }
    );
}

#[test]
fn link_lost_reconnects() {
    let now = Instant::now();
    let mut m = connected(false, now);
    let a = m.handle(Event::LinkLost, now);
    assert_eq!(a.len(), 3, "{a:?}");
    assert_eq!(a[0], Action::BringTunnelDown);
    assert!(matches!(a[1], Action::ScheduleRetry(_)));
    assert_eq!(
        a[2],
        Action::Notify(State::Reconnecting {
            server: srv(),
            attempt: 1
        })
    );
}

#[test]
fn disconnect_clears_everything() {
    let now = Instant::now();
    let mut m = connected(true, now);
    assert_eq!(
        m.handle(Event::Disconnect, now),
        [
            Action::BringTunnelDown,
            Action::ClearFirewall,
            Action::Notify(State::Disconnected)
        ]
    );
    assert!(m.handle(Event::Disconnect, now).is_empty());
}

#[test]
fn backoff_is_capped_with_jitter() {
    let ms = |d: Duration| d.as_millis();
    for seed in 0..1000u64 {
        let mut rng = seed;
        assert!((800..=1200).contains(&ms(backoff(1, &mut rng))));
        assert!((12_800..=19_200).contains(&ms(backoff(5, &mut rng))));
        assert!((24_000..=30_000).contains(&ms(backoff(10, &mut rng))));
        assert!(ms(backoff(u32::MAX, &mut rng)) <= 30_000);
    }
}

#[test]
fn fatal_while_connecting_gives_error() {
    let now = Instant::now();
    let mut m = Machine::new(settings(false), 7);
    m.handle(Event::Connect { server: srv() }, now);
    let a = m.handle(
        Event::Fatal {
            code: ErrorCode::TunnelPermission,
        },
        now,
    );
    let error = State::Error {
        code: ErrorCode::TunnelPermission,
    };
    assert_eq!(a, [Action::BringTunnelDown, Action::Notify(error.clone())]);
    assert_eq!(m.state(), &error);
}

#[test]
fn fatal_with_kill_switch_blocks() {
    let now = Instant::now();
    let mut m = connected(true, now);
    let a = m.handle(
        Event::Fatal {
            code: ErrorCode::TunnelUnavailable,
        },
        now,
    );
    assert!(!a.contains(&Action::ClearFirewall), "{a:?}");
    assert_eq!(
        m.state(),
        &State::Blocked {
            reason: ErrorCode::TunnelUnavailable
        }
    );
    // Depuis Disconnected : ignoré.
    let mut idle = Machine::new(settings(false), 7);
    assert!(
        idle.handle(
            Event::Fatal {
                code: ErrorCode::Internal
            },
            now
        )
        .is_empty()
    );
}

#[test]
fn set_settings_enables_kill_switch_for_next_connect() {
    let mut m = Machine::new(settings(false), 7);
    m.set_settings(settings(true));
    assert_eq!(m.state(), &State::Disconnected);
    let a = m.handle(Event::Connect { server: srv() }, Instant::now());
    assert!(
        a.contains(&Action::ApplyFirewall(Policy::KillSwitch { server: srv() })),
        "{a:?}"
    );
}
