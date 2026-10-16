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

struct Row {
    id: Uuid,
    name: String,
    country: String,
    city: String,
    endpoint: String,
    public_key: String,
    tags: Vec<String>,
    active: bool,
}

impl TryFrom<Row> for Server {
    type Error = StoreError;
    fn try_from(r: Row) -> Result<Self, StoreError> {
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
            Row,
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
            Row,
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
            Row,
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
