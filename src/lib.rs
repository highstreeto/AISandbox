//! AI Sandbox control app: a desktop for a project, reachable only through an allowlist proxy.

pub mod cli;
pub mod commands;
pub mod config;
pub mod engine;
pub mod paths;
pub mod process;
pub mod project;
pub mod runtime;
pub mod ui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use commands::{lifecycle, llm, maintenance, network, upgrade};
use engine::compose;
use project::Project;
use runtime::{Ctx, Runtime};
use ui::Ui;

/// The app version, `<version>+<commit>` when built from git (see `build.rs`).
pub const VERSION: &str = env!("SANDBOX_VERSION");

/// Runs the app with the command line (without the program name) and returns the exit code.
///
/// The order of side effects is what the tests assert: `claude` is handled before the command line is parsed (all its
/// arguments, `--help` and `-v` included, belong to claude); `init`, `allowlist`, `info` and `upgrade` need no container
/// runtime (`upgrade` no project either); every other command loads the project, selects and checks the runtime (Docker or Podman), refuses a container group that belongs to another project,
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
        Command::Upgrade { version, check } => upgrade::run(ui, check, version.as_deref()),
        command => {
            let project = Project::load(ui)?;
            let rt = runtime::select(Ctx {
                project: &project,
                verbose,
            })?;
            let info = runtime::need_runtime(ui, rt.as_ref())?;

            let sync_result = compose::generate(&project, &info)?;
            if sync_result.has_restored_or_updated() {
                ui.rule("Syncing .state/");
                for path in sync_result.restored {
                    lifecycle::warn_restored(ui, &path);
                }
                for path in sync_result.updated {
                    lifecycle::info_updated(ui, &path);
                }
            }
            run_runtime_command(ui, rt.as_ref(), command)
        }
    }
}

/// The commands that run after the project is loaded and the container runtime is checked.
fn run_runtime_command(ui: &Ui, rt: &dyn Runtime, command: Command) -> Result<i32> {
    match command {
        Command::Up { no_open, no_pull } => lifecycle::up(ui, rt, !no_open, !no_pull),
        Command::Stop => {
            lifecycle::compose_command(ui, rt, "Stopping", "stop", |rt| rt.compose_stop(ui))
        }
        Command::Down => lifecycle::compose_command(ui, rt, "Removing containers", "down", |rt| {
            rt.compose_down(ui)
        }),
        Command::Restart => lifecycle::compose_command(ui, rt, "Restarting", "restart", |rt| {
            rt.compose_restart(ui, &[])
        }),
        Command::Status => lifecycle::status(ui, rt),
        Command::Open => lifecycle::open(ui, rt),
        Command::Shell => lifecycle::shell(ui, rt),
        Command::Logs {
            service,
            tail,
            no_follow,
        } => lifecycle::logs(ui, rt, &service, tail, !no_follow),
        Command::Allow { host } => network::allow(ui, rt, &host),
        Command::Deny { host } => network::deny(ui, rt, &host),
        Command::Blocked => network::blocked(ui, rt),
        Command::Reload => network::reload_proxy(ui, rt),
        Command::Llm { command } => llm::run(ui, rt, command),
        Command::Rebuild => lifecycle::rebuild(ui, rt, false),
        Command::Update => lifecycle::rebuild(ui, rt, true),
        Command::ResetHome { yes } => maintenance::reset_home(ui, rt, yes),
        // handled before a project is loaded
        Command::Claude { .. }
        | Command::Init { .. }
        | Command::Allowlist { .. }
        | Command::Info
        | Command::Upgrade { .. } => {
            unreachable!("handled by run_inner")
        }
    }
}
