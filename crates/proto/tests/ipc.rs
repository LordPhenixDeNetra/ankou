#![allow(clippy::unwrap_used)]

use ankou_proto::TunnelConfig;
use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, Status};
use serde_json::json;

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
fn ipc_messages_are_tagged() {
    let hello = serde_json::to_value(ClientToDaemon::Hello {
        protocol_version: 1,
    })
    .unwrap();
    assert_eq!(hello, json!({"type": "hello", "protocol_version": 1}));
    let status = serde_json::to_value(DaemonToClient::Status {
        status: Status::Connected {
            server: "vps1".into(),
            since: 5,
        },
    })
    .unwrap();
    assert_eq!(
        status,
        json!({"type": "status", "status": {"state": "connected", "server": "vps1", "since": 5}})
    );
}

#[test]
fn connect_roundtrip_keeps_config() {
    let msg = ClientToDaemon::Connect {
        config: Some(config()),
    };
    let text = serde_json::to_string(&msg).unwrap();
    assert_eq!(serde_json::from_str::<ClientToDaemon>(&text).unwrap(), msg);
    assert_eq!(
        serde_json::to_value(ClientToDaemon::Connect { config: None }).unwrap(),
        json!({"type": "connect", "config": null})
    );
}
