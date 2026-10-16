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

#[test]
fn connect_fastest_is_tagged() {
    use ankou_proto::ipc::ServerEntry;
    let entry = ServerEntry {
        name: "vps1".into(),
        public_key: "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
            .parse()
            .unwrap(),
        endpoint: "1.2.3.4:51820".parse().unwrap(),
    };
    let msg = ClientToDaemon::ConnectFastest {
        config: None,
        servers: vec![entry],
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "connect_fastest");
    assert_eq!(v["servers"][0]["endpoint"], "1.2.3.4:51820");
    assert_eq!(serde_json::from_value::<ClientToDaemon>(v).unwrap(), msg);
    assert_eq!(
        serde_json::to_value(DaemonToClient::Chosen {
            server: "vps1".into(),
            rtt_ms: 12
        })
        .unwrap(),
        json!({"type": "chosen", "server": "vps1", "rtt_ms": 12})
    );
    assert_eq!(ankou_proto::ipc::MAX_PROBED, 16);
}
