use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::features::v1::benchmark::{model::BuildCommand, validation};

use super::{
    file::{HeaderFile, ReadinessFile, StartupFile, TargetFile, TargetsFile, WorkloadFile},
    resolve::build_request,
    secret::SecretValue,
};

// One saved target with every value already resolved from the environment
#[derive(Debug, Clone)]
pub struct TargetEntry {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub startup: Option<StartupSpec>,
    pub workload: Option<WorkloadSpec>,
}

#[derive(Debug, Clone)]
pub struct StartupSpec {
    pub program: String,
    pub args: Vec<String>,
    pub host: String,
    pub port: u16,
    pub readiness_path: String,
    pub method: String,
    pub expected_status: u16,
    // None means the server's health credentials are sent, as for a raw request
    pub readiness_headers: Option<Vec<(String, SecretValue)>>,
}

#[derive(Debug, Clone)]
pub struct WorkloadSpec {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, SecretValue)>,
    pub requests: Option<u64>,
    pub duration_seconds: Option<u64>,
    pub concurrency: u64,
}

// Every saved target, keyed by id, read once at startup
#[derive(Debug, Clone, Default)]
pub struct TargetCatalog {
    entries: BTreeMap<String, TargetEntry>,
}

impl TargetCatalog {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&TargetEntry> {
        self.entries.get(id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &TargetEntry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Reads and checks the file. Errors name a target and a field, never a value.
    pub fn load(path: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!(
                "Could not read the targets file {}: {error}",
                path.display()
            )
        })?;

        // Only the position is reported, because a serde message may quote a value
        let file: TargetsFile = serde_json::from_str(&text).map_err(|error| {
            format!(
                "The targets file does not match the expected format (line {}, column {}).",
                error.line(),
                error.column()
            )
        })?;

        let mut entries = BTreeMap::new();

        for (id, target) in file.targets {
            if !valid_target_id(&id) {
                return Err(format!(
                    "Target id \"{id}\" is not valid: use 1 to 63 lowercase letters, digits or hyphens."
                ));
            }

            let entry = build_entry(&id, target, lookup)?;
            entries.insert(id, entry);
        }

        Ok(Self { entries })
    }
}

// Matches [a-z0-9][a-z0-9-]{0,62}
pub fn valid_target_id(id: &str) -> bool {
    let bytes = id.as_bytes();

    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn build_entry(
    id: &str,
    file: TargetFile,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<TargetEntry, String> {
    let name = file.name.trim().to_string();

    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(format!(
            "targets.{id}.name must be 1 to 100 characters without control characters."
        ));
    }

    if !file.path.is_absolute() {
        return Err(format!("targets.{id}.path must be an absolute path."));
    }

    let startup = match file.startup {
        Some(startup) => Some(build_startup(id, startup, lookup)?),
        None => None,
    };

    let workload = match file.workload {
        Some(workload) => Some(build_workload(id, workload, lookup)?),
        None => None,
    };

    let entry = TargetEntry {
        id: id.to_string(),
        name,
        path: file.path,
        startup,
        workload,
    };

    // The same checks a run applies, without overrides, so a broken target fails at startup
    let request = build_request(&entry, None);

    validation::validate(&request).map_err(|message| format!("targets.{id}: {message}"))?;

    Ok(entry)
}

fn build_startup(
    id: &str,
    startup: StartupFile,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<StartupSpec, String> {
    let readiness: ReadinessFile = startup.readiness;

    let readiness_headers = match readiness.headers {
        Some(headers) => Some(resolve_headers(
            &format!("targets.{id}.startup.readiness.headers"),
            headers,
            lookup,
        )?),
        None => None,
    };

    let command: BuildCommand = BuildCommand {
        program: startup.command.program,
        args: startup.command.args,
    };

    Ok(StartupSpec {
        program: command.program,
        args: command.args,
        host: readiness.host,
        port: readiness.port,
        readiness_path: readiness.path,
        method: readiness.method,
        expected_status: readiness.expected_status,
        readiness_headers,
    })
}

fn build_workload(
    id: &str,
    workload: WorkloadFile,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<WorkloadSpec, String> {
    match (workload.requests, workload.duration_seconds) {
        (Some(_), None) | (None, Some(_)) => {}
        _ => {
            return Err(format!(
                "targets.{id}.workload must set exactly one of requests or duration_seconds."
            ));
        }
    }

    let headers = resolve_headers(
        &format!("targets.{id}.workload.headers"),
        workload.headers,
        lookup,
    )?;

    Ok(WorkloadSpec {
        method: workload.method,
        path: workload.path,
        headers,
        requests: workload.requests,
        duration_seconds: workload.duration_seconds,
        concurrency: workload.concurrency,
    })
}

// Literal values are kept as they are; references are read from the environment once.
// A missing variable is reported by its name only.
fn resolve_headers(
    field: &str,
    headers: BTreeMap<String, HeaderFile>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<(String, SecretValue)>, String> {
    let mut resolved = Vec::with_capacity(headers.len());

    for (name, value) in headers {
        let text = match value {
            HeaderFile::Literal(text) => text,
            HeaderFile::Env(reference) => lookup(&reference.env).ok_or_else(|| {
                format!(
                    "{field}.{name} reads the environment variable {}, which is not set.",
                    reference.env
                )
            })?,
        };

        resolved.push((name, SecretValue::new(text)));
    }

    Ok(resolved)
}
