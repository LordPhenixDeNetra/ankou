use ankou_core::{Action, CoreSettings, Event, Machine, State};
use ankou_proto::{ErrorCode, ServerId};
use proptest::prelude::*;
use std::time::Instant;

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        Just(Event::Connect {
            server: ServerId("a".into())
        }),
        Just(Event::Disconnect),
        Just(Event::TunnelUp),
        Just(Event::Failure {
            code: ErrorCode::ServerUnreachable
        }),
        Just(Event::LinkLost),
        Just(Event::Resume),
        Just(Event::NetworkChanged),
        Just(Event::RetryDue),
        Just(Event::Fatal {
            code: ErrorCode::TunnelPermission,
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn kill_switch_never_leaks(
        events in proptest::collection::vec(event(), 0..40),
        auto_reconnect: bool,
        max_attempts in 0u32..4,
        seed: u64,
    ) {
        let mut m = Machine::new(CoreSettings { kill_switch: true, auto_reconnect, max_attempts }, seed);
        let mut firewall_on = false;
        for ev in events {
            let is_disconnect = ev == Event::Disconnect;
            for a in m.handle(ev, Instant::now()) {
                match a {
                    Action::ApplyFirewall(_) => firewall_on = true,
                    Action::ClearFirewall => {
                        prop_assert!(is_disconnect, "ClearFirewall hors Disconnect");
                        firewall_on = false;
                    }
                    Action::BringTunnelUp { .. } => prop_assert!(firewall_on, "tunnel avant pare-feu"),
                    _ => {}
                }
            }
            if *m.state() != State::Disconnected {
                prop_assert!(firewall_on, "fuite possible dans {:?}", m.state());
            }
        }
    }
}
