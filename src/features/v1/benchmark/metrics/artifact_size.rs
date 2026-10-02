use std::path::Path;

use walkdir::WalkDir;

use super::super::model::{ArtifactSize, ArtifactSizeCategory};

pub fn measure(path: &Path) -> Result<ArtifactSize, String> {
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

        let is_build = relative.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("target") | Some("dist") | Some("build")
            )
        });

        let is_excluded = relative
            .components()
            .any(|component| matches!(component.as_os_str().to_str(), Some("node_modules")));

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

fn format_size(bytes: u64) -> String {
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
