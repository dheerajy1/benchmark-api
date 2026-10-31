use std::{collections::BTreeSet, path::Component, path::Path};

use super::{
    command,
    config::{ENV_FILE_PLACEHOLDER, WorkspaceEnv, WorkspaceRequest},
    env_file,
};

const MAX_WORKSPACES: usize = 8;
const MAX_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
const MAX_IDLE_SECONDS: u64 = 60 * 60;

// Rejects an invalid workspace list before anything is built or started
pub fn validate_all(workspaces: &[WorkspaceRequest]) -> Result<(), String> {
    if workspaces.len() > MAX_WORKSPACES {
        return Err(format!(
            "workspaces may hold at most {MAX_WORKSPACES} entries"
        ));
    }

    let mut names = BTreeSet::new();

    for (index, workspace) in workspaces.iter().enumerate() {
        validate(workspace).map_err(|message| format!("workspaces[{index}]: {message}"))?;

        if !names.insert(workspace.name.as_str()) {
            return Err(format!(
                "workspaces[{index}]: name '{}' is used twice",
                workspace.name
            ));
        }
    }

    Ok(())
}

pub fn validate(workspace: &WorkspaceRequest) -> Result<(), String> {
    validate_name(&workspace.name)?;
    validate_directory(&workspace.directory)?;

    if !workspace.native_enabled() && !workspace.docker_enabled() {
        return Err("enable native or docker, a workspace needs at least one".to_string());
    }

    if workspace.ready_log_line.trim().is_empty() {
        return Err("ready_log_line is required when native or docker is enabled".to_string());
    }

    if !(1..=MAX_TIMEOUT_SECONDS).contains(&workspace.ready_timeout_seconds) {
        return Err(format!(
            "ready_timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        ));
    }

    if !(1..=MAX_IDLE_SECONDS).contains(&workspace.idle_seconds) {
        return Err(format!(
            "idle_seconds must be between 1 and {MAX_IDLE_SECONDS}"
        ));
    }

    if let Some(load) = &workspace.load {
        command::split(&load.command).map_err(|message| format!("load.command: {message}"))?;
    }

    let has_env = validate_env(workspace.env.as_ref())?;

    if let Some(native) = workspace.native.as_ref().filter(|native| native.enabled) {
        command::split(&native.command).map_err(|message| format!("native.command: {message}"))?;

        if let Some(build) = &native.build_command {
            command::split(build).map_err(|message| format!("native.build_command: {message}"))?;
        }
    }

    if let Some(docker) = workspace.docker.as_ref().filter(|docker| docker.enabled) {
        command::split(&docker.build_command)
            .map_err(|message| format!("docker.build_command: {message}"))?;

        if !(1..=MAX_TIMEOUT_SECONDS).contains(&docker.build_timeout_seconds) {
            return Err(format!(
                "docker.build_timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
            ));
        }

        validate_image(&docker.image)?;
        validate_container_name(&docker.container_name)?;

        command::split(&docker.run_command)
            .map_err(|message| format!("docker.run_command: {message}"))?;

        let has_placeholder = docker.run_command.contains(ENV_FILE_PLACEHOLDER);

        if has_env && !has_placeholder {
            return Err(format!(
                "docker.run_command must contain {ENV_FILE_PLACEHOLDER} when env is given"
            ));
        }

        if !has_env && has_placeholder {
            return Err(format!(
                "docker.run_command contains {ENV_FILE_PLACEHOLDER} but no env is given"
            ));
        }
    }

    Ok(())
}

// Answers whether an env was given. Exactly one of the two sources may be set, or neither.
fn validate_env(env: Option<&WorkspaceEnv>) -> Result<bool, String> {
    let Some(env) = env else {
        return Ok(false);
    };

    match (&env.file_path, &env.pasted) {
        (None, None) => Ok(false),
        (Some(_), Some(_)) => Err("env: set either file_path or pasted, not both".to_string()),
        (Some(path), None) => {
            if path.trim().is_empty() {
                return Err("env.file_path must not be empty".to_string());
            }

            Ok(true)
        }
        (None, Some(text)) => {
            // The parser's message holds a line number and a problem, never a value
            env_file::parse(text).map_err(|message| format!("env.pasted: {message}"))?;

            Ok(true)
        }
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });

    if valid {
        Ok(())
    } else {
        Err("name must be 1 to 64 characters of letters, digits, '-', '_' or '.'".to_string())
    }
}

// Relative to the project path and never leaving it
fn validate_directory(directory: &str) -> Result<(), String> {
    let path = Path::new(directory);

    if directory.trim().is_empty() {
        return Err("directory must not be empty".to_string());
    }

    let leaves_project = path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)));

    if leaves_project {
        return Err(
            "directory must be relative to the project path and contain no '..'".to_string(),
        );
    }

    Ok(())
}

fn validate_image(image: &str) -> Result<(), String> {
    let valid = !image.is_empty()
        && !image.starts_with('-')
        && image.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '@')
        });

    if valid {
        Ok(())
    } else {
        Err("docker.image must be an image name such as name:tag, without spaces".to_string())
    }
}

// A container name is passed to docker as an argument, so a leading '-' is refused
fn validate_container_name(name: &str) -> Result<(), String> {
    let mut characters = name.chars();

    let valid = matches!(characters.next(), Some(first) if first.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });

    if valid {
        Ok(())
    } else {
        Err("docker.container_name must start with a letter or digit and use letters, digits, '-', '_' or '.'".to_string())
    }
}
