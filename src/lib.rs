//! AI Sandbox control app: a desktop for a project, reachable only through an allowlist proxy.

pub mod allowlist;
pub mod cli;
pub mod commands;
pub mod compose;
pub mod config;
pub mod docker;
pub mod engine;
pub mod paths;
pub mod process;
pub mod project;
pub mod ui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use commands::{lifecycle, maintenance, network};
use docker::Docker;
use project::Project;
use ui::Ui;

/// The app version, `0.1.0+<commit>` when built from git (see `build.rs`).
pub const VERSION: &str = env!("SANDBOX_VERSION");

/// Runs the app with the command line (without the program name) and returns the exit code.
///
/// The order of side effects is what the tests assert: `claude` is handled before the command line is parsed (all its
/// arguments, `--help` and `-v` included, belong to claude); `init`, `allowlist` and `info` need no Docker; every
/// other command loads the project, checks Docker, refuses a container group that belongs to another project,
/// regenerates `.state/` and then runs.
pub fn run(args: Vec<String>) -> i32 {
    let ui = Ui::new();
    match run_inner(&ui, args) {
        Ok(code) => code,
        Err(e) => {
            ui.error(&format!("{e:#}"));
            1
        }
    }
}

fn run_inner(ui: &Ui, args: Vec<String>) -> Result<i32> {
    if args.first().is_some_and(|a| a == "claude") {
        return lifecycle::claude(ui, false, &args[1..]);
    }

    let cli = match Cli::try_parse_from(std::iter::once("sandbox".to_string()).chain(args)) {
        Ok(cli) => cli,
        Err(e) => {
            // Help and --version go to stdout with exit code 0; usage errors exit with 1 (clap's own code is 2).
            let code = if e.use_stderr() { 1 } else { 0 };
            e.print()?;
            return Ok(code);
        }
    };
    let verbose = cli.verbose;

    match cli.command {
        Command::Claude { args } => lifecycle::claude(ui, verbose, &args),
        Command::Init { folder } => maintenance::init(ui, folder.as_deref()),
        Command::Allowlist { all } => network::show_allowlist(ui, &Project::load(ui)?, all),
        Command::Info => maintenance::info(ui, &Project::load(ui)?),
        command => {
            let project = Project::load(ui)?;
            let docker = Docker::new(&project, verbose, ui);
            docker.need_docker()?;
            docker.check_collision()?;
            for rel in compose::generate(&project)?.restored {
                lifecycle::warn_restored(ui, &rel);
            }
            run_docker_command(&docker, command)
        }
    }
}

/// The commands that run after the project is loaded and Docker is checked.
fn run_docker_command(d: &Docker, command: Command) -> Result<i32> {
    match command {
        Command::Up { no_open } => lifecycle::up(d, !no_open),
        Command::Stop => lifecycle::compose_command(d, "Stopping", &["stop"]),
        Command::Down => lifecycle::compose_command(d, "Removing containers", &["down"]),
        Command::Restart => lifecycle::compose_command(d, "Restarting", &["restart"]),
        Command::Status => lifecycle::status(d),
        Command::Open => lifecycle::open(d),
        Command::Shell => lifecycle::shell(d),
        Command::Logs {
            service,
            tail,
            no_follow,
        } => lifecycle::logs(d, &service, tail, !no_follow),
        Command::Allow { host } => network::allow(d, &host),
        Command::Deny { host } => network::deny(d, &host),
        Command::Blocked => network::blocked(d),
        Command::Reload => network::reload_proxy(d),
        Command::Rebuild => lifecycle::rebuild(d, false),
        Command::Update => lifecycle::rebuild(d, true),
        Command::ResetHome { yes } => maintenance::reset_home(d, yes),
        // handled before a project is loaded
        Command::Claude { .. }
        | Command::Init { .. }
        | Command::Allowlist { .. }
        | Command::Info => {
            unreachable!("handled by run_inner")
        }
    }
}
