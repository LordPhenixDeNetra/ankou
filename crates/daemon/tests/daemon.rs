//! Socket local dans un dossier temporaire (Unix) ou named pipe unique (Windows).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_daemon::{DaemonConfig, System, run};
use ankou_ipc::{Connection, connect};
use ankou_platform::fake::{FakeDns, FakeFirewall, FakeProber, FakeTunnel, FakeWatch};
use ankou_platform::{FirewallPolicy, INTERFACE, PlatformError};
use ankou_proto::ipc::{ClientToDaemon, DaemonSettings, DaemonToClient, ServerEntry, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;
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
    firewall: FakeFirewall,
    dns: FakeDns,
    prober: FakeProber,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Daemon {
    async fn start(state_dir: &Path, tunnel: FakeTunnel) -> Daemon {
        Self::start_with(state_dir, tunnel, FakeFirewall::default()).await
    }

    async fn start_with(state_dir: &Path, tunnel: FakeTunnel, firewall: FakeFirewall) -> Daemon {
        let socket = ankou_ipc::testing::test_socket(state_dir);
        let watch = FakeWatch::default();
        let dns = FakeDns::default();
        let prober = FakeProber::default();
        let mut cfg = DaemonConfig::new(socket.clone(), state_dir.to_owned());
        cfg.tick = Duration::from_millis(50);
        let (stop, stop_rx) = watch::channel(false);
        // Comme en vrai : sous kill switch, tunnel retiré, le DNS ne répond plus.
        let (fw, tn) = (firewall.clone(), tunnel.clone());
        let resolve = move |ep: &ankou_proto::Endpoint| {
            if ep.host.starts_with("fail") {
                return Err(PlatformError::Endpoint(format!("{ep} : nom inconnu")));
            }
            if fw.active().is_some_and(|p| p.kill_switch) && !tn.is_up() {
                return Err(PlatformError::Endpoint(format!("{ep} : DNS bloqué")));
            }
            let ip = ep.host.parse().unwrap_or([127, 0, 0, 1].into());
            Ok(std::net::SocketAddr::new(ip, ep.port))
        };
        let system = System {
            tunnel: Box::new(tunnel.clone()),
            firewall: Box::new(firewall.clone()),
            dns: Box::new(dns.clone()),
            watch: Box::new(watch.clone()),
            resolve: Box::new(resolve),
            prober: Box::new(prober.clone()),
        };
        let task = tokio::spawn(async move {
            run(cfg, system, stop_rx).await.unwrap();
        });
        for _ in 0..100 {
            if connect(&socket).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Daemon {
            socket,
            tunnel,
            watch,
            firewall,
            dns,
            prober,
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
    assert_eq!(
        d.tunnel.calls(),
        ["down", &format!("up {INTERFACE}"), "down"]
    );
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
    let on = DaemonSettings {
        auto_connect: true,
        ..DaemonSettings::default()
    };
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
    assert_eq!(tunnel.calls(), ["down", &format!("up {INTERFACE}")]);
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
    assert_eq!(
        calls[calls.len() - 2..],
        ["down", &format!("up {INTERFACE}")]
    );
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
    // Sans kill switch : Error ; avec, le cœur garde Blocked (voir firewall_failure_is_fatal_with_kill_switch).
    set_kill_switch(&d, false).await;
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
    use tokio::io::AsyncWriteExt;
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
    #[cfg(unix)]
    assert!(!socket.exists());
    let _ = socket;
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

fn policy(kill_switch: bool) -> FirewallPolicy {
    FirewallPolicy {
        kill_switch,
        interface: INTERFACE.into(),
        endpoints: vec!["127.0.0.1:51820".parse().unwrap()],
    }
}

async fn set_kill_switch(d: &Daemon, on: bool) {
    let mut c = d.client().await;
    let settings = DaemonSettings {
        kill_switch: on,
        ..DaemonSettings::default()
    };
    assert_eq!(
        ask(&mut c, ClientToDaemon::SetSettings { settings }).await,
        DaemonToClient::Done
    );
}

async fn connect_with(d: &Daemon, config: TunnelConfig) {
    let mut c = d.client().await;
    ask(
        &mut c,
        ClientToDaemon::Connect {
            config: Some(config),
        },
    )
    .await;
}

#[tokio::test]
async fn firewall_follows_state() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_with(&d, config()).await;
    let seen = wait_status(&mut w, connected).await;
    assert!(matches!(seen[0], Status::Connecting { .. }));
    assert_eq!(d.firewall.active(), Some(policy(true)));
    let mut c = d.client().await;
    ask(&mut c, ClientToDaemon::Disconnect).await;
    wait_status(&mut w, |s| *s == Status::Disconnected).await;
    assert_eq!(d.firewall.active(), None);
    d.stop().await;
}

#[tokio::test]
async fn kill_switch_off_still_blocks_ipv6() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    set_kill_switch(&d, false).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    assert_eq!(d.firewall.active(), Some(policy(false)));
    d.stop().await;
}

#[tokio::test]
async fn firewall_kept_while_reconnecting() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.tunnel.age_handshake(Duration::from_secs(200));
    wait_status(&mut w, |s| matches!(s, Status::Reconnecting { .. })).await;
    assert_eq!(d.firewall.active(), Some(policy(true)));
    d.stop().await;
}

#[tokio::test]
async fn firewall_failure_is_fatal_with_kill_switch() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    firewall.fail(true);
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall).await;
    let mut w = d.watcher().await;
    connect_with(&d, config()).await;
    // Kill switch : le cœur garde l'état bloqué (invariant), avec la cause.
    let seen = wait_status(&mut w, |s| matches!(s, Status::Blocked { .. })).await;
    assert_eq!(
        seen.last(),
        Some(&Status::Blocked {
            reason: ErrorCode::FirewallUnavailable
        })
    );
    assert_eq!(ups(&d.tunnel), 0, "{:?}", d.tunnel.calls());
    d.stop().await;
}

#[tokio::test]
async fn firewall_failure_tolerated_without_kill_switch() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    firewall.fail(true);
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall).await;
    set_kill_switch(&d, false).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;
}

#[tokio::test]
async fn kill_switch_toggle_applies_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    set_kill_switch(&d, false).await;
    assert_eq!(d.firewall.active(), Some(policy(false)));
    set_kill_switch(&d, true).await;
    assert_eq!(d.firewall.active(), Some(policy(true)));
    let mut c = d.client().await;
    assert!(matches!(
        ask(&mut c, ClientToDaemon::GetStatus).await,
        DaemonToClient::Status {
            status: Status::Connected { .. }
        }
    ));
    d.stop().await;
}

#[tokio::test]
async fn dns_set_when_connected() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    assert_eq!(
        d.dns.current(),
        Some((INTERFACE.into(), ankou_proto::DEFAULT_DNS.to_vec()))
    );
    let mut custom = config();
    custom.interface.dns = vec!["1.1.1.1".parse().unwrap()];
    connect_with(&d, custom).await;
    wait_status(&mut w, connected).await;
    assert_eq!(
        d.dns.current(),
        Some((INTERFACE.into(), vec!["1.1.1.1".parse().unwrap()]))
    );
    let mut c = d.client().await;
    ask(&mut c, ClientToDaemon::Disconnect).await;
    wait_status(&mut w, |s| *s == Status::Disconnected).await;
    assert_eq!(d.dns.current(), None);
    d.stop().await;
}

#[tokio::test]
async fn dns_failure_is_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    d.dns.fail(true);
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;
}

#[tokio::test]
async fn leftover_firewall_resumes_connection() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;
    // Arrêt brutal simulé : les règles sont restées.
    let firewall = FakeFirewall::default();
    firewall.set_leftover(policy(true));
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall).await;
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
    ask(&mut c, ClientToDaemon::Disconnect).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(d.firewall.active(), None);
    d.stop().await;
}

#[tokio::test]
async fn leftover_firewall_without_profile_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    firewall.set_leftover(policy(true));
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall).await;
    let _w = d.watcher().await;
    assert_eq!(d.firewall.active(), None);
    d.stop().await;
}

fn with_endpoint(host: &str) -> TunnelConfig {
    let mut c = config();
    c.peer.endpoint = format!("{host}:51820").parse().unwrap();
    c
}

/// Changer de serveur (nom jamais résolu) sous kill switch : résolu avant de couper l'ancien tunnel.
#[tokio::test]
async fn switching_server_by_name_under_kill_switch() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_with(&d, with_endpoint("a.test")).await;
    wait_status(&mut w, connected).await;
    connect_with(&d, with_endpoint("b.test")).await;
    let seen = wait_status(
        &mut w,
        |s| matches!(s, Status::Connected { server, .. } if server == "b.test:51820"),
    )
    .await;
    assert!(
        !seen
            .iter()
            .any(|s| matches!(s, Status::Reconnecting { .. })),
        "{seen:?}"
    );
    d.stop().await;
}

/// Arrêt du service (mise à jour, redémarrage) : la protection reste, la connexion reprend.
#[tokio::test]
async fn stop_keeps_protection_and_restart_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall.clone()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.stop().await;
    assert_eq!(
        firewall.active(),
        Some(policy(true)),
        "kill switch retiré par l'arrêt"
    );
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall.clone()).await;
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
    d.stop().await;
}

/// Arrêt sans connexion en cours : rien ne reste.
#[tokio::test]
async fn stop_while_disconnected_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall.clone()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    let mut c = d.client().await;
    ask(&mut c, ClientToDaemon::Disconnect).await;
    wait_status(&mut w, |s| *s == Status::Disconnected).await;
    d.stop().await;
    assert_eq!(firewall.active(), None);
}

fn entry(name: &str, host: &str) -> ServerEntry {
    ServerEntry {
        name: name.into(),
        public_key: config().peer.public_key,
        endpoint: format!("{host}:51820").parse().unwrap(),
    }
}

async fn fastest(
    d: &Daemon,
    config: Option<TunnelConfig>,
    servers: Vec<ServerEntry>,
) -> DaemonToClient {
    let mut c = d.client().await;
    ask(&mut c, ClientToDaemon::ConnectFastest { config, servers }).await
}

fn rtt(ms: u64) -> Vec<Duration> {
    vec![Duration::from_millis(ms); 3]
}

#[tokio::test]
async fn fastest_server_is_chosen_and_saved() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    d.prober.set("127.0.0.2:51820", rtt(30));
    d.prober.set("127.0.0.3:51820", rtt(10));
    d.prober.set("127.0.0.4:51820", rtt(20));
    let mut servers = vec![
        entry("a", "127.0.0.2"),
        entry("b", "127.0.0.3"),
        entry("c", "127.0.0.4"),
    ];
    servers.extend((10..24).map(|i| entry(&format!("x{i}"), &format!("127.0.1.{i}"))));
    assert_eq!(servers.len(), 17);
    let mut w = d.watcher().await;
    let reply = fastest(&d, Some(config()), servers).await;
    assert_eq!(
        reply,
        DaemonToClient::Chosen {
            server: "b".into(),
            rtt_ms: 10
        }
    );
    let seen = wait_status(&mut w, connected).await;
    assert!(
        matches!(seen.last(), Some(Status::Connected { server, .. }) if server == "127.0.0.3:51820")
    );
    assert_eq!(d.prober.probed().len(), 16);
    d.stop().await;
    // Le profil enregistré est le serveur choisi.
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    ask(&mut c, ClientToDaemon::Connect { config: None }).await;
    let seen = wait_status(&mut w, connected).await;
    assert!(
        matches!(seen.last(), Some(Status::Connected { server, .. }) if server == "127.0.0.3:51820")
    );
    d.stop().await;
}

#[tokio::test]
async fn probe_opens_only_candidate_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    d.prober.set("127.0.0.2:51820", rtt(10));
    d.prober.set("127.0.0.3:51820", rtt(20));
    let reply = fastest(
        &d,
        None,
        vec![entry("a", "127.0.0.2"), entry("b", "127.0.0.3")],
    )
    .await;
    assert!(matches!(reply, DaemonToClient::Chosen { .. }), "{reply:?}");
    let all: Vec<std::net::SocketAddr> = ["127.0.0.1:51820", "127.0.0.2:51820", "127.0.0.3:51820"]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
    assert!(
        d.firewall.history().iter().any(|p| {
            let mut e = p.endpoints.clone();
            e.sort();
            e == all && p.kill_switch
        }),
        "{:?}",
        d.firewall.history()
    );
    wait_status(&mut w, connected).await;
    assert_eq!(
        d.firewall.active().unwrap().endpoints,
        vec!["127.0.0.2:51820".parse::<std::net::SocketAddr>().unwrap()]
    );
    d.stop().await;
}

#[tokio::test]
async fn no_answer_keeps_current_connection() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    let reply = fastest(&d, None, vec![entry("a", "127.0.0.2")]).await;
    assert!(
        matches!(
            reply,
            DaemonToClient::Failed {
                code: ErrorCode::ServerUnreachable,
                ..
            }
        ),
        "{reply:?}"
    );
    let mut c = d.client().await;
    assert!(matches!(
        ask(&mut c, ClientToDaemon::GetStatus).await,
        DaemonToClient::Status { status: Status::Connected { server, .. } } if server == "127.0.0.1:51820"
    ));
    assert_eq!(d.firewall.active(), Some(policy(true)));
    d.stop().await;
}

#[tokio::test]
async fn unresolvable_candidate_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    d.prober.set("127.0.0.3:51820", rtt(10));
    let reply = fastest(
        &d,
        Some(config()),
        vec![entry("a", "fail.test"), entry("b", "127.0.0.3")],
    )
    .await;
    assert_eq!(
        reply,
        DaemonToClient::Chosen {
            server: "b".into(),
            rtt_ms: 10
        }
    );
    assert_eq!(d.prober.probed(), ["127.0.0.3:51820"]);
    d.stop().await;
}

#[tokio::test]
async fn probe_interfaces_cleaned_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let _w = d.watcher().await;
    assert_eq!(d.prober.cleanups(), 1);
    d.stop().await;
}

#[tokio::test]
async fn fastest_without_any_profile_fails() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let reply = fastest(&d, None, vec![entry("a", "127.0.0.2")]).await;
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
async fn stats_reported_while_connected() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.set_traffic(1_000, 2_000);
    let d = Daemon::start(dir.path(), tunnel).await;
    let mut w = d.watcher().await;
    let mut c = d.client().await;
    let zero = DaemonToClient::Stats {
        rx_bytes: 0,
        tx_bytes: 0,
    };
    assert_eq!(ask(&mut c, ClientToDaemon::GetStats).await, zero);
    connect_now(&d, &mut w).await;
    assert_eq!(
        ask(&mut c, ClientToDaemon::GetStats).await,
        DaemonToClient::Stats {
            rx_bytes: 1_000,
            tx_bytes: 2_000
        }
    );
    assert_eq!(
        ask(&mut c, ClientToDaemon::Disconnect).await,
        DaemonToClient::Done
    );
    wait_status(&mut w, |s| *s == Status::Disconnected).await;
    assert_eq!(ask(&mut c, ClientToDaemon::GetStats).await, zero);
    d.stop().await;
}

#[tokio::test]
async fn unreadable_stats_are_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    let d = Daemon::start(dir.path(), tunnel.clone()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    tunnel.fail_stats(true);
    let mut c = d.client().await;
    assert!(matches!(
        ask(&mut c, ClientToDaemon::GetStats).await,
        DaemonToClient::Failed {
            code: ErrorCode::Internal,
            ..
        }
    ));
    d.stop().await;
}

/// Sous Windows, la règle « tunnel autorisé » vise la carte, qui n'existe qu'après la montée :
/// la politique est réappliquée juste après `up`.
#[tokio::test]
async fn firewall_reapplied_after_tunnel_up() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default()).await;
    let mut w = d.watcher().await;
    connect_now(&d, &mut w).await;
    assert_eq!(d.firewall.history(), [policy(true), policy(true)]);
    assert_eq!(d.firewall.active(), Some(policy(true)));
    d.stop().await;
}

/// Protection apparue sans que le service le sache (pare-feu Windows démarré après lui) :
/// « Se déconnecter » la retire quand même, sinon la machine resterait coupée.
#[tokio::test]
async fn disconnect_clears_unknown_protection() {
    let dir = tempfile::tempdir().unwrap();
    let firewall = FakeFirewall::default();
    let d = Daemon::start_with(dir.path(), FakeTunnel::default(), firewall.clone()).await;
    let _w = d.watcher().await;
    firewall.set_leftover(policy(true));
    let mut c = d.client().await;
    assert_eq!(
        ask(&mut c, ClientToDaemon::Disconnect).await,
        DaemonToClient::Done
    );
    assert_eq!(firewall.active(), None);
    d.stop().await;
}
