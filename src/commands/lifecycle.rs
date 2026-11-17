//! Building, starting, stopping and using the sandbox: up, rebuild, stop/down/restart, status, open, shell, claude,
//! logs.

use super::llm;
use crate::commands::llm::MODELS_VOLUME;
use crate::config::BrowserSetting;
use crate::engine::compose;
use crate::process::{self, ProcResult};
use crate::project::Project;
use crate::runtime::{self, Ctx, Runtime};
use crate::ui::Ui;
use anyhow::{Result, bail};
use console::{Color, Term};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CLAUDE_SCRIPT: &str = "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set claude_code under [tools] in sandbox.toml and rebuild.\"; exit 1; }; exec claude \"$@\"";

pub fn up(rt: &dyn Runtime, open_browser: bool, pull_models: bool) -> Result<i32> {
    rt.check_before_up()?;
    build(rt, false)?;
    ensure_llm_volume(rt)?;
    start_and_wait(rt, true)?;
    if pull_models {
        llm::pull_missing(rt)?;
    }
    std::thread::sleep(Duration::from_secs(2));
    let Some(url) = runtime::ui_url(rt) else {
        bail!("The sandbox started, but its UI port could not be determined.");
    };
    let ui = rt.ui();
    let p = rt.project();
    let lines = [
        format!("{}   {}", ui.bold("Project"), p.name),
        format!("{}       {url}", ui.bold("URL")),
        format!("{}   {}", ui.bold("Mounted"), p.work_dir),
        ui.dim("Accept the self-signed certificate in the browser."),
    ];
    ui.print_panel("Sandbox running", &lines, Some(Color::Green), false);
    if open_browser {
        process::open_browser(ui, &url, &p.settings.browser);
    }
    Ok(0)
}

pub fn rebuild(rt: &dyn Runtime, pull: bool) -> Result<i32> {
    rt.check_before_up()?;
    build(rt, pull)?;
    ensure_llm_volume(rt)?;
    start_and_wait(rt, false)?;
    llm::pull_missing(rt)?;
    rt.ui().success("Rebuilt and running.");
    Ok(0)
}

/// The models volume is external: it has to exist before compose starts the stack.
fn ensure_llm_volume(rt: &dyn Runtime) -> Result<()> {
    if rt.project().settings.llm.is_some() {
        if rt.volume_exits(MODELS_VOLUME) {
            return Ok(());
        }
        
        let res = rt.volume_create(MODELS_VOLUME);
        if res.exit_code != 0 {
            rt.ui()
                .show_output(&format!("{} volume create", rt.program()), &res);
            bail!("Could not create the volume {MODELS_VOLUME} for the models.");
        }
    }
    Ok(())
}

/// Runs one compose command with a spinner (stop, down, restart); `label` names it in the error output.
pub fn compose_command(
    rt: &dyn Runtime,
    what: &str,
    label: &str,
    call: impl FnOnce(&dyn Runtime) -> ProcResult + Send,
) -> Result<i32> {
    let name = &rt.project().name;
    let r = rt.ui().spin(&format!("{what} '{name}'…"), || call(rt));
    if r.exit_code != 0 {
        rt.ui()
            .show_output(&format!("{} compose {label}", rt.program()), &r);
        bail!("{what} failed.");
    }
    rt.ui().success(&format!("{what} '{name}': done."));
    Ok(0)
}

pub fn status(rt: &dyn Runtime) -> Result<i32> {
    let ui = rt.ui();
    let name = &rt.project().name;
    let ps = rt.compose_ps()?;
    if ps.is_empty() {
        println!(
            "{} Start the sandbox with: {}",
            ui.yellow(format!("No containers for '{name}'.")),
            ui.bold(format!("{} up", ui.self_name()))
        );
        return Ok(0);
    }
    let mut t = ui.new_table(&["Service", "State", "Health", "Status", "Ports"]);
    for s in &ps {
        t.add_row([
            s.service.clone(),
            ui.state(&s.state),
            ui.health(&s.health, &s.state),
            s.status.clone(),
            s.ports.clone(),
        ]);
    }
    ui.print_table(Some(&ui.bold(name)), &t, None);
    if let Some(url) = runtime::ui_url(rt) {
        println!("{} {url}", ui.bold("URL:"));
    }
    Ok(0)
}

pub fn open(rt: &dyn Runtime) -> Result<i32> {
    let Some(url) = runtime::ui_url(rt) else {
        bail!(
            "The sandbox is not running (start it with: {} up).",
            rt.ui().self_name()
        );
    };
    if rt.project().settings.browser == BrowserSetting::Off {
        println!("{} {url}", rt.ui().bold("URL:"));
    } else {
        process::open_browser(rt.ui(), &url, &rt.project().settings.browser);
    }
    Ok(0)
}

pub fn shell(rt: &dyn Runtime) -> Result<i32> {
    rt.exec_desktop(&["bash".to_string(), "-l".to_string()])
}

/// The Claude Code CLI in the sandbox, with every argument passed on verbatim. Loads the project itself: it also
/// runs before the command line is parsed (so `--help`, `-v` and `--` reach claude).
pub fn claude(ui: &Ui, verbose: bool, args: &[String]) -> Result<i32> {
    let project = Project::load(ui)?;
    let rt = runtime::select(Ctx {
        project: &project,
        verbose,
        ui,
    })?;
    let info = runtime::need_runtime(rt.as_ref())?;
    for path in compose::generate(&project, &info)?.restored {
        warn_restored(ui, &path);
    }
    let mut command = vec![
        "sh".to_string(),
        "-c".to_string(),
        CLAUDE_SCRIPT.to_string(),
        "claude".to_string(),
    ];
    command.extend_from_slice(args);
    rt.exec_desktop(&command)
}

pub fn logs(rt: &dyn Runtime, services: &[String], tail: i64, follow: bool) -> Result<i32> {
    rt.compose_logs(services, tail, follow)
}

/// The warning for an engine file that was edited by hand and restored.
pub fn warn_restored(ui: &Ui, rel: &str) {
    ui.warn(&format!(
        ".state/{rel} was edited by hand and has been restored. Generated files are overwritten; change sandbox.toml instead."
    ));
}

pub fn info_updated(ui: &Ui, rel: &str) {
    ui.info(&format!(".state/{rel} was updated to the latest version"));
}

/// Image build: the runtime's own progress output is shown as-is (it can take minutes the first time).
fn build(rt: &dyn Runtime, pull: bool) -> Result<()> {
    rt.ui().rule(if pull {
        "Building images with the newest tools"
    } else {
        "Building images"
    });
    let cache_bust = pull.then(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |t| t.as_secs())
    });
    if rt.compose_build(cache_bust)? != 0 {
        bail!("Building the sandbox images failed (see the output above).");
    }
    rt.ui().plain_rule();
    Ok(())
}

/// `up -d --wait` in the background while a live table shows each service's state and health (on a terminal).
fn start_and_wait(rt: &dyn Runtime, tolerate_unhealthy: bool) -> Result<()> {
    let up: ProcResult = if rt.ui().interactive && !rt.ctx().verbose {
        live_up(rt)?
    } else {
        rt.compose_up()
    };
    if up.exit_code == 0 {
        return Ok(());
    }
    if tolerate_unhealthy
        && rt
            .compose_ps()?
            .iter()
            .any(|s| s.service == "desktop" && s.state == "running")
    {
        rt.ui().warn("The desktop isn't reporting ready yet; opening anyway (reload the page if it's blank).");
        return Ok(());
    }
    rt.ui()
        .show_output(&format!("{} compose up", rt.program()), &up);
    if let Some(hint) = llm::gpu_hint(rt, &up) {
        rt.ui().warn(&hint);
    }
    bail!("Starting the sandbox failed (see the output above).");
}

/// Runs `compose up` on a second thread; the first one redraws the table once a second.
fn live_up(rt: &dyn Runtime) -> Result<ProcResult> {
    let term = Term::stdout();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        scope.spawn(move || {
            let _ = tx.send(rt.compose_up());
        });
        let mut drawn = 0usize;
        loop {
            let table = starting_table(rt, started)?;
            if drawn > 0 {
                let _ = term.clear_last_lines(drawn);
            }
            drawn = table.lines().count();
            println!("{table}");
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(result) => {
                    // the final state
                    let table = starting_table(rt, started)?;
                    let _ = term.clear_last_lines(drawn);
                    println!("{table}");
                    return Ok(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("The compose call ended unexpectedly.")
                }
            }
        }
    })
}

fn starting_table(rt: &dyn Runtime, started: Instant) -> Result<String> {
    let ui = rt.ui();
    let secs = started.elapsed().as_secs();
    let title = format!(
        "{} {}",
        ui.bold(format!("Starting '{}'", rt.project().name)),
        ui.dim(format!("({:02}:{:02})", secs / 60, secs % 60))
    );
    let ps = rt.compose_ps()?;
    let mut t = ui.new_table(&["Service", "State", "Health"]);
    if ps.is_empty() {
        t.add_row([ui.dim("creating containers…"), String::new(), String::new()]);
    }
    for s in &ps {
        t.add_row([
            s.service.clone(),
            ui.state(&s.state),
            ui.health(&s.health, &s.state),
        ]);
    }
    Ok(format!("{title}\n{t}"))
}
