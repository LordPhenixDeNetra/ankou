#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_daemon::{DaemonConfig, run};
use ankou_ipc::{Connection, connect};
use ankou_platform::PlatformError;
use ankou_platform::fake::{FakeTunnel, FakeWatch};
use ankou_proto::ipc::{ClientToDaemon, DaemonSettings, DaemonToClient, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

/// Au-delà du délai de handshake du service (5 s).
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

fn server() -> String {
    config().server_id().0
}

struct Daemon {
    socket: PathBuf,
    tunnel: FakeTunnel,
    watch: FakeWatch,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Daemon {
    async fn start(state_dir: &Path, tunnel: FakeTunnel) -> Daemon {
        let socket = state_dir.join("ankoud.sock");
        let watch = FakeWatch::default();
        let mut cfg = DaemonConfig::new(socket.clone(), state_dir.to_owned());
        cfg.tick = Duration::from_millis(50);
        let (stop, stop_rx) = watch::channel(false);
        let (t, w) = (tunnel.clone(), watch.clone());
        let task = tokio::spawn(async move {
            run(cfg, Box::new(t), Box::new(w), stop_rx).await.unwrap();
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Daemon {
            socket,
            tunnel,
            watch,
            stop,
            task,
        }
    }

    async fn client(&self) -> Connection {
        connect(&self.socket).await.unwrap()
    }

    /// Connexion abonnée : reçoit le statut courant puis chaque changement.
    async fn watcher(&self) -> Connection {
        let mut c = self.client().await;
        c.send(&ClientToDaemon::Subscribe).await.unwrap();
        assert!(matches!(
            c.recv::<DaemonToClient>().await.unwrap(),
            Some(DaemonToClient::Status { .. })
        ));
        c
    }

    async fn stop(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(WAIT, self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

async fn ask(c: &mut Connection, msg: ClientToDaemon) -> DaemonToClient {
    c.send(&msg).await.unwrap();
    c.recv().await.unwrap().unwrap()
}

/// Lit les changements d'état jusqu'à `pred` ; renvoie la suite reçue.
async fn wait_status(c: &mut Connection, pred: impl Fn(&Status) -> bool) -> Vec<Status> {
    let mut seen = Vec::new();
    tokio::time::timeout(WAIT, async {
        loop {
            match c.recv::<DaemonToClient>().await.unwrap() {
                Some(DaemonToClient::StatusChanged { status }) => {
                    let done = pred(&status);
                    seen.push(status);
                    if done {
                        return;
                    }
                }
                other => panic!("message inattendu : {other:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("délai dépassé ; reçu : {seen:?}"));
    seen
}

fn connected(s: &Status) -> bool {
    matches!(s, Status::Connected { .. })
}

fn ups(t: &FakeTunnel) -> usize {
    t.calls().iter().filter(|c| c.starts_with("up")).count()
}

async fn connect_now(d: &Daemon, w: &mut Connection) {
    let mut c = d.client().await;
    assert_eq!(
        ask(
            &mut c,
            ClientToDaemon::Connect {
                config: Some(config())
            }
        )
        .await,
        DaemonToClient::Done
    );
    wait_status(w, connected).await;
}

#[tokio::test]
async fn connect_then_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    let reply = ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config()),
        },
    )
    .await;
    assert_eq!(reply, DaemonToClient::Done);
    let seen = wait_status(&mut w, connected).await;
    assert_eq!(seen[0], Status::Connecting { server: server() });
    assert!(matches!(&seen[1], Status::Connected { server: s, .. } if *s == server()));
    assert!(matches!(
        ask(&mut c, ClientToDaemon::GetStatus).await,
        DaemonToClient::Status {
            status: Status::Connected { .. }
        }
    ));
    assert_eq!(
        ask(&mut c, ClientToDaemon::Disconnect).await,
        DaemonToClient::Done
    );
    wait_status(&mut w, |s| *s == Status::Disconnected).await;
    assert_eq!(d.tunnel.calls(), ["down", "up ankou0", "down"]);
    d.stop().await;
}

#[tokio::test]
async fn start_cleans_leftover_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let _w = d.watcher().await;
    assert_eq!(d.tunnel.calls(), ["down"]);
    d.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn profile_is_saved_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;
    let profile = dir.path().join("profile.toml");
    let mode = std::fs::metadata(&profile).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    assert_eq!(
        ask(&mut c, ClientToDaemon::Connect { config: None }).await,
        DaemonToClient::Done
    );
    wait_status(&mut w, connected).await;
    d.stop().await;
}

#[tokio::test]
async fn connect_without_profile_fails() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut c = d.client().await;
    let reply = ask(&mut c, ClientToDaemon::Connect { config: None }).await;
    assert!(
        matches!(
            reply,
            DaemonToClient::Failed {
                code: ErrorCode::ProfileMissing,
                ..
            }
        ),
        "{reply:?}"
    );
    d.stop().await;
}

#[tokio::test]
async fn auto_connect_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut c = d.client().await;
    let on = DaemonSettings { auto_connect: true };
    assert_eq!(
        ask(
            &mut c,
            ClientToDaemon::SetSettings {
                settings: on.clone()
            }
        )
        .await,
        DaemonToClient::Done
    );
    assert_eq!(
        ask(&mut c, ClientToDaemon::GetSettings).await,
        DaemonToClient::Settings {
            settings: on.clone()
        }
    );
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;

    let tunnel = FakeTunnel::default();
    let d = Daemon::start(dir.path(), tunnel.clone()).await;
    let mut c = d.client().await;
    tokio::time::timeout(WAIT, async {
        loop {
            if let DaemonToClient::Status {
                status: Status::Connected { .. },
            } = ask(&mut c, ClientToDaemon::GetStatus).await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(tunnel.calls(), ["down", "up ankou0"]);
    d.stop().await;
}

#[tokio::test]
async fn link_loss_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.tunnel.age_handshake(Duration::from_secs(200));
    let seen = wait_status(&mut w, connected).await;
    assert!(
        matches!(seen[0], Status::Reconnecting { attempt: 1, .. }),
        "{seen:?}"
    );
    let calls = d.tunnel.calls();
    assert_eq!(calls[calls.len() - 2..], ["down", "up ankou0"]);
    d.stop().await;
}

#[tokio::test]
async fn network_change_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.watch.set_route(Some("wlan0 via 10.1.1.1"));
    let seen = wait_status(&mut w, connected).await;
    assert!(matches!(seen[0], Status::Reconnecting { .. }), "{seen:?}");
    d.stop().await;
}

#[tokio::test]
async fn resume_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.watch.jump_clock(Duration::from_secs(3600));
    let seen = wait_status(&mut w, connected).await;
    assert!(matches!(seen[0], Status::Reconnecting { .. }), "{seen:?}");
    d.stop().await;
}

#[tokio::test]
async fn network_change_while_reconnecting_is_harmless() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.set_handshakes(false);
    let d = Daemon::start(dir.path(), tunnel.clone()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config()),
        },
    )
    .await;
    let seen = wait_status(&mut w, |s| matches!(s, Status::Reconnecting { .. })).await;
    assert!(!seen.iter().any(connected));
    d.watch.set_route(Some("wlan0 via 10.1.1.1"));
    d.watch.jump_clock(Duration::from_secs(3600));
    tunnel.set_handshakes(true);
    let seen = wait_status(&mut w, connected).await;
    let attempts = seen
        .iter()
        .filter(|s| matches!(s, Status::Reconnecting { .. }))
        .count();
    // Une montée initiale + une par tentative de reconnexion (la dernière a réussi).
    assert_eq!(ups(&tunnel), 2 + attempts, "{:?}", tunnel.calls());
    d.stop().await;
}

#[tokio::test]
async fn permission_error_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.fail_up(Some(PlatformError::Permission));
    let d = Daemon::start(dir.path(), tunnel.clone()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config()),
        },
    )
    .await;
    let seen = wait_status(&mut w, |s| matches!(s, Status::Error { .. })).await;
    assert!(matches!(
        seen.last(),
        Some(Status::Error {
            code: ErrorCode::TunnelPermission,
            ..
        })
    ));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(ups(&tunnel), 1, "{:?}", tunnel.calls());
    d.stop().await;
}

#[tokio::test]
async fn unreachable_server_keeps_retrying() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.set_handshakes(false);
    let d = Daemon::start(dir.path(), tunnel).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config()),
        },
    )
    .await;
    // Chaque tentative : 5 s d'attente du handshake, puis le backoff.
    let mut seen = wait_status(&mut w, |s| {
        matches!(s, Status::Reconnecting { attempt: 1, .. })
    })
    .await;
    seen.extend(
        wait_status(&mut w, |s| {
            matches!(s, Status::Reconnecting { attempt: 2, .. })
        })
        .await,
    );
    assert!(!seen.iter().any(|s| matches!(s, Status::Error { .. })));
    d.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn bad_client_does_not_disturb_others() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    let mut garbage = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    garbage.write_u32(7).await.unwrap();
    garbage.write_all(b"garbage").await.unwrap();
    let mut huge = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    huge.write_u32(2 << 20).await.unwrap();
    connect_now(&d, &mut w).await;
    d.stop().await;
}

#[tokio::test]
async fn concurrent_clients_see_same_status() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let (mut w1, mut w2) = (d.watcher().await, d.watcher().await);
    let (mut a, mut b) = (d.client().await, d.client().await);
    let connect = ClientToDaemon::Connect {
        config: Some(config()),
    };
    a.send(&connect).await.unwrap();
    b.send(&ClientToDaemon::Disconnect).await.unwrap();
    a.send(&connect).await.unwrap();
    for c in [&mut a, &mut b] {
        assert_eq!(
            c.recv::<DaemonToClient>().await.unwrap(),
            Some(DaemonToClient::Done)
        );
    }
    assert_eq!(
        a.recv::<DaemonToClient>().await.unwrap(),
        Some(DaemonToClient::Done)
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let drain = async |c: &mut Connection| {
        let mut seen = Vec::new();
        while let Ok(Ok(Some(DaemonToClient::StatusChanged { status }))) =
            tokio::time::timeout(Duration::from_millis(200), c.recv::<DaemonToClient>()).await
        {
            seen.push(status);
        }
        seen
    };
    let (s1, s2) = (drain(&mut w1).await, drain(&mut w2).await);
    assert_eq!(s1, s2);
    assert!(s1.last().is_some_and(connected), "{s1:?}");
    d.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn no_hello_is_dropped() {
    use tokio::io::AsyncReadExt;
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut silent = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(7), silent.read(&mut buf))
        .await
        .expect("connexion fermée par le service")
        .unwrap();
    assert_eq!(n, 0);
    d.stop().await;
}

#[tokio::test]
async fn stop_brings_tunnel_down() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    let (tunnel, socket) = (d.tunnel.clone(), d.socket.clone());
    d.stop().await;
    assert_eq!(tunnel.calls().last().map(String::as_str), Some("down"));
    assert!(!tunnel.is_up());
    assert!(!socket.exists());
}

/// Sans handshake, le tunnel est retiré pendant l'attente : la machine garde Internet.
#[tokio::test]
async fn tunnel_is_down_while_waiting_to_retry() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.set_handshakes(false);
    let d = Daemon::start(dir.path(), tunnel.clone()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config()),
        },
    )
    .await;
    wait_status(&mut w, |s| matches!(s, Status::Reconnecting { .. })).await;
    assert!(!tunnel.is_up(), "{:?}", tunnel.calls());
    d.stop().await;
}

/// Interface disparue (stats en échec) alors que connecté : reconnexion, pas « Connecté » à vie.
#[tokio::test]
async fn vanished_interface_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.tunnel.fail_stats(true);
    wait_status(&mut w, |s| matches!(s, Status::Reconnecting { .. })).await;
    d.tunnel.fail_stats(false);
    wait_status(&mut w, connected).await;
    d.stop().await;
}
