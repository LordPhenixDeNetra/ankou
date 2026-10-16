#![allow(clippy::unwrap_used)]

use ankou_server_domain::hash_secret;
use ankou_server_store::Store;
use assert_cmd::Command;
use sqlx::PgPool;
use time::OffsetDateTime;

/// URL de la base de test créée par `#[sqlx::test]`.
fn url(pool: &PgPool) -> String {
    let base = std::env::var("DATABASE_URL").unwrap();
    let db = pool.connect_options().get_database().unwrap().to_owned();
    let (prefix, _) = base.rsplit_once('/').unwrap();
    format!("{prefix}/{db}")
}

fn admin(pool: &PgPool, lang: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::cargo_bin("ankou-admin")
        .unwrap()
        .args(["--lang", lang, "--database-url", &url(pool)])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn invite_then_register_through_store(pool: PgPool) {
    let (code, out, err) = admin(&pool, "en", &["invite", "create", "--days", "3"]);
    assert_eq!(code, 0, "{err}");
    let invitation = out.trim().rsplit(' ').next().unwrap();
    assert_eq!(invitation.len(), 43, "{out}");
    Store::from_pool(pool)
        .register_user(
            Some(hash_secret(invitation)),
            "a@x.org",
            "h",
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn server_add_prints_token_once(pool: PgPool) {
    let key = ankou_proto::PrivateKey::generate().public_key().to_string();
    let (code, out, err) = admin(
        &pool,
        "en",
        &[
            "server",
            "add",
            "--name",
            "vps1",
            "--country",
            "FR",
            "--city",
            "Paris",
            "--endpoint",
            "vps1.example.org:51820",
            "--public-key",
            &key,
            "--tag",
            "p2p",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let token = out.trim().rsplit(' ').next().unwrap().to_owned();
    assert_eq!(token.len(), 43, "{out}");
    let (code, list, _) = admin(&pool, "en", &["server", "list"]);
    assert_eq!(code, 0);
    assert!(
        list.contains("vps1") && list.contains("vps1.example.org:51820"),
        "{list}"
    );
    assert!(!list.contains(&token));
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn user_disable_and_enable(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    store
        .register_user(None, "alice@x.org", "h", OffsetDateTime::now_utc())
        .await
        .unwrap();
    let (code, _, err) = admin(&pool, "en", &["user", "disable", "Alice@X.org"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        store
            .user_by_email("alice@x.org")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
    let (code, list, _) = admin(&pool, "en", &["user", "list"]);
    assert_eq!(code, 0);
    assert!(list.contains("alice@x.org"), "{list}");
    let (code, _, _) = admin(&pool, "en", &["user", "enable", "alice@x.org"]);
    assert_eq!(code, 0);
    assert!(
        !store
            .user_by_email("alice@x.org")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
    let (code, _, err) = admin(&pool, "fr", &["user", "disable", "nobody@x.org"]);
    assert_eq!(code, 1);
    assert!(err.contains("introuvable"), "{err}");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn device_revoke_unknown_id(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    let user = store
        .register_user(None, "a@x.org", "h", OffsetDateTime::now_utc())
        .await
        .unwrap();
    let key = ankou_proto::PrivateKey::generate().public_key();
    let d = store
        .enroll_device(user.id, "Laptop", &key, "10.8.0.0/16".parse().unwrap())
        .await
        .unwrap();
    let (code, list, _) = admin(&pool, "en", &["device", "list", "--email", "a@x.org"]);
    assert_eq!(code, 0);
    assert!(
        list.contains("Laptop") && list.contains("10.8.0.2"),
        "{list}"
    );
    let (code, _, _) = admin(
        &pool,
        "en",
        &["device", "revoke", &uuid::Uuid::now_v7().to_string()],
    );
    assert_eq!(code, 1);
    let (code, _, _) = admin(&pool, "en", &["device", "revoke", &d.id.to_string()]);
    assert_eq!(code, 0);
    assert!(store.devices_of(user.id).await.unwrap().is_empty());
}
