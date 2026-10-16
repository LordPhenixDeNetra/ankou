use crate::error::AppError;
use ankou_proto::api::ApiErrorCode;
use axum::extract::FromRequest;
use axum::extract::rejection::JsonRejection;

/// `Json` dont les corps invalides donnent `422 E_VALIDATION` au format `ApiError`.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct ApiJson<T>(pub T);

impl From<JsonRejection> for AppError {
    fn from(r: JsonRejection) -> Self {
        AppError::new(ApiErrorCode::Validation, r.body_text())
    }
}
