use crate::{Store, StoreError};
use ankou_proto::{Endpoint, PublicKey};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub id: Uuid,
    pub name: String,
    pub country: String,
    pub city: String,
    pub endpoint: Endpoint,
    pub public_key: PublicKey,
    pub tags: Vec<String>,
    pub active: bool,
}

pub struct NewServer {
    pub name: String,
    pub country: String,
    pub city: String,
    pub endpoint: Endpoint,
    pub public_key: PublicKey,
    pub tags: Vec<String>,
    pub node_token_hash: [u8; 32],
}

/// Valeur stockée par nous mais illisible : erreur de décodage.
pub(crate) fn decode<T, E>(r: Result<T, E>) -> Result<T, StoreError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    r.map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))
}

pub(crate) struct ServerRow {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) country: String,
    pub(crate) city: String,
    pub(crate) endpoint: String,
    pub(crate) public_key: String,
    pub(crate) tags: Vec<String>,
    pub(crate) active: bool,
}

impl TryFrom<ServerRow> for Server {
    type Error = StoreError;
    fn try_from(r: ServerRow) -> Result<Self, StoreError> {
        Ok(Server {
            id: r.id,
            name: r.name,
            country: r.country,
            city: r.city,
            endpoint: decode(r.endpoint.parse())?,
            public_key: decode(r.public_key.parse())?,
            tags: r.tags,
            active: r.active,
        })
    }
}

impl Store {
    pub async fn add_server(&self, s: NewServer) -> Result<Server, StoreError> {
        let row = sqlx::query_as!(
            ServerRow,
            "INSERT INTO servers (id, name, country, city, endpoint, public_key, tags, node_token_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING id, name, country, city, endpoint, public_key, tags, active",
            Uuid::now_v7(),
            s.name,
            s.country,
            s.city,
            s.endpoint.to_string(),
            s.public_key.to_string(),
            &s.tags,
            &s.node_token_hash[..]
        )
        .fetch_one(&self.pool)
        .await?;
        row.try_into()
    }

    pub async fn active_servers(&self) -> Result<Vec<Server>, StoreError> {
        sqlx::query_as!(
            ServerRow,
            "SELECT id, name, country, city, endpoint, public_key, tags, active
             FROM servers WHERE active ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Server::try_from)
        .collect()
    }

    pub async fn all_servers(&self) -> Result<Vec<Server>, StoreError> {
        sqlx::query_as!(
            ServerRow,
            "SELECT id, name, country, city, endpoint, public_key, tags, active
             FROM servers ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Server::try_from)
        .collect()
    }
}
