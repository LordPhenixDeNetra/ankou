#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_api::{ApiConfig, AppState, PeerHub, auth, listen_peer_changes, router};
use ankou_proto::node::{ApiToNode, NodeToApi, PeerStat};
use ankou_proto::{PrivateKey, PublicKey};
use ankou_server_domain::hash_secret;
use ankou_server_store::{NewServer, Store};
use futures_util::{SinkExt, StreamExt};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const TOKEN: &str = "node-token";

/// API servie sur un port libre, avec l'écoute des notifications PostgreSQL.
async fn serve(pool: PgPool) -> (SocketAddr, Store) {
    let store = Store::from_pool(pool);
    store
        .add_server(NewServer {
            name: "vps1".into(),
            country: "FR".into(),
            city: "Paris".into(),
            endpoint: "vps1.example.org:51820".parse().unwrap(),
            public_key: PrivateKey::generate().public_key(),
            tags: vec![],
            node_token_hash: hash_secret(TOKEN),
        })
        .await
        .unwrap();
    let hub = PeerHub::new();
    let state = AppState {
        store: store.clone(),
        key: auth::random_key(),
        config: Arc::new(ApiConfig::for_tests()),
        argon2: AppState::argon2_permits(),
        hub: hub.clone(),
    };
    tokio::spawn(listen_peer_changes(store.clone(), hub));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(200)).await; // écoute PostgreSQL prête
    (addr, store)
}

async fn connect(
    addr: SocketAddr,
    token: &str,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/v1/nodes/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn send(ws: &mut Ws, msg: NodeToApi) {
    ws.send(Message::text(serde_json::to_string(&msg).unwrap()))
        .await
        .unwrap();
}

/// Prochain message applicatif (les ping/pong sont ignorés), en moins d'une seconde.
async fn recv(ws: &mut Ws) -> ApiToNode {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match ws.next().await.expect("connexion fermée").unwrap() {
                Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                Message::Close(_) => panic!("fermé par l'API"),
                _ => {}
            }
        }
    })
    .await
    .expect("rien reçu en 1 s")
}

async fn hello(addr: SocketAddr) -> Ws {
    let mut ws = connect(addr, TOKEN).await.unwrap();
    send(
        &mut ws,
        NodeToApi::Hello {
            protocol_version: 1,
        },
    )
    .await;
    assert_eq!(recv(&mut ws).await, ApiToNode::Welcome);
    ws
}

async fn user_with_device(store: &Store, email: &str) -> (uuid::Uuid, PublicKey, uuid::Uuid) {
    let user = store
        .register_user(None, email, "h", OffsetDateTime::now_utc())
        .await
        .unwrap();
    let key = PrivateKey::generate().public_key();
    let d = store
        .enroll_device(user.id, "A", &key, "10.8.0.0/16".parse().unwrap())
        .await
        .unwrap();
    (user.id, key, d.id)
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn node_with_bad_token_is_rejected(pool: PgPool) {
    let (addr, _store) = serve(pool.clone()).await;
    let err = connect(addr, "wrong").await.unwrap_err();
    assert!(err.to_string().contains("401"), "{err}");
    sqlx::query("UPDATE servers SET active = false")
        .execute(&pool)
        .await
        .unwrap();
    let err = connect(addr, TOKEN).await.unwrap_err();
    assert!(err.to_string().contains("401"), "{err}");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn node_gets_full_sync_then_changes(pool: PgPool) {
    let (addr, store) = serve(pool).await;
    let (user, existing, _) = user_with_device(&store, "a@x.org").await;
    let mut ws = hello(addr).await;
    let ApiToNode::FullSync { peers } = recv(&mut ws).await else {
        panic!("FullSync attendu")
    };
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].public_key, existing);
    let key = PrivateKey::generate().public_key();
    let d = store
        .enroll_device(user, "B", &key, "10.8.0.0/16".parse().unwrap())
        .await
        .unwrap();
    let ApiToNode::PeerAdded { peer } = recv(&mut ws).await else {
        panic!("PeerAdded attendu")
    };
    assert_eq!(peer.public_key, key);
    assert_eq!(peer.ipv4.to_string(), "10.8.0.3");
    store.revoke_device(Some(user), d.id).await.unwrap();
    assert_eq!(
        recv(&mut ws).await,
        ApiToNode::PeerRemoved { public_key: key }
    );
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn admin_revocation_reaches_connected_nodes(pool: PgPool) {
    let (addr, store) = serve(pool).await;
    let (_, key, device) = user_with_device(&store, "a@x.org").await;
    let mut a = hello(addr).await;
    let mut b = hello(addr).await;
    recv(&mut a).await;
    recv(&mut b).await;
    store.revoke_device(None, device).await.unwrap();
    let expected = ApiToNode::PeerRemoved { public_key: key };
    assert_eq!(recv(&mut a).await, expected);
    assert_eq!(recv(&mut b).await, expected);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn incompatible_node_version_is_told(pool: PgPool) {
    let (addr, _store) = serve(pool).await;
    let mut ws = connect(addr, TOKEN).await.unwrap();
    send(
        &mut ws,
        NodeToApi::Hello {
            protocol_version: 99,
        },
    )
    .await;
    assert_eq!(
        recv(&mut ws).await,
        ApiToNode::Incompatible { api_version: 1 }
    );
    let end = tokio::time::timeout(Duration::from_secs(1), ws.next())
        .await
        .unwrap();
    assert!(
        matches!(end, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "{end:?}"
    );
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn stats_update_last_handshake(pool: PgPool) {
    let (addr, store) = serve(pool.clone()).await;
    let (_, key, _) = user_with_device(&store, "a@x.org").await;
    let mut ws = hello(addr).await;
    recv(&mut ws).await;
    let at = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
    send(
        &mut ws,
        NodeToApi::Stats {
            peers: vec![PeerStat {
                public_key: key,
                last_handshake: Some(at),
                rx_bytes: 10,
                tx_bytes: 20,
            }],
        },
    )
    .await;
    let mut got = None;
    for _ in 0..20 {
        got = sqlx::query_scalar::<_, Option<OffsetDateTime>>("SELECT last_handshake FROM devices")
            .fetch_one(&pool)
            .await
            .unwrap();
        if got.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(got, Some(at));
}
