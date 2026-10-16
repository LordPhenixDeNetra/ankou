#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_proto::{Endpoint, PrivateKey, PublicKey};
use ankou_server_domain::{DomainError, hash_secret};
use ankou_server_store::{NewServer, Store, StoreError};
use ipnet::Ipv4Net;
use sqlx::PgPool;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use time::{Duration, OffsetDateTime};

fn pool_net() -> Ipv4Net {
    "10.8.0.0/16".parse().unwrap()
}

fn key() -> PublicKey {
    PrivateKey::generate().public_key()
}

async fn invite(store: &Store, code: &str, valid_for: Duration) {
    store
        .create_invitation(
            hash_secret(code),
            None,
            OffsetDateTime::now_utc() + valid_for,
        )
        .await
        .unwrap();
}

async fn user(store: &Store, email: &str) -> uuid::Uuid {
    store
        .register_user(None, email, "hash", OffsetDateTime::now_utc())
        .await
        .unwrap()
        .id
}

#[sqlx::test(migrations = false)]
#[ignore = "PostgreSQL requis"]
async fn migrations_apply_and_seed_plan(pool: PgPool) {
    let _store = Store::from_pool(pool.clone());
    sqlx::migrate!().run(&pool).await.unwrap();
    let max: Option<i32> =
        sqlx::query_scalar("SELECT max_devices FROM plans WHERE name = 'famille'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(max, None);
    sqlx::migrate!().run(&pool).await.unwrap();
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn register_consumes_invitation(pool: PgPool) {
    let store = Store::from_pool(pool);
    invite(&store, "code-1", Duration::days(7)).await;
    let now = OffsetDateTime::now_utc();
    let u = store
        .register_user(Some(hash_secret("code-1")), "a@x.org", "h", now)
        .await
        .unwrap();
    assert_eq!(u.email, "a@x.org");
    assert!(!u.disabled);
    assert_eq!(u.max_devices, None);
    let again = store
        .register_user(Some(hash_secret("code-1")), "b@x.org", "h", now)
        .await;
    assert!(matches!(
        again,
        Err(StoreError::Domain(DomainError::InvitationInvalid))
    ));
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn invitation_is_single_use_under_concurrency(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    invite(&store, "shared", Duration::days(7)).await;
    let now = OffsetDateTime::now_utc();
    let (a, b) = tokio::join!(
        store.register_user(Some(hash_secret("shared")), "a@x.org", "h", now),
        store.register_user(Some(hash_secret("shared")), "b@x.org", "h", now),
    );
    let ok = [a.is_ok(), b.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(ok, 1, "{a:?} / {b:?}");
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn expired_invitation_is_rejected(pool: PgPool) {
    let store = Store::from_pool(pool);
    invite(&store, "old", -Duration::seconds(1)).await;
    let r = store
        .register_user(
            Some(hash_secret("old")),
            "a@x.org",
            "h",
            OffsetDateTime::now_utc(),
        )
        .await;
    assert!(matches!(
        r,
        Err(StoreError::Domain(DomainError::InvitationInvalid))
    ));
    let unknown = store
        .register_user(
            Some(hash_secret("nope")),
            "a@x.org",
            "h",
            OffsetDateTime::now_utc(),
        )
        .await;
    assert!(matches!(
        unknown,
        Err(StoreError::Domain(DomainError::InvitationInvalid))
    ));
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn duplicate_email_is_reported(pool: PgPool) {
    let store = Store::from_pool(pool);
    invite(&store, "c1", Duration::days(1)).await;
    invite(&store, "c2", Duration::days(1)).await;
    let now = OffsetDateTime::now_utc();
    store
        .register_user(Some(hash_secret("c1")), "a@x.org", "h", now)
        .await
        .unwrap();
    let dup = store
        .register_user(Some(hash_secret("c2")), "a@x.org", "h", now)
        .await;
    assert!(matches!(dup, Err(StoreError::EmailTaken)), "{dup:?}");
    store
        .register_user(Some(hash_secret("c2")), "b@x.org", "h", now)
        .await
        .unwrap();
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn session_rotation(pool: PgPool) {
    let store = Store::from_pool(pool);
    let uid = user(&store, "a@x.org").await;
    let now = OffsetDateTime::now_utc();
    let (h1, h2, h3) = (hash_secret("r1"), hash_secret("r2"), hash_secret("r3"));
    store
        .create_session(uid, h1, now + Duration::days(30))
        .await
        .unwrap();
    assert_eq!(
        store
            .rotate_session(h1, h2, now + Duration::days(30), now)
            .await
            .unwrap(),
        uid
    );
    assert!(matches!(
        store
            .rotate_session(h1, h3, now + Duration::days(30), now)
            .await,
        Err(StoreError::NotFound)
    ));
    store
        .create_session(uid, h3, now - Duration::seconds(1))
        .await
        .unwrap();
    assert!(matches!(
        store
            .rotate_session(h3, hash_secret("r4"), now + Duration::days(30), now)
            .await,
        Err(StoreError::NotFound)
    ));
    store.delete_session(h2).await.unwrap();
    assert!(matches!(
        store
            .rotate_session(h2, hash_secret("r5"), now + Duration::days(30), now)
            .await,
        Err(StoreError::NotFound)
    ));
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn enroll_allocates_and_lists(pool: PgPool) {
    let store = Store::from_pool(pool);
    let uid = user(&store, "a@x.org").await;
    let a = store
        .enroll_device(uid, "Laptop", &key(), pool_net())
        .await
        .unwrap();
    let b = store
        .enroll_device(uid, "Phone", &key(), pool_net())
        .await
        .unwrap();
    assert_eq!(a.ipv4, Ipv4Addr::new(10, 8, 0, 2));
    assert_eq!(b.ipv4, Ipv4Addr::new(10, 8, 0, 3));
    let list = store.devices_of(uid).await.unwrap();
    assert_eq!(
        list.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["Laptop", "Phone"]
    );
    assert_eq!(list[0].public_key, a.public_key);
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn concurrent_enrollments_get_distinct_ips(pool: PgPool) {
    let store = Store::from_pool(pool);
    let uid = user(&store, "a@x.org").await;
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .enroll_device(uid, &format!("d{i}"), &key(), pool_net())
                    .await
            })
        })
        .collect();
    let mut ips = BTreeSet::new();
    for t in tasks {
        ips.insert(t.await.unwrap().unwrap().ipv4);
    }
    assert_eq!(ips.len(), 10);
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn duplicate_active_key_is_rejected(pool: PgPool) {
    let store = Store::from_pool(pool);
    let (u1, u2) = (user(&store, "a@x.org").await, user(&store, "b@x.org").await);
    let k = key();
    store.enroll_device(u1, "A", &k, pool_net()).await.unwrap();
    let r = store.enroll_device(u2, "B", &k, pool_net()).await;
    assert!(matches!(r, Err(StoreError::KeyTaken)), "{r:?}");
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn device_limit_from_plan(pool: PgPool) {
    let store = Store::from_pool(pool.clone());
    sqlx::query("UPDATE plans SET max_devices = 1")
        .execute(&pool)
        .await
        .unwrap();
    let uid = user(&store, "a@x.org").await;
    store
        .enroll_device(uid, "A", &key(), pool_net())
        .await
        .unwrap();
    let r = store.enroll_device(uid, "B", &key(), pool_net()).await;
    assert!(
        matches!(r, Err(StoreError::Domain(DomainError::DeviceLimit))),
        "{r:?}"
    );
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn revoke_scopes_to_owner(pool: PgPool) {
    let store = Store::from_pool(pool);
    let (u1, u2) = (user(&store, "a@x.org").await, user(&store, "b@x.org").await);
    let d = store
        .enroll_device(u1, "A", &key(), pool_net())
        .await
        .unwrap();
    assert!(matches!(
        store.revoke_device(Some(u2), d.id).await,
        Err(StoreError::NotFound)
    ));
    store.revoke_device(Some(u1), d.id).await.unwrap();
    assert!(matches!(
        store.revoke_device(Some(u1), d.id).await,
        Err(StoreError::NotFound)
    ));
    assert!(store.devices_of(u1).await.unwrap().is_empty());
    let d2 = store
        .enroll_device(u1, "B", &key(), pool_net())
        .await
        .unwrap();
    store.revoke_device(None, d2.id).await.unwrap();
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn disabling_user_drops_sessions(pool: PgPool) {
    let store = Store::from_pool(pool);
    let uid = user(&store, "a@x.org").await;
    let now = OffsetDateTime::now_utc();
    store
        .create_session(uid, hash_secret("r1"), now + Duration::days(1))
        .await
        .unwrap();
    store.set_user_disabled("a@x.org", true).await.unwrap();
    assert!(matches!(
        store
            .rotate_session(
                hash_secret("r1"),
                hash_secret("r2"),
                now + Duration::days(1),
                now
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(
        store
            .user_by_email("a@x.org")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
    store.set_user_disabled("a@x.org", false).await.unwrap();
    assert!(!store.user_by_id(uid).await.unwrap().unwrap().disabled);
    assert!(matches!(
        store.set_user_disabled("z@x.org", true).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(store.users().await.unwrap().len(), 1);
}

#[sqlx::test]
#[ignore = "PostgreSQL requis"]
async fn servers_roundtrip(pool: PgPool) {
    let store = Store::from_pool(pool);
    let k = key();
    let endpoint: Endpoint = "vps1.example.org:51820".parse().unwrap();
    store
        .add_server(NewServer {
            name: "vps1".into(),
            country: "FR".into(),
            city: "Paris".into(),
            endpoint: endpoint.clone(),
            public_key: k.clone(),
            tags: vec!["p2p".into()],
            node_token_hash: hash_secret("node"),
        })
        .await
        .unwrap();
    let list = store.active_servers().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].endpoint, endpoint);
    assert_eq!(list[0].public_key, k);
    assert_eq!(list[0].tags, ["p2p"]);
    assert!(list[0].active);
    assert_eq!(store.all_servers().await.unwrap().len(), 1);
}

mod nodes {
    use super::*;
    use ankou_proto::node::{NodePeer, PeerStat};
    use ankou_server_store::{PEER_CHANNEL, PeerChange};
    use sqlx::postgres::PgListener;

    async fn next_change(listener: &mut PgListener) -> PeerChange {
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
            .await
            .expect("aucune notification")
            .unwrap();
        PeerChange::from_payload(n.payload()).unwrap()
    }

    #[sqlx::test]
    #[ignore = "PostgreSQL requis"]
    async fn enroll_and_revoke_notify(pool: PgPool) {
        let store = Store::from_pool(pool.clone());
        let mut listener = PgListener::connect_with(&pool).await.unwrap();
        listener.listen(PEER_CHANNEL).await.unwrap();
        let uid = user(&store, "a@x.org").await;
        let k = key();
        let d = store.enroll_device(uid, "A", &k, pool_net()).await.unwrap();
        assert_eq!(
            next_change(&mut listener).await,
            PeerChange::Added(NodePeer {
                public_key: k.clone(),
                ipv4: Ipv4Addr::new(10, 8, 0, 2)
            })
        );
        store.revoke_device(None, d.id).await.unwrap();
        assert_eq!(next_change(&mut listener).await, PeerChange::Removed(k));
    }

    #[sqlx::test]
    #[ignore = "PostgreSQL requis"]
    async fn node_token_lookup(pool: PgPool) {
        let store = Store::from_pool(pool.clone());
        store
            .add_server(NewServer {
                name: "vps1".into(),
                country: "FR".into(),
                city: "Paris".into(),
                endpoint: "vps1.example.org:51820".parse().unwrap(),
                public_key: key(),
                tags: vec![],
                node_token_hash: hash_secret("tok"),
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .server_by_node_token(hash_secret("tok"))
                .await
                .unwrap()
                .unwrap()
                .name,
            "vps1"
        );
        assert!(
            store
                .server_by_node_token(hash_secret("x"))
                .await
                .unwrap()
                .is_none()
        );
        sqlx::query("UPDATE servers SET active = false")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            store
                .server_by_node_token(hash_secret("tok"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[sqlx::test]
    #[ignore = "PostgreSQL requis"]
    async fn active_peers_skip_revoked(pool: PgPool) {
        let store = Store::from_pool(pool);
        let uid = user(&store, "a@x.org").await;
        let a = store
            .enroll_device(uid, "A", &key(), pool_net())
            .await
            .unwrap();
        store
            .enroll_device(uid, "B", &key(), pool_net())
            .await
            .unwrap();
        store.revoke_device(None, a.id).await.unwrap();
        let peers = store.active_peers().await.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].ipv4, Ipv4Addr::new(10, 8, 0, 3));
    }

    #[sqlx::test]
    #[ignore = "PostgreSQL requis"]
    async fn stats_keep_latest_handshake(pool: PgPool) {
        let store = Store::from_pool(pool.clone());
        let uid = user(&store, "a@x.org").await;
        let k = key();
        store.enroll_device(uid, "A", &k, pool_net()).await.unwrap();
        let t2 = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
        let t1 = t2 - Duration::minutes(5);
        let stat = |t| PeerStat {
            public_key: k.clone(),
            last_handshake: Some(t),
            rx_bytes: 1,
            tx_bytes: 2,
        };
        store.record_stats(&[stat(t2)]).await.unwrap();
        store.record_stats(&[stat(t1)]).await.unwrap();
        store
            .record_stats(&[PeerStat {
                public_key: key(),
                last_handshake: Some(t2),
                rx_bytes: 0,
                tx_bytes: 0,
            }])
            .await
            .unwrap();
        let got: Option<OffsetDateTime> = sqlx::query_scalar("SELECT last_handshake FROM devices")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(got, Some(t2));
    }
}
