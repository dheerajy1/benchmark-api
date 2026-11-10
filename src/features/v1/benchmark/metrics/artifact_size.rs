use std::path::{Component, Path, PathBuf};

use walkdir::WalkDir;

use super::super::model::{ArtifactSize, ArtifactSizeCategory, BuildRequest};

const DEFAULT_OUTPUT_DIR: &str = "dist";
const RUST_OUTPUT_DIR: &str = "target";

// The directory to measure after the build, relative to the project: the one the caller gave,
// otherwise target for Rust and dist for every other runtime
pub fn output_dir(build: &BuildRequest, runtime: Option<&str>) -> String {
    match build
        .output_dir
        .as_deref()
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
    {
        Some(dir) => dir.to_string(),
        None if runtime == Some("rust") => RUST_OUTPUT_DIR.to_string(),
        None => DEFAULT_OUTPUT_DIR.to_string(),
    }
}

// Same path without "." parts, so it can be compared with the paths found while walking
fn normalized(path: &str) -> PathBuf {
    Path::new(path)
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

// Measures the project after its build. Files inside the build output directory are the build
// artifact, node_modules and other build folders are excluded, everything else is source.
pub fn measure(path: &Path, output_dir: &str) -> Result<ArtifactSize, String> {
    let output = normalized(output_dir);

    if !path.join(&output).is_dir() {
        return Err(format!(
            "The build output directory was not found after the build: {output_dir}"
        ));
    }

    let mut source_bytes = 0;
    let mut source_file_count = 0;

    let mut build_bytes = 0;
    let mut build_file_count = 0;

    let mut excluded_bytes = 0;
    let mut excluded_file_count = 0;

    for entry in WalkDir::new(path).into_iter() {
        let entry = entry.map_err(|error| error.to_string())?;

        if !entry.file_type().is_file() {
            continue;
        }

        let size = entry.metadata().map_err(|error| error.to_string())?.len();

        let relative = entry.path().strip_prefix(path).unwrap_or(entry.path());

        let is_build = relative.starts_with(&output);

        let is_excluded = relative.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("node_modules") | Some("target") | Some("dist") | Some("build")
            )
        });

        if is_build {
            build_bytes += size;
            build_file_count += 1;
        } else if is_excluded {
            excluded_bytes += size;
            excluded_file_count += 1;
        } else {
            source_bytes += size;
            source_file_count += 1;
        }
    }

    Ok(ArtifactSize {
        source: ArtifactSizeCategory {
            bytes: source_bytes,
            size: format_size(source_bytes),
            file_count: source_file_count,
        },
        build: ArtifactSizeCategory {
            bytes: build_bytes,
            size: format_size(build_bytes),
            file_count: build_file_count,
        },
        excluded: ArtifactSizeCategory {
            bytes: excluded_bytes,
            size: format_size(excluded_bytes),
            file_count: excluded_file_count,
        },
    })
}

pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes = bytes as f64;

    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes as u64)
    }
}
