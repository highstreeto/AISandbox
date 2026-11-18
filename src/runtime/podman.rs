//! Podman with `podman compose`, which hands the compose file to docker-compose (through Podman's socket) or to
//! podman-compose, whichever is installed.

use super::{Ctx, Runtime, RuntimeInfo, ServiceState, ps};
use crate::config::LlmGpu;
use crate::process;
use crate::ui::Ui;
use anyhow::{Result, bail};
use serde_json::Value;

/// The CDI name of every NVIDIA GPU (`nvidia-ctk cdi generate`).
const NVIDIA_CDI_DEVICE: &str = "nvidia.com/gpu=all";
/// Where CDI specs are read from.
const CDI_DIRS: &[&str] = &["/etc/cdi", "/var/run/cdi"];

pub struct Podman<'a> {
    ctx: Ctx<'a>,
}

impl<'a> Podman<'a> {
    pub fn new(ctx: Ctx<'a>) -> Self {
        Self { ctx }
    }

    /// Whether `nvidia.com/gpu=all` is known: `nvidia-ctk cdi list`, or without `nvidia-ctk` a spec file.
    fn nvidia_cdi_available(&self, ui: &Ui) -> bool {
        let r = process::capture(ui, self.verbose(), "nvidia-ctk", &["cdi", "list"]);
        if r.exit_code == 0 {
            return r.stdout.contains(NVIDIA_CDI_DEVICE);
        }
        CDI_DIRS.iter().any(|dir| {
            std::fs::read_dir(dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("nvidia.com/gpu"))
                })
            })
        })
    }
}

impl Runtime for Podman<'_> {
    fn ctx(&self) -> &Ctx<'_> {
        &self.ctx
    }

    fn name(&self) -> &'static str {
        "Podman"
    }

    fn program(&self) -> &'static str {
        "podman"
    }

    /// No "Executing external compose provider" banner on every call.
    fn env(&self) -> &'static [(&'static str, &'static str)] {
        &[("PODMAN_COMPOSE_WARNING_LOGS", "false")]
    }

    /// `--ansi` is a docker-compose flag that podman-compose doesn't know; both read these.
    fn captured_env(&self) -> &'static [(&'static str, &'static str)] {
        &[("COMPOSE_ANSI", "never"), ("NO_COLOR", "1")]
    }

    fn not_running_message(&self) -> String {
        if cfg!(target_os = "linux") {
            "Podman is not working (podman info failed). Run \"podman info\" to see why.".into()
        } else {
            "Podman is not running. Start the Podman machine (podman machine start) and try again."
                .into()
        }
    }

    fn check_running(&self, ui: &Ui) -> Result<RuntimeInfo> {
        let r = self.run(
            ui,
            &[
                "info",
                "--format",
                "{{.Version.Version}} {{.Host.Security.Rootless}}",
            ],
        );
        if r.exit_code != 0 {
            bail!(self.not_running_message());
        }
        let mut words = r.stdout.split_whitespace();
        let version = words.next().unwrap_or_default().to_string();
        let rootless = words.next() == Some("true");
        Ok(RuntimeInfo {
            kind: "podman",
            version,
            rootless,
        })
    }

    /// `podman ps` has the labels as a map, not docker's `.Label` function.
    fn project_owner_ids(&self, ui: &Ui) -> Vec<String> {
        let name = &self.project().name;
        self.run(
            ui,
            &[
                "ps",
                "-a",
                "--filter",
                &format!("label=com.docker.compose.project={name}"),
                "--filter",
                "label=sandbox.id",
                "--format",
                "{{index .Labels \"sandbox.id\"}}",
            ],
        )
        .lines()
    }

    /// docker-compose prints its own JSON; podman-compose prints `podman ps` JSON.
    fn compose_ps(&self, ui: &Ui) -> Result<Vec<ServiceState>> {
        let r = self.compose(ui, &["ps", "-a", "--format", "json"]);
        if r.exit_code != 0 || r.stdout.trim().is_empty() {
            return Ok(Vec::new());
        }
        ps::sorted(ps::parse_entries(&r.stdout).map(|entries| {
            entries
                .iter()
                .map(|e| {
                    if e.get("Service").is_some() {
                        ps::to_state(e)
                    } else {
                        from_podman_ps(e)
                    }
                })
                .collect()
        }))
    }

    /// The NVIDIA GPU needs a CDI spec on a Linux host; in a Podman machine it lives in the VM (no check).
    fn check_before_up(&self, ui: &Ui) -> Result<()> {
        let wants_nvidia = self
            .project()
            .settings
            .llm
            .as_ref()
            .is_some_and(|l| l.gpu == LlmGpu::Nvidia);
        if !wants_nvidia || !cfg!(target_os = "linux") || self.nvidia_cdi_available(ui) {
            return Ok(());
        }
        bail!(
            "The NVIDIA GPU is not available to Podman (no CDI device {NVIDIA_CDI_DEVICE}). Install the NVIDIA \
             Container Toolkit and run: sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml\n\
             To run the local LLM on the CPU instead (slow), set gpu = \"none\" under [llm] in {}.",
            crate::config::LOCAL_FILE_NAME
        )
    }

    fn gpu_hint(&self) -> &'static str {
        "Linux: install the NVIDIA Container Toolkit and generate its CDI spec \
         (sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml); AMD: the ROCm driver. \
         Windows/macOS: the GPU has to be set up inside the Podman machine."
    }
}

/// One entry of `podman ps --format json`: the service is a label, health is in the status text, ports are
/// `host_ip` / `host_port` / `container_port`.
fn from_podman_ps(e: &Value) -> ServiceState {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_string();
    let service = text(
        e.get("Labels")
            .and_then(|l| l.get("com.docker.compose.service")),
    );
    let status = text(e.get("Status"));
    let health = ["unhealthy", "healthy", "starting"]
        .into_iter()
        .find(|h| status.contains(&format!("({h})")))
        .unwrap_or("")
        .to_string();
    let mut ports: Vec<String> = Vec::new();
    for p in e
        .get("Ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let published = p.get("host_port").and_then(Value::as_i64).unwrap_or(0);
        if published <= 0 {
            continue;
        }
        let target = p.get("container_port").and_then(Value::as_i64).unwrap_or(0);
        let ip = text(p.get("host_ip"));
        let ip = if ip.is_empty() {
            "0.0.0.0".to_string()
        } else {
            ip
        };
        let entry = format!("{ip}:{published}→{target}");
        if !ports.contains(&entry) {
            ports.push(entry);
        }
    }
    ServiceState {
        service,
        state: text(e.get("State")),
        health,
        status,
        ports: ports.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_podman_ps_entries() {
        let json = r#"[{"Names":["demo_desktop_1"],"State":"running","Status":"Up 2 minutes (healthy)",
            "Labels":{"com.docker.compose.service":"desktop"},"Ports":null},
            {"State":"running","Status":"Up 2 minutes","Labels":{"com.docker.compose.service":"gateway"},
            "Ports":[{"host_ip":"127.0.0.1","container_port":3001,"host_port":49153,"range":1,"protocol":"tcp"},
                     {"host_ip":"","container_port":80,"host_port":0}]}]"#;
        let entries = ps::parse_entries(json).unwrap();
        let s: Vec<ServiceState> = entries.iter().map(from_podman_ps).collect();
        assert_eq!(
            s[0],
            ServiceState {
                service: "desktop".into(),
                state: "running".into(),
                health: "healthy".into(),
                status: "Up 2 minutes (healthy)".into(),
                ports: "".into(),
            }
        );
        assert_eq!(s[1].service, "gateway");
        assert_eq!(s[1].health, "");
        assert_eq!(s[1].ports, "127.0.0.1:49153→3001");
    }

    #[test]
    fn unhealthy_is_not_read_as_healthy() {
        let e: Value =
            serde_json::from_str(r#"{"State":"running","Status":"Up 1 minute (unhealthy)"}"#)
                .unwrap();
        assert_eq!(from_podman_ps(&e).health, "unhealthy");
    }
}
