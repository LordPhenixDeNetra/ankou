//! Accès PostgreSQL du serveur aNkou : seule couche qui écrit du SQL.

mod error;

pub use error::StoreError;

use sqlx::PgPool;

#[derive(Clone)]
pub struct Store {
    #[allow(dead_code)] // lu par les opérations ajoutées juste après (Task 3)
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
