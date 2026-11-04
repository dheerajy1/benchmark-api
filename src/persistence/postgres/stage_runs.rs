use sqlx::{PgPool, Row};

use crate::persistence::{
    error::PersistenceError,
    history::{StageDuration, StageRun},
};

// The newest done runs of one target that are not deleted, with their stage rows. The outer
// join keeps a done run that has no stage rows, so it still counts as a run.
const SELECT_STAGE_RUNS: &str = "SELECT recent.id AS run_id, s.stage, s.position, s.duration_ms \
     FROM (SELECT id, created_at FROM benchmark_runs \
           WHERE deleted_at IS NULL AND status = 'done' AND target_path = $1 \
           ORDER BY created_at DESC, id DESC LIMIT $2) recent \
     LEFT JOIN benchmark_run_stages s ON s.benchmark_run_id = recent.id \
     ORDER BY recent.created_at DESC, recent.id DESC, s.position";

pub async fn recent(
    pool: &PgPool,
    target_path: &str,
    limit: i64,
) -> Result<Vec<StageRun>, PersistenceError> {
    let rows = sqlx::query(SELECT_STAGE_RUNS)
        .bind(target_path)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    let mut runs: Vec<StageRun> = Vec::new();

    for row in &rows {
        let run_id = row.try_get("run_id")?;

        if runs.last().is_none_or(|run| run.run_id != run_id) {
            runs.push(StageRun {
                run_id,
                stages: Vec::new(),
            });
        }

        let stage: Option<String> = row.try_get("stage")?;

        if let (Some(stage), Some(run)) = (stage, runs.last_mut()) {
            run.stages.push(StageDuration {
                stage,
                position: row.try_get("position")?,
                duration_ms: row.try_get("duration_ms")?,
            });
        }
    }

    Ok(runs)
}

pub async fn ping(pool: &PgPool) -> Result<(), PersistenceError> {
    sqlx::query("SELECT 1").execute(pool).await?;

    Ok(())
}
