//! Renders `compose.yml.j2` (the embedded template) into the one compose file docker compose runs with.
//!
//! The template holds every service, network and hardening setting literally; only the values that come from
//! `sandbox.toml` or this machine are expressions. Text values go through `q`, which writes a quoted YAML string,
//! so a path like `C:\Users\x` or a value containing `": #"` can't break or inject YAML.

use crate::engine::{self, embedded};
use crate::paths;
use crate::process::user_ids;
use crate::project::{Project, write_state_ignore};
use anyhow::{Result, anyhow};
use minijinja::value::{Serde, Value, ValueKind};
use minijinja::{AutoEscape, Environment, UndefinedBehavior, syntax::SyntaxConfig};
use serde::Serialize;
use std::fmt::Write;
use std::fs;

const TEMPLATE_NAME: &str = "compose.yml.j2";

/// The template's variables.
#[derive(Debug, Serialize)]
struct Model {
    host: Host,
    tools: Vec<Tool>,
    desktop: Desktop,
    env: Vec<EnvVar>,
    mounts: Vec<MountModel>,
}

#[derive(Debug, Serialize)]
struct Host {
    id: String,
    workdir: String,
    uid: String,
    gid: String,
    ui_port: String,
}

#[derive(Debug, Serialize)]
struct Tool {
    arg: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct Desktop {
    title: String,
    sudo: String,
    auth: Option<Auth>,
    timezone: Option<String>,
    memory: String,
    cpus: Option<String>,
    devices: Vec<String>,
    security_opt: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Auth {
    user: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct EnvVar {
    key: String,
    value: String,
}

/// A bind mount of the desktop container.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename = "mount")]
pub struct MountModel {
    /// On this machine.
    pub source: String,
    /// In the container.
    pub target: String,
    pub read_only: bool,
}

/// Writes `.state/.gitignore`, syncs the engine into `.state/desktop` and `.state/proxy`, and writes
/// `.state/compose.yml`, the only file docker compose runs with: the embedded template rendered with the settings
/// from `sandbox.toml` and this machine's facts (user ids, time zone, mounts, UI port).
///
/// Returns the engine files that were edited by hand and restored (the caller prints the warnings). The UI port, the
/// project mounts and the proxy settings are written by the template itself, so `sandbox.toml` has no way to change
/// them.
pub fn generate(project: &Project) -> Result<engine::SyncReport> {
    let state_dir = project.state_dir();
    write_state_ignore(&state_dir)?;
    let report = engine::sync(&state_dir)?;
    let text = render(&embedded::read_text(TEMPLATE_NAME)?, &model(project))?;
    paths::write_text(
        &project.gen_path(),
        &format!("# {}\n{text}", engine::header_text()),
    )?;
    // left over from the multi-file setup
    let _ = fs::remove_file(state_dir.join("compose.generated.yml"));
    Ok(report)
}

fn render(template: &str, model: &Model) -> Result<String> {
    let mut env = Environment::new();
    env.set_syntax(
        SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .keep_trailing_newline(true)
            .build()
            .map_err(|e| anyhow!("{TEMPLATE_NAME}: {e}"))?,
    );
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.add_filter("q", |v: Value| quote(&scalar_text(&v)));
    env.add_template(TEMPLATE_NAME, template)
        .map_err(|e| anyhow!("{TEMPLATE_NAME} is not a valid template: {e}"))?;
    let tmpl = env
        .get_template(TEMPLATE_NAME)
        .map_err(|e| anyhow!("{TEMPLATE_NAME}: {e}"))?;
    let text = tmpl
        .render(Serde(model))
        .map_err(|e| anyhow!("{TEMPLATE_NAME}: {e:#}"))?;
    Ok(text.replace("\r\n", "\n"))
}

/// A value as text, the way `q` writes it: none is empty, booleans are `true` / `false`.
fn scalar_text(v: &Value) -> String {
    match v.kind() {
        ValueKind::Undefined | ValueKind::None => String::new(),
        ValueKind::Bool => v.is_true().to_string(),
        _ => match v.as_str() {
            Some(s) => s.to_string(),
            None => v.to_string(),
        },
    }
}

/// A double-quoted YAML scalar: backslash, quote and control characters escaped.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' || matches!(c, '\u{7f}' | '\u{85}' | '\u{2028}' | '\u{2029}') => {
                // \uXXXX of the code unit; these are all in the BMP
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn model(project: &Project) -> Model {
    let s = &project.settings;
    let (uid, gid) = user_ids();
    Model {
        host: Host {
            id: project.id.clone(),
            workdir: project.work_dir.clone(),
            uid: uid.to_string(),
            gid: gid.to_string(),
            ui_port: s.ui_port.map(|p| p.to_string()).unwrap_or_default(),
        },
        tools: s
            .install
            .iter()
            .map(|t| Tool {
                arg: t.arg.clone(),
                value: t.value.clone(),
            })
            .collect(),
        desktop: Desktop {
            title: s.title.clone(),
            sudo: s.sudo.to_string(),
            auth: s.auth.as_ref().map(|a| Auth {
                user: a.user.clone(),
                password: a.password.clone(),
            }),
            timezone: s.time_zone.clone().or_else(host_time_zone),
            memory: s.memory.clone(),
            cpus: s.cpus.clone(),
            devices: s.devices.clone(),
            security_opt: s.security_opt.clone(),
        },
        env: s
            .env
            .iter()
            .map(|e| EnvVar {
                key: e.key.clone(),
                value: e.value.clone(),
            })
            .collect(),
        mounts: mounts(project),
    }
}

/// Bind mounts of the desktop: the project, the read-only .sandbox, the protected .git paths, the shares.
pub fn mounts(project: &Project) -> Vec<MountModel> {
    let s = &project.settings;
    let wd = &project.work_dir;
    let mut mounts = vec![MountModel {
        source: project.project_dir.to_string(),
        target: wd.clone(),
        read_only: s.read_only,
    }];
    if !s.read_only {
        // The sandbox config is visible but read-only: the agent can't widen its allowlist.
        mounts.push(MountModel {
            source: project.dir.to_string(),
            target: format!("{wd}/{}", project.dir.file_name().unwrap_or(".sandbox")),
            read_only: true,
        });
    }
    if s.protect_git && !s.read_only {
        let git_dir = project.project_dir.join(".git");
        if git_dir.exists() {
            mounts.push(MountModel {
                source: git_dir.to_string(),
                target: format!("{wd}/.git"),
                read_only: true,
            });
        }
    }
    for sh in &s.shares {
        mounts.push(MountModel {
            source: sh.host.clone(),
            target: sh.path.clone(),
            read_only: sh.read_only,
        });
    }
    mounts
}

/// `$TZ`, else this machine's IANA time zone, else none (the template leaves the `TZ:` line out).
fn host_time_zone() -> Option<String> {
    if let Some(tz) = std::env::var("TZ").ok().filter(|t| !t.trim().is_empty()) {
        return Some(tz);
    }
    iana_time_zone::get_timezone()
        .ok()
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_like_the_dotnet_original() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote("l1\nl2\r\tx"), "\"l1\\nl2\\r\\tx\"");
        assert_eq!(
            quote("\u{1}\u{7f}\u{85}\u{2028}\u{2029}"),
            "\"\\u0001\\u007f\\u0085\\u2028\\u2029\""
        );
        assert_eq!(quote("ä€😀"), "\"ä€😀\"");
        assert_eq!(quote(""), "\"\"");
    }

    fn empty_model() -> Model {
        Model {
            host: Host {
                id: "x\"y".into(),
                workdir: String::new(),
                uid: String::new(),
                gid: String::new(),
                ui_port: String::new(),
            },
            tools: vec![],
            desktop: Desktop {
                title: String::new(),
                sudo: "true".into(),
                auth: None,
                timezone: None,
                memory: String::new(),
                cpus: None,
                devices: vec![],
                security_opt: vec![],
            },
            env: vec![],
            mounts: vec![MountModel {
                source: "/a".into(),
                target: "/b".into(),
                read_only: true,
            }],
        }
    }

    #[test]
    fn the_q_filter_and_booleans() {
        let m = empty_model();
        let t = "{{ host.id | q }}|{{ desktop.sudo | q }}|{{ desktop.auth | q }}|{{ (\"a:\" ~ host.ui_port ~ \":b\") | q }}|{% for m in mounts %}{{ \"true\" if m.read_only else \"false\" }}{% endfor %}";
        assert_eq!(
            render(t, &m).unwrap(),
            "\"x\\\"y\"|\"true\"|\"\"|\"a::b\"|true"
        );
    }

    #[test]
    fn strict_undefined_variables_are_errors() {
        let m = empty_model();
        assert!(render("{{ host.nope }}", &m).is_err());
        assert!(render("{{ nothing }}", &m).is_err());
        assert!(
            render("{% if %}", &m)
                .unwrap_err()
                .to_string()
                .contains("not a valid template")
        );
    }

    #[test]
    fn blocks_leave_no_blank_lines_and_the_final_newline_stays() {
        let m = empty_model();
        let t = "a\n  {% for x in mounts %}\n  - {{ x.source }}\n  {% endfor %}\nb\n";
        assert_eq!(render(t, &m).unwrap(), "a\n  - /a\nb\n");
    }

    #[test]
    fn scalar_text_of_values() {
        assert_eq!(scalar_text(&Value::from(true)), "true");
        assert_eq!(scalar_text(&Value::from(false)), "false");
        assert_eq!(scalar_text(&Value::from(7)), "7");
        assert_eq!(scalar_text(&Value::from("x")), "x");
        assert_eq!(scalar_text(&Value::from(())), "");
    }
}
