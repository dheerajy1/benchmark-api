use std::fmt;

use serde::Deserialize;
use validator::Validate;

#[derive(Clone, Deserialize, Validate)]
pub struct DatabaseSchema {
    // PostgreSQL connection string, read from DATABASE_URL
    #[validate(length(min = 1, message = "DATABASE_URL must not be empty"))]
    pub database_url: String,
}

// The connection string contains the password, so it is never printed
impl fmt::Debug for DatabaseSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseSchema")
            .field("database_url", &"<redacted>")
            .finish()
    }
}
