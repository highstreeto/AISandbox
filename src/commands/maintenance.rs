//! init, reset-home and info.

use super::lifecycle;
use crate::config::{self, BrowserSetting, FILE_NAME, Settings};
use crate::engine::{self, embedded};
use crate::paths::{self, full_path};
use crate::project::{Project, check_old_layout, load_or_create_id, write_state_ignore};
use crate::runtime::{self, Runtime, RuntimeKind};
use crate::ui::Ui;
use anyhow::{Result, bail};
use console::measure_text_width;

/// The files a project owns, created by `init` from the embedded templates: (file in `.sandbox/`, template in
/// `resources/init/`). The ignore file's template has no leading dot so that it isn't a real `.gitignore` here.
const PROJECT_FILES: [(&str, &str); 3] = [
    (FILE_NAME, "sandbox.toml"),
    ("allowlist.txt", "allowlist.txt"),
    (".gitignore", "gitignore"),
];

/// Creates `<folder>/.sandbox/` with `sandbox.toml`, `allowlist.txt` and `.gitignore` (files that exist are kept),
/// the `.state` folder and the engine. Runs without runtime and ignores `SANDBOX_DIR`.
pub fn init(ui: &Ui, folder: Option<&str>) -> Result<i32> {
    let cwd = paths::current_dir()?;
    let folder = folder.filter(|f| !f.trim().is_empty()).unwrap_or(".");
    let project_dir = full_path(&cwd, folder);
    if !project_dir.is_dir() {
        bail!("Folder {project_dir} does not exist.");
    }
    let dir = project_dir.join(".sandbox");

    for parent in project_dir.ancestors().skip(1) {
        let outer = parent.join(".sandbox");
        if outer.is_dir() {
            ui.warn(&format!(
                "{outer} already exists; this creates a separate sandbox for {project_dir}"
            ));
            break;
        }
    }
    check_old_layout(ui, &dir);

    paths::create_dir_all(&dir)?;
    for (name, template) in PROJECT_FILES {
        let path = dir.join(name);
        if path.is_file() {
            println!("{}", ui.dim(format!("{name} exists \u{2014} kept")));
            continue;
        }
        paths::write_text(&path, &embedded::read_text(&format!("init/{template}"))?)?;
        ui.success(&format!("Created .sandbox/{name}"));
    }

    let state_dir = dir.join(".state");
    write_state_ignore(&state_dir)?;
    load_or_create_id(&state_dir, &project_dir)?;
    let sync_result = engine::sync(&state_dir)?;
    if sync_result.has_restored_or_updated() {
        ui.rule("Syncing .state/");
        for path in sync_result.restored {
            lifecycle::warn_restored(ui, &path);
        }
        for path in sync_result.updated {
            lifecycle::info_updated(ui, &path);
        }
    }
    ui.success("Wrote .sandbox/.state/ (desktop/ and proxy/ show what the sandbox builds)");

    println!(
        "Next: edit {}, then \"{}\".",
        ui.bold(".sandbox/sandbox.toml"),
        ui.bold("sandbox up")
    );
    Ok(0)
}

pub fn reset_home(ui: &Ui, rt: &dyn Runtime, yes: bool) -> Result<i32> {
    if !yes {
        if !ui.interactive {
            bail!("Refusing to delete the home volume without confirmation (use --yes).");
        }
        let question = format!(
            "Delete the home volume of {} (logins, IDE settings)?",
            ui.bold(&rt.project().name)
        );
        if !ui.confirm(&question) {
            return Ok(0);
        }
    }
    lifecycle::compose_command(ui, rt, "Removing containers", "down", |rt| {
        rt.compose_down(ui)
    })?;
    let vol = format!("{}_home", rt.project().name);
    let r = ui.spin(&format!("Removing volume {vol}…"), || {
        rt.volume_remove(ui, &vol)
    });
    if r.exit_code != 0 {
        ui.show_output(&format!("{} volume rm", rt.program()), &r);
        bail!("Could not remove volume {vol}.");
    }
    ui.success(&format!("Home volume {vol} deleted."));
    Ok(0)
}

pub fn info(ui: &Ui, project: &Project) -> Result<i32> {
    let st = &project.settings;

    let rows: Vec<(String, String)> = vec![
        ("Project".into(), project.name.clone()),
        ("Project folder".into(), project.project_dir.to_string()),
        ("Mounted at".into(), project.work_dir.clone()),
        ("ID".into(), project.id.clone()),
        (
            "Workspace".into(),
            if st.read_only {
                "read-only"
            } else {
                "read-write"
            }
            .into(),
        ),
        ("Protect .git".into(), yes_no(st.protect_git).into()),
        (
            "UI port".into(),
            st.ui_port
                .map_or("random free port".to_string(), |p| p.to_string()),
        ),
        ("Browser".into(), browser_text(st)),
        ("Runtime".into(), runtime_text(project)),
        ("LLM".into(), llm_text(st)),
        (
            "Engine".into(),
            format!(
                "sandbox {}, engine {} \u{2192} .sandbox/.state/desktop, proxy (rewritten before each command)",
                engine::short_app_version(),
                engine::short_engine_hash()
            ),
        ),
    ];
    ui.print_panel("Sandbox", &grid(ui, &rows), None, false);

    let mut rows: Vec<(String, String)> = vec![
        ("Title".into(), st.title.clone()),
        ("Sudo".into(), yes_no(st.sudo).into()),
        ("Memory".into(), st.memory.clone()),
        (
            "CPUs".into(),
            st.cpus.clone().unwrap_or_else(|| "no limit".into()),
        ),
        (
            "Time zone".into(),
            st.time_zone
                .clone()
                .unwrap_or_else(|| "from this machine".into()),
        ),
        (
            "Login".into(),
            st.auth
                .as_ref()
                .map_or("none".to_string(), |a| format!("basic auth ({})", a.user)),
        ),
    ];
    if !st.devices.is_empty() {
        rows.push(("Devices".into(), st.devices.join(", ")));
    }
    if !st.security_opt.is_empty() {
        rows.push(("Security options".into(), st.security_opt.join(", ")));
    }
    for e in &st.env {
        rows.push((format!("${}", e.key), e.value.clone()));
    }
    ui.print_panel("Desktop", &grid(ui, &rows), None, false);

    let mut tools = ui.new_table(&["Tool", "Value"]);
    for t in &st.install {
        let off = t.value.is_empty() || t.value == "false";
        let value = if off {
            ui.dim("not installed")
        } else if t.value == "true" {
            ui.green("default version")
        } else {
            ui.green(&t.value)
        };
        tools.add_row([config::tool_name(&t.arg), value]);
    }
    ui.print_table(Some(&ui.bold("Tools [tools]")), &tools, None);

    if !st.shares.is_empty() {
        let mut shares = ui.new_table(&["On this machine", "In the sandbox", "Access"]);
        for s in &st.shares {
            shares.add_row([
                s.host.clone(),
                s.path.clone(),
                if s.read_only {
                    "read-only"
                } else {
                    "read-write"
                }
                .to_string(),
            ]);
        }
        ui.print_table(Some(&ui.bold("Shared folders [[share]]")), &shares, None);
    }
    Ok(0)
}

/// The runtime commands would use and why, without running it: "Podman (auto)",
/// "Docker (SANDBOX_RUNTIME=docker): not installed".
fn runtime_text(project: &Project) -> String {
    let (setting, from) = match runtime::requested(project) {
        Ok(r) => r,
        Err(e) => return e.to_string(),
    };
    match (
        runtime::available(setting),
        RuntimeKind::from_setting(setting),
    ) {
        (Some(kind), _) => format!("{} ({from})", kind.name()),
        (None, Some(kind)) => format!("{} ({from}): not installed", kind.name()),
        (None, None) => "neither Podman nor Docker is installed".to_string(),
    }
}

/// `[host] browser` and the file it comes from, e.g. `chromium --incognito (sandbox.local.toml)`.
fn browser_text(st: &Settings) -> String {
    let what = match &st.browser {
        BrowserSetting::Default => "system default".to_string(),
        BrowserSetting::Off => "off (nothing is opened)".to_string(),
        BrowserSetting::Command(cmd) => cmd.join(" "),
    };
    match st.browser_source.file_name() {
        Some(file) => format!("{what} ({file})"),
        None => what,
    }
}

/// `[llm]`: `qwen3.6:27b (sandbox.toml), GPU nvidia, http://llm:11434`, or why it is off.
fn llm_text(st: &Settings) -> String {
    let Some(llm) = &st.llm else {
        return if st.llm_off_locally {
            format!("off ({})", config::LOCAL_FILE_NAME)
        } else {
            "off".to_string()
        };
    };
    let from = llm
        .model_source
        .file_name()
        .map_or("built-in default", |f| f);
    let mut text = format!(
        "{} ({from}), GPU {}, {}",
        llm.model,
        llm.gpu.name(),
        super::llm::ENDPOINT
    );
    if !llm.extra_models.is_empty() {
        text.push_str(&format!(", also {}", llm.extra_models.join(", ")));
    }
    text
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Key/value rows with the keys in bold, padded to the same width plus three spaces.
fn grid(ui: &Ui, rows: &[(String, String)]) -> Vec<String> {
    let width = rows
        .iter()
        .map(|(k, _)| measure_text_width(k))
        .max()
        .unwrap_or(0)
        + 3;
    rows.iter()
        .map(|(k, v)| {
            format!(
                "{}{}{v}",
                ui.bold(k),
                " ".repeat(width - measure_text_width(k))
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_pads_keys_to_the_longest_plus_three() {
        console::set_colors_enabled(false);
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let g = grid(
            &ui,
            &[("ID".into(), "x".into()), ("Project".into(), "y".into())],
        );
        assert_eq!(g, vec!["ID        x", "Project   y"]);
    }
}
