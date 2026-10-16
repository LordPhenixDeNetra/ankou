use ankou_proto::api::{ApiError, ApiErrorCode};
use ankou_server_domain::DomainError;
use ankou_server_store::StoreError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Erreur d'API : code stable + détail technique.
#[derive(Debug)]
pub struct AppError(pub ApiErrorCode, pub String);

impl AppError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self(code, message.into())
    }

    /// Erreur interne : le détail va dans les journaux, pas dans la réponse.
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "erreur interne");
        Self(ApiErrorCode::Internal, "internal error".into())
    }
}

pub fn status_of(code: ApiErrorCode) -> StatusCode {
    use ApiErrorCode::*;
    match code {
        AuthInvalid | AuthExpired => StatusCode::UNAUTHORIZED,
        InvitationInvalid | PasswordWeak | Validation => StatusCode::UNPROCESSABLE_ENTITY,
        EmailTaken | DeviceKeyTaken => StatusCode::CONFLICT,
        DeviceLimit | PoolExhausted => StatusCode::FORBIDDEN,
        NotFound => StatusCode::NOT_FOUND,
        RateLimited => StatusCode::TOO_MANY_REQUESTS,
        Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn error_response(code: ApiErrorCode, message: String) -> Response {
    (status_of(code), Json(ApiError { code, message })).into_response()
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error_response(self.0, self.1)
    }
}

impl From<DomainError> for AppError {
    fn from(e: DomainError) -> Self {
        let code = match e {
            DomainError::InvalidEmail | DomainError::InvalidDeviceName => ApiErrorCode::Validation,
            DomainError::PasswordWeak => ApiErrorCode::PasswordWeak,
            DomainError::InvitationInvalid => ApiErrorCode::InvitationInvalid,
            DomainError::DeviceLimit => ApiErrorCode::DeviceLimit,
            DomainError::PoolExhausted => ApiErrorCode::PoolExhausted,
        };
        Self(code, e.to_string())
    }
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Domain(d) => d.into(),
            StoreError::EmailTaken => {
                Self::new(ApiErrorCode::EmailTaken, "email already registered")
            }
            StoreError::KeyTaken => Self::new(
                ApiErrorCode::DeviceKeyTaken,
                "public key already registered",
            ),
            StoreError::NotFound => Self::new(ApiErrorCode::NotFound, "not found"),
            StoreError::Db(_) | StoreError::Migrate(_) => Self::internal(e),
        }
    }
}
