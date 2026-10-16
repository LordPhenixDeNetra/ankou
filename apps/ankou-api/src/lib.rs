//! API REST `/v1` du serveur aNkou (spec §7) ; `main.rs` ne fait que l'assemblage.

pub mod auth;
mod config;
mod error;
mod json;
mod nodes;
mod openapi;
mod routes;

pub use config::ApiConfig;
pub use error::AppError;
pub use nodes::{HubEvent, PeerHub, listen_peer_changes};

use ankou_proto::api::ApiErrorCode;
use ankou_server_store::Store;
use axum::Router;
use axum::routing::{get, post};
use pasetors::keys::SymmetricKey;
use pasetors::version4::V4;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub key: SymmetricKey<V4>,
    pub config: Arc<ApiConfig>,
    /// Jetons pour les hachages Argon2 simultanés (~19 Mio chacun).
    pub argon2: Arc<Semaphore>,
    /// Changements de pairs à relayer aux nodes.
    pub hub: PeerHub,
}

impl AppState {
    /// Autant de hachages simultanés que de cœurs : au-delà, ils attendent.
    pub fn argon2_permits() -> Arc<Semaphore> {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Arc::new(Semaphore::new(cores))
    }
}

/// Requêtes par seconde et rafale autorisées par IP sur `/v1/auth/*`.
const AUTH_PER_SECOND: u64 = 5;
const AUTH_BURST: u32 = 10;

pub fn router(state: AppState) -> Router {
    // L'IP client vient de X-Forwarded-For : l'API n'écoute que derrière Caddy, qui remplace
    // cet en-tête pour les clients directs (non « trusted proxies »).
    let governor = GovernorConfigBuilder::default()
        .per_second(AUTH_PER_SECOND)
        .burst_size(AUTH_BURST)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .map(|c| {
            GovernorLayer::new(c).error_handler(|_| {
                error::error_response(ApiErrorCode::RateLimited, "too many requests".into())
            })
        });
    let mut auth = Router::new()
        .route("/register", post(routes::auth::register))
        .route("/login", post(routes::auth::login))
        .route("/refresh", post(routes::auth::refresh))
        .route("/logout", post(routes::auth::logout));
    if let Some(layer) = governor {
        auth = auth.layer(layer);
    }
    Router::new()
        .route(
            "/v1/health",
            get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
        )
        .route("/v1/me", get(routes::auth::me))
        .route("/v1/servers", get(routes::servers::list))
        .route("/v1/nodes/ws", get(routes::nodes::ws))
        .route(
            "/v1/devices",
            get(routes::devices::list).post(routes::devices::enroll),
        )
        .route(
            "/v1/devices/{id}",
            axum::routing::delete(routes::devices::revoke),
        )
        .route(
            "/v1/openapi.json",
            get(|| async { axum::Json(<openapi::ApiDoc as utoipa::OpenApi>::openapi()) }),
        )
        .nest("/v1/auth", auth)
        .with_state(state)
}
