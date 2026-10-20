#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_api::{ApiConfig, AppState, PeerHub, auth, router};
use ankou_api_client::{ApiClient, ClientError, Session};
use ankou_proto::PrivateKey;
use ankou_proto::api::ApiErrorCode;
use ankou_server_domain::hash_secret;
use ankou_server_store::{NewServer, Store};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use time::{Duration, OffsetDateTime};

const PASSWORD: &str = "correct horse battery";

struct Api {
    url: String,
    store: Store,
    state: AppState,
}

async fn serve(pool: PgPool) -> Api {
    let store = Store::from_pool(pool);
    store
        .add_server(NewServer {
            name: "vps1".into(),
            country: "FR".into(),
            city: "Paris".into(),
            endpoint: "vps1.example.org:51820".parse().unwrap(),
            public_key: PrivateKey::generate().public_key(),
            tags: vec![],
            node_token_hash: hash_secret("tok"),
        })
        .await
        .unwrap();
    let state = AppState {
        store: store.clone(),
        key: auth::random_key(),
        config: Arc::new(ApiConfig::for_tests()),
        argon2: AppState::argon2_permits(),
        hub: PeerHub::new(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Api {
        url: format!("http://{addr}"),
        store,
        state,
    }
}

async fn invitation(store: &Store) -> String {
    let code = auth::random_secret();
    store
        .create_invitation(
            hash_secret(&code),
            None,
            OffsetDateTime::now_utc() + Duration::days(1),
        )
        .await
        .unwrap();
    code
}

async fn registered(api: &Api) -> ApiClient {
    let code = invitation(&api.store).await;
    let mut client = ApiClient::new(&api.url).unwrap();
    client.register(&code, "a@x.org", PASSWORD).await.unwrap();
    client
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn register_login_enroll_flow(pool: PgPool) {
    let api = serve(pool).await;
    let mut client = registered(&api).await;
    assert_eq!(client.servers().await.unwrap()[0].name, "vps1");
    let key = PrivateKey::generate().public_key();
    let enrolled = client.enroll("Laptop", &key).await.unwrap();
    assert_eq!(enrolled.address, vec!["10.8.0.2/32".parse().unwrap()]);
    let before = client.session().unwrap().refresh_token.clone();
    let mut other = ApiClient::new(&api.url).unwrap();
    let session = other.login("A@X.org", PASSWORD).await.unwrap();
    assert_ne!(session.refresh_token, before);
    assert_eq!(session.api_url, api.url);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn new_device_builds_tunnel_config(pool: PgPool) {
    let api = serve(pool).await;
    let mut client = registered(&api).await;
    assert!(matches!(
        client.new_device("Laptop", Some("nope")).await,
        Err(ankou_api_client::ClientError::UnknownServer(n)) if n == "nope"
    ));
    let (config, server) = client.new_device("Laptop", None).await.unwrap();
    assert_eq!(server, "vps1");
    assert_eq!(
        config.interface.address,
        vec!["10.8.0.2/32".parse().unwrap()]
    );
    assert_eq!(config.peer.allowed_ips, vec!["0.0.0.0/0".parse().unwrap()]);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn client_refreshes_expired_access_token(pool: PgPool) {
    let api = serve(pool).await;
    let mut client = registered(&api).await;
    let user = api
        .store
        .user_by_email("a@x.org")
        .await
        .unwrap()
        .unwrap()
        .id;
    let expired = auth::issue_access_token_at(
        &api.state.key,
        user,
        OffsetDateTime::now_utc() - Duration::hours(1),
    )
    .unwrap();
    client.set_access_token(expired);
    let before = client.session().unwrap().refresh_token.clone();
    assert_eq!(client.servers().await.unwrap().len(), 1);
    assert_ne!(
        client.session().unwrap().refresh_token,
        before,
        "rotation attendue"
    );
    // Une session rechargée (sans jeton d'accès) fonctionne aussi.
    let mut reloaded = ApiClient::with_session(client.session().unwrap().clone()).unwrap();
    assert_eq!(reloaded.servers().await.unwrap().len(), 1);
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn revoked_session_reports_expired(pool: PgPool) {
    let api = serve(pool).await;
    let mut client = registered(&api).await;
    api.store.set_user_disabled("a@x.org", true).await.unwrap();
    assert!(matches!(
        client.servers().await,
        Err(ClientError::SessionExpired)
    ));
}

#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]
#[ignore = "PostgreSQL requis"]
async fn api_errors_keep_their_code(pool: PgPool) {
    let api = serve(pool).await;
    let mut client = ApiClient::new(&api.url).unwrap();
    let err = client
        .register("fausse", "a@x.org", PASSWORD)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::Api(ApiErrorCode::InvitationInvalid, _)),
        "{err:?}"
    );
}

#[test]
fn refuses_plain_http_except_localhost() {
    assert!(ApiClient::new("http://api.example.org").is_err());
    assert!(ApiClient::new("ftp://api.example.org").is_err());
    assert!(ApiClient::new("http://127.0.0.1:8080").is_ok());
    assert!(ApiClient::new("http://localhost:8080").is_ok());
    assert!(ApiClient::new("https://api.example.org").is_ok());
}

#[test]
fn session_file_is_private() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("session.toml");
    assert!(Session::load(&path).unwrap().is_none());
    let s = Session {
        api_url: "https://api.example.org".into(),
        refresh_token: "r".into(),
    };
    s.save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap(), Some(s));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
