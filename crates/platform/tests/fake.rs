#![allow(clippy::unwrap_used)]

use ankou_platform::fake::FakeTunnel;
use ankou_platform::{PlatformError, Tunnel, resolve_endpoint};
use ankou_proto::{Endpoint, ErrorCode, TunnelConfig};

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
    t.up(&config()).unwrap();
    assert!(t.stats().unwrap().last_handshake.is_some());
    t.down().unwrap();
    assert_eq!(t.calls, ["up ankou0", "down"]);
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
