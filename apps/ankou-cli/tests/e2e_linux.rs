//! Bout en bout Linux : vrai tunnel WireGuard entre deux namespaces réseau.
//!
//! Nécessite root, `ip` (iproute2), `wg` (wireguard-tools) et le module `wireguard`.
//! Lancement : `sudo -E env "PATH=$PATH" cargo test -p ankou-cli --test e2e_linux -- --ignored --test-threads=1`
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_proto::PrivateKey;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CLIENT_NS: &str = "ankou-c";
const SERVER_NS: &str = "ankou-s";

fn run(args: &[&str]) -> String {
    let out = Command::new(args[0]).args(&args[1..]).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?} : {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn succeeds(args: &[&str]) -> bool {
    Command::new(args[0])
        .args(&args[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Deux namespaces reliés par une paire veth ; `wg0` côté serveur.
struct Lab {
    dir: tempfile::TempDir,
}

impl Drop for Lab {
    fn drop(&mut self) {
        for ns in [CLIENT_NS, SERVER_NS] {
            let _ = succeeds(&["ip", "netns", "del", ns]);
        }
    }
}

impl Lab {
    /// `good_server_key = false` : le client reçoit une clé serveur fausse (aucun handshake possible).
    /// `with_gateway` : le client a une route par défaut via le serveur, comme un poste derrière une box.
    fn new(good_server_key: bool, with_gateway: bool) -> (Self, PathBuf) {
        for ns in [CLIENT_NS, SERVER_NS] {
            let _ = succeeds(&["ip", "netns", "del", ns]);
            run(&["ip", "netns", "add", ns]);
            run(&["ip", "-n", ns, "link", "set", "lo", "up"]);
        }
        let lab = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        run(&[
            "ip", "link", "add", "veth-c", "netns", CLIENT_NS, "type", "veth", "peer", "name",
            "veth-s", "netns", SERVER_NS,
        ]);
        run(&[
            "ip",
            "-n",
            CLIENT_NS,
            "addr",
            "add",
            "192.168.77.1/24",
            "dev",
            "veth-c",
        ]);
        run(&["ip", "-n", CLIENT_NS, "link", "set", "veth-c", "up"]);
        run(&[
            "ip",
            "-n",
            SERVER_NS,
            "addr",
            "add",
            "192.168.77.2/24",
            "dev",
            "veth-s",
        ]);
        run(&["ip", "-n", SERVER_NS, "link", "set", "veth-s", "up"]);

        let server_key = PrivateKey::generate();
        let client_key = PrivateKey::generate();
        let key_file = lab.dir.path().join("server.key");
        std::fs::write(&key_file, server_key.to_base64()).unwrap();
        run(&[
            "ip",
            "-n",
            SERVER_NS,
            "link",
            "add",
            "wg0",
            "type",
            "wireguard",
        ]);
        run(&[
            "ip",
            "netns",
            "exec",
            SERVER_NS,
            "wg",
            "set",
            "wg0",
            "listen-port",
            "51820",
            "private-key",
            key_file.to_str().unwrap(),
            "peer",
            &client_key.public_key().to_string(),
            "allowed-ips",
            "10.99.0.2/32",
        ]);
        run(&[
            "ip",
            "-n",
            SERVER_NS,
            "addr",
            "add",
            "10.99.0.1/24",
            "dev",
            "wg0",
        ]);
        run(&["ip", "-n", SERVER_NS, "link", "set", "wg0", "up"]);
        if with_gateway {
            run(&[
                "ip",
                "-n",
                CLIENT_NS,
                "route",
                "add",
                "default",
                "via",
                "192.168.77.2",
            ]);
        }
        // Adresse « Internet » du serveur, hors du réseau veth.
        run(&[
            "ip",
            "-n",
            SERVER_NS,
            "addr",
            "add",
            "10.200.0.1/32",
            "dev",
            "lo",
        ]);

        let server_pub = if good_server_key {
            server_key.public_key()
        } else {
            PrivateKey::generate().public_key()
        };
        let config = lab.dir.path().join("client.toml");
        std::fs::write(
            &config,
            format!(
                "[interface]\nprivate_key = \"{}\"\naddress = [\"10.99.0.2/32\"]\n\n[peer]\n\
                 public_key = \"{server_pub}\"\nendpoint = \"192.168.77.2:51820\"\n\
                 allowed_ips = [\"0.0.0.0/0\"]\n",
                client_key.to_base64()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&config, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        (lab, config)
    }
}

/// Client lancé dans son namespace ; ses lignes stdout arrivent sur le canal.
fn spawn_client(config: &PathBuf, retries: u32) -> (Child, mpsc::Receiver<String>) {
    let mut child = Command::new("ip")
        .args(["netns", "exec", CLIENT_NS, env!("CARGO_BIN_EXE_ankou-cli")])
        .args([
            "connect",
            "--lang",
            "en",
            "--retries",
            &retries.to_string(),
            "--config",
        ])
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    (child, rx)
}

fn wait_for_line(rx: &mpsc::Receiver<String>, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(line) if line.contains(needle) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

fn wait_exit(child: &mut Child, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    None
}

/// Plus d'interface, de règle de routage ni de route vers l'endpoint chez le client.
fn assert_clean() {
    assert!(
        !succeeds(&["ip", "-n", CLIENT_NS, "link", "show", "ankou0"]),
        "interface ankou0 restée"
    );
    let rules = run(&["ip", "-n", CLIENT_NS, "rule"]);
    assert!(
        !rules.contains("fwmark") && !rules.contains("suppress_prefixlength"),
        "règles restées :\n{rules}"
    );
    let routes = run(&["ip", "-n", CLIENT_NS, "route"]);
    assert!(
        !routes
            .lines()
            .any(|l| l.starts_with("192.168.77.2 ") || l.contains("blackhole")),
        "routes restées :\n{routes}"
    );
}

fn signal(child: &Child, sig: &str) {
    run(&["kill", sig, &child.id().to_string()]);
}

fn connect_then_signal(sig: &str) {
    let (_lab, config) = Lab::new(true, true);
    let (mut child, rx) = spawn_client(&config, 0);
    assert!(wait_for_line(&rx, "Connected to", Duration::from_secs(10)));
    let route = run(&["ip", "-n", CLIENT_NS, "route", "get", "10.200.0.1"]);
    assert!(route.contains("dev ankou0"), "trafic hors tunnel : {route}");
    signal(&child, sig);
    assert_eq!(wait_exit(&mut child, Duration::from_secs(5)), Some(0));
    assert_clean();
}

/// Sans passerelle : seule la route du tunnel mène à 10.200.0.1.
#[test]
#[ignore]
fn connects_and_routes_all_traffic() {
    let (_lab, config) = Lab::new(true, false);
    let (mut child, rx) = spawn_client(&config, 0);
    assert!(
        wait_for_line(&rx, "Connected to", Duration::from_secs(10)),
        "pas de connexion"
    );
    assert!(
        succeeds(&[
            "ip",
            "netns",
            "exec",
            CLIENT_NS,
            "ping",
            "-c1",
            "-W2",
            "10.200.0.1"
        ]),
        "10.200.0.1 injoignable par le tunnel"
    );
    signal(&child, "-INT");
    assert_eq!(wait_exit(&mut child, Duration::from_secs(5)), Some(0));
    assert_clean();
}

#[test]
#[ignore]
fn sigint_removes_interface() {
    connect_then_signal("-INT");
}

#[test]
#[ignore]
fn sigterm_removes_interface() {
    connect_then_signal("-TERM");
}

#[test]
#[ignore]
fn unreachable_server_exits_and_cleans_up() {
    let (_lab, config) = Lab::new(false, true);
    let (mut child, _rx) = spawn_client(&config, 1);
    assert_eq!(wait_exit(&mut child, Duration::from_secs(20)), Some(1));
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(stderr.contains("is not answering"), "{stderr}");
    assert_clean();
}
