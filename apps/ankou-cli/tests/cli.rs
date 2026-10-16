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

#[cfg(unix)]
#[test]
fn world_readable_config_warns() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), VALID);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let a = cli()
        .args(["connect", "--lang", "en", "--retries", "0", "--config"])
        .arg(&path)
        .assert()
        .failure();
    assert!(stderr(&a).contains("chmod 600"), "{}", stderr(&a));
}

#[cfg(target_os = "linux")]
fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
#[test]
fn connect_without_root_says_sudo() {
    if is_root() {
        eprintln!("ignoré : lancé en root");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), VALID);
    let a = cli()
        .args(["connect", "--lang", "en", "--config"])
        .arg(&path)
        .assert()
        .code(1);
    assert!(stderr(&a).contains("sudo"), "{}", stderr(&a));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn connect_is_unavailable_off_linux() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), VALID);
    let a = cli()
        .args(["connect", "--lang", "en", "--config"])
        .arg(&path)
        .assert()
        .code(1);
    assert!(stderr(&a).contains("WireGuard"), "{}", stderr(&a));
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
