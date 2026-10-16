//! Protocole WebSocket entre `ankou-node` et l'API (`/v1/nodes/ws`), en JSON.

use crate::PublicKey;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use time::OffsetDateTime;

/// Version du protocole annoncée dans `Hello`.
pub const NODE_PROTOCOL_VERSION: u32 = 1;

/// Pair WireGuard d'un appareil, tel que les nodes l'appliquent (`ipv4/32` autorisée).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodePeer {
    pub public_key: PublicKey,
    pub ipv4: Ipv4Addr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerStat {
    pub public_key: PublicKey,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_handshake: Option<OffsetDateTime>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeToApi {
    Hello { protocol_version: u32 },
    Stats { peers: Vec<PeerStat> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiToNode {
    Welcome,
    Incompatible { api_version: u32 },
    FullSync { peers: Vec<NodePeer> },
    PeerAdded { peer: NodePeer },
    PeerRemoved { public_key: PublicKey },
}
