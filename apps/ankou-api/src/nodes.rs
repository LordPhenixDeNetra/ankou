//! Diffusion des changements de pairs aux nodes connectés (ADR 0004).

use ankou_server_store::{PEER_CHANNEL, PeerChange, Store};
use sqlx::postgres::PgListener;
use std::time::Duration;
use tokio::sync::broadcast;

/// Capacité du canal ; un node trop lent reçoit `Lagged` et repart d'un `FullSync`.
const HUB_CAPACITY: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum HubEvent {
    Change(PeerChange),
    /// Des notifications ont pu être perdues : chaque node doit se resynchroniser.
    Resync,
}

#[derive(Clone)]
pub struct PeerHub {
    tx: broadcast::Sender<HubEvent>,
}

impl Default for PeerHub {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerHub {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(HUB_CAPACITY).0,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: HubEvent) {
        // Aucun node connecté : rien à faire.
        let _ = self.tx.send(event);
    }
}

/// Relaie les NOTIFY de PostgreSQL vers le hub, indéfiniment.
pub async fn listen_peer_changes(store: Store, hub: PeerHub) -> anyhow::Result<()> {
    loop {
        match PgListener::connect_with(store.pool()).await {
            Ok(mut listener) => {
                if let Err(e) = listener.listen(PEER_CHANNEL).await {
                    tracing::warn!(error = %e, "écoute des pairs impossible");
                } else {
                    // Notifications d'avant l'écoute peut-être manquées.
                    hub.publish(HubEvent::Resync);
                    loop {
                        match listener.try_recv().await {
                            Ok(Some(n)) => match PeerChange::from_payload(n.payload()) {
                                Some(change) => hub.publish(HubEvent::Change(change)),
                                None => {
                                    tracing::warn!(payload = n.payload(), "notification illisible")
                                }
                            },
                            // Connexion perdue puis rétablie par sqlx : resynchroniser.
                            Ok(None) => hub.publish(HubEvent::Resync),
                            Err(e) => {
                                tracing::warn!(error = %e, "écoute des pairs interrompue");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "connexion d'écoute impossible"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
