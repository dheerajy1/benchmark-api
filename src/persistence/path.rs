// The one place that decides how a project path is spelled. A run stores it this way, a
// filter reads it this way and a saved target reports it this way, so `/a/b` and `/a/b/`
// are always the same project.
pub fn trim_trailing_slashes(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');

    // The root folder stays "/"
    if trimmed.is_empty() && !path.is_empty() {
        "/"
    } else {
        trimmed
    }
}

// The name shown for a project: the saved target name, otherwise the last folder of the path
pub fn display_name(target_name: Option<&str>, target_path: &str) -> String {
    if let Some(name) = target_name
        && !name.is_empty()
    {
        return name.to_string();
    }

    let trimmed = trim_trailing_slashes(target_path);

    match trimmed.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => trimmed.to_string(),
    }
}
