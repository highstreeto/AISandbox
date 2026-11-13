//! Building, starting, stopping and using the sandbox: up, rebuild, stop/down/restart, status, open, shell, claude,
//! logs.

use crate::compose;
use crate::docker::Docker;
use crate::process::{self, ProcResult};
use crate::project::Project;
use crate::ui::Ui;
use anyhow::{Result, bail};
use console::{Color, Term};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CLAUDE_SCRIPT: &str = "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set claude_code under [tools] in sandbox.toml and rebuild.\"; exit 1; }; exec claude \"$@\"";

pub fn up(d: &Docker, open_browser: bool) -> Result<i32> {
    build(d, false)?;
    start_and_wait(d, true)?;
    std::thread::sleep(Duration::from_secs(2));
    let Some(url) = d.ui_url() else {
        bail!("The sandbox started, but its UI port could not be determined.");
    };
    let ui = d.ui;
    let p = d.project;
    let lines = [
        format!("{}   {}", ui.bold("Project"), p.name),
        format!("{}       {url}", ui.bold("URL")),
        format!("{}   {}", ui.bold("Mounted"), p.work_dir),
        ui.dim("Accept the self-signed certificate in the browser."),
    ];
    ui.print_panel("Sandbox running", &lines, Some(Color::Green), false);
    if open_browser {
        process::open_browser(&url);
    }
    Ok(0)
}

pub fn rebuild(d: &Docker, pull: bool) -> Result<i32> {
    build(d, pull)?;
    start_and_wait(d, false)?;
    d.ui.success("Rebuilt and running.");
    Ok(0)
}

/// Runs one docker compose command with a spinner (stop, down, restart).
pub fn compose_command(d: &Docker, what: &str, args: &[&str]) -> Result<i32> {
    let name = &d.project.name;
    let r = d.ui.spin(&format!("{what} '{name}'…"), || d.compose(args));
    if r.exit_code != 0 {
        d.ui.show_output(&format!("docker compose {}", args.join(" ")), &r);
        bail!("{what} failed.");
    }
    d.ui.success(&format!("{what} '{name}': done."));
    Ok(0)
}

pub fn status(d: &Docker) -> Result<i32> {
    let ui = d.ui;
    let name = &d.project.name;
    let ps = d.services()?;
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
    if let Some(url) = d.ui_url() {
        println!("{} {url}", ui.bold("URL:"));
    }
    Ok(0)
}

pub fn open(d: &Docker) -> Result<i32> {
    let Some(url) = d.ui_url() else {
        bail!(
            "The sandbox is not running (start it with: {} up).",
            d.ui.self_name()
        );
    };
    process::open_browser(&url);
    Ok(0)
}

pub fn shell(d: &Docker) -> Result<i32> {
    d.exec(&["bash", "-l"])
}

/// The Claude Code CLI in the sandbox, with every argument passed on verbatim. Loads the project itself: it also
/// runs before the command line is parsed (so `--help`, `-v` and `--` reach claude).
pub fn claude(ui: &Ui, verbose: bool, args: &[String]) -> Result<i32> {
    let project = Project::load(ui)?;
    let d = Docker::new(&project, verbose, ui);
    d.need_docker()?;
    d.check_collision()?;
    for path in compose::generate(&project)?.restored {
        warn_restored(ui, &path);
    }
    let mut command = vec![
        "sh".to_string(),
        "-c".to_string(),
        CLAUDE_SCRIPT.to_string(),
        "claude".to_string(),
    ];
    command.extend_from_slice(args);
    d.exec(&command)
}

pub fn logs(d: &Docker, services: &[String], tail: i64, follow: bool) -> Result<i32> {
    let mut args = vec!["logs".to_string(), format!("--tail={tail}")];
    if follow {
        args.push("-f".to_string());
    }
    args.extend_from_slice(services);
    d.compose_interactive(&args)
}

/// The warning for an engine file that was edited by hand and restored.
pub fn warn_restored(ui: &Ui, rel: &str) {
    ui.warn(&format!(
        ".state/{rel} was edited by hand and has been restored. Generated files are overwritten; change sandbox.toml instead."
    ));
}

/// Image build: docker's own progress output is shown as-is (it can take minutes the first time).
fn build(d: &Docker, pull: bool) -> Result<()> {
    d.ui.rule(if pull {
        "Building images with the newest tools"
    } else {
        "Building images"
    });
    let args: Vec<String> = if pull {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |t| t.as_secs());
        ["build", "--pull", "--build-arg"]
            .iter()
            .map(|s| s.to_string())
            .chain([format!("CACHE_BUST={now}")])
            .collect()
    } else {
        vec!["build".to_string()]
    };
    if d.compose_interactive(&args)? != 0 {
        bail!("Building the sandbox images failed (see the output above).");
    }
    d.ui.plain_rule();
    Ok(())
}

/// `up -d --wait` in the background while a live table shows each service's state and health (on a terminal).
fn start_and_wait(d: &Docker, tolerate_unhealthy: bool) -> Result<()> {
    let up_args = ["up", "-d", "--wait", "--wait-timeout", "300"];
    let up: ProcResult = if d.ui.interactive {
        live_up(d, &up_args)?
    } else {
        d.compose(&up_args)
    };
    if up.exit_code == 0 {
        return Ok(());
    }
    if tolerate_unhealthy
        && d.services()?
            .iter()
            .any(|s| s.service == "desktop" && s.state == "running")
    {
        d.ui.warn("The desktop isn't reporting ready yet; opening anyway (reload the page if it's blank).");
        return Ok(());
    }
    d.ui.show_output("docker compose up", &up);
    bail!("Starting the sandbox failed (see the output above).");
}

/// Runs `compose up` on a second thread; the first one redraws the table once a second.
fn live_up(d: &Docker, up_args: &[&str]) -> Result<ProcResult> {
    let term = Term::stdout();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        scope.spawn(move || {
            let _ = tx.send(d.compose(up_args));
        });
        let mut drawn = 0usize;
        loop {
            let table = starting_table(d, started)?;
            if drawn > 0 {
                let _ = term.clear_last_lines(drawn);
            }
            drawn = table.lines().count();
            println!("{table}");
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(result) => {
                    // the final state
                    let table = starting_table(d, started)?;
                    let _ = term.clear_last_lines(drawn);
                    println!("{table}");
                    return Ok(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("The docker compose call ended unexpectedly.")
                }
            }
        }
    })
}

fn starting_table(d: &Docker, started: Instant) -> Result<String> {
    let ui = d.ui;
    let secs = started.elapsed().as_secs();
    let title = format!(
        "{} {}",
        ui.bold(format!("Starting '{}'", d.project.name)),
        ui.dim(format!("({:02}:{:02})", secs / 60, secs % 60))
    );
    let ps = d.services()?;
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
