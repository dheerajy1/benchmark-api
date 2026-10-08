use std::time::Duration;

use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};

use crate::{config::database::DatabaseConfig, persistence::error::PersistenceError};

const MAX_CONNECTIONS: u32 = 5;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

// Migrations are committed under ./migrations and embedded into the binary
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, PersistenceError> {
    PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect(&config.database_url)
        .await
        .map_err(PersistenceError::from)
}

// Applies every pending migration, already applied ones are skipped
pub async fn migrate(pool: &PgPool) -> Result<(), PersistenceError> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|error| PersistenceError::Migration(error.to_string()))
}
