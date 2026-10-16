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
