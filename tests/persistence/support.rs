use benchmark_api::persistence::postgres::connection::MIGRATOR;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

// A throwaway schema inside TEST_DATABASE_URL, migrated from zero for every test
pub struct TestDatabase {
    pub pool: PgPool,
    admin: PgPool,
    schema: String,
}

// None when TEST_DATABASE_URL is not set, so the test is skipped
pub async fn setup() -> Option<TestDatabase> {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("skipped: TEST_DATABASE_URL is not set");

        return None;
    };

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("TEST_DATABASE_URL must be reachable");

    let schema = format!("test_{}", Uuid::now_v7().simple());

    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .expect("the test schema must be created");

    let search_path = schema.clone();

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |connection, _metadata| {
            let statement = format!("SET search_path TO {search_path}");

            Box::pin(async move {
                connection.execute(statement.as_str()).await?;

                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("the test pool must connect");

    MIGRATOR
        .run(&pool)
        .await
        .expect("migrations must apply to an empty schema");

    Some(TestDatabase {
        pool,
        admin,
        schema,
    })
}

impl TestDatabase {
    // Drops the schema, a test that panics earlier leaves its schema behind
    pub async fn teardown(self) {
        self.pool.close().await;

        self.admin
            .execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str())
            .await
            .expect("the test schema must be dropped");
    }
}

// SQLSTATE of a database error, for example 23505 for a unique violation
pub fn sql_state(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|database_error| database_error.code())
        .map(|code| code.into_owned())
        .unwrap_or_default()
}

// Inserts a bare run row and returns its id
pub async fn insert_bare_run(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();

    sqlx::query("INSERT INTO benchmark_runs (id, target_path, created_at) VALUES ($1, $2, now())")
        .bind(id)
        .bind("/tmp/bare")
        .execute(pool)
        .await
        .expect("a bare run must insert");

    id
}

pub async fn count_for_run(pool: &PgPool, table: &str, run_id: Uuid) -> i64 {
    let column = if table == "benchmark_runs" {
        "id"
    } else {
        "benchmark_run_id"
    };

    let sql = format!("SELECT count(*) FROM {table} WHERE {column} = $1");

    sqlx::query_scalar::<_, i64>(&sql)
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}
