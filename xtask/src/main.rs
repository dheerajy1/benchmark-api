mod quiet;

use std::env;
use std::process::{Command, exit};

use benchmark_api::config::env::Env;

use quiet::Show;

fn main() {
    let task = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cargo <dev|start|build|verify>");
        exit(1);
    });

    match task.as_str() {
        "dev" => run_server("development", false),
        "start" => run_server("production", true),
        "build" => run_build(),
        "verify" => run_verify(),
        _ => {
            eprintln!("Unknown task: {task}");
            exit(1);
        }
    }
}

fn load_env(profile: &str) -> Env {
    let env_file = format!(".env.{profile}");

    dotenvy::from_filename(&env_file)
        .unwrap_or_else(|_| panic!("Failed to load environment file: {env_file}"));

    Env::load().unwrap_or_else(|error| {
        eprintln!("[env] Validation failed for {env_file}: {error}");
        exit(1);
    })
}

fn run_server(profile: &str, release: bool) {
    let env = load_env(profile);

    println!(
        "-> Loaded .env.{profile} (Port: {}, Mode: {profile})",
        env.app_port
    );

    build_quiet(&package_args("build", "benchmark-api", release));

    let mut cmd = Command::new("cargo");
    cmd.args(package_args("run", "benchmark-api", release));
    cmd.arg("--quiet");

    let status = cmd
        .status()
        .expect("Failed to execute cargo run for benchmark-api");

    exit(status.code().unwrap_or(1));
}

fn run_build() {
    load_env("production");

    println!("-> Validated .env.production");

    build_quiet(&package_args("build", "benchmark-api", true));
}

fn package_args(command: &str, package: &str, release: bool) -> Vec<String> {
    let mut args = vec![
        command.to_string(),
        "--package".to_string(),
        package.to_string(),
    ];

    if release {
        args.push("--release".to_string());
    }

    args
}

fn run_step(args: &[String], show: Show) {
    let mut cmd = Command::new("cargo");
    cmd.args(args);

    match quiet::run(&cmd, show) {
        Ok(true) => {}
        Ok(false) => exit(1),
        Err(error) => {
            eprintln!("Failed to run cargo: {error}");
            exit(1);
        }
    }
}

fn build_quiet(args: &[String]) {
    run_step(args, Show::All);
}

fn run_verify() {
    let steps: [(&str, Vec<&str>, Show); 4] = [
        ("cargo fmt", vec!["fmt"], Show::All),
        (
            "cargo check",
            vec!["check", "--workspace", "--all-targets"],
            Show::All,
        ),
        (
            "cargo clippy",
            vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            Show::All,
        ),
        ("cargo test", vec!["test", "--workspace"], Show::TestSummary),
    ];

    for (label, args, show) in steps {
        println!("-> {label}");

        let args: Vec<String> = args.into_iter().map(str::to_string).collect();

        run_step(&args, show);
    }

    println!("-> verify passed");
}
