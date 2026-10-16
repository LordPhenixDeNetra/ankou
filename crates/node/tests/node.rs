#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_node_core::{FakeBackend, PeerBackend, PeerCache, PeerOp, reconcile};
use ankou_proto::node::NodePeer;
use ankou_proto::{PrivateKey, PublicKey};
use std::net::Ipv4Addr;

fn key() -> PublicKey {
    PrivateKey::generate().public_key()
}

fn peer(k: &PublicKey, last: u8) -> NodePeer {
    NodePeer {
        public_key: k.clone(),
        ipv4: Ipv4Addr::new(10, 8, 0, last),
    }
}

#[test]
fn reconcile_adds_and_removes() {
    let (a, b, c) = (key(), key(), key());
    let ops = reconcile(
        &[peer(&a, 2), peer(&b, 3)],
        &[peer(&b, 3), peer(&c, 4)],
        &[],
    );
    assert_eq!(ops, [PeerOp::Remove(a), PeerOp::Add(peer(&c, 4))]);
}

#[test]
fn reconcile_updates_changed_address() {
    let a = key();
    assert_eq!(
        reconcile(&[peer(&a, 2)], &[peer(&a, 9)], &[]),
        [PeerOp::Update(peer(&a, 9))]
    );
}

#[test]
fn reconcile_is_empty_when_in_sync() {
    let (a, b) = (key(), key());
    let state = [peer(&a, 2), peer(&b, 3)];
    assert!(reconcile(&state, &[state[1].clone(), state[0].clone()], &[]).is_empty());
}

#[test]
fn reconcile_removes_unknown_peers_except_kept() {
    let (a, kept) = (key(), key());
    let ops = reconcile(
        &[peer(&a, 2), peer(&kept, 200)],
        &[],
        std::slice::from_ref(&kept),
    );
    assert_eq!(ops, [PeerOp::Remove(a)]);
}

#[test]
fn cache_roundtrip_and_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let cache = PeerCache::new(dir.path().join("peers.json"));
    assert!(cache.load().is_empty());
    let peers = vec![peer(&key(), 2), peer(&key(), 3)];
    cache.save(&peers).unwrap();
    assert_eq!(cache.load(), peers);
    std::fs::write(dir.path().join("peers.json"), "pas du json").unwrap();
    assert!(cache.load().is_empty());
    let missing = PeerCache::new(dir.path().join("absent").join("peers.json"));
    assert!(missing.save(&peers).is_err());
}

#[test]
fn fake_backend_applies_ops() {
    let mut backend = FakeBackend::default();
    let p = peer(&key(), 2);
    backend.apply(&PeerOp::Add(p.clone())).unwrap();
    assert_eq!(backend.peers().unwrap(), std::slice::from_ref(&p));
    backend
        .apply(&PeerOp::Remove(p.public_key.clone()))
        .unwrap();
    assert!(backend.peers().unwrap().is_empty());
    assert_eq!(backend.applied.len(), 2);
}
