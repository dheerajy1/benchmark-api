use sqlx::PgPool;
use uuid::Uuid;

use crate::persistence::error::PersistenceError;

// Marks the run as deleted and keeps every part row, so a restore is one update. A run that is
// already deleted keeps its first deletion time. False only when no such run was ever stored.
const SOFT_DELETE: &str =
    "UPDATE benchmark_runs SET deleted_at = COALESCE(deleted_at, now()) WHERE id = $1";

pub async fn delete(pool: &PgPool, run_id: Uuid) -> Result<bool, PersistenceError> {
    let result = sqlx::query(SOFT_DELETE).bind(run_id).execute(pool).await?;

    Ok(result.rows_affected() > 0)
}
