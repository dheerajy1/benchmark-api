use serde::Deserialize;

// Settings of saved targets, read from the environment by envy
#[derive(Clone, Debug, Deserialize)]
pub struct RunsSchema {
    // Raw requests are refused unless this is explicitly true, so production is closed by default
    #[serde(default)]
    pub allow_custom_runs: bool,
    // Path of the JSON targets file, unset or empty means there are no saved targets
    #[serde(default)]
    pub targets_file: Option<String>,
}
