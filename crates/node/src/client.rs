//! Boucle d'`ankou-node` : cache, WebSocket vers l'API, réconciliation, reconnexion (ADR 0004).

use crate::{NodeError, PeerBackend, PeerCache, PeerOp, reconcile};
use ankou_proto::PublicKey;
use ankou_proto::node::{ApiToNode, NODE_PROTOCOL_VERSION, NodePeer, NodeToApi};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `/etc/ankou/node.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// `wss://api.exemple.org/v1/nodes/ws`
    pub api_url: String,
    pub token_file: PathBuf,
    #[serde(default = "default_interface")]
    pub interface: String,
    #[serde(default = "default_cache")]
    pub cache_file: PathBuf,
    /// Pairs gérés à la main sur l'interface, jamais retirés.
    #[serde(default)]
    pub keep_peers: Vec<PublicKey>,
    #[serde(default = "default_stats")]
    pub stats_interval_secs: u64,
}

fn default_interface() -> String {
    "wg0".into()
}
fn default_cache() -> PathBuf {
    "/var/lib/ankou-node/peers.json".into()
}
fn default_stats() -> u64 {
    60
}

pub type SharedBackend = Arc<Mutex<dyn PeerBackend>>;

enum End {
    Stop,
    Lost(String),
}

struct Node {
    config: NodeConfig,
    backend: SharedBackend,
    cache: PeerCache,
    /// État voulu : dernier état reçu de l'API (ou du cache).
    desired: Vec<NodePeer>,
}

impl Node {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, dyn PeerBackend + 'static>, NodeError> {
        self.backend
            .lock()
            .map_err(|_| NodeError::Backend("verrou empoisonné".into()))
    }

    /// Amène l'interface à `desired` ; une opération en échec est journalisée, pas fatale.
    fn sync(&self) -> Result<(), NodeError> {
        let mut backend = self.lock()?;
        let ops = reconcile(&backend.peers()?, &self.desired, &self.config.keep_peers);
        for op in &ops {
            if let Err(e) = backend.apply(op) {
                tracing::warn!(?op, error = %e, "opération WireGuard en échec");
            }
        }
        Ok(())
    }

    fn apply(&mut self, msg: ApiToNode) -> Result<(), NodeError> {
        match msg {
            ApiToNode::FullSync { peers } => {
                self.desired = peers;
                self.sync()?;
            }
            ApiToNode::PeerAdded { peer } => {
                let op = if self.desired.iter().any(|p| p.public_key == peer.public_key) {
                    PeerOp::Update(peer.clone())
                } else {
                    PeerOp::Add(peer.clone())
                };
                self.desired.retain(|p| p.public_key != peer.public_key);
                self.desired.push(peer);
                self.lock()?.apply(&op)?;
            }
            ApiToNode::PeerRemoved { public_key } => {
                self.desired.retain(|p| p.public_key != public_key);
                if !self.config.keep_peers.contains(&public_key) {
                    self.lock()?.apply(&PeerOp::Remove(public_key))?;
                }
            }
            ApiToNode::Welcome | ApiToNode::Incompatible { .. } => return Ok(()),
        }
        self.cache.save(&self.desired)
    }

    /// Une connexion à l'API, jusqu'à sa perte ou l'arrêt demandé.
    async fn session(
        &mut self,
        token: &str,
        stop: &mut watch::Receiver<bool>,
        attempt: &mut u32,
    ) -> Result<End, NodeError> {
        let mut req = self
            .config
            .api_url
            .as_str()
            .into_client_request()
            .map_err(|e| NodeError::Api(e.to_string()))?;
        let auth = format!("Bearer {token}")
            .parse()
            .map_err(|_| NodeError::Api("jeton invalide".into()))?;
        req.headers_mut().insert("authorization", auth);
        let (mut ws, _) = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(req),
        )
        .await
        {
            Ok(Ok(ws)) => ws,
            Ok(Err(e)) => return Ok(End::Lost(e.to_string())),
            Err(_) => return Ok(End::Lost("délai de connexion dépassé".into())),
        };
        let hello = serde_json::to_string(&NodeToApi::Hello {
            protocol_version: NODE_PROTOCOL_VERSION,
        })
        .map_err(|e| NodeError::Api(e.to_string()))?;
        if let Err(e) = ws.send(Message::text(hello)).await {
            return Ok(End::Lost(e.to_string()));
        }
        let mut stats =
            tokio::time::interval(Duration::from_secs(self.config.stats_interval_secs.max(1)));
        stats.tick().await; // le premier tick est immédiat
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    let _ = ws.close(None).await;
                    return Ok(End::Stop);
                }
                incoming = ws.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let msg: ApiToNode = serde_json::from_str(&text).map_err(|e| NodeError::Api(e.to_string()))?;
                        match msg {
                            ApiToNode::Incompatible { api_version } => return Err(NodeError::Incompatible(api_version)),
                            ApiToNode::Welcome => {
                                tracing::info!("connecté à l'API");
                                *attempt = 0;
                            }
                            other => self.apply(other)?,
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(End::Lost("fermé par l'API".into())),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Ok(End::Lost(e.to_string())),
                },
                _ = stats.tick() => {
                    let peers = self.lock()?.stats().unwrap_or_default();
                    let msg = serde_json::to_string(&NodeToApi::Stats { peers }).map_err(|e| NodeError::Api(e.to_string()))?;
                    if let Err(e) = ws.send(Message::text(msg)).await {
                        return Ok(End::Lost(e.to_string()));
                    }
                }
            }
        }
    }
}

/// Applique le cache, puis reste connecté à l'API (reconnexion avec backoff) jusqu'à `stop`.
pub async fn run(
    config: NodeConfig,
    token: String,
    backend: SharedBackend,
    mut stop: watch::Receiver<bool>,
) -> Result<(), NodeError> {
    let cache = PeerCache::new(config.cache_file.clone());
    let desired = cache.load();
    let mut node = Node {
        config,
        backend,
        cache,
        desired,
    };
    // Un cache vide ne dit rien : ne rien retirer avant d'avoir parlé à l'API.
    if !node.desired.is_empty() {
        node.sync()?;
    }
    let mut rng = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut attempt = 0;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        match node.session(&token, &mut stop, &mut attempt).await? {
            End::Stop => return Ok(()),
            End::Lost(reason) => tracing::warn!(%reason, "connexion à l'API perdue"),
        }
        attempt += 1;
        let delay = ankou_core::backoff(attempt, &mut rng);
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}
