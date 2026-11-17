//! The container runtime that runs the sandbox: one [`Runtime`] implementation per program (`docker.rs`,
//! `podman.rs`), chosen by [`select`].
//!
//! The trait is the list of things the commands do with containers. Most methods have a default implementation
//! built on [`Runtime::program`] and a few small hooks, so an implementation only spells out what differs.
//! `src/engine/` is the other half: what gets generated into `.state/`; this module runs it.

pub mod docker;
pub mod podman;
pub mod ps;

pub use docker::Docker;
pub use podman::Podman;
pub use ps::ServiceState;

use crate::config::{RuntimeSetting, SettingSource};
use crate::process::{self, ProcResult};
use crate::project::Project;
use crate::ui::Ui;
use anyhow::{Result, bail};

/// Overrides `[host] runtime` for one call or one shell: `auto`, `docker` or `podman`.
pub const ENV_VAR: &str = "SANDBOX_RUNTIME";

/// What every runtime needs to run commands for one project.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub project: &'a Project,
    /// Print the commands that are run (`-v`).
    pub verbose: bool,
    pub ui: &'a Ui,
}

/// What a running runtime reports about itself; the compose template gets it as `host.runtime` /
/// `host.rootless`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// `docker` or `podman`.
    pub kind: &'static str,
    pub version: String,
    /// Containers run in the user's own user namespace (rootless Podman).
    pub rootless: bool,
}

pub trait Runtime: Sync {
    fn ctx(&self) -> &Ctx<'_>;
    /// For messages: "Docker".
    fn name(&self) -> &'static str;
    /// The program on `PATH`: "docker".
    fn program(&self) -> &'static str;

    fn project(&self) -> &Project {
        self.ctx().project
    }
    fn ui(&self) -> &Ui {
        self.ctx().ui
    }
    fn verbose(&self) -> bool {
        self.ctx().verbose
    }

    // --- hooks ---

    /// Arguments put after the compose file in captured calls (output for spinners, tables and error panels).
    fn captured_flags(&self) -> &'static [&'static str] {
        &[]
    }

    /// Environment variables for every call.
    fn env(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// More environment variables for captured compose calls.
    fn captured_env(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// Checks before `up` / `rebuild` builds anything, e.g. that the GPU the LLM asks for is usable.
    fn check_before_up(&self) -> Result<()> {
        Ok(())
    }

    /// What to do when starting failed and the output mentions the GPU.
    fn gpu_hint(&self) -> &'static str;

    /// What to say when the program is installed but doesn't answer.
    fn not_running_message(&self) -> String;

    /// The version, or an error that says how to start the runtime.
    fn check_running(&self) -> Result<RuntimeInfo>;

    // --- host level ---

    /// Creates a volume (idempotent).
    fn volume_create(&self, name: &str) -> ProcResult {
        self.run(&["volume", "create", name])
    }

    fn volume_remove(&self, name: &str) -> ProcResult {
        self.run(&["volume", "rm", name])
    }

    fn volume_exits(&self, name: &str) -> bool {
        self.run(&["volume", "inspect", name]).exit_code == 0
    }

    /// The `sandbox.id` labels of every container in this project's compose group.
    fn project_owner_ids(&self) -> Vec<String> {
        let name = &self.project().name;
        self.run(&[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={name}"),
            "--filter",
            "label=sandbox.id",
            "--format",
            "{{.Label \"sandbox.id\"}}",
        ])
        .lines()
    }

    // --- the project's compose group ---

    /// `up -d --wait`: starts everything and waits until it's healthy.
    fn compose_up(&self) -> ProcResult {
        self.compose(&["up", "-d", "--wait", "--wait-timeout", "300"])
    }

    /// Builds the images with the build output on this terminal. `cache_bust` rebuilds the tool layers and pulls
    /// newer base images.
    fn compose_build(&self, cache_bust: Option<u64>) -> Result<i32> {
        let mut args = vec!["build".to_string()];
        if let Some(now) = cache_bust {
            args.extend([
                "--pull".to_string(),
                "--build-arg".to_string(),
                format!("CACHE_BUST={now}"),
            ]);
        }
        self.compose_attached(&args)
    }

    fn compose_stop(&self) -> ProcResult {
        self.compose(&["stop"])
    }

    fn compose_down(&self) -> ProcResult {
        self.compose(&["down"])
    }

    /// Restarts the given services, or all of them.
    fn compose_restart(&self, services: &[&str]) -> ProcResult {
        let mut args = vec!["restart"];
        args.extend_from_slice(services);
        self.compose(&args)
    }

    /// The containers of this project, by service name.
    fn compose_ps(&self) -> Result<Vec<ServiceState>> {
        let r = self.compose(&["ps", "-a", "--format", "json"]);
        if r.exit_code != 0 || r.stdout.trim().is_empty() {
            return Ok(Vec::new());
        }
        ps::sorted(ps::parse_ps(&r.stdout))
    }

    /// `host:port` that a container port is published on, if it is.
    fn compose_port(&self, service: &str, port: u16) -> Option<String> {
        let r = self.compose(&["port", service, &port.to_string()]);
        if r.exit_code != 0 {
            return None;
        }
        r.lines().into_iter().next()
    }

    /// The logs on this terminal.
    fn compose_logs(&self, services: &[String], tail: i64, follow: bool) -> Result<i32> {
        let mut args = vec!["logs".to_string(), format!("--tail={tail}")];
        if follow {
            args.push("-f".to_string());
        }
        args.extend_from_slice(services);
        self.compose_attached(&args)
    }

    /// One service's whole log, without the service prefix, captured.
    fn compose_logs_captured(&self, service: &str) -> ProcResult {
        self.compose(&["logs", "--no-log-prefix", service])
    }

    /// `exec -T <service> <command…>`, captured.
    fn compose_exec(&self, service: &str, command: &[&str]) -> ProcResult {
        let mut args = vec!["exec", "-T", service];
        args.extend_from_slice(command);
        self.compose(&args)
    }

    /// `exec <service> <command…>` on this terminal; `tty: false` adds `-T` (no terminal to attach).
    fn compose_exec_attached(&self, service: &str, command: &[&str], tty: bool) -> Result<i32> {
        let mut args = vec!["exec".to_string()];
        if !tty {
            args.push("-T".to_string());
        }
        args.push(service.to_string());
        args.extend(command.iter().map(|c| c.to_string()));
        self.compose_attached(&args)
    }

    /// A command in the desktop container as the desktop user, in the project folder, on this terminal.
    fn exec_desktop(&self, command: &[String]) -> Result<i32> {
        let mut args: Vec<String> = [
            "exec",
            "-u",
            "abc",
            "-w",
            &self.project().work_dir,
            "desktop",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.extend_from_slice(command);
        self.compose_attached(&args)
    }

    // --- plumbing ---

    /// `<program> <args>`, captured.
    fn run(&self, args: &[&str]) -> ProcResult {
        process::capture_env(self.ui(), self.verbose(), self.program(), args, self.env())
    }

    /// `compose -p <name> --project-directory <dir> -f <generated file>`
    fn compose_base(&self) -> Vec<String> {
        let p = self.project();
        [
            "compose",
            "-p",
            &p.name,
            "--project-directory",
            p.dir.as_str(),
            "-f",
            p.gen_path().as_str(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// A compose call with captured output (for spinners, tables and error panels).
    fn compose(&self, args: &[&str]) -> ProcResult {
        let mut full = self.compose_base();
        full.extend(self.captured_flags().iter().map(|s| s.to_string()));
        full.extend(args.iter().map(|a| a.to_string()));
        let envs: Vec<(&str, &str)> = self
            .env()
            .iter()
            .chain(self.captured_env())
            .copied()
            .collect();
        process::capture_env(self.ui(), self.verbose(), self.program(), &full, &envs)
    }

    /// A compose call attached to this terminal (build output, shell, logs, claude).
    fn compose_attached(&self, args: &[String]) -> Result<i32> {
        let mut full = self.compose_base();
        full.extend_from_slice(args);
        process::interactive_env(self.ui(), self.verbose(), self.program(), &full, self.env())
    }
}

/// Which runtime `SANDBOX_RUNTIME` and `[host] runtime` ask for, and where that came from (for messages).
pub fn requested(project: &Project) -> Result<(RuntimeSetting, String)> {
    if let Some(v) = std::env::var(ENV_VAR).ok().filter(|v| !v.trim().is_empty()) {
        let Some(r) = RuntimeSetting::parse(&v) else {
            bail!(
                "{ENV_VAR} must be \"auto\", \"docker\" or \"podman\" (got \"{}\").",
                v.trim()
            );
        };
        return Ok((r, format!("{ENV_VAR}={}", r.name())));
    }
    let s = &project.settings;
    let from = match s.runtime_source {
        SettingSource::Default => "auto".to_string(),
        src => format!(
            "runtime = \"{}\" in {}",
            s.runtime.name(),
            src.file_name().unwrap_or_default()
        ),
    };
    Ok((s.runtime, from))
}

/// The runtime for this project: the one asked for, or with `auto` Podman if it is installed, else Docker.
pub fn select<'a>(ctx: Ctx<'a>) -> Result<Box<dyn Runtime + 'a>> {
    let (setting, from) = requested(ctx.project)?;
    let docker = || -> Box<dyn Runtime + 'a> { Box::new(Docker::new(ctx)) };
    let podman = || -> Box<dyn Runtime + 'a> { Box::new(Podman::new(ctx)) };
    let rt = match setting {
        RuntimeSetting::Docker => docker(),
        RuntimeSetting::Podman => podman(),
        RuntimeSetting::Auto => match available(RuntimeSetting::Auto) {
            Some(RuntimeSetting::Podman) => podman(),
            Some(_) => docker(),
            None => bail!("Neither Podman nor Docker is installed."),
        },
    };
    if !process::on_path(rt.program()) {
        bail!("{} is not installed ({from}).", rt.name());
    }
    Ok(rt)
}

/// The runtime `setting` resolves to on this machine, if it is installed. `Auto` prefers Podman.
pub fn available(setting: RuntimeSetting) -> Option<RuntimeSetting> {
    let installed = |r: RuntimeSetting| process::on_path(r.name()).then_some(r);
    match setting {
        RuntimeSetting::Auto => {
            installed(RuntimeSetting::Podman).or_else(|| installed(RuntimeSetting::Docker))
        }
        r => installed(r),
    }
}

/// Before any container command: the runtime is installed and running, and the compose group isn't another
/// project's.
pub fn need_runtime(rt: &dyn Runtime) -> Result<RuntimeInfo> {
    let info = rt
        .ui()
        .spin(&format!("Checking {}…", rt.name()), || rt.check_running())?;
    check_collision(rt)?;
    Ok(info)
}

/// Fails if containers of another project (another `sandbox.id`) use the same compose project name.
fn check_collision(rt: &dyn Runtime) -> Result<()> {
    let p = rt.project();
    if rt.project_owner_ids().iter().any(|l| *l != p.id) {
        bail!(
            "Another project already uses the container group '{}'. Set name under [project] in sandbox.toml.",
            p.name
        );
    }
    Ok(())
}

/// The desktop's URL, or none if the gateway's port isn't published (not running).
pub fn ui_url(rt: &dyn Runtime) -> Option<String> {
    let p = rt.compose_port("gateway", 3001)?;
    if p.is_empty() || p.ends_with(":0") {
        return None;
    }
    let port = p.rsplit(':').next().unwrap_or(&p);
    Some(format!("https://localhost:{port}"))
}
