//! Mots de passe (Argon2id), jetons d'accès (PASETO v4.local) et secrets aléatoires (ADR 0003).

use crate::AppState;
use crate::error::AppError;
use ankou_proto::api::ApiErrorCode;
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::errors::{ClaimValidationError, Error as PasetoError};
use pasetors::keys::SymmetricKey;
use pasetors::token::UntrustedToken;
use pasetors::version4::V4;
use pasetors::{Local, local};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Durée de vie d'un jeton d'accès.
pub const ACCESS_TTL: Duration = Duration::minutes(15);
/// Durée de vie d'un refresh token.
pub const REFRESH_TTL: Duration = Duration::days(30);

fn random_bytes() -> [u8; 32] {
    let mut b = [0u8; 32];
    // ponytail: l'aléa système ne fait pas défaut sur les systèmes visés ; panique sinon.
    #[allow(clippy::expect_used)]
    getrandom::fill(&mut b).expect("aléa système indisponible");
    b
}

/// Secret aléatoire de 32 octets en base64url sans remplissage (43 caractères).
pub fn random_secret() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes())
}

pub fn random_key() -> SymmetricKey<V4> {
    key_from_bytes(&random_bytes()).unwrap_or_else(|_| unreachable!("32 octets"))
}

pub fn key_from_bytes(bytes: &[u8]) -> Result<SymmetricKey<V4>, PasetoError> {
    SymmetricKey::<V4>::from(bytes)
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(AppError::internal)
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

/// Jeton d'accès émis à l'instant `now` (expiration `now + ACCESS_TTL`).
pub fn issue_access_token_at(
    key: &SymmetricKey<V4>,
    user: Uuid,
    now: OffsetDateTime,
) -> Result<String, AppError> {
    let fmt = |t: OffsetDateTime| t.format(&Rfc3339).map_err(AppError::internal);
    let mut claims = Claims::new().map_err(AppError::internal)?;
    claims
        .subject(&user.to_string())
        .map_err(AppError::internal)?;
    claims.issued_at(&fmt(now)?).map_err(AppError::internal)?;
    claims.not_before(&fmt(now)?).map_err(AppError::internal)?;
    claims
        .expiration(&fmt(now + ACCESS_TTL)?)
        .map_err(AppError::internal)?;
    local::encrypt(key, &claims, None, None).map_err(AppError::internal)
}

/// Utilisateur authentifié par `Authorization: Bearer <v4.local…>` (compte actif).
pub struct AuthUser(pub Uuid);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let invalid = || AppError::new(ApiErrorCode::AuthInvalid, "invalid credentials");
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(invalid)?;
        let untrusted = UntrustedToken::<Local, V4>::try_from(token).map_err(|_| invalid())?;
        let trusted = local::decrypt(
            &state.key,
            &untrusted,
            &ClaimsValidationRules::new(),
            None,
            None,
        )
        .map_err(|e| match e {
            PasetoError::ClaimValidation(ClaimValidationError::Exp) => {
                AppError::new(ApiErrorCode::AuthExpired, "access token expired")
            }
            _ => invalid(),
        })?;
        let user: Uuid = trusted
            .payload_claims()
            .and_then(|c| c.get_claim("sub"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or_else(invalid)?;
        match state.store.user_by_id(user).await? {
            Some(u) if !u.disabled => Ok(AuthUser(user)),
            _ => Err(invalid()),
        }
    }
}
