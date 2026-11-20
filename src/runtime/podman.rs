//! Podman with `podman compose`, which hands the compose file to docker-compose (through Podman's socket).
//! podman-compose, the other provider `podman compose` can use, doesn't know flags every compose call passes
//! (`--project-directory`, `up --wait`, `ps -a`), so [`Podman::check_running`] refuses it.

use super::{Ctx, Runtime, RuntimeInfo, RuntimeKind};
use crate::config::LlmGpu;
use crate::process;
use crate::ui::Ui;
use anyhow::{Result, bail};

/// The CDI name of every NVIDIA GPU (`nvidia-ctk cdi generate`).
const NVIDIA_CDI_DEVICE: &str = "nvidia.com/gpu=all";
/// Where CDI specs are read from.
const CDI_DIRS: &[&str] = &["/etc/cdi", "/var/run/cdi"];

const PODMAN_COMPOSE_MESSAGE: &str = "podman compose uses podman-compose, which the sandbox doesn't support. \
    Install docker-compose (Podman uses it automatically) and try again. If docker-compose is installed, \
    set compose_providers under [engine] in containers.conf to its path.";

pub struct Podman<'a> {
    ctx: Ctx<'a>,
}

impl<'a> Podman<'a> {
    pub fn new(ctx: Ctx<'a>) -> Self {
        Self { ctx }
    }

    /// Whether `nvidia.com/gpu=all` is known: `nvidia-ctk cdi list`, or without `nvidia-ctk` a spec file.
    fn nvidia_cdi_available(&self, ui: &Ui) -> bool {
        let r = process::capture(
            ui,
            self.verbose(),
            "nvidia-ctk",
            &["cdi", "list"],
            &[],
            false,
        );
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

    /// Whether `podman compose` hands over to podman-compose: `podman compose version` names the provider
    /// ("podman-compose version 1.5.0"; docker-compose: "Docker Compose version v2…"). A failing call (no
    /// provider at all) is left to the first compose call to report.
    fn uses_podman_compose(&self, ui: &Ui) -> bool {
        let r = self.run(ui, &["compose", "version"]);
        r.exit_code == 0 && is_podman_compose(&format!("{}\n{}", r.stdout, r.stderr))
    }
}

/// The output of `podman compose version` comes from podman-compose.
fn is_podman_compose(version_output: &str) -> bool {
    version_output
        .to_ascii_lowercase()
        .contains("podman-compose")
}

impl Runtime for Podman<'_> {
    fn ctx(&self) -> &Ctx<'_> {
        &self.ctx
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Podman
    }

    /// No "Executing external compose provider" banner on every call.
    fn env(&self) -> &'static [(&'static str, &'static str)] {
        &[("PODMAN_COMPOSE_WARNING_LOGS", "false")]
    }

    /// Instead of `--ansi never` (Docker passes it as a flag); docker-compose reads these too.
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
        if self.uses_podman_compose(ui) {
            bail!(PODMAN_COMPOSE_MESSAGE);
        }
        let mut words = r.stdout.split_whitespace();
        let version = words.next().unwrap_or_default().to_string();
        let rootless = words.next() == Some("true");
        Ok(RuntimeInfo {
            kind: self.kind(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_compose_providers_apart() {
        assert!(is_podman_compose(
            "podman-compose version 1.5.0\npodman version 5.7.0\n"
        ));
        assert!(is_podman_compose(
            ">>>> Executing external compose provider \"/usr/bin/podman-compose\". <<<<\n"
        ));
        assert!(!is_podman_compose("Docker Compose version v2.40.3\n"));
        assert!(!is_podman_compose(""));
    }
}
