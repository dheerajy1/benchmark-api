use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::features::v1::benchmark::{
    docker::{Docker, find_running},
    live::RunContext,
    metrics::memory::MemoryConfig,
    model::BuildCommand,
    stop::Stop,
};

use super::{
    command,
    config::{WorkspaceEnv, WorkspaceRequest},
    container, env_file, mode,
    mode::Shared,
    native,
    result::{MODE_DOCKER, MODE_NATIVE, WorkspaceResult},
};

// Refuses to start when a copy of a workspace is already running, so the numbers are never
// measured next to another instance. Answers what to stop, nothing has been started yet.
pub fn precheck(
    target: &Path,
    workspaces: &[WorkspaceRequest],
    docker: &dyn Docker,
) -> Result<(), String> {
    for workspace in workspaces {
        if workspace.native_enabled()
            && let Some(native) = &workspace.native
            && let Ok(start) = command::split(&native.command)
        {
            let directory = target.join(&workspace.directory);

            if let Some(pid) = running_native(&start, &directory) {
                return Err(format!(
                    "Workspace '{}' is already running natively (process {pid}). Stop it and run the benchmark again.",
                    workspace.name
                ));
            }
        }
    }

    let docker_workspaces: Vec<&WorkspaceRequest> = workspaces
        .iter()
        .filter(|workspace| workspace.docker_enabled())
        .collect();

    if docker_workspaces.is_empty() {
        return Ok(());
    }

    // When Docker cannot be asked, the Docker mode records its own failure later
    let Ok(containers) = docker.running_containers() else {
        return Ok(());
    };

    for workspace in docker_workspaces {
        let Some(request) = &workspace.docker else {
            continue;
        };

        if let Some(running) = find_running(&containers, &request.container_name, &request.image) {
            return Err(format!(
                "Workspace '{}' is already running in Docker (container {}, image {}). Stop it with 'docker stop {}' and run the benchmark again.",
                workspace.name, running.name, running.image, running.name
            ));
        }
    }

    Ok(())
}

// The workspaces one after the other, native first and Docker second, never at the same time.
// Everything a workspace started is stopped before the next one begins, on success and after
// a failure. A mode that fails keeps its status and the other mode still runs.
pub fn run_all(
    target: &Path,
    workspaces: &[WorkspaceRequest],
    docker: &dyn Docker,
    memory: &MemoryConfig,
    ctx: &RunContext,
) -> Result<Vec<WorkspaceResult>, Stop> {
    let control = ctx.control();
    let mut results = Vec::new();

    for workspace in workspaces {
        if control.is_cancel_requested() {
            return Err(Stop::Cancelled);
        }

        match prepare(target, workspace) {
            Err(message) => results.extend(failed_modes(workspace, &message)),
            Ok(prepared) => {
                let shared = Shared {
                    workspace,
                    directory: &prepared.directory,
                    variables: &prepared.variables,
                    variable_names: &prepared.names,
                    has_env: prepared.has_env,
                    memory,
                    control,
                    ctx,
                };

                if let Some(request) = workspace.native.as_ref().filter(|n| n.enabled) {
                    ctx.workspace_phase(&workspace.name, MODE_NATIVE, "started", None);

                    let result = native::run(&shared, request)?;

                    ctx.workspace_phase(
                        &workspace.name,
                        MODE_NATIVE,
                        "finished",
                        Some(result.status.as_str()),
                    );
                    results.push(result);
                }

                if control.is_cancel_requested() {
                    return Err(Stop::Cancelled);
                }

                if let Some(request) = workspace.docker.as_ref().filter(|d| d.enabled) {
                    ctx.workspace_phase(&workspace.name, MODE_DOCKER, "started", None);

                    let result = container::run(&shared, request, docker)?;

                    ctx.workspace_phase(
                        &workspace.name,
                        MODE_DOCKER,
                        "finished",
                        Some(result.status.as_str()),
                    );
                    results.push(result);
                }
            }
        }
    }

    Ok(results)
}

struct Prepared {
    directory: PathBuf,
    variables: BTreeMap<String, String>,
    // Variable names for the results, never a value
    names: Vec<String>,
    has_env: bool,
}

fn prepare(target: &Path, workspace: &WorkspaceRequest) -> Result<Prepared, String> {
    let directory = target.join(&workspace.directory);

    if !directory.is_dir() {
        return Err(format!(
            "The workspace directory does not exist: {}",
            directory.display()
        ));
    }

    let variables = load_variables(target, workspace.env.as_ref())?;

    Ok(Prepared {
        directory,
        names: variables
            .as_ref()
            .map(|variables| variables.keys().cloned().collect())
            .unwrap_or_default(),
        has_env: variables.is_some(),
        variables: variables.unwrap_or_default(),
    })
}

// None when the workspace has no env. The messages carry paths and line numbers, no values.
fn load_variables(
    target: &Path,
    env: Option<&WorkspaceEnv>,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(env) = env else {
        return Ok(None);
    };

    match (&env.file_path, &env.pasted) {
        (Some(path), None) => {
            let path = resolve_env_path(target, path);

            let text = fs::read_to_string(&path).map_err(|error| {
                format!("The env file {} could not be read: {error}", path.display())
            })?;

            env_file::parse(&text)
                .map(Some)
                .map_err(|message| format!("The env file {}: {message}", path.display()))
        }
        (None, Some(text)) => env_file::parse(text)
            .map(Some)
            .map_err(|message| format!("The pasted env: {message}")),
        _ => Ok(None),
    }
}

// "~/" is the home folder, a relative path starts at the project path
fn resolve_env_path(target: &Path, path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return Path::new(&home).join(rest);
    }

    let path = Path::new(path);

    if path.is_absolute() {
        path.to_path_buf()
    } else {
        target.join(path)
    }
}

// Every enabled mode of a workspace that could not be prepared, failed with the same reason
fn failed_modes(workspace: &WorkspaceRequest, message: &str) -> Vec<WorkspaceResult> {
    let mut modes = Vec::new();

    if workspace.native_enabled() {
        modes.push(MODE_NATIVE);
    }

    if workspace.docker_enabled() {
        modes.push(MODE_DOCKER);
    }

    modes
        .into_iter()
        .map(|mode_name| {
            let mut result = WorkspaceResult::empty(&workspace.name, mode_name, Vec::new());

            mode::fail(&mut result, message.to_string());

            result
        })
        .collect()
}

// The pid of a running process that has the start command's words in its command line and the
// workspace folder as its working directory. Both must match so an unrelated "bun run dev" of
// another project is not mistaken for the subscriber.
fn running_native(start: &BuildCommand, directory: &Path) -> Option<u32> {
    let directory = directory.canonicalize().ok()?;
    let own = std::process::id();

    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };

        if pid == own {
            continue;
        }

        let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };

        let words = command_words(&raw);

        if !words_contain(&words, start) {
            continue;
        }

        // Another user's process cannot be inspected, it is not counted
        let Ok(working) = fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };

        if working == directory {
            return Some(pid);
        }
    }

    None
}

// /proc/<pid>/cmdline holds the words separated by NUL
pub fn command_words(raw: &[u8]) -> Vec<String> {
    raw.split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect()
}

// Whether the words hold the command's program and arguments one after the other. The program
// is compared by file name, so "bun" matches "/usr/local/bin/bun".
pub fn words_contain(words: &[String], command: &BuildCommand) -> bool {
    let wanted = file_name(&command.program);

    words.iter().enumerate().any(|(index, word)| {
        file_name(word) == wanted
            && words
                .get(index + 1..index + 1 + command.args.len())
                .is_some_and(|following| following == command.args.as_slice())
    })
}

fn file_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str]) -> BuildCommand {
        BuildCommand {
            program: program.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        }
    }

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn splits_a_nul_separated_command_line() {
        assert_eq!(
            command_words(b"bun\0run\0dev\0"),
            words(&["bun", "run", "dev"])
        );
    }

    #[test]
    fn matches_the_command_anywhere_in_the_words() {
        let start = command("bun", &["run", "dev"]);

        assert!(words_contain(&words(&["bun", "run", "dev"]), &start));
        assert!(words_contain(
            &words(&["setsid", "/usr/bin/bun", "run", "dev"]),
            &start
        ));
        assert!(!words_contain(&words(&["bun", "run", "build"]), &start));
        assert!(!words_contain(&words(&["bun", "run"]), &start));
        assert!(!words_contain(&words(&["node", "run", "dev"]), &start));
    }

    #[test]
    fn a_command_without_arguments_matches_by_program_name() {
        let start = command("./target/release/subscriber", &[]);

        assert!(words_contain(
            &words(&["/work/target/release/subscriber"]),
            &start
        ));
    }

    #[test]
    fn env_paths_resolve_against_the_project() {
        assert_eq!(
            resolve_env_path(Path::new("/p"), "sub/.env"),
            PathBuf::from("/p/sub/.env")
        );
        assert_eq!(
            resolve_env_path(Path::new("/p"), "/abs/.env"),
            PathBuf::from("/abs/.env")
        );
    }
}
