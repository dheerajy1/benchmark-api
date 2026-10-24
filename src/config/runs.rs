use crate::schemas::runs::RunsSchema;

// Kept apart from Env so the existing application settings stay untouched
pub type RunsConfig = RunsSchema;

impl RunsConfig {
    pub fn load() -> Result<Self, String> {
        envy::from_env::<Self>().map_err(|error| error.to_string())
    }
}
