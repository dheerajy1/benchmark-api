use serde::Deserialize;
use std::env;
use std::process::Command;
use validator::Validate;

#[derive(Clone, Debug, Deserialize, Validate)]
pub struct ApiEnvSchema {
    #[validate(range(min = 1, message = "APP_PORT must be a valid port number"))]
    pub app_port: u16,

    #[validate(length(
        min = 1,
        max = 50,
        message = "APP_HEALTH_CLIENT_ID must be 1-50 characters"
    ))]
    pub app_health_client_id: String,

    #[validate(length(
        min = 1,
        max = 50,
        message = "APP_HEALTH_CLIENT_SECRET must be 1-50 characters"
    ))]
    pub app_health_client_secret: String,

    #[validate(length(min = 1, message = "DATABASE_URL must not be empty"))]
    pub database_url: String,
}

fn main() {
    let task = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cargo <dev|start>");
        std::process::exit(1);
    });

    match task.as_str() {
        "dev" => run_server("development", false),
        "start" => run_server("production", true),
        "build" => run_build(),
        _ => {
            eprintln!("Unknown task: {task}. Use dev | start");
            std::process::exit(1);
        }
    }
}

fn run_server(profile: &str, release: bool) {
    let env_file = format!(".env.{}", profile);

    // 1. Load the corresponding environment file
    dotenvy::from_filename(&env_file)
        .unwrap_or_else(|_| panic!("Failed to load environment file: {}", env_file));

    // 2. Validate environment schema
    let config: ApiEnvSchema = envy::from_env().unwrap_or_else(|err| {
        panic!(
            "Failed to parse environment variables from {}: {}",
            env_file, err
        );
    });

    config.validate().unwrap_or_else(|err| {
        panic!("Environment validation failed for {}: {}", env_file, err);
    });

    println!(
        "-> Loaded {} successfully (Port: {}, Mode: {})",
        env_file,
        config.app_port,
        if release { "production" } else { "development" }
    );

    // 3. Build and run the benchmark-api package
    let mut cmd = Command::new("cargo");
    cmd.arg("run").arg("--package").arg("benchmark-api");

    if release {
        cmd.arg("--release");
    }

    let status = cmd
        .status()
        .expect("Failed to execute cargo run for benchmark-api");

    std::process::exit(status.code().unwrap_or(1));
}

fn run_build() {
    let env_file = ".env.production";
    dotenvy::from_filename(env_file).expect("Failed to load .env.production");
    
    let status = Command::new("cargo")
        .args(["build", "--release", "--package", "benchmark-api"])
        .status()
        .expect("Failed to execute cargo build");

    std::process::exit(status.code().unwrap_or(1));
}