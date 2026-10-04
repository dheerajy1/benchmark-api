use axum::Router;
use benchmark_api::{config::env::Env, features::v1};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let env = Env::load().unwrap_or_else(|error| {
        eprintln!("[env] Environment validation failed: {error}");
        std::process::exit(1);
    });

    let app = Router::new().merge(v1::router()).with_state(env.clone());

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", env.app_port))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
