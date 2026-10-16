use ankou_proto::PublicKey;
use ankou_proto::node::NodePeer;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, PartialEq)]
pub enum PeerOp {
    Add(NodePeer),
    Remove(PublicKey),
    /// Même clé, autre adresse.
    Update(NodePeer),
}

fn by_key(peers: &[NodePeer]) -> BTreeMap<&PublicKey, Ipv4Addr> {
    peers.iter().map(|p| (&p.public_key, p.ipv4)).collect()
}

/// Opérations qui amènent `current` à `desired` ; les clés de `keep` ne sont jamais retirées.
/// Ordre : retraits, mises à jour, ajouts (une adresse libérée peut être réattribuée).
pub fn reconcile(current: &[NodePeer], desired: &[NodePeer], keep: &[PublicKey]) -> Vec<PeerOp> {
    let (current, desired) = (by_key(current), by_key(desired));
    let removes = current
        .keys()
        .filter(|k| !desired.contains_key(*k) && !keep.contains(k))
        .map(|k| PeerOp::Remove((*k).clone()));
    let peer = |k: &PublicKey, ip: Ipv4Addr| NodePeer {
        public_key: k.clone(),
        ipv4: ip,
    };
    let updates = desired
        .iter()
        .filter(|(k, ip)| current.get(*k).is_some_and(|c| c != *ip))
        .map(|(k, ip)| PeerOp::Update(peer(k, *ip)));
    let adds = desired
        .iter()
        .filter(|(k, _)| !current.contains_key(*k))
        .map(|(k, ip)| PeerOp::Add(peer(k, *ip)));
    removes.chain(updates).chain(adds).collect()
}
