use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::json::ApiJson;
use crate::routes::servers::info;
use ankou_proto::api::{DeviceInfo, EnrollDeviceRequest, EnrollDeviceResponse};
use ankou_server_domain::validate_device_name;
use ankou_server_store::Device;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ipnet::{IpNet, Ipv4Net};
use uuid::Uuid;

fn device_info(d: Device) -> DeviceInfo {
    DeviceInfo {
        id: d.id,
        name: d.name,
        public_key: d.public_key,
        ipv4: d.ipv4,
        created_at: d.created_at,
    }
}

/// Enregistre un appareil (clé publique WireGuard générée sur l'appareil).
#[utoipa::path(post, path = "/v1/devices", request_body = EnrollDeviceRequest,
    responses((status = 201, body = EnrollDeviceResponse)), security(("bearer" = [])))]
pub async fn enroll(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    ApiJson(req): ApiJson<EnrollDeviceRequest>,
) -> Result<(StatusCode, Json<EnrollDeviceResponse>), AppError> {
    let name = validate_device_name(&req.name)?;
    let device = state
        .store
        .enroll_device(user, &name, &req.public_key, state.config.ip_pool)
        .await?;
    let servers = state
        .store
        .active_servers()
        .await?
        .into_iter()
        .map(info)
        .collect();
    Ok((
        StatusCode::CREATED,
        Json(EnrollDeviceResponse {
            address: vec![IpNet::V4(Ipv4Net::from(device.ipv4))],
            device: device_info(device),
            servers,
        }),
    ))
}

/// Appareils actifs de l'utilisateur.
#[utoipa::path(get, path = "/v1/devices", responses((status = 200, body = Vec<DeviceInfo>)), security(("bearer" = [])))]
pub async fn list(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeviceInfo>>, AppError> {
    Ok(Json(
        state
            .store
            .devices_of(user)
            .await?
            .into_iter()
            .map(device_info)
            .collect(),
    ))
}

/// Révoque un appareil de l'utilisateur.
#[utoipa::path(delete, path = "/v1/devices/{id}", params(("id" = Uuid, Path)),
    responses((status = 204)), security(("bearer" = [])))]
pub async fn revoke(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state.store.revoke_device(Some(user), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
