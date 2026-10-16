#![allow(clippy::unwrap_used)]

use ankou_proto::{Endpoint, ErrorCode, PrivateKey, PublicKey, TunnelConfig};

const PRIV: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
const PEER: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

fn sample() -> String {
    format!(
        r#"[interface]
private_key = "{PRIV}"
address = ["10.8.0.2/32"]

[peer]
public_key = "{PEER}"
endpoint = "vps1.example.org:51820"
allowed_ips = ["0.0.0.0/0", "::/0"]
"#
    )
}

#[test]
fn parses_full_config() {
    let c: TunnelConfig = toml::from_str(&sample()).unwrap();
    assert_eq!(c.interface.private_key.to_base64(), PRIV);
    assert_eq!(c.interface.address, vec!["10.8.0.2/32".parse().unwrap()]);
    assert_eq!(c.peer.public_key.to_string(), PEER);
    assert_eq!(c.peer.endpoint.host, "vps1.example.org");
    assert_eq!(c.peer.endpoint.port, 51820);
    assert_eq!(c.peer.allowed_ips.len(), 2);
    assert_eq!(c.peer.persistent_keepalive, 25);
    assert_eq!(c.server_id().to_string(), "vps1.example.org:51820");
}

#[test]
fn serializes_roundtrip() {
    let c: TunnelConfig = toml::from_str(&sample()).unwrap();
    let again: TunnelConfig = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
    assert_eq!(again, c);
}

#[test]
fn rejects_invalid_fields() {
    let short = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="; // 31 octets
    let cases = [
        (sample().replace(PRIV, "pas-base64!"), "private_key"),
        (sample().replace(PEER, short), "public_key"),
        (sample().replace("10.8.0.2/32", "10.8.0.2/33"), "address"),
        (
            sample().replace("vps1.example.org:51820", "vps1.example.org"),
            "endpoint",
        ),
        (
            sample().replace(r#"["0.0.0.0/0", "::/0"]"#, "[]"),
            "allowed_ips",
        ),
        (sample().split("[peer]").next().unwrap().to_string(), "peer"),
        (
            sample().replace("[peer]", "[peer]\ndns = \"1.1.1.1\""),
            "dns",
        ),
    ];
    for (text, field) in cases {
        let err = toml::from_str::<TunnelConfig>(&text)
            .unwrap_err()
            .to_string();
        assert!(err.contains(field), "champ {field} absent de : {err}");
    }
}

#[test]
fn public_key_matches_wireguard() {
    let k: PrivateKey = PRIV.parse().unwrap();
    assert_eq!(
        k.public_key().to_string(),
        "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw="
    );
    let g = PrivateKey::generate();
    assert_ne!(g.to_base64(), PrivateKey::generate().to_base64());
    assert_eq!(
        g.public_key(),
        g.to_base64().parse::<PrivateKey>().unwrap().public_key()
    );
}

#[test]
fn private_key_debug_is_redacted() {
    let k: PrivateKey = PRIV.parse().unwrap();
    let dbg = format!("{k:?}");
    assert!(!dbg.contains(PRIV) && !dbg.contains("yAnz"), "{dbg}");
}

#[test]
fn endpoint_accepts_ipv6() {
    let e: Endpoint = "[2001:db8::1]:51820".parse().unwrap();
    assert_eq!(e.host, "2001:db8::1");
    assert_eq!(e.port, 51820);
    assert_eq!(e.to_string(), "[2001:db8::1]:51820");
    assert!("hote:0".parse::<Endpoint>().is_err());
    assert!(":51820".parse::<Endpoint>().is_err());
    let _: PublicKey = PEER.parse().unwrap();
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(ErrorCode::TunnelPermission.as_str(), "E_TUNNEL_PERMISSION");
    assert_eq!(
        ErrorCode::TunnelUnavailable.as_str(),
        "E_TUNNEL_UNAVAILABLE"
    );
    assert_eq!(
        ErrorCode::ServerUnreachable.as_str(),
        "E_SERVER_UNREACHABLE"
    );
    assert_eq!(ErrorCode::Internal.as_str(), "E_INTERNAL");
    assert_eq!(
        toml::Value::try_from(ErrorCode::ServerUnreachable).unwrap(),
        toml::Value::String("E_SERVER_UNREACHABLE".into())
    );
}

#[test]
fn dns_is_optional_and_roundtrips() {
    let c: TunnelConfig = toml::from_str(&sample()).unwrap();
    assert!(c.interface.dns.is_empty());
    assert!(!toml::to_string(&c).unwrap().contains("dns"));
    let with = sample().replace(
        "address = [\"10.8.0.2/32\"]",
        "address = [\"10.8.0.2/32\"]\ndns = [\"1.1.1.1\"]",
    );
    let c: TunnelConfig = toml::from_str(&with).unwrap();
    assert_eq!(
        c.interface.dns,
        vec!["1.1.1.1".parse::<std::net::IpAddr>().unwrap()]
    );
    let again: TunnelConfig = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
    assert_eq!(again, c);
    assert_eq!(ankou_proto::DEFAULT_DNS.len(), 2);
    assert_eq!(ankou_proto::DEFAULT_DNS[0].to_string(), "9.9.9.9");
}

#[test]
fn kill_switch_is_on_by_default() {
    use ankou_proto::ipc::DaemonSettings;
    assert!(DaemonSettings::default().kill_switch);
    let s: DaemonSettings = toml::from_str("auto_connect = true").unwrap();
    assert!(s.auto_connect && s.kill_switch);
    assert_eq!(
        ErrorCode::FirewallUnavailable.as_str(),
        "E_FIREWALL_UNAVAILABLE"
    );
}
