#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_api::{ApiConfig, AppState, auth, router};
use ankou_server_domain::hash_secret;
use ankou_server_store::Store;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";

struct App {
    router: Router,
    store: Store,
    state: AppState,
}

fn app_with(pool: PgPool, registration_open: bool) -> App {
    let store = Store::from_pool(pool);
    let state = AppState {
        store: store.clone(),
        key: auth::random_key(),
        argon2: AppState::argon2_permits(),
        config: Arc::new(ApiConfig {
            registration_open,
            ..ApiConfig::for_tests()
        }),
    };
    App {
        router: router(state.clone()),
        store,
        state,
    }
}

fn app(pool: PgPool) -> App {
    app_with(pool, false)
}

/// Requête JSON depuis l'IP `ip` (le limiteur de débit est par IP).
async fn call_from(
    app: &App,
    ip: &str,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", ip);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

async fn call(
    app: &App,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_from(app, "203.0.113.1", method, uri, token, body).await
}

async fn invite(store: &Store) -> String {
    let code = auth::random_secret();
    store
        .create_invitation(
            hash_secret(&code),
            None,
            OffsetDateTime::now_utc() + Duration::days(7),
        )
        .await
        .unwrap();
    code
}

async fn register(app: &App, email: &str) -> Value {
    let code = invite(&app.store).await;
    let (status, body) = call(
        app,
        "POST",
        "/v1/auth/register",
        None,
        Some(json!({"invitation_code": code, "email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn health_is_public(pool: PgPool) {
    let app = app(pool);
    let (status, body) = call(&app, "GET", "/v1/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"status": "ok"}));
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn register_with_invitation(pool: PgPool) {
    let app = app(pool);
    let tokens = register(&app, "alice@example.org").await;
    assert!(
        tokens["access_token"]
            .as_str()
            .unwrap()
            .starts_with("v4.local.")
    );
    assert_eq!(tokens["expires_in"], 900);
    assert_eq!(tokens["refresh_token"].as_str().unwrap().len(), 43);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn register_requires_invitation_when_closed(pool: PgPool) {
    let body = json!({"email": "a@x.org", "password": PASSWORD});
    let closed = app(pool.clone());
    let (status, err) = call(
        &closed,
        "POST",
        "/v1/auth/register",
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "E_INVITATION_INVALID");
    let open = app_with(pool, true);
    let (status, _) = call(&open, "POST", "/v1/auth/register", None, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn register_rejects_weak_password(pool: PgPool) {
    let app = app(pool);
    let code = invite(&app.store).await;
    let (status, err) = call(
        &app,
        "POST",
        "/v1/auth/register",
        None,
        Some(json!({"invitation_code": code, "email": "a@x.org", "password": "short-pass1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "E_PASSWORD_WEAK");
    let (status, _) = call(
        &app,
        "POST",
        "/v1/auth/register",
        None,
        Some(json!({"invitation_code": code, "email": "a@x.org", "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn login_errors_do_not_reveal_accounts(pool: PgPool) {
    let app = app(pool);
    register(&app, "alice@example.org").await;
    register(&app, "bob@example.org").await;
    app.store
        .set_user_disabled("bob@example.org", true)
        .await
        .unwrap();
    let mut answers = Vec::new();
    for (email, password) in [
        ("nobody@example.org", PASSWORD),
        ("alice@example.org", "wrong password!!"),
        ("bob@example.org", PASSWORD),
    ] {
        let (status, body) = call(
            &app,
            "POST",
            "/v1/auth/login",
            None,
            Some(json!({"email": email, "password": password})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "E_AUTH_INVALID");
        answers.push(body);
    }
    assert!(answers.windows(2).all(|w| w[0] == w[1]), "{answers:?}");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn login_is_case_insensitive_on_email(pool: PgPool) {
    let app = app(pool);
    register(&app, "Alice@Example.org").await;
    let (status, _) = call(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        Some(json!({"email": "alice@example.ORG", "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn refresh_token_is_single_use(pool: PgPool) {
    let app = app(pool);
    let r1 = register(&app, "a@x.org").await["refresh_token"].clone();
    let refresh = |t: Value| {
        call(
            &app,
            "POST",
            "/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": t})),
        )
    };
    let (status, pair2) = refresh(r1.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, err) = refresh(r1).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(err["code"], "E_AUTH_INVALID");
    let (status, _) = refresh(pair2["refresh_token"].clone()).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn logout_revokes_refresh_token(pool: PgPool) {
    let app = app(pool);
    let r1 = register(&app, "a@x.org").await["refresh_token"].clone();
    let (status, _) = call(
        &app,
        "POST",
        "/v1/auth/logout",
        None,
        Some(json!({"refresh_token": r1})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(json!({"refresh_token": r1})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn expired_access_token_is_rejected(pool: PgPool) {
    let app = app(pool);
    let tokens = register(&app, "a@x.org").await;
    let (status, me) = call(&app, "GET", "/v1/me", tokens["access_token"].as_str(), None).await;
    assert_eq!(status, StatusCode::OK);
    let id: uuid::Uuid = me["id"].as_str().unwrap().parse().unwrap();
    let old = auth::issue_access_token_at(
        &app.state.key,
        id,
        OffsetDateTime::now_utc() - Duration::hours(1),
    )
    .unwrap();
    let (status, err) = call(&app, "GET", "/v1/me", Some(&old), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(err["code"], "E_AUTH_EXPIRED");
    let (status, err) = call(&app, "GET", "/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(err["code"], "E_AUTH_INVALID");
    let (status, _) = call(&app, "GET", "/v1/me", Some("v4.local.garbage"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn auth_routes_are_rate_limited(pool: PgPool) {
    let app = app(pool);
    let mut limited = 0;
    for _ in 0..15 {
        let (status, body) = call_from(
            &app,
            "198.51.100.7",
            "POST",
            "/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": "invalid"})),
        )
        .await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(body["code"], "E_RATE_LIMITED");
            limited += 1;
        }
    }
    assert!(limited > 0);
    let (status, _) = call_from(&app, "198.51.100.8", "GET", "/v1/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

async fn add_server(store: &Store) {
    store
        .add_server(ankou_server_store::NewServer {
            name: "vps1".into(),
            country: "FR".into(),
            city: "Paris".into(),
            endpoint: "vps1.example.org:51820".parse().unwrap(),
            public_key: ankou_proto::PrivateKey::generate().public_key(),
            tags: vec![],
            node_token_hash: hash_secret(&auth::random_secret()),
        })
        .await
        .unwrap();
}

async fn token(app: &App, email: &str) -> String {
    register(app, email).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn new_key() -> String {
    ankou_proto::PrivateKey::generate().public_key().to_string()
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn servers_require_auth(pool: PgPool) {
    let app = app(pool);
    add_server(&app.store).await;
    let (status, err) = call(&app, "GET", "/v1/servers", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(err["code"], "E_AUTH_INVALID");
    let t = token(&app, "a@x.org").await;
    let (status, list) = call(&app, "GET", "/v1/servers", Some(&t), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["name"], "vps1");
    assert_eq!(list[0]["endpoint"], "vps1.example.org:51820");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn enroll_returns_address_and_servers(pool: PgPool) {
    let app = app(pool);
    add_server(&app.store).await;
    let t = token(&app, "a@x.org").await;
    let (status, body) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "  Laptop ", "public_key": new_key()})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["address"], json!(["10.8.0.2/32"]));
    assert_eq!(body["servers"].as_array().unwrap().len(), 1);
    assert_eq!(body["device"]["name"], "Laptop");
    let (_, list) = call(&app, "GET", "/v1/devices", Some(&t), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn enroll_rejects_bad_input(pool: PgPool) {
    let app = app(pool);
    let t = token(&app, "a@x.org").await;
    let (status, err) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": " ", "public_key": new_key()})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "E_VALIDATION");
    let (status, err) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "A", "public_key": "pas-une-clé"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "E_VALIDATION");
    let k = new_key();
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "A", "public_key": k})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, err) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "B", "public_key": k})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(err["code"], "E_DEVICE_KEY_TAKEN");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn revoked_device_frees_ip_and_key(pool: PgPool) {
    let app = app(pool);
    let t = token(&app, "a@x.org").await;
    let k = new_key();
    let (_, a) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "A", "public_key": k})),
    )
    .await;
    let id = a["device"]["id"].as_str().unwrap().to_owned();
    let (status, _) = call(&app, "DELETE", &format!("/v1/devices/{id}"), Some(&t), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = call(&app, "GET", "/v1/devices", Some(&t), None).await;
    assert_eq!(list, json!([]));
    let (status, again) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t),
        Some(json!({"name": "A", "public_key": k})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(again["device"]["ipv4"], "10.8.0.2");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn cannot_revoke_someone_elses_device(pool: PgPool) {
    let app = app(pool);
    let t1 = token(&app, "a@x.org").await;
    let t2 = token(&app, "b@x.org").await;
    let (_, a) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(&t1),
        Some(json!({"name": "A", "public_key": new_key()})),
    )
    .await;
    let id = a["device"]["id"].as_str().unwrap().to_owned();
    let (status, err) = call(
        &app,
        "DELETE",
        &format!("/v1/devices/{id}"),
        Some(&t2),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(err["code"], "E_NOT_FOUND");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn openapi_lists_routes(pool: PgPool) {
    let app = app(pool);
    let (status, doc) = call(&app, "GET", "/v1/openapi.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let text = doc.to_string();
    for route in [
        "/v1/devices",
        "/v1/auth/login",
        "/v1/servers",
        "/v1/devices/{id}",
    ] {
        assert!(text.contains(route), "{route} absent");
    }
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn refresh_during_database_outage_is_internal(pool: PgPool) {
    let app = app(pool.clone());
    let r = register(&app, "a@x.org").await["refresh_token"].clone();
    pool.close().await;
    let (status, err) = call(
        &app,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(json!({"refresh_token": r})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err["code"], "E_INTERNAL");
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn argon2_runs_are_capped(pool: PgPool) {
    let app = app(pool);
    let permits = app.state.argon2.available_permits() as u32;
    assert!(permits >= 1);
    let held = app
        .state
        .argon2
        .clone()
        .acquire_many_owned(permits)
        .await
        .unwrap();
    let router = app.router.clone();
    let login = tokio::spawn(async move {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/login")
            .header("x-forwarded-for", "203.0.113.9")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"email": "x@x.org", "password": PASSWORD}).to_string(),
            ))
            .unwrap();
        router.oneshot(req).await.unwrap().status()
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !login.is_finished(),
        "le hachage a démarré sans jeton du sémaphore"
    );
    drop(held);
    assert_eq!(login.await.unwrap(), StatusCode::UNAUTHORIZED);
}
