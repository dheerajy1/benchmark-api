use std::path::Path;

use crate::config::runs::RunsConfig;

use super::catalog::TargetCatalog;

// Whether raw requests are accepted, and which saved targets exist
#[derive(Debug, Clone)]
pub struct RunPolicy {
    custom_runs_allowed: bool,
    catalog: TargetCatalog,
}

impl RunPolicy {
    pub fn new(custom_runs_allowed: bool, catalog: TargetCatalog) -> Self {
        Self {
            custom_runs_allowed,
            catalog,
        }
    }

    // Raw requests allowed and no saved targets, the behaviour before this feature existed.
    // The server binary reads its settings instead, so production is closed by default.
    pub fn open() -> Self {
        Self::new(true, TargetCatalog::empty())
    }

    pub fn from_settings(
        settings: &RunsConfig,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let catalog = match configured_file(settings) {
            Some(file) => TargetCatalog::load(Path::new(file), lookup)?,
            None => TargetCatalog::empty(),
        };

        Ok(Self::new(settings.allow_custom_runs, catalog))
    }

    pub fn custom_runs_allowed(&self) -> bool {
        self.custom_runs_allowed
    }

    pub fn catalog(&self) -> &TargetCatalog {
        &self.catalog
    }
}

// The targets file path when one is configured
pub fn configured_file(settings: &RunsConfig) -> Option<&str> {
    settings
        .targets_file
        .as_deref()
        .map(str::trim)
        .filter(|file| !file.is_empty())
}

// A warning when other users can read the targets file, None when it is private
#[cfg(unix)]
pub fn permission_warning(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path).ok()?.permissions().mode();

    if mode & 0o077 != 0 {
        Some(format!(
            "{} is readable by other users, make it readable only by its owner (chmod 600).",
            path.display()
        ))
    } else {
        None
    }
}

#[cfg(not(unix))]
pub fn permission_warning(_path: &Path) -> Option<String> {
    None
}
