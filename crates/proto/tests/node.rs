#![allow(clippy::unwrap_used)]

use ankou_proto::PrivateKey;
use ankou_proto::node::{ApiToNode, NodePeer, NodeToApi};
use serde_json::{Value, json};

fn peer(ip: [u8; 4]) -> NodePeer {
    NodePeer {
        public_key: PrivateKey::generate().public_key(),
        ipv4: ip.into(),
    }
}

#[test]
fn messages_are_tagged() {
    let hello = serde_json::to_value(NodeToApi::Hello {
        protocol_version: 1,
    })
    .unwrap();
    assert_eq!(hello, json!({"type": "hello", "protocol_version": 1}));
    let p = peer([10, 8, 0, 2]);
    let removed = serde_json::to_value(ApiToNode::PeerRemoved {
        public_key: p.public_key.clone(),
    })
    .unwrap();
    assert_eq!(removed["type"], "peer_removed");
    assert_eq!(
        removed["public_key"],
        Value::String(p.public_key.to_string())
    );
    assert_eq!(
        serde_json::to_value(ApiToNode::Welcome).unwrap(),
        json!({"type": "welcome"})
    );
}

#[test]
fn full_sync_roundtrip() {
    let msg = ApiToNode::FullSync {
        peers: vec![peer([10, 8, 0, 2]), peer([10, 8, 0, 3])],
    };
    let text = serde_json::to_string(&msg).unwrap();
    assert_eq!(serde_json::from_str::<ApiToNode>(&text).unwrap(), msg);
    assert!(text.contains("\"ipv4\":\"10.8.0.3\""));
}
