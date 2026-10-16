use crate::{Store, StoreError};
use ankou_server_domain::{DomainError, InvitationState, check_invitation};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub disabled: bool,
    /// Limite d'appareils du plan ; `None` = illimité.
    pub max_devices: Option<u32>,
}

/// Contrainte d'unicité violée (code SQLSTATE 23505) sur `constraint`.
pub(crate) fn is_unique_violation(e: &sqlx::Error, constraint: &str) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") && db.constraint() == Some(constraint))
}

impl Store {
    pub async fn create_invitation(
        &self,
        code_hash: [u8; 32],
        created_by: Option<Uuid>,
        expires_at: OffsetDateTime,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO invitations (id, code_hash, created_by, expires_at) VALUES ($1, $2, $3, $4)",
            id,
            &code_hash[..],
            created_by,
            expires_at
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Crée le compte et consomme l'invitation dans une même transaction ; l'invitation
    /// est verrouillée pour qu'elle ne serve qu'une fois, même en concurrence.
    pub async fn register_user(
        &self,
        invitation: Option<[u8; 32]>,
        email: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<User, StoreError> {
        let mut tx = self.pool.begin().await?;
        let invitation_id = match invitation {
            Some(hash) => {
                let row = sqlx::query!(
                    "SELECT id, expires_at, used_by FROM invitations WHERE code_hash = $1 FOR UPDATE",
                    &hash[..]
                )
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(DomainError::InvitationInvalid)?;
                let state = InvitationState {
                    expires_at: row.expires_at,
                    used: row.used_by.is_some(),
                };
                check_invitation(&state, now)?;
                Some(row.id)
            }
            None => None,
        };
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO users (id, email, password_hash, plan_id)
             VALUES ($1, $2, $3, (SELECT id FROM plans WHERE name = 'famille'))",
            id,
            email,
            password_hash
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e, "users_email_key") {
                StoreError::EmailTaken
            } else {
                e.into()
            }
        })?;
        if let Some(inv) = invitation_id {
            sqlx::query!(
                "UPDATE invitations SET used_by = $1, used_at = $2 WHERE id = $3",
                id,
                now,
                inv
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.user_by_id(id).await?.ok_or(StoreError::NotFound)
    }

    pub async fn user_by_email(&self, email: &str) -> Result<Option<User>, StoreError> {
        let row = sqlx::query!(
            r#"SELECT u.id, u.email, u.password_hash, u.disabled_at IS NOT NULL AS "disabled!", p.max_devices
               FROM users u JOIN plans p ON p.id = u.plan_id WHERE u.email = $1"#,
            email
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| User {
            id: r.id,
            email: r.email,
            password_hash: r.password_hash,
            disabled: r.disabled,
            max_devices: r.max_devices.and_then(|m| u32::try_from(m).ok()),
        }))
    }

    pub async fn user_by_id(&self, id: Uuid) -> Result<Option<User>, StoreError> {
        let row = sqlx::query!(
            r#"SELECT u.id, u.email, u.password_hash, u.disabled_at IS NOT NULL AS "disabled!", p.max_devices
               FROM users u JOIN plans p ON p.id = u.plan_id WHERE u.id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| User {
            id: r.id,
            email: r.email,
            password_hash: r.password_hash,
            disabled: r.disabled,
            max_devices: r.max_devices.and_then(|m| u32::try_from(m).ok()),
        }))
    }

    pub async fn users(&self) -> Result<Vec<User>, StoreError> {
        let rows = sqlx::query!(
            r#"SELECT u.id, u.email, u.password_hash, u.disabled_at IS NOT NULL AS "disabled!", p.max_devices
               FROM users u JOIN plans p ON p.id = u.plan_id ORDER BY u.created_at, u.id"#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| User {
                id: r.id,
                email: r.email,
                password_hash: r.password_hash,
                disabled: r.disabled,
                max_devices: r.max_devices.and_then(|m| u32::try_from(m).ok()),
            })
            .collect())
    }

    /// Désactiver un compte supprime aussi ses sessions.
    pub async fn set_user_disabled(&self, email: &str, disabled: bool) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let id = sqlx::query_scalar!(
            "UPDATE users SET disabled_at = CASE WHEN $2 THEN now() ELSE NULL END
             WHERE email = $1 RETURNING id",
            email,
            disabled
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        if disabled {
            sqlx::query!("DELETE FROM sessions WHERE user_id = $1", id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
