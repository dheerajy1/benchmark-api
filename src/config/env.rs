use validator::Validate;

use crate::schemas::env::EnvSchema;

// Keeps `Env` working everywhere it is already used
pub type Env = EnvSchema;

impl Env {
    pub fn load() -> Result<Self, String> {
        // Reads APP_PORT -> app_port, etc.
        let env = envy::from_env::<Self>().map_err(|error| error.to_string())?;

        env.validate().map_err(|error| error.to_string())?;

        Ok(env)
    }
}
