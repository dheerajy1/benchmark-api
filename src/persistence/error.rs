use std::fmt;

// Failures of the persistence layer, independent of the database driver
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceError {
    // The database rejected or failed an operation
    Database(String),
    // Applying the migrations failed
    Migration(String),
    // A lookup row (runtime, artifact type, status) is not seeded
    UnknownLookup { table: &'static str, code: String },
    // The benchmark result cannot be represented in the schema
    InvalidResult(String),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PersistenceError::Database(message) => write!(formatter, "database error: {message}"),
            PersistenceError::Migration(message) => {
                write!(formatter, "migration error: {message}")
            }
            PersistenceError::UnknownLookup { table, code } => {
                write!(formatter, "no row with code '{code}' in {table}")
            }
            PersistenceError::InvalidResult(message) => {
                write!(formatter, "invalid benchmark result: {message}")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

impl From<sqlx::Error> for PersistenceError {
    fn from(error: sqlx::Error) -> Self {
        PersistenceError::Database(error.to_string())
    }
}
