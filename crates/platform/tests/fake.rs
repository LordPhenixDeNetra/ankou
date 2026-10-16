#![allow(clippy::unwrap_used)]

use ankou_platform::fake::{FakeTunnel, FakeWatch};
use ankou_platform::{PlatformError, SystemWatch, Tunnel, parse_default_route, resolve_endpoint};
use ankou_proto::{Endpoint, ErrorCode, TunnelConfig};
use std::time::{Duration, SystemTime};

fn config() -> TunnelConfig {
    toml::from_str(
        r#"[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]
[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "127.0.0.1:51820"
allowed_ips = ["0.0.0.0/0"]
"#,
    )
    .unwrap()
}

#[test]
fn fake_records_up_and_down() {
    let mut t = FakeTunnel::default();
    let handle = t.clone();
    t.up(&config()).unwrap();
    assert!(t.stats().unwrap().last_handshake.is_some());
    assert!(handle.is_up());
    t.down().unwrap();
    assert_eq!(handle.calls(), ["up ankou0", "down"]);
}

#[test]
fn fake_tunnel_ages_handshake() {
    let mut t = FakeTunnel::default();
    t.up(&config()).unwrap();
    t.age_handshake(Duration::from_secs(200));
    let seen = t.stats().unwrap().last_handshake.unwrap();
    assert!(seen <= SystemTime::now() - Duration::from_secs(200));
    t.set_handshakes(false);
    t.up(&config()).unwrap();
    assert_eq!(t.stats().unwrap().last_handshake, None);
    t.fail_up(Some(PlatformError::Permission));
    assert_eq!(t.up(&config()), Err(PlatformError::Permission));
}

#[test]
fn fake_watch_is_steerable() {
    let w = FakeWatch::default();
    assert_eq!(w.default_route().as_deref(), Some("eth0 via 10.0.0.1"));
    w.set_route(Some("wlan0 via 10.1.1.1"));
    assert_eq!(w.default_route().as_deref(), Some("wlan0 via 10.1.1.1"));
    let before = w.wall_clock();
    w.jump_clock(Duration::from_secs(3600));
    assert!(w.wall_clock() >= before + Duration::from_secs(3600));
}

#[test]
fn parses_default_route() {
    let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                 eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
                 eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    assert_eq!(
        parse_default_route(table).as_deref(),
        Some("eth0 via 192.168.1.1")
    );
    assert_eq!(parse_default_route("Iface\tDestination\n"), None);
}

#[test]
fn resolve_endpoint_rejects_unknown_host() {
    let ep = Endpoint {
        host: "nom.invalid".into(),
        port: 51820,
    };
    let err = resolve_endpoint(&ep).unwrap_err();
    assert!(matches!(err, PlatformError::Endpoint(_)), "{err:?}");
    assert_eq!(err.code(), ErrorCode::ServerUnreachable);
}

#[test]
fn resolve_endpoint_accepts_literal_ip() {
    let ep: Endpoint = "127.0.0.1:51820".parse().unwrap();
    assert_eq!(
        resolve_endpoint(&ep).unwrap(),
        "127.0.0.1:51820".parse().unwrap()
    );
}
