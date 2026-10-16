use crate::servers::{Server, ServerRow, decode};
use crate::{Store, StoreError};
use ankou_proto::PublicKey;
use ankou_proto::node::{NodePeer, PeerStat};
use ipnet::IpNet;
use serde::Deserialize;
use std::net::IpAddr;

/// Canal PostgreSQL des changements de pairs (voir la migration 0002).
pub const PEER_CHANNEL: &str = "ankou_peers";

#[derive(Debug, Clone, PartialEq)]
pub enum PeerChange {
    Added(NodePeer),
    Removed(PublicKey),
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Payload {
    Added {
        public_key: PublicKey,
        ipv4: std::net::Ipv4Addr,
    },
    Removed {
        public_key: PublicKey,
    },
}

impl PeerChange {
    /// Charge utile JSON d'un NOTIFY ; `None` si elle est illisible.
    pub fn from_payload(s: &str) -> Option<PeerChange> {
        Some(match serde_json::from_str(s).ok()? {
            Payload::Added { public_key, ipv4 } => PeerChange::Added(NodePeer { public_key, ipv4 }),
            Payload::Removed { public_key } => PeerChange::Removed(public_key),
        })
    }
}

impl Store {
    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    /// Serveur **actif** dont le jeton de node a ce haché.
    pub async fn server_by_node_token(
        &self,
        token_hash: [u8; 32],
    ) -> Result<Option<Server>, StoreError> {
        sqlx::query_as!(
            ServerRow,
            "SELECT id, name, country, city, endpoint, public_key, tags, active
             FROM servers WHERE node_token_hash = $1 AND active",
            &token_hash[..]
        )
        .fetch_optional(&self.pool)
        .await?
        .map(Server::try_from)
        .transpose()
    }

    /// Pairs de tous les appareils non révoqués.
    pub async fn active_peers(&self) -> Result<Vec<NodePeer>, StoreError> {
        sqlx::query!("SELECT public_key, ipv4 FROM devices WHERE revoked_at IS NULL ORDER BY ipv4")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| {
                let ipv4 = match IpNet::addr(&r.ipv4) {
                    IpAddr::V4(ip) => ip,
                    IpAddr::V6(_) => return decode(Err(std::io::Error::other("ipv4 attendue"))),
                };
                Ok(NodePeer {
                    public_key: decode(r.public_key.parse())?,
                    ipv4,
                })
            })
            .collect()
    }

    /// Garde le handshake le plus récent connu de chaque appareil ; clés inconnues ignorées.
    pub async fn record_stats(&self, peers: &[PeerStat]) -> Result<(), StoreError> {
        for p in peers {
            if let Some(at) = p.last_handshake {
                sqlx::query!(
                    "UPDATE devices SET last_handshake = GREATEST(last_handshake, $2)
                     WHERE public_key = $1 AND revoked_at IS NULL",
                    p.public_key.to_string(),
                    at
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }
}
