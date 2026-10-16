#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_api::{ApiConfig, AppState, PeerHub, auth, listen_peer_changes, router};
use ankou_node_core::{FakeBackend, NodeConfig, PeerOp, SharedBackend, run};
use ankou_proto::node::NodePeer;
use ankou_proto::{PrivateKey, PublicKey};
use ankou_server_domain::hash_secret;
use ankou_server_store::{NewServer, Store};
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use tokio::sync::watch;

const TOKEN: &str = "node-token";

/// API dans son propre runtime : l'arrêter coupe vraiment toutes ses connexions.
struct Api {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Api {
    fn start(opts: PgConnectOptions, addr: SocketAddr) -> Api {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let store = Store::from_pool(PgPool::connect_with(opts).await.unwrap());
                let hub = PeerHub::new();
                tokio::spawn(listen_peer_changes(store.clone(), hub.clone()));
                let state = AppState {
                    store,
                    key: auth::random_key(),
                    config: Arc::new(ApiConfig::for_tests()),
                    argon2: AppState::argon2_permits(),
                    hub,
                };
                let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                let serve = axum::serve(
                    listener,
                    router(state).into_make_service_with_connect_info::<SocketAddr>(),
                );
                tokio::select! {
                    _ = serve.into_future() => {}
                    _ = stop_rx => {}
                }
            });
            rt.shutdown_background();
        });
        let addr = addr_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        std::thread::sleep(Duration::from_millis(200)); // écoute PostgreSQL prête
        Api {
            addr,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn stop(mut self) -> SocketAddr {
        let _ = self.stop.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
        self.addr
    }
}

async fn add_server(store: &Store) {
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
}

fn config(api: SocketAddr, cache: std::path::PathBuf) -> NodeConfig {
    NodeConfig {
        api_url: format!("ws://{api}/v1/nodes/ws"),
        token_file: Default::default(),
        interface: "wg0".into(),
        cache_file: cache,
        keep_peers: vec![],
        stats_interval_secs: 60,
        api_timeout_secs: 90,
    }
}

struct Node {
    backend: Arc<Mutex<FakeBackend>>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

fn start_node(config: NodeConfig, backend: FakeBackend) -> Node {
    let backend = Arc::new(Mutex::new(backend));
    let shared: SharedBackend = backend.clone();
    let (stop, stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        run(config, TOKEN.into(), shared, stop_rx).await.unwrap();
    });
    Node {
        backend,
        stop,
        task,
    }
}

impl Node {
    fn has(&self, key: &PublicKey) -> bool {
        self.backend.lock().unwrap().peers.contains_key(key)
    }

    async fn wait(&self, what: &str, cond: impl Fn(&Node) -> bool) {
        let start = Instant::now();
        while !cond(self) {
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "délai dépassé : {what}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn shutdown(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

async fn user(store: &Store) -> uuid::Uuid {
    store
        .register_user(None, "a@x.org", "h", OffsetDateTime::now_utc())
        .await
        .unwrap()
        .id
}

async fn device(store: &Store, user: uuid::Uuid) -> (PublicKey, uuid::Uuid) {
    let key = PrivateKey::generate().public_key();
    let d = store
        .enroll_device(user, "d", &key, "10.8.0.0/16".parse().unwrap())
        .await
        .unwrap();
    (key, d.id)
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn node_applies_full_sync_and_live_changes(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    add_server(&store).await;
    let uid = user(&store).await;
    let (existing, _) = device(&store, uid).await;
    let api = Api::start(
        pool.connect_options().as_ref().clone(),
        "127.0.0.1:0".parse().unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let cache_file = dir.path().join("peers.json");
    let node = start_node(config(api.addr, cache_file.clone()), FakeBackend::default());
    node.wait("FullSync", |n| n.has(&existing)).await;
    let (added, id) = device(&store, uid).await;
    node.wait("PeerAdded", |n| n.has(&added)).await;
    store.revoke_device(None, id).await.unwrap();
    node.wait("PeerRemoved", |n| !n.has(&added)).await;
    let cached: Vec<NodePeer> =
        serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].public_key, existing);
    node.shutdown().await;
    api.stop();
}

#[tokio::test]
async fn node_starts_from_cache_without_api() {
    let dir = tempfile::tempdir().unwrap();
    let cache_file = dir.path().join("peers.json");
    let peers: Vec<NodePeer> = (2..4)
        .map(|i| NodePeer {
            public_key: PrivateKey::generate().public_key(),
            ipv4: Ipv4Addr::new(10, 8, 0, i),
        })
        .collect();
    std::fs::write(&cache_file, serde_json::to_vec(&peers).unwrap()).unwrap();
    // Port fermé : l'API est injoignable.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let node = start_node(config(closed, cache_file), FakeBackend::default());
    node.wait("cache appliqué", |n| {
        peers.iter().all(|p| n.has(&p.public_key))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !node.task.is_finished(),
        "la boucle doit continuer d'essayer"
    );
    node.shutdown().await;
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn node_survives_api_restart(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    add_server(&store).await;
    let uid = user(&store).await;
    let (kept, _) = device(&store, uid).await;
    let opts = pool.connect_options().as_ref().clone();
    let api = Api::start(opts.clone(), "127.0.0.1:0".parse().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let node = start_node(
        config(api.addr, dir.path().join("peers.json")),
        FakeBackend::default(),
    );
    node.wait("FullSync", |n| n.has(&kept)).await;
    let addr = tokio::task::spawn_blocking(move || api.stop())
        .await
        .unwrap();
    assert!(node.has(&kept), "les pairs restent pendant la panne");
    let (late, _) = device(&store, uid).await;
    let api = tokio::task::spawn_blocking(move || Api::start(opts, addr))
        .await
        .unwrap();
    // Reconnexion avec backoff (1 s ± 20 % au premier essai).
    let start = Instant::now();
    while !node.has(&late) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "pas de resynchronisation"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let removed_kept = node
        .backend
        .lock()
        .unwrap()
        .applied
        .iter()
        .any(|op| matches!(op, PeerOp::Remove(k) if *k == kept));
    assert!(!removed_kept, "un pair inchangé a été retiré");
    node.shutdown().await;
    tokio::task::spawn_blocking(move || api.stop())
        .await
        .unwrap();
}

/// `wss://` vers un service qui ne parle pas TLS : échec propre et nouvelle tentative, pas de panique.
#[tokio::test]
async fn wss_failure_is_clean() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            drop(sock);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(addr, dir.path().join("peers.json"));
    cfg.api_url = format!("wss://{addr}/v1/nodes/ws");
    let node = start_node(cfg, FakeBackend::default());
    tokio::time::sleep(Duration::from_millis(300)).await;
    node.shutdown().await;
}

/// API muette (connexion à moitié ouverte) : le node le détecte et se reconnecte.
#[tokio::test]
async fn silent_api_is_detected() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let count = accepted.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            held.push(tokio_tungstenite::accept_async(sock).await.unwrap());
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(addr, dir.path().join("peers.json"));
    cfg.api_timeout_secs = 1;
    let node = start_node(cfg, FakeBackend::default());
    let start = Instant::now();
    while accepted.load(std::sync::atomic::Ordering::SeqCst) < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "pas de reconnexion"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    node.shutdown().await;
}
