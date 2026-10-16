//! `GET /v1/nodes/ws` : WebSocket des nodes (spec §7.3, §7.4).

use crate::AppState;
use crate::error::AppError;
use crate::nodes::HubEvent;
use ankou_proto::api::ApiErrorCode;
use ankou_proto::node::{ApiToNode, NODE_PROTOCOL_VERSION, NodeToApi};
use ankou_server_domain::hash_secret;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use axum::response::Response;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const PING_EVERY: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let invalid = || AppError::new(ApiErrorCode::AuthInvalid, "invalid node token");
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(invalid)?;
    let server = state
        .store
        .server_by_node_token(hash_secret(token))
        .await?
        .ok_or_else(invalid)?;
    Ok(upgrade.on_upgrade(move |socket| async move {
        tracing::info!(server = %server.name, "node connecté");
        if let Err(e) = session(state, socket).await {
            tracing::info!(server = %server.name, error = %e, "node déconnecté");
        }
    }))
}

async fn send(socket: &mut WebSocket, msg: &ApiToNode) -> anyhow::Result<()> {
    socket
        .send(Message::text(serde_json::to_string(msg)?))
        .await?;
    Ok(())
}

async fn full_sync(state: &AppState, socket: &mut WebSocket) -> anyhow::Result<()> {
    let peers = state.store.active_peers().await?;
    send(socket, &ApiToNode::FullSync { peers }).await
}

async fn session(state: AppState, mut socket: WebSocket) -> anyhow::Result<()> {
    let hello = tokio::time::timeout(HELLO_TIMEOUT, socket.recv()).await;
    let version = match hello {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str(&t) {
            Ok(NodeToApi::Hello { protocol_version }) => protocol_version,
            _ => anyhow::bail!("Hello attendu"),
        },
        _ => anyhow::bail!("pas de Hello"),
    };
    if version != NODE_PROTOCOL_VERSION {
        send(
            &mut socket,
            &ApiToNode::Incompatible {
                api_version: NODE_PROTOCOL_VERSION,
            },
        )
        .await?;
        let _ = socket.send(Message::Close(None)).await;
        anyhow::bail!("protocole {version} incompatible");
    }
    send(&mut socket, &ApiToNode::Welcome).await?;
    // S'abonner avant de lire la base : aucun changement ne peut tomber entre les deux.
    let mut events = state.hub.subscribe();
    full_sync(&state, &mut socket).await?;
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Ok(NodeToApi::Stats { peers }) = serde_json::from_str(&t) {
                        state.store.record_stats(&peers).await?;
                    }
                }
                Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
            },
            event = events.recv() => match event {
                Ok(HubEvent::Change(ankou_server_store::PeerChange::Added(peer))) => {
                    send(&mut socket, &ApiToNode::PeerAdded { peer }).await?;
                }
                Ok(HubEvent::Change(ankou_server_store::PeerChange::Removed(public_key))) => {
                    send(&mut socket, &ApiToNode::PeerRemoved { public_key }).await?;
                }
                Ok(HubEvent::Resync) | Err(RecvError::Lagged(_)) => full_sync(&state, &mut socket).await?,
                Err(RecvError::Closed) => return Ok(()),
            },
            _ = ping.tick() => {
                if last_pong.elapsed() > PONG_TIMEOUT {
                    anyhow::bail!("pas de pong depuis {PONG_TIMEOUT:?}");
                }
                socket.send(Message::Ping(Vec::new().into())).await?;
            }
        }
    }
}
