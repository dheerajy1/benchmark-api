use std::process::Command;

use super::model::BuildCommand;

// A running container as `docker ps` lists it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningContainer {
    pub name: String,
    pub image: String,
}

// What the benchmark needs from the Docker command line. The build and run commands are
// the ones the caller configured and run like any other command, so only the small
// questions live here. Behind a trait so tests can fake Docker.
pub trait Docker: Send + Sync {
    fn running_containers(&self) -> Result<Vec<RunningContainer>, String>;

    // Size of the image in bytes, as `docker image inspect` reports it
    fn image_size_bytes(&self, image: &str) -> Result<u64, String>;

    // Disk usage of the unpacked image as `docker image ls` lists it, None when this Docker
    // does not report it. It never fails a run, so a fake can leave the default.
    fn image_disk_usage_bytes(&self, _image: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }

    // Main process id of the container on the host, None while it does not exist or run
    fn container_pid(&self, container: &str) -> Result<Option<u32>, String>;

    // Stops and removes the container, Ok when it does not exist
    fn stop_and_remove(&self, container: &str) -> Result<(), String>;

    // The command that follows the container's log from its start, readiness is read from it.
    // A fake can answer a command that prints a line instead.
    fn logs_command(&self, container: &str) -> BuildCommand {
        BuildCommand {
            program: "docker".to_string(),
            args: vec![
                "logs".to_string(),
                "--follow".to_string(),
                container.to_string(),
            ],
        }
    }
}

// The real thing, the `docker` program on the PATH
pub struct DockerCli;

impl Docker for DockerCli {
    fn running_containers(&self) -> Result<Vec<RunningContainer>, String> {
        let output = docker(&["ps", "--format", "{{.Names}}\t{{.Image}}"])?;

        if !output.status.success() {
            return Err(failure("docker ps", &output));
        }

        Ok(parse_running(&String::from_utf8_lossy(&output.stdout)))
    }

    fn image_size_bytes(&self, image: &str) -> Result<u64, String> {
        let output = docker(&["image", "inspect", "--format", "{{.Size}}", image])?;

        if !output.status.success() {
            return Err(failure("docker image inspect", &output));
        }

        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .map_err(|_| "docker image inspect did not answer a size".to_string())
    }

    fn image_disk_usage_bytes(&self, image: &str) -> Result<Option<u64>, String> {
        let output = docker(&["image", "ls", "--format", "{{.Size}}", image])?;

        if !output.status.success() {
            return Ok(None);
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .and_then(parse_size))
    }

    fn container_pid(&self, container: &str) -> Result<Option<u32>, String> {
        let output = docker(&["inspect", "--format", "{{.State.Pid}}", container])?;

        // A container that does not exist yet is an expected answer while it starts
        if !output.status.success() {
            return Ok(None);
        }

        let pid = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .unwrap_or(0);

        Ok((pid != 0).then_some(pid))
    }

    fn stop_and_remove(&self, container: &str) -> Result<(), String> {
        let output = docker(&["rm", "--force", container])?;

        if output.status.success()
            || String::from_utf8_lossy(&output.stderr).contains("No such container")
        {
            return Ok(());
        }

        Err(failure("docker rm", &output))
    }
}

fn docker(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("docker")
        .args(args)
        .output()
        .map_err(|error| format!("Docker is not available: {error}"))
}

fn failure(what: &str, output: &std::process::Output) -> String {
    format!(
        "{what} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

pub fn parse_running(listing: &str) -> Vec<RunningContainer> {
    listing
        .lines()
        .filter_map(|line| {
            let (name, image) = line.split_once('\t')?;

            Some(RunningContainer {
                name: name.trim().to_string(),
                image: image.trim().to_string(),
            })
        })
        .collect()
}

// A size as Docker prints it, for example "276MB" or "1.01GB", in decimal units
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|character: char| !(character.is_ascii_digit() || character == '.'))?;
    let (number, unit) = text.split_at(split);
    let value: f64 = number.parse().ok()?;

    let factor = match unit.trim() {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };

    Some((value * factor).round() as u64)
}

// The running container that has the configured name or the configured image. A missing
// tag and ":latest" mean the same image.
pub fn find_running<'a>(
    containers: &'a [RunningContainer],
    name: &str,
    image: &str,
) -> Option<&'a RunningContainer> {
    containers
        .iter()
        .find(|container| container.name == name || same_image(&container.image, image))
}

fn same_image(left: &str, right: &str) -> bool {
    strip_latest(left) == strip_latest(right)
}

fn strip_latest(image: &str) -> &str {
    image.strip_suffix(":latest").unwrap_or(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(name: &str, image: &str) -> RunningContainer {
        RunningContainer {
            name: name.to_string(),
            image: image.to_string(),
        }
    }

    #[test]
    fn parses_the_ps_listing() {
        let listing = "web\tnginx:1\nsub\trecordlog-sub:latest\n\nbroken line\n";

        assert_eq!(
            parse_running(listing),
            vec![
                container("web", "nginx:1"),
                container("sub", "recordlog-sub:latest")
            ]
        );
    }

    #[test]
    fn reads_the_sizes_docker_prints() {
        assert_eq!(parse_size("276MB"), Some(276_000_000));
        assert_eq!(parse_size("1.01GB\n"), Some(1_010_000_000));
        assert_eq!(parse_size("512B"), Some(512));
        assert_eq!(parse_size("12 parsecs"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("276"), None);
    }

    #[test]
    fn finds_by_name_or_by_image() {
        let containers = vec![container("web", "nginx:1"), container("a", "sub:latest")];

        assert!(find_running(&containers, "web", "other:1").is_some());
        assert!(find_running(&containers, "none", "sub").is_some());
        assert!(find_running(&containers, "none", "sub:latest").is_some());
        assert!(find_running(&containers, "none", "sub:2").is_none());
    }
}
