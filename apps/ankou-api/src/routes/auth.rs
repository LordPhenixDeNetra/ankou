use crate::AppState;
use crate::auth::{
    ACCESS_TTL, AuthUser, REFRESH_TTL, hash_password, issue_access_token_at, random_secret,
    verify_password,
};
use crate::error::AppError;
use crate::json::ApiJson;
use ankou_proto::api::{
    ApiError, ApiErrorCode, LoginRequest, RefreshRequest, RegisterRequest, TokenPair,
};
use ankou_server_domain::{hash_secret, normalize_email, validate_password};
use ankou_server_store::StoreError;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::LazyLock;
use time::OffsetDateTime;
use uuid::Uuid;

/// Haché vérifié quand l'e-mail est inconnu : même coût qu'un vrai mot de passe.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("dummy password for timing").unwrap_or_default());

fn invalid() -> AppError {
    AppError::new(ApiErrorCode::AuthInvalid, "invalid credentials")
}

async fn issue_tokens(state: &AppState, user: Uuid) -> Result<TokenPair, AppError> {
    let now = OffsetDateTime::now_utc();
    let refresh = random_secret();
    state
        .store
        .create_session(user, hash_secret(&refresh), now + REFRESH_TTL)
        .await?;
    Ok(TokenPair {
        access_token: issue_access_token_at(&state.key, user, now)?,
        refresh_token: refresh,
        expires_in: ACCESS_TTL.whole_seconds().unsigned_abs(),
    })
}

/// Argon2 est coûteux en CPU et en mémoire : hors du fil asynchrone, et en nombre limité.
async fn hashing<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, AppError> {
    let permit = state
        .argon2
        .clone()
        .acquire_owned()
        .await
        .map_err(AppError::internal)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        f()
    })
    .await
    .map_err(AppError::internal)
}

/// Crée un compte avec un code d'invitation.
#[utoipa::path(post, path = "/v1/auth/register", request_body = RegisterRequest, responses((status = 201, body = TokenPair), (status = 422, body = ApiError)))]
pub async fn register(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<RegisterRequest>,
) -> Result<(StatusCode, Json<TokenPair>), AppError> {
    let email = normalize_email(&req.email)?;
    validate_password(&req.password)?;
    let invitation = match (&req.invitation_code, state.config.registration_open) {
        (Some(code), _) => Some(hash_secret(code.trim())),
        (None, true) => None,
        (None, false) => {
            return Err(AppError::new(
                ApiErrorCode::InvitationInvalid,
                "invitation required",
            ));
        }
    };
    let password = req.password;
    let hash = hashing(&state, move || hash_password(&password)).await??;
    let user = state
        .store
        .register_user(invitation, &email, &hash, OffsetDateTime::now_utc())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(issue_tokens(&state, user.id).await?),
    ))
}

/// Connexion par e-mail et mot de passe.
#[utoipa::path(post, path = "/v1/auth/login", request_body = LoginRequest, responses((status = 200, body = TokenPair), (status = 401, body = ApiError)))]
pub async fn login(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Json<TokenPair>, AppError> {
    let user = match normalize_email(&req.email) {
        Ok(email) => state.store.user_by_email(&email).await?,
        Err(_) => None,
    };
    let hash = user
        .as_ref()
        .map_or_else(|| DUMMY_HASH.clone(), |u| u.password_hash.clone());
    let password = req.password;
    let ok = hashing(&state, move || verify_password(&password, &hash)).await?;
    match user {
        Some(u) if ok && !u.disabled => Ok(Json(issue_tokens(&state, u.id).await?)),
        _ => Err(invalid()),
    }
}

/// Échange un refresh token contre une nouvelle paire (l'ancien devient invalide).
#[utoipa::path(post, path = "/v1/auth/refresh", request_body = RefreshRequest, responses((status = 200, body = TokenPair), (status = 401, body = ApiError)))]
pub async fn refresh(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<RefreshRequest>,
) -> Result<Json<TokenPair>, AppError> {
    let now = OffsetDateTime::now_utc();
    let new = random_secret();
    let user = state
        .store
        .rotate_session(
            hash_secret(&req.refresh_token),
            hash_secret(&new),
            now + REFRESH_TTL,
            now,
        )
        .await
        .map_err(|e| match e {
            // Jeton inconnu, expiré ou déjà utilisé ; une panne de base reste une erreur interne.
            StoreError::NotFound => invalid(),
            e => e.into(),
        })?;
    Ok(Json(TokenPair {
        access_token: issue_access_token_at(&state.key, user, now)?,
        refresh_token: new,
        expires_in: ACCESS_TTL.whole_seconds().unsigned_abs(),
    }))
}

/// Révoque un refresh token.
#[utoipa::path(post, path = "/v1/auth/logout", request_body = RefreshRequest, responses((status = 204)))]
pub async fn logout(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<RefreshRequest>,
) -> Result<StatusCode, AppError> {
    state
        .store
        .delete_session(hash_secret(&req.refresh_token))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Identifiant de l'utilisateur du jeton.
#[utoipa::path(get, path = "/v1/me", responses((status = 200)), security(("bearer" = [])))]
pub async fn me(AuthUser(id): AuthUser) -> Json<Value> {
    Json(json!({ "id": id }))
}
