//! Bout en bout Linux : `ankoud` dans un namespace réseau client, vrai tunnel WireGuard vers
//! un namespace serveur ; `ankou-cli` hors namespace, par le socket du service.
//!
//! Nécessite root, `ip` (iproute2), `wg` (wireguard-tools) et le module `wireguard`.
//! Lancement : `cargo build -p ankou-cli -p ankoud` puis
//! `sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1`
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_proto::PrivateKey;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
        // IPv6 sur le réseau local, pour les tests de fuite (nodad : utilisable tout de suite).
        for (ns, dev, addr) in [
            (CLIENT_NS, "veth-c", "fd00:77::1/64"),
            (SERVER_NS, "veth-s", "fd00:77::2/64"),
        ] {
            run(&[
                "ip", "-n", ns, "-6", "addr", "add", addr, "dev", dev, "nodad",
            ]);
        }

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

/// `ankoud` lancé dans le namespace client ; état et socket dans le dossier du `Lab`.
fn spawn_daemon(dir: &Path) -> Child {
    let child = Command::new("ip")
        .args(["netns", "exec", CLIENT_NS, env!("CARGO_BIN_EXE_ankoud")])
        .arg("--socket")
        .arg(dir.join("ankoud.sock"))
        .arg("--state-dir")
        .arg(dir.join("state"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.join("ankoud.sock").exists() {
        assert!(Instant::now() < deadline, "socket d'ankoud absent");
        std::thread::sleep(Duration::from_millis(50));
    }
    child
}

/// `ankou-cli` compilé à côté d'`ankoud` (`cargo build -p ankou-cli`).
fn cli(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let bin = Path::new(env!("CARGO_BIN_EXE_ankoud")).with_file_name("ankou-cli");
    assert!(
        bin.exists(),
        "cargo build -p ankou-cli d'abord ({})",
        bin.display()
    );
    let out = Command::new(bin)
        .env("ANKOU_SOCKET", dir.join("ankoud.sock"))
        .args(["--lang", "en"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `connect` doit réussir ; sinon échec avec l'état complet du réseau.
fn connect(dir: &Path, config: &Path) {
    let (code, stdout, stderr) = cli(
        dir,
        &[
            "connect",
            "--wait",
            "10",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    if code == Some(0) && stdout.contains("Connected to") {
        return;
    }
    let mut report = format!("pas de connexion ({code:?})\n{stdout}{stderr}\n");
    for args in [
        &["ip", "netns", "exec", CLIENT_NS, "wg", "show"][..],
        &["ip", "netns", "exec", SERVER_NS, "wg", "show"],
        &["ip", "-n", CLIENT_NS, "addr"],
        &["ip", "-n", CLIENT_NS, "rule"],
        &["ip", "-n", CLIENT_NS, "route", "show", "table", "all"],
    ] {
        let out = Command::new(args[0]).args(&args[1..]).output();
        let text = out.map_or_else(
            |e| e.to_string(),
            |o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            },
        );
        report.push_str(&format!("--- {} ---\n{text}", args.join(" ")));
    }
    panic!("{report}");
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
    assert!(
        !succeeds(&[
            "ip", "netns", "exec", CLIENT_NS, "nft", "list", "table", "inet", "ankou"
        ]),
        "table nftables restée"
    );
}

/// `ping` depuis le namespace client (`-6` pour une adresse IPv6).
fn reachable(addr: &str) -> bool {
    let family = if addr.contains(':') { "-6" } else { "-4" };
    succeeds(&[
        "ip", "netns", "exec", CLIENT_NS, "ping", family, "-c1", "-W1", addr,
    ])
}

/// Envoi UDP depuis le namespace client ; nftables refuse l'envoi (EPERM) s'il bloque.
fn udp_send(ip: &str, port: u16) -> bool {
    let script = format!("echo x > /dev/udp/{ip}/{port}");
    succeeds(&["ip", "netns", "exec", CLIENT_NS, "bash", "-c", &script])
}

fn signal(child: &Child, sig: &str) {
    run(&["kill", sig, &child.id().to_string()]);
}

fn stop_daemon(mut daemon: Child) {
    signal(&daemon, "-TERM");
    assert_eq!(wait_exit(&mut daemon, Duration::from_secs(5)), Some(0));
}

/// Sans passerelle : seule la route du tunnel mène à 10.200.0.1.
#[test]
#[ignore]
fn connects_and_routes_all_traffic() {
    let (lab, config) = Lab::new(true, false);
    let daemon = spawn_daemon(lab.dir.path());
    connect(lab.dir.path(), &config);
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
    let (code, stdout, _) = cli(lab.dir.path(), &["disconnect"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert_clean();
    stop_daemon(daemon);
}

/// Arrêt du service pendant une connexion : tunnel retiré, protection gardée ; au
/// redémarrage, la connexion reprend ; seul `disconnect` lève la protection.
#[test]
#[ignore]
fn daemon_stop_keeps_protection() {
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    connect(dir, &config);
    let route = run(&["ip", "-n", CLIENT_NS, "route", "get", "10.200.0.1"]);
    assert!(route.contains("dev ankou0"), "trafic hors tunnel : {route}");
    stop_daemon(daemon);
    assert!(!succeeds(&[
        "ip", "-n", CLIENT_NS, "link", "show", "ankou0"
    ]));
    assert!(!reachable("192.168.77.2"), "fuite après l'arrêt du service");
    let daemon = spawn_daemon(dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cli(dir, &["status"]).1.contains("Connected to") {
        assert!(
            Instant::now() < deadline,
            "pas de reprise après redémarrage"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn crashed_daemon_reconnects_on_restart() {
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let mut daemon = spawn_daemon(dir);
    assert_eq!(cli(dir, &["autoconnect", "on"]).0, Some(0));
    connect(dir, &config);
    signal(&daemon, "-KILL");
    let _ = daemon.wait();
    assert!(succeeds(&["ip", "-n", CLIENT_NS, "link", "show", "ankou0"]));
    // Socket laissé par le crash : le nouveau service le remplace.
    let daemon = spawn_daemon(dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, stdout, _) = cli(dir, &["status"]);
        if stdout.contains("Connected to") {
            break;
        }
        assert!(Instant::now() < deadline, "pas reconnecté : {stdout}");
        std::thread::sleep(Duration::from_millis(200));
    }
    let rules = run(&["ip", "-n", CLIENT_NS, "rule"]);
    assert_eq!(
        rules.matches("not from all fwmark 0xca6c").count(),
        1,
        "règles en double :\n{rules}"
    );
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn unreachable_server_keeps_trying() {
    let (lab, config) = Lab::new(false, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    let (code, _, stderr) = cli(
        dir,
        &[
            "connect",
            "--wait",
            "8",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(1));
    assert!(stderr.contains("Still trying"), "{stderr}");
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn ipv6_blocked_while_connected() {
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    assert!(reachable("fd00:77::2"), "IPv6 local absent avant connexion");
    assert_eq!(cli(dir, &["killswitch", "off"]).0, Some(0));
    connect(dir, &config);
    assert!(!reachable("fd00:77::2"), "fuite IPv6");
    assert!(
        reachable("192.168.77.2"),
        "IPv4 local bloqué sans kill switch"
    );
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert!(reachable("fd00:77::2"));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn kill_switch_blocks_outside_tunnel() {
    let (lab, config) = Lab::new(true, false);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    connect(dir, &config);
    assert!(!reachable("192.168.77.2"), "fuite IPv4 hors tunnel");
    assert!(reachable("10.200.0.1"), "tunnel inutilisable");
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert!(reachable("192.168.77.2"));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn dns_outside_tunnel_is_blocked() {
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    assert!(
        udp_send("192.168.77.2", 53),
        "envoi UDP impossible avant connexion"
    );
    connect(dir, &config);
    assert!(
        !udp_send("192.168.77.2", 53),
        "fuite DNS vers le réseau local"
    );
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert!(udp_send("192.168.77.2", 53));
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn kill_switch_holds_while_server_unreachable() {
    let (lab, config) = Lab::new(false, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    let (code, _, _) = cli(
        dir,
        &[
            "connect",
            "--wait",
            "6",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(1));
    assert!(
        !reachable("192.168.77.2"),
        "fuite pendant les nouvelles tentatives"
    );
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert!(reachable("192.168.77.2"));
    assert_clean();
    stop_daemon(daemon);
}

#[test]
#[ignore]
fn kill_switch_survives_daemon_crash() {
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let mut daemon = spawn_daemon(dir);
    connect(dir, &config);
    signal(&daemon, "-KILL");
    let _ = daemon.wait();
    // Tunnel disparu, service mort : les règles restent, rien ne sort.
    run(&["ip", "-n", CLIENT_NS, "link", "del", "ankou0"]);
    assert!(
        !reachable("192.168.77.2"),
        "fuite après le crash du service"
    );
    // auto_connect désactivé : la reprise vient des règles restées.
    let daemon = spawn_daemon(dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, stdout, _) = cli(dir, &["status"]);
        if stdout.contains("Connected to") {
            break;
        }
        assert!(Instant::now() < deadline, "pas de reprise : {stdout}");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert!(reachable("192.168.77.2"));
    assert_clean();
    stop_daemon(daemon);
}

/// Deux candidats sur le même serveur : clé fausse (aucun handshake) et vraie clé.
#[test]
#[ignore]
fn fastest_server_is_chosen() {
    use ankou_proto::TunnelConfig;
    use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, ServerEntry};
    let (lab, config) = Lab::new(true, true);
    let dir = lab.dir.path();
    let daemon = spawn_daemon(dir);
    let cfg: TunnelConfig = toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let entry = |name: &str, public_key| ServerEntry {
        name: name.into(),
        public_key,
        endpoint: cfg.peer.endpoint.clone(),
    };
    let servers = vec![
        entry("bad", PrivateKey::generate().public_key()),
        entry("good", cfg.peer.public_key.clone()),
    ];
    let socket = dir.join("ankoud.sock");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let reply = rt.block_on(async {
        let mut c = ankou_ipc::connect(&socket).await.unwrap();
        c.send(&ClientToDaemon::ConnectFastest {
            config: Some(cfg.clone()),
            servers,
        })
        .await
        .unwrap();
        c.recv::<DaemonToClient>().await.unwrap().unwrap()
    });
    match reply {
        DaemonToClient::Chosen { server, rtt_ms } => {
            assert_eq!(server, "good");
            assert!(rtt_ms < 1000, "{rtt_ms} ms");
        }
        other => panic!("réponse inattendue : {other:?}"),
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cli(dir, &["status"])
        .1
        .contains("Connected to 192.168.77.2:51820")
    {
        assert!(Instant::now() < deadline, "pas connecté au serveur choisi");
        std::thread::sleep(Duration::from_millis(200));
    }
    let links = run(&["ip", "-n", CLIENT_NS, "link"]);
    assert!(
        !links.contains("ankoup"),
        "interface de mesure restée :\n{links}"
    );
    assert!(reachable("10.200.0.1"), "tunnel inutilisable");
    assert_eq!(cli(dir, &["disconnect"]).0, Some(0));
    assert_clean();
    stop_daemon(daemon);
}
