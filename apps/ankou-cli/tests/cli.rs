#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::path::Path;

fn version(lang: &str) -> String {
    let out = Command::cargo_bin("ankou-cli")
        .unwrap()
        .args(["version", "--lang", lang])
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn version_in_french() {
    assert_eq!(version("fr"), "aNkou 0.1.0 (protocole 1)\n");
}

#[test]
fn version_in_english() {
    assert_eq!(version("en"), "aNkou 0.1.0 (protocol 1)\n");
}

#[test]
fn unknown_lang_falls_back_to_english() {
    assert!(version("de").contains("(protocol 1)"));
}

#[test]
fn locales_are_complete() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locales");
    lpdnsoft_i18n::check_complete(&lpdnsoft_i18n::load_catalog(&dir).unwrap()).unwrap();
}

const VALID: &str = r#"[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]

[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "127.0.0.1:9"
allowed_ips = ["0.0.0.0/0"]
"#;

fn cli() -> Command {
    Command::cargo_bin("ankou-cli").unwrap()
}

fn write_config(dir: &Path, text: &str) -> std::path::PathBuf {
    let path = dir.join("client.toml");
    std::fs::write(&path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

#[test]
fn init_writes_private_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    let out = cli()
        .args(["init", "--lang", "en", "--out"])
        .arg(&path)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let key = stdout.trim().rsplit(' ').next().unwrap();
    assert_eq!(key.len(), 44, "{stdout}");
    assert!(key.parse::<ankou_proto::PublicKey>().is_ok(), "{stdout}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let before = std::fs::read_to_string(&path).unwrap();
    cli().args(["init", "--out"]).arg(&path).assert().failure();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn invalid_config_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &VALID.replace("127.0.0.1:9", "x"));
    let a = cli()
        .args(["connect", "--lang", "en", "--config"])
        .arg(&path)
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("client.toml") && err.contains("endpoint"),
        "{err}"
    );
    let a = cli()
        .args(["connect", "--lang", "fr", "--config"])
        .arg(&path)
        .assert()
        .code(2);
    assert!(stderr(&a).contains("Configuration invalide"));
}

/// Commande de démarrage du service que le message d'erreur cite (Unix).
#[cfg(unix)]
const START_HINT: &str = if cfg!(target_os = "macos") {
    "launchctl bootstrap system"
} else {
    "systemctl enable --now ankoud"
};

/// `ankou-cli` avec un socket inexistant : aucun service ne répond.
fn cli_without_daemon(dir: &Path) -> Command {
    let mut c = cli();
    c.env("ANKOU_SOCKET", dir.join("absent.sock"));
    c
}

#[cfg(unix)]
#[test]
fn world_readable_config_warns() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), VALID);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let a = cli_without_daemon(dir.path())
        .args(["connect", "--lang", "en", "--config"])
        .arg(&path)
        .assert()
        .failure();
    assert!(stderr(&a).contains("chmod 600"), "{}", stderr(&a));
}

/// Message propre à Linux (systemd) ; le service Windows arrive à l'étape 5.
#[cfg(unix)]
#[test]
fn connect_without_daemon_says_start_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), VALID);
    let a = cli_without_daemon(dir.path())
        .args(["connect", "--lang", "en", "--config"])
        .arg(&path)
        .assert()
        .code(1);
    assert!(stderr(&a).contains(START_HINT), "{}", stderr(&a));
}

#[test]
fn status_without_daemon_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    for args in [["status"], ["disconnect"]] {
        cli_without_daemon(dir.path())
            .args(args)
            .args(["--lang", "en"])
            .assert()
            .code(1);
    }
}

#[test]
fn enroll_without_session_says_login() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("ankou-cli")
        .unwrap()
        .env("ANKOU_SESSION_FILE", dir.path().join("session.toml"))
        .args(["enroll", "--lang", "en", "--name", "Laptop", "--out"])
        .arg(dir.path().join("client.toml"))
        .assert()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("ankou-cli login"), "{stderr}");
    assert!(!dir.path().join("client.toml").exists());
}

/// Chemin de sortie inutilisable : refusé avant tout appel à l'API (aucun appareil consommé).
#[test]
fn enroll_checks_output_before_calling_api() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.toml");
    std::fs::write(
        &session,
        "api_url = \"http://127.0.0.1:9\"\nrefresh_token = \"r\"\n",
    )
    .unwrap();
    let out = Command::cargo_bin("ankou-cli")
        .unwrap()
        .env("ANKOU_SESSION_FILE", &session)
        .args(["enroll", "--lang", "en", "--name", "Laptop", "--out"])
        .arg(dir.path().join("missing").join("client.toml"))
        .assert()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("Local error"), "{stderr}");
}

#[test]
fn fastest_without_session_says_login() {
    let dir = tempfile::tempdir().unwrap();
    let a = cli_without_daemon(dir.path())
        .env("ANKOU_SESSION_FILE", dir.path().join("session.toml"))
        .args(["connect", "--lang", "en", "--fastest"])
        .assert()
        .code(1);
    assert!(stderr(&a).contains("ankou-cli login"), "{}", stderr(&a));
}

#[test]
fn country_requires_fastest() {
    let dir = tempfile::tempdir().unwrap();
    cli_without_daemon(dir.path())
        .args(["connect", "--country", "FR"])
        .assert()
        .code(2);
}

/// API injoignable (tunnel mort, kill switch) : la dernière liste des serveurs sert encore.
#[cfg(unix)]
#[test]
fn fastest_uses_cached_servers_when_api_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.toml");
    std::fs::write(
        &session,
        "api_url = \"http://127.0.0.1:9\"\nrefresh_token = \"r\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("servers.json"),
        r#"[{"id":"00000000-0000-0000-0000-000000000000","name":"vps1","country":"FR","city":"Paris",
            "endpoint":"192.0.2.1:51820","public_key":"xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=","tags":[]}]"#,
    )
    .unwrap();
    // L'API ne répond pas, la liste en cache est prise : on arrive jusqu'au service (absent).
    let a = cli_without_daemon(dir.path())
        .env("ANKOU_SESSION_FILE", &session)
        .args(["connect", "--lang", "en", "--fastest"])
        .assert()
        .code(1);
    assert!(stderr(&a).contains(START_HINT), "{}", stderr(&a));
}
