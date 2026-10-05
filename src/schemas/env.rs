use serde::Deserialize;
use validator::Validate;

#[derive(Clone, Debug, Deserialize, Validate)]
pub struct EnvSchema {
    // Port the API listens on
    #[validate(range(min = 1, message = "APP_PORT must be a valid port number"))]
    pub app_port: u16,

    // Health check client identifier
    #[validate(length(
        min = 1,
        max = 50,
        message = "APP_HEALTH_CLIENT_ID must be 1-50 characters"
    ))]
    pub app_health_client_id: String,

    // Health check client secret
    #[validate(length(
        min = 1,
        max = 50,
        message = "APP_HEALTH_CLIENT_SECRET must be 1-50 characters"
    ))]
    pub app_health_client_secret: String,
}
