use std::{path::Path, sync::Arc};

use benchmark_api::{
    app,
    config::{database::DatabaseConfig, env::Env, runs::RunsConfig},
    features::v1::{
        benchmark::live::{LiveRuns, RunSlot},
        targets::policy::{RunPolicy, configured_file, permission_warning},
    },
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

    // Custom runs are off unless ALLOW_CUSTOM_RUNS is explicitly true
    let runs = RunsConfig::load().unwrap_or_else(|error| {
        eprintln!("[runs] Settings failed: {error}");
        std::process::exit(1);
    });

    let lookup = |name: &str| std::env::var(name).ok();

    let policy = RunPolicy::from_settings(&runs, &lookup).unwrap_or_else(|error| {
        eprintln!("[targets] {error}");
        std::process::exit(1);
    });

    if let Some(file) = configured_file(&runs) {
        if let Some(warning) = permission_warning(Path::new(file)) {
            eprintln!("[targets] {warning}");
        }
    }

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

    println!(
        "[runs] custom runs: {}",
        if policy.custom_runs_allowed() {
            "enabled"
        } else {
            "disabled (ALLOW_CUSTOM_RUNS is not true)"
        }
    );
    println!("[targets] saved targets loaded: {}", policy.catalog().len());

    let repository: Arc<dyn BenchmarkRepository> = Arc::new(PgBenchmarkRepository::new(pool));

    let live = LiveRuns::new(env.clone(), Arc::clone(&repository), RunSlot::new());

    let app = app::router_with_policy(env.clone(), repository, live, policy);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", env.app_port))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
