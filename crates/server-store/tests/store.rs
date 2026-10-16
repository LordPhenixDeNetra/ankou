#![allow(clippy::unwrap_used)]

use ankou_server_store::Store;
use sqlx::PgPool;

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
