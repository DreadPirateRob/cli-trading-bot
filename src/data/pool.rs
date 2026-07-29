//! Database connection pool with embedded migrations

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tracing::info;

use crate::config::DatabaseConfig;

/// Create a PostgreSQL connection pool and run migrations
pub async fn create_pool(config: &DatabaseConfig) -> Result<PgPool, sqlx::Error> {
    info!(
        max_connections = config.max_connections,
        min_connections = config.min_connections,
        "Creating database connection pool"
    );

    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs as u64))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect(&config.url)
        .await?;

    info!("Running database migrations");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    info!("Database pool ready");
    Ok(pool)
}
