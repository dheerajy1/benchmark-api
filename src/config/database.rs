use validator::Validate;

use crate::schemas::database::DatabaseSchema;

// Kept separate from `Env` so the existing application settings stay untouched
pub type DatabaseConfig = DatabaseSchema;

impl DatabaseConfig {
    pub fn load() -> Result<Self, String> {
        // Reads DATABASE_URL -> database_url
        let config = envy::from_env::<Self>().map_err(|error| error.to_string())?;

        config.validate().map_err(|error| error.to_string())?;

        Ok(config)
    }
}
