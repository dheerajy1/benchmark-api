use sqlx::PgPool;

use crate::{
    features::v1::benchmark::model::BenchmarkResult,
    persistence::{
        error::PersistenceError,
        repository::{BenchmarkRepository, PersistFuture},
    },
};

use super::{insert, mapping};

pub struct PgBenchmarkRepository {
    pool: PgPool,
}

impl PgBenchmarkRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn insert(&self, result: &BenchmarkResult) -> Result<(), PersistenceError> {
        // Mapping fails before any connection is used, so nothing is half written
        let rows = mapping::map_result(result)?;

        let mut transaction = self.pool.begin().await?;

        // Any error returns here and the dropped transaction rolls everything back
        insert::insert_all(&mut transaction, &rows).await?;

        transaction.commit().await?;

        Ok(())
    }
}

impl BenchmarkRepository for PgBenchmarkRepository {
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a> {
        Box::pin(self.insert(result))
    }
}
