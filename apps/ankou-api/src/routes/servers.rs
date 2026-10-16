use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use ankou_proto::api::ServerInfo;
use ankou_server_store::Server;
use axum::Json;
use axum::extract::State;

pub fn info(s: Server) -> ServerInfo {
    ServerInfo {
        id: s.id,
        name: s.name,
        country: s.country,
        city: s.city,
        endpoint: s.endpoint,
        public_key: s.public_key,
        tags: s.tags,
    }
}

/// Serveurs actifs.
#[utoipa::path(get, path = "/v1/servers", responses((status = 200, body = Vec<ServerInfo>)), security(("bearer" = [])))]
pub async fn list(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServerInfo>>, AppError> {
    Ok(Json(
        state
            .store
            .active_servers()
            .await?
            .into_iter()
            .map(info)
            .collect(),
    ))
}
