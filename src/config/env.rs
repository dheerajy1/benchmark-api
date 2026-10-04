use std::env;

use crate::schemas::env::EnvSchema;

#[derive(Clone)]
pub struct Env {
    pub app_port: u16,
    pub app_health_client_id: String,
    pub app_health_client_secret: String,
}

impl Env {
    pub fn load() -> Result<Self, String> {
        let app_port = required(EnvSchema::APP_PORT)?
            .parse::<u16>()
            .map_err(|error| format!("APP_PORT must be a valid port number: {error}"))?;

        let app_health_client_id = required(EnvSchema::APP_HEALTH_CLIENT_ID)?;
        let app_health_client_secret = required(EnvSchema::APP_HEALTH_CLIENT_SECRET)?;

        Ok(Self {
            app_port,
            app_health_client_id,
            app_health_client_secret,
        })
    }
}

fn required(key: &str) -> Result<String, String> {
    let value = env::var(key).map_err(|_| format!("{key} is required and cannot be empty"))?;

    let value = value.trim();

    if value.is_empty() {
        return Err(format!("{key} is required and cannot be empty"));
    }

    Ok(value.to_string())
}
