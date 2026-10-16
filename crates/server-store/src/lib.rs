//! Accès PostgreSQL du serveur aNkou : seule couche qui écrit du SQL.

mod devices;
mod error;
mod servers;
mod sessions;
mod users;

pub use devices::Device;
pub use error::StoreError;
pub use servers::{NewServer, Server};
pub use users::User;

use sqlx::PgPool;

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Se connecte et applique les migrations embarquées.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPool::connect(url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}
