use serde::Deserialize;
use validator::Validate;

// Settings of saved targets, read from the environment by envy
#[derive(Clone, Debug, Deserialize, Validate)]
pub struct RunsSchema {
    // Raw requests are refused unless this is explicitly true, so production is closed by default
    #[serde(default)]
    pub allow_custom_runs: bool,
    // Path of the JSON targets file, unset or empty means there are no saved targets
    #[serde(default)]
    #[validate(length(max = 4096, message = "TARGETS_FILE must be at most 4096 characters"))]
    pub targets_file: Option<String>,
}
