//! Contrat de l'API REST `/v1` partagé par le client et le serveur.

use crate::{Endpoint, PublicKey};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterRequest {
    #[serde(default)]
    pub invitation_code: Option<String>,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// Durée de validité du jeton d'accès, en secondes.
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: Uuid,
    pub name: String,
    pub country: String,
    pub city: String,
    pub endpoint: Endpoint,
    pub public_key: PublicKey,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrollDeviceRequest {
    pub name: String,
    pub public_key: PublicKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: Uuid,
    pub name: String,
    pub public_key: PublicKey,
    pub ipv4: Ipv4Addr,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrollDeviceResponse {
    pub device: DeviceInfo,
    /// Adresses de l'interface du client (`ipv4/32`).
    pub address: Vec<IpNet>,
    pub servers: Vec<ServerInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ApiErrorCode,
    /// Détail technique (anglais) ; les clients traduisent à partir de `code`.
    pub message: String,
}

/// Codes d'erreur stables de l'API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiErrorCode {
    #[serde(rename = "E_AUTH_INVALID")]
    AuthInvalid,
    #[serde(rename = "E_AUTH_EXPIRED")]
    AuthExpired,
    #[serde(rename = "E_INVITATION_INVALID")]
    InvitationInvalid,
    #[serde(rename = "E_EMAIL_TAKEN")]
    EmailTaken,
    #[serde(rename = "E_PASSWORD_WEAK")]
    PasswordWeak,
    #[serde(rename = "E_DEVICE_LIMIT")]
    DeviceLimit,
    #[serde(rename = "E_DEVICE_KEY_TAKEN")]
    DeviceKeyTaken,
    #[serde(rename = "E_POOL_EXHAUSTED")]
    PoolExhausted,
    #[serde(rename = "E_NOT_FOUND")]
    NotFound,
    #[serde(rename = "E_RATE_LIMITED")]
    RateLimited,
    #[serde(rename = "E_VALIDATION")]
    Validation,
    #[serde(rename = "E_INTERNAL")]
    Internal,
}

impl ApiErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthInvalid => "E_AUTH_INVALID",
            Self::AuthExpired => "E_AUTH_EXPIRED",
            Self::InvitationInvalid => "E_INVITATION_INVALID",
            Self::EmailTaken => "E_EMAIL_TAKEN",
            Self::PasswordWeak => "E_PASSWORD_WEAK",
            Self::DeviceLimit => "E_DEVICE_LIMIT",
            Self::DeviceKeyTaken => "E_DEVICE_KEY_TAKEN",
            Self::PoolExhausted => "E_POOL_EXHAUSTED",
            Self::NotFound => "E_NOT_FOUND",
            Self::RateLimited => "E_RATE_LIMITED",
            Self::Validation => "E_VALIDATION",
            Self::Internal => "E_INTERNAL",
        }
    }
}
