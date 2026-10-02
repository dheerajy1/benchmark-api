use std::path::Path;

use super::model::Discovery;

pub fn discover(path: &Path) -> Result<Discovery, String> {
    let bun = path.join("bun.lock").is_file() || path.join("bun.lockb").is_file();

    let mut candidates = Vec::new();

    if bun {
        candidates.push("bun");
    } else if path.join("package.json").is_file() {
        candidates.push("node");
    }

    if path.join("pyproject.toml").is_file() {
        candidates.push("python");
    }

    if path.join("Cargo.toml").is_file() {
        candidates.push("rust");
    }

    if path.join("go.mod").is_file() {
        candidates.push("go");
    }

    match candidates.as_slice() {
        [] => Ok(Discovery {
            runtime: None,
            build: None,
        }),
        [runtime] => Ok(Discovery {
            runtime: Some((*runtime).to_string()),
            build: None,
        }),
        _ => Err(format!(
            "Multiple runtime candidates detected: {}",
            candidates.join(", ")
        )),
    }
}
