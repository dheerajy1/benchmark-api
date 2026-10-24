use std::path::Path;

use validator::Validate;

use crate::schemas::runs::RunsSchema;

// Kept separate from Env so the existing application settings stay untouched
pub type RunsConfig = RunsSchema;

impl RunsConfig {
    pub fn load() -> Result<Self, String> {
        // Reads ALLOW_CUSTOM_RUNS -> allow_custom_runs and TARGETS_FILE -> targets_file
        let config = envy::from_env::<Self>().map_err(|error| error.to_string())?;

        config.validate().map_err(|error| error.to_string())?;
        config.check_targets_file()?;

        Ok(config)
    }

    // A configured targets file must be an absolute path. Unset or empty is allowed.
    pub fn check_targets_file(&self) -> Result<(), String> {
        match self
            .targets_file
            .as_deref()
            .map(str::trim)
            .filter(|file| !file.is_empty())
        {
            Some(file) if !Path::new(file).is_absolute() => {
                Err("TARGETS_FILE must be an absolute path.".to_string())
            }
            _ => Ok(()),
        }
    }
}
