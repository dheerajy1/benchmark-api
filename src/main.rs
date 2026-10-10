use std::sync::Arc;

use benchmark_api::{
    app,
    config::{database::DatabaseConfig, env::Env},
    persistence::{
        postgres::{connection, repository::PgBenchmarkRepository},
        repository::BenchmarkRepository,
    },
};

#[tokio::main]
async fn main() {
    // dotenvy::dotenv().ok();
    // Load .env if present; fallback to .env.development for direct cargo run
    if dotenvy::dotenv().is_err() {
        let _ = dotenvy::from_filename(".env.development");
    }

    let env = Env::load().unwrap_or_else(|error| {
        eprintln!("[env] Environment validation failed: {error}");
        std::process::exit(1);
    });

    let database = DatabaseConfig::load().unwrap_or_else(|error| {
        eprintln!("[env] Database configuration failed: {error}");
        std::process::exit(1);
    });

    let pool = connection::connect(&database)
        .await
        .unwrap_or_else(|error| {
            eprintln!("[database] Connection failed: {error}");
            std::process::exit(1);
        });

    // Migrations run exactly once, here, before the application accepts requests
    connection::migrate(&pool).await.unwrap_or_else(|error| {
        eprintln!("[database] Migration failed: {error}");
        std::process::exit(1);
    });

    let repository: Arc<dyn BenchmarkRepository> = Arc::new(PgBenchmarkRepository::new(pool));

    let app = app::router(env.clone(), repository);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", env.app_port))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
