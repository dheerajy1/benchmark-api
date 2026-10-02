use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::{
    discovery,
    metrics::artifact_size,
    model::{BenchmarkRequest, BenchmarkResult, Metrics, Target},
};

pub fn run(request: &BenchmarkRequest) -> Result<BenchmarkResult, String> {
    let target = prepare_target(&request.path)?;
    let run_id = Uuid::now_v7();

    let discovery = discovery::discover(&target)?;
    let artifact_size = artifact_size::measure(&target)?;

    Ok(BenchmarkResult {
        run_id,
        target: Target {
            path: target.display().to_string(),
        },
        discovery,
        metrics: Metrics { artifact_size },
    })
}

fn prepare_target(path: &Path) -> Result<PathBuf, String> {
    let prepared = if path.starts_with("~") {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable is not set".to_string())?;

        let suffix = path.strip_prefix("~").unwrap_or(path);
        Path::new(&home).join(suffix)
    } else {
        path.to_path_buf()
    };

    if !prepared.exists() {
        return Err(format!("Path does not exist: {}", prepared.display()));
    }

    if !prepared.is_dir() {
        return Err(format!("Path is not a directory: {}", prepared.display()));
    }

    Ok(prepared)
}
