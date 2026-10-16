//! `ankou-cli` face à un vrai `ankou-daemon` (tunnel factice) servi dans ce processus.
//! Unix seulement : socket dans un dossier temporaire (named pipe Windows à l'étape 5).
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_daemon::{DaemonConfig, System, run};
use ankou_platform::fake::{FakeDns, FakeFirewall, FakeTunnel, FakeWatch};
use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONFIG: &str = r#"[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]
[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "127.0.0.1:51820"
allowed_ips = ["0.0.0.0/0"]
"#;

struct Daemon {
    socket: PathBuf,
    stop: tokio::sync::watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    fn start(dir: &Path, tunnel: FakeTunnel) -> Daemon {
        let socket = dir.join("ankoud.sock");
        let config = DaemonConfig::new(socket.clone(), dir.join("state"));
        let (stop, stop_rx) = tokio::sync::watch::channel(false);
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let system = System {
                tunnel: Box::new(tunnel),
                firewall: Box::new(FakeFirewall::default()),
                dns: Box::new(FakeDns::default()),
                watch: Box::new(FakeWatch::default()),
                resolve: Box::new(ankou_platform::resolve_endpoint),
            };
            rt.block_on(run(config, system, stop_rx)).unwrap();
        });
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Daemon {
            socket,
            stop,
            thread: Some(thread),
        }
    }

    fn cli(&self) -> Command {
        let mut c = Command::cargo_bin("ankou-cli").unwrap();
        c.env("ANKOU_SOCKET", &self.socket).args(["--lang", "en"]);
        c
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn out(a: &assert_cmd::assert::Assert) -> (String, String) {
    let o = a.get_output();
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn config_file(dir: &Path) -> PathBuf {
    let path = dir.join("client.toml");
    std::fs::write(&path, CONFIG).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

#[test]
fn cli_connect_status_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default());
    let a = d
        .cli()
        .args(["connect", "--config"])
        .arg(config_file(dir.path()))
        .assert()
        .code(0);
    assert!(
        out(&a).0.contains("Connected to 127.0.0.1:51820"),
        "{:?}",
        out(&a)
    );
    let a = d.cli().arg("status").assert().code(0);
    assert!(
        out(&a).0.contains("Connected to 127.0.0.1:51820"),
        "{:?}",
        out(&a)
    );
    let a = d.cli().arg("disconnect").assert().code(0);
    assert!(out(&a).0.contains("Disconnected."), "{:?}", out(&a));
    d.cli().args(["autoconnect", "on"]).assert().code(0);
    let settings = std::fs::read_to_string(dir.path().join("state/settings.toml")).unwrap();
    assert!(settings.contains("auto_connect = true"), "{settings}");
    // Sans --config : le profil enregistré par le service.
    let a = d.cli().arg("connect").assert().code(0);
    assert!(out(&a).0.contains("Connected to"), "{:?}", out(&a));
}

#[test]
fn cli_connect_gives_up_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let tunnel = FakeTunnel::default();
    tunnel.set_handshakes(false);
    let d = Daemon::start(dir.path(), tunnel);
    let a = d
        .cli()
        .args(["connect", "--wait", "2", "--config"])
        .arg(config_file(dir.path()))
        .assert()
        .code(1);
    assert!(out(&a).1.contains("Still trying"), "{:?}", out(&a));
}

#[test]
fn cli_killswitch_toggle_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(dir.path(), FakeTunnel::default());
    let a = d.cli().arg("status").assert().code(0);
    assert!(out(&a).0.contains("Kill switch: on"), "{:?}", out(&a));
    d.cli().args(["killswitch", "off"]).assert().code(0);
    let settings = std::fs::read_to_string(dir.path().join("state/settings.toml")).unwrap();
    assert!(settings.contains("kill_switch = false"), "{settings}");
    let a = d.cli().arg("status").assert().code(0);
    assert!(out(&a).0.contains("Kill switch: off"), "{:?}", out(&a));
    d.cli().args(["killswitch", "on"]).assert().code(0);
    let a = d.cli().arg("status").assert().code(0);
    assert!(out(&a).0.contains("Kill switch: on"), "{:?}", out(&a));
}
