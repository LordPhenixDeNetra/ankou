use crate::{Store, StoreError};
use time::OffsetDateTime;
use uuid::Uuid;

impl Store {
    pub async fn create_session(
        &self,
        user_id: Uuid,
        refresh_hash: [u8; 32],
        expires_at: OffsetDateTime,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            "INSERT INTO sessions (id, refresh_hash, user_id, expires_at) VALUES ($1, $2, $3, $4)",
            Uuid::now_v7(),
            &refresh_hash[..],
            user_id,
            expires_at
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remplace un refresh token valide par un nouveau ; l'ancien ne sert plus jamais.
    pub async fn rotate_session(
        &self,
        old: [u8; 32],
        new: [u8; 32],
        new_expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<Uuid, StoreError> {
        let mut tx = self.pool.begin().await?;
        let user_id = sqlx::query_scalar!(
            "DELETE FROM sessions WHERE refresh_hash = $1 AND expires_at > $2 RETURNING user_id",
            &old[..],
            now
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        sqlx::query!(
            "INSERT INTO sessions (id, refresh_hash, user_id, expires_at) VALUES ($1, $2, $3, $4)",
            Uuid::now_v7(),
            &new[..],
            user_id,
            new_expires_at
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(user_id)
    }

    pub async fn delete_session(&self, refresh_hash: [u8; 32]) -> Result<(), StoreError> {
        sqlx::query!(
            "DELETE FROM sessions WHERE refresh_hash = $1",
            &refresh_hash[..]
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
