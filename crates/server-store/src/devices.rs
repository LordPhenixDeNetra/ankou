use crate::servers::decode;
use crate::users::is_unique_violation;
use crate::{Store, StoreError};
use ankou_proto::PublicKey;
use ankou_server_domain::{allocate_ipv4, check_device_limit};
use ipnet::{IpNet, Ipv4Net};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use time::OffsetDateTime;
use uuid::Uuid;

/// Verrou transactionnel PostgreSQL qui sérialise les enrôlements (allocation d'adresse).
const ENROLL_LOCK: i64 = 4242;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub public_key: PublicKey,
    pub ipv4: Ipv4Addr,
    pub created_at: OffsetDateTime,
}

fn ipv4_of(net: IpNet) -> Result<Ipv4Addr, StoreError> {
    match net.addr() {
        IpAddr::V4(ip) => Ok(ip),
        IpAddr::V6(_) => decode(Err(std::io::Error::other(
            "adresse IPv6 dans la colonne ipv4",
        ))),
    }
}

struct Row {
    id: Uuid,
    user_id: Uuid,
    name: String,
    public_key: String,
    ipv4: IpNet,
    created_at: OffsetDateTime,
}

impl TryFrom<Row> for Device {
    type Error = StoreError;
    fn try_from(r: Row) -> Result<Self, StoreError> {
        Ok(Device {
            id: r.id,
            user_id: r.user_id,
            name: r.name,
            public_key: decode(r.public_key.parse())?,
            ipv4: ipv4_of(r.ipv4)?,
            created_at: r.created_at,
        })
    }
}

impl Store {
    /// Enregistre un appareil et lui attribue la plus petite adresse libre du pool.
    pub async fn enroll_device(
        &self,
        user_id: Uuid,
        name: &str,
        public_key: &PublicKey,
        pool: Ipv4Net,
    ) -> Result<Device, StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!("SELECT pg_advisory_xact_lock($1)", ENROLL_LOCK)
            .execute(&mut *tx)
            .await?;
        let limit = sqlx::query!(
            r#"SELECT p.max_devices,
                      (SELECT count(*) FROM devices d WHERE d.user_id = u.id AND d.revoked_at IS NULL) AS "active!"
               FROM users u JOIN plans p ON p.id = u.plan_id WHERE u.id = $1"#,
            user_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        check_device_limit(
            u32::try_from(limit.active).unwrap_or(u32::MAX),
            limit.max_devices.and_then(|m| u32::try_from(m).ok()),
        )?;
        let key = public_key.to_string();
        let taken = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM devices WHERE public_key = $1 AND revoked_at IS NULL) AS "taken!""#,
            key
        )
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            return Err(StoreError::KeyTaken);
        }
        let used = sqlx::query_scalar!("SELECT ipv4 FROM devices WHERE revoked_at IS NULL")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(ipv4_of)
            .collect::<Result<BTreeSet<_>, _>>()?;
        let ip = allocate_ipv4(pool, &used)?;
        let row = sqlx::query_as!(
            Row,
            "INSERT INTO devices (id, user_id, name, public_key, ipv4) VALUES ($1, $2, $3, $4, $5)
             RETURNING id, user_id, name, public_key, ipv4, created_at",
            Uuid::now_v7(),
            user_id,
            name,
            key,
            IpNet::from(Ipv4Net::from(ip))
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e, "devices_active_key") {
                StoreError::KeyTaken
            } else {
                e.into()
            }
        })?;
        tx.commit().await?;
        row.try_into()
    }

    /// Appareils actifs de l'utilisateur, du plus ancien au plus récent.
    pub async fn devices_of(&self, user_id: Uuid) -> Result<Vec<Device>, StoreError> {
        sqlx::query_as!(
            Row,
            "SELECT id, user_id, name, public_key, ipv4, created_at FROM devices
             WHERE user_id = $1 AND revoked_at IS NULL ORDER BY created_at, id",
            user_id
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Device::try_from)
        .collect()
    }

    /// Tous les appareils actifs (administration).
    pub async fn all_devices(&self) -> Result<Vec<Device>, StoreError> {
        sqlx::query_as!(
            Row,
            "SELECT id, user_id, name, public_key, ipv4, created_at FROM devices
             WHERE revoked_at IS NULL ORDER BY created_at, id"
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Device::try_from)
        .collect()
    }

    /// `user_id = None` : révocation par l'administrateur, sans contrôle de propriétaire.
    pub async fn revoke_device(
        &self,
        user_id: Option<Uuid>,
        device_id: Uuid,
    ) -> Result<(), StoreError> {
        let done = sqlx::query!(
            "UPDATE devices SET revoked_at = now()
             WHERE id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR user_id = $2)",
            device_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
