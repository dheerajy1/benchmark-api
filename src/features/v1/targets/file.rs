use std::{collections::BTreeMap, path::PathBuf};

use serde::Deserialize;

// The whole targets file: { "targets": { "<id>": { ... } } }. Deliberately without Debug,
// because header values may be literals.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetsFile {
    pub targets: BTreeMap<String, TargetFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetFile {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub startup: Option<StartupFile>,
    #[serde(default)]
    pub workload: Option<WorkloadFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupFile {
    pub command: CommandFile,
    pub readiness: ReadinessFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandFile {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessFile {
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    pub path: String,
    #[serde(default = "default_method")]
    pub method: String,
    pub expected_status: u16,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, HeaderFile>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadFile {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub requests: Option<u64>,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    pub concurrency: u64,
    #[serde(default)]
    pub headers: BTreeMap<String, HeaderFile>,
}

// A header value: a literal string, or a reference to an environment variable
#[derive(Deserialize)]
#[serde(untagged)]
pub enum HeaderFile {
    Literal(String),
    Env(EnvRef),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvRef {
    pub env: String,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_method() -> String {
    "GET".to_string()
}
