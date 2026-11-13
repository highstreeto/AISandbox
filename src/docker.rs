//! Everything that runs `docker`: compose calls, the Docker check, the collision check, the UI URL, the services.

use crate::process::{self, ProcResult};
use crate::project::Project;
use crate::ui::Ui;
use anyhow::{Context, Result, bail};

/// One container as `docker compose ps` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    pub service: String,
    pub state: String,
    pub health: String,
    pub status: String,
    pub ports: String,
}

pub struct Docker<'a> {
    pub project: &'a Project,
    pub verbose: bool,
    pub ui: &'a Ui,
}

impl<'a> Docker<'a> {
    pub fn new(project: &'a Project, verbose: bool, ui: &'a Ui) -> Self {
        Self {
            project,
            verbose,
            ui,
        }
    }

    /// Runs docker compose with captured output (for spinners, tables and error panels).
    pub fn compose<S: AsRef<str>>(&self, args: &[S]) -> ProcResult {
        let mut full = vec!["--ansi".to_string(), "never".to_string()];
        full.extend(args.iter().map(|a| a.as_ref().to_string()));
        process::capture(self.ui, self.verbose, "docker", &self.compose_args(&full))
    }

    /// Runs docker compose attached to this terminal (build output, shell, logs, claude).
    pub fn compose_interactive<S: AsRef<str>>(&self, args: &[S]) -> Result<i32> {
        let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        process::interactive(self.ui, self.verbose, "docker", &self.compose_args(&args))
    }

    /// Runs a command in the desktop container as the desktop user, in the project folder, attached to this
    /// terminal.
    pub fn exec<S: AsRef<str>>(&self, command: &[S]) -> Result<i32> {
        let mut args: Vec<String> = ["exec", "-u", "abc", "-w", &self.project.work_dir, "desktop"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.extend(command.iter().map(|c| c.as_ref().to_string()));
        self.compose_interactive(&args)
    }

    /// Fails if docker isn't installed or its daemon isn't running.
    pub fn need_docker(&self) -> Result<()> {
        if !process::on_path("docker") {
            bail!("Docker is not installed.");
        }
        let r = self.ui.spin("Checking Docker…", || {
            process::capture(
                self.ui,
                self.verbose,
                "docker",
                &["info", "--format", "{{.ServerVersion}}"],
            )
        });
        if r.exit_code != 0 {
            bail!(
                "Docker is not running. Start Docker Desktop (or the Docker service) and try again."
            );
        }
        Ok(())
    }

    /// Fails if containers of another project (another `sandbox.id`) use the same compose project name.
    pub fn check_collision(&self) -> Result<()> {
        let name = &self.project.name;
        let r = process::capture(
            self.ui,
            self.verbose,
            "docker",
            &[
                "ps",
                "-a",
                "--filter",
                &format!("label=com.docker.compose.project={name}"),
                "--filter",
                "label=sandbox.id",
                "--format",
                "{{.Label \"sandbox.id\"}}",
            ],
        );
        if r.lines().iter().any(|l| *l != self.project.id) {
            bail!(
                "Another project already uses the container group '{name}'. Set name under [project] in sandbox.toml."
            );
        }
        Ok(())
    }

    /// The desktop's URL, or none if the gateway's port isn't published (not running).
    pub fn ui_url(&self) -> Option<String> {
        let r = self.compose(&["port", "gateway", "3001"]);
        let p = if r.exit_code == 0 {
            r.lines().into_iter().next()
        } else {
            None
        }?;
        if p.is_empty() || p.ends_with(":0") {
            return None;
        }
        let port = p.rsplit(':').next().unwrap_or(&p);
        Some(format!("https://localhost:{port}"))
    }

    /// The containers of this project, by service name.
    pub fn services(&self) -> Result<Vec<ServiceState>> {
        let r = self.compose(&["ps", "-a", "--format", "json"]);
        if r.exit_code != 0 || r.stdout.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut services =
            parse_ps(&r.stdout).context("Could not read the output of docker compose ps")?;
        services.sort_by(|a, b| a.service.as_bytes().cmp(b.service.as_bytes()));
        Ok(services)
    }

    /// `compose -p <name> --project-directory <dir> -f <generated file> <args>`
    fn compose_args(&self, args: &[String]) -> Vec<String> {
        let mut full: Vec<String> = [
            "compose",
            "-p",
            &self.project.name,
            "--project-directory",
            self.project.dir.as_str(),
            "-f",
            self.project.gen_path().as_str(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        full.extend_from_slice(args);
        full
    }
}

/// Reads the output of `docker compose ps --format json`: newer compose prints one JSON object per line, older
/// versions a single array.
pub fn parse_ps(json: &str) -> Result<Vec<ServiceState>> {
    let text = json.trim();
    let docs: Vec<&str> = if text.starts_with('[') {
        vec![text]
    } else {
        text.split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    };
    let mut result = Vec::new();
    for d in docs {
        match serde_json::from_str::<serde_json::Value>(d)? {
            serde_json::Value::Array(items) => result.extend(items.iter().map(to_state)),
            other => result.push(to_state(&other)),
        }
    }
    Ok(result)
}

fn to_state(e: &serde_json::Value) -> ServiceState {
    ServiceState {
        service: text(e, "Service"),
        state: text(e, "State"),
        health: text(e, "Health"),
        status: text(e, "Status"),
        ports: format_ports(e),
    }
}

/// A missing or non-text field is "".
fn text(e: &serde_json::Value, key: &str) -> String {
    e.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// The published ports of one entry, e.g. "127.0.0.1:49153→3001".
fn format_ports(e: &serde_json::Value) -> String {
    let Some(publishers) = e.get("Publishers").and_then(|p| p.as_array()) else {
        return String::new();
    };
    let mut ports: Vec<String> = Vec::new();
    for x in publishers {
        let published = x
            .get("PublishedPort")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if published <= 0 {
            continue;
        }
        let target = x
            .get("TargetPort")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let entry = format!("{}:{published}→{target}", text(x, "URL"));
        if !ports.contains(&entry) {
            ports.push(entry);
        }
    }
    ports.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_object_per_line() {
        let json = "{\"Service\":\"b\",\"State\":\"running\",\"Health\":\"healthy\",\"Status\":\"Up\",\"Publishers\":[{\"URL\":\"127.0.0.1\",\"TargetPort\":3001,\"PublishedPort\":49153},{\"URL\":\"127.0.0.1\",\"TargetPort\":3001,\"PublishedPort\":49153},{\"URL\":\"::\",\"TargetPort\":80,\"PublishedPort\":0}]}\n\n{\"Service\":\"a\"}\n";
        let s = parse_ps(json).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].ports, "127.0.0.1:49153→3001");
        assert_eq!(s[0].health, "healthy");
        assert_eq!(
            s[1],
            ServiceState {
                service: "a".into(),
                state: "".into(),
                health: "".into(),
                status: "".into(),
                ports: "".into()
            }
        );
    }

    #[test]
    fn parses_a_single_array_and_ignores_non_text_fields() {
        let s = parse_ps("[{\"Service\":\"x\",\"State\":1,\"Publishers\":null},{\"Service\":\"y\",\"Publishers\":[]}]").unwrap();
        assert_eq!(
            s.iter().map(|x| x.service.as_str()).collect::<Vec<_>>(),
            vec!["x", "y"]
        );
        assert_eq!(s[0].state, "");
        assert_eq!(s[0].ports, "");
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_ps("not json").is_err());
    }
}
