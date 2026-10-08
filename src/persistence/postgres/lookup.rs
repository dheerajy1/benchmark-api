use sqlx::PgConnection;
use uuid::Uuid;

use crate::persistence::error::PersistenceError;

// Lookup rows are always resolved by their stable code, never by a hard-coded id
#[derive(Debug, Clone, Copy)]
pub enum Lookup {
    Runtime,
    ArtifactType,
    Status,
}

impl Lookup {
    fn table(self) -> &'static str {
        match self {
            Lookup::Runtime => "runtimes",
            Lookup::ArtifactType => "artifact_types",
            Lookup::Status => "benchmark_statuses",
        }
    }

    fn query(self) -> &'static str {
        match self {
            Lookup::Runtime => "SELECT id FROM runtimes WHERE code = $1",
            Lookup::ArtifactType => "SELECT id FROM artifact_types WHERE code = $1",
            Lookup::Status => "SELECT id FROM benchmark_statuses WHERE code = $1",
        }
    }
}

pub async fn lookup_id(
    connection: &mut PgConnection,
    lookup: Lookup,
    code: &str,
) -> Result<Uuid, PersistenceError> {
    let id = sqlx::query_scalar::<_, Uuid>(lookup.query())
        .bind(code)
        .fetch_optional(&mut *connection)
        .await?;

    id.ok_or_else(|| PersistenceError::UnknownLookup {
        table: lookup.table(),
        code: code.to_string(),
    })
}
