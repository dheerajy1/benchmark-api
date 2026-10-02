use axum::Router;
use benchmark_api::features::v1;

#[tokio::main]
async fn main() {
    let app = Router::new().merge(v1::router());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:5000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
