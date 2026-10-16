//! Accès PostgreSQL du serveur aNkou : seule couche qui écrit du SQL.

mod devices;
mod error;
mod nodes;
mod servers;
mod sessions;
mod users;

pub use devices::Device;
pub use error::StoreError;
pub use nodes::{PEER_CHANNEL, PeerChange};
pub use servers::{NewServer, Server};
pub use users::User;

use sqlx::PgPool;
use sqlx::migrate::Migrator;

/// Migrations embarquées (aussi pour `#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]`).
pub static MIGRATOR: Migrator = sqlx::migrate!();

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Se connecte et applique les migrations embarquées.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPool::connect(url).await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}
