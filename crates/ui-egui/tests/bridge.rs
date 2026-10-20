//! Le pont contre un vrai `ankou-daemon` sur plateforme factice. Socket Unix seulement.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_daemon::{DaemonConfig, System, run};
use ankou_ipc::IpcListener;
use ankou_platform::fake::{FakeDns, FakeFirewall, FakeProber, FakeTunnel, FakeWatch};
use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use ankou_ui_egui::bridge::Bridge;
use ankou_ui_egui::model::{Link, Problem, Request, Update};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const WAIT: Duration = Duration::from_secs(10);

fn config() -> TunnelConfig {
    toml::from_str(
        r#"[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]
[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "127.0.0.1:51820"
allowed_ips = ["0.0.0.0/0"]
"#,
    )
    .unwrap()
}

struct Daemon {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

async fn start_daemon(state_dir: &Path, socket: &Path, tunnel: FakeTunnel) -> Daemon {
    let mut cfg = DaemonConfig::new(socket.to_owned(), state_dir.to_owned());
    cfg.tick = Duration::from_millis(50);
    let (stop, stop_rx) = watch::channel(false);
    let system = System {
        tunnel: Box::new(tunnel),
        firewall: Box::new(FakeFirewall::default()),
        dns: Box::new(FakeDns::default()),
        watch: Box::new(FakeWatch::default()),
        resolve: Box::new(|ep: &ankou_proto::Endpoint| {
            Ok(std::net::SocketAddr::new([127, 0, 0, 1].into(), ep.port))
        }),
        prober: Box::new(FakeProber::default()),
    };
    let task = tokio::spawn(async move { run(cfg, system, stop_rx).await.unwrap() });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Daemon { stop, task }
}

impl Daemon {
    async fn stop(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(WAIT, self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

fn bridge(socket: &Path, dir: &Path) -> Bridge {
    Bridge::spawn(
        socket.to_owned(),
        Some(dir.join("session.toml")),
        Arc::new(|| {}),
    )
}

/// Attend un `Update` qui satisfait `pred` ; panique avec ce qui a été reçu sinon.
async fn next_matching(b: &Bridge, within: Duration, pred: impl Fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + within;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        for u in b.try_iter() {
            if pred(&u) {
                return u;
            }
            seen.push(format!("{u:?}"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("délai dépassé ; reçu : {seen:#?}");
}

async fn ipc_connect(socket: &Path) {
    let mut c = ankou_ipc::connect(socket).await.unwrap();
    c.send(&ClientToDaemon::Connect {
        config: Some(config()),
    })
    .await
    .unwrap();
    assert_eq!(
        c.recv::<DaemonToClient>().await.unwrap(),
        Some(DaemonToClient::Done)
    );
}

fn socket_in(dir: &Path) -> PathBuf {
    dir.join("ankoud.sock")
}

#[tokio::test]
async fn bridge_reports_status_and_traffic() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let tunnel = FakeTunnel::default();
    tunnel.set_traffic(1_000, 2_000);
    let d = start_daemon(dir.path(), &socket, tunnel).await;
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Ready))).await;
    next_matching(&b, WAIT, |u| {
        matches!(u, Update::Status(Status::Disconnected))
    })
    .await;
    ipc_connect(&socket).await;
    next_matching(&b, WAIT, |u| {
        matches!(u, Update::Status(Status::Connected { .. }))
    })
    .await;
    next_matching(&b, WAIT, |u| {
        matches!(
            u,
            Update::Traffic {
                rx: 1_000,
                tx: 2_000,
                ..
            }
        )
    })
    .await;
    drop(b);
    d.stop().await;
}

#[tokio::test]
async fn disconnect_request_reaches_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let d = start_daemon(dir.path(), &socket, FakeTunnel::default()).await;
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Ready))).await;
    ipc_connect(&socket).await;
    next_matching(&b, WAIT, |u| {
        matches!(u, Update::Status(Status::Connected { .. }))
    })
    .await;
    b.send(Request::Disconnect);
    // Le changement d'état et la fin de la requête arrivent dans un ordre quelconque.
    let (mut finished, mut disconnected) = (false, false);
    while !(finished && disconnected) {
        match next_matching(&b, WAIT, |u| {
            matches!(u, Update::Finished | Update::Status(Status::Disconnected))
        })
        .await
        {
            Update::Finished => finished = true,
            _ => disconnected = true,
        }
    }
    drop(b);
    d.stop().await;
}

#[tokio::test]
async fn connect_without_profile_fails_with_profile_missing() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let d = start_daemon(dir.path(), &socket, FakeTunnel::default()).await;
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Ready))).await;
    b.send(Request::Connect);
    next_matching(&b, WAIT, |u| {
        matches!(
            u,
            Update::Failed(Problem::Daemon(ErrorCode::ProfileMissing, _))
        )
    })
    .await;
    drop(b);
    d.stop().await;
}

#[tokio::test]
async fn bridge_recovers_when_daemon_returns() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Missing))).await;
    let d = start_daemon(dir.path(), &socket, FakeTunnel::default()).await;
    next_matching(&b, Duration::from_secs(5), |u| {
        matches!(u, Update::Link(Link::Ready))
    })
    .await;
    drop(b);
    d.stop().await;
}

#[tokio::test]
async fn incompatible_daemon_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let listener = IpcListener::bind(&socket).await.unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    let server = tokio::spawn(async move {
        while let Ok(mut conn) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = conn.recv::<ClientToDaemon>().await;
            let _ = conn
                .send(&DaemonToClient::Incompatible {
                    protocol_version: 1,
                })
                .await;
        }
    });
    let started = Instant::now();
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| {
        matches!(u, Update::Link(Link::Incompatible(1)))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3).saturating_sub(started.elapsed())).await;
    assert!(
        accepted.load(Ordering::SeqCst) <= 2,
        "reconnexions trop rapides"
    );
    drop(b);
    server.abort();
}

#[tokio::test]
async fn bridge_reports_loss_then_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_in(dir.path());
    let d = start_daemon(dir.path(), &socket, FakeTunnel::default()).await;
    let b = bridge(&socket, dir.path());
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Ready))).await;
    d.stop().await;
    next_matching(&b, WAIT, |u| matches!(u, Update::Link(Link::Missing))).await;
    let d = start_daemon(dir.path(), &socket, FakeTunnel::default()).await;
    next_matching(&b, Duration::from_secs(5), |u| {
        matches!(u, Update::Link(Link::Ready))
    })
    .await;
    drop(b);
    d.stop().await;
}
