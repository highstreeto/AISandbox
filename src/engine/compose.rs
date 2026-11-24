//! Renders `compose.yml.j2` (the embedded template) into the one compose file the container runtime runs with.
//!
//! The template holds every service, network and hardening setting literally; only the values that come from
//! `sandbox.toml` or this machine are expressions. Text values go through `q`, which writes a quoted YAML string,
//! so a path like `C:\Users\x` or a value containing `": #"` can't break or inject YAML.

use crate::config::SandboxProtection;
use crate::engine::{self, embedded};
use crate::paths;
use crate::process::user_ids;
use crate::project::{Project, inside_sandbox, write_state_ignore};
use crate::runtime::{RuntimeInfo, RuntimeKind};
use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use minijinja::value::{Serde, Value, ValueKind};
use minijinja::{AutoEscape, Environment, UndefinedBehavior, syntax::SyntaxConfig};
use serde::Serialize;
use std::fmt::Write;
use std::fs;
use std::net::{SocketAddr, ToSocketAddrs};

const TEMPLATE_NAME: &str = "compose.yml.j2";
/// What the desktop needs for nested containers (`[tools] podman`), added to `[desktop] devices` / `security_opt`.
const NESTING_DEVICES: &[&str] = &["/dev/fuse", "/dev/net/tun"];
/// `unmask=ALL` lets nested containers mount their own `/proc`; `label=disable`: no nested SELinux labels.
const NESTING_SECURITY_OPT: &[&str] = &["label=disable", "unmask=ALL"];
/// The outer proxy's port (the template's own proxy: `proxy:3128`).
const UPSTREAM_PROXY_PORT: u16 = 3128;

/// The template's variables.
#[derive(Debug, Serialize)]
struct Model {
    host: Host,
    tools: Vec<Tool>,
    desktop: Desktop,
    env: Vec<EnvVar>,
    mounts: Vec<MountModel>,
    /// The local LLM (`[llm]`); `None` = no `ollama` and `llm` services.
    llm: Option<Llm>,
}

#[derive(Debug, Serialize)]
struct Llm {
    model: String,
    context: u32,
    keep_alive: Option<String>,
    /// `nvidia`, `amd` or `none`.
    gpu: String,
}

#[derive(Debug, Serialize)]
struct Host {
    /// `docker` or `podman`.
    runtime: RuntimeKind,
    /// Rootless Podman: the desktop keeps the host user's id (`userns_mode: keep-id`).
    rootless: bool,
    /// `[tools] podman`: the desktop gets the devices, options and storage volume nested containers need.
    nesting: bool,
    /// Started inside a sandbox desktop: the proxy forwards to the outer proxy, no `hostname`, no host-gateway.
    nested: bool,
    /// Nested: the outer proxy's address (`sandbox-upstream` in the proxy container); empty otherwise.
    upstream_proxy: String,
    /// Nested: the outer proxy as a URL, passed to image builds as `http_proxy` / `https_proxy` (build args).
    build_proxy: String,
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
    /// On this machine (empty for a hiding mount).
    pub source: String,
    /// In the container.
    pub target: String,
    pub read_only: bool,
}

/// Writes `.state/.gitignore`, syncs the engine into `.state/desktop`, `.state/proxy` and `.state/llm`, and writes
/// `.state/compose.yml`, the only file docker compose runs with: the embedded template rendered with the settings
/// from `sandbox.toml` and this machine's facts (user ids, time zone, mounts, UI port).
///
/// Returns the engine files that were edited by hand and restored (the caller prints the warnings). The UI port, the
/// project mounts and the proxy settings are written by the template itself, so `sandbox.toml` has no way to change
/// them.
pub fn generate(project: &Project, runtime: &RuntimeInfo) -> Result<engine::SyncReport> {
    let state_dir = project.state_dir();
    check_writable(&state_dir)?;
    check_nesting(project, runtime)?;
    let upstream = if inside_sandbox() {
        Some(upstream_proxy()?)
    } else {
        None
    };
    write_state_ignore(&state_dir)?;
    let report = engine::sync(&state_dir)?;
    empty_dir(&state_dir.join(EMPTY_DIR))?;
    let text = render(
        &embedded::read_text(TEMPLATE_NAME)?,
        &model(project, runtime, upstream),
    )?;
    paths::write_text(
        &project.gen_path(),
        &format!("# {}\n{text}", engine::header_text()),
    )?;
    // left over from the multi-file setup
    let _ = fs::remove_file(state_dir.join("compose.generated.yml"));
    Ok(report)
}

/// The folder in `.state/` that is mounted over `.sandbox` to hide it (`protect_sandbox = "hide"`).
pub const EMPTY_DIR: &str = "empty";

/// Creates `dir`, or empties it: whatever is in it would be visible in the sandbox as `.sandbox/`.
fn empty_dir(dir: &Utf8Path) -> Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).with_context(|| format!("Could not read {dir}."))? {
            let path = entry?.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
            .with_context(|| format!("Could not delete {}.", path.display()))?;
        }
    } else {
        let _ = fs::remove_file(dir);
        paths::create_dir_all(dir)?;
    }
    Ok(())
}

/// `.state/` must be writable. Inside a sandbox `.sandbox/` is hidden or read-only (`protect_sandbox`; with
/// `workspace = "ro"` the whole project is read-only), so a nested sandbox runs from a writable copy.
fn check_writable(state_dir: &Utf8Path) -> Result<()> {
    let probe = state_dir.join(".write-test");
    let writable = fs::create_dir_all(state_dir)
        .and_then(|()| fs::write(&probe, b""))
        .is_ok();
    let _ = fs::remove_file(&probe);
    if writable {
        Ok(())
    } else if inside_sandbox() {
        bail!(
            "{state_dir} is read-only here. Copy the project to a writable folder (e.g. under /config) and run sandbox there."
        )
    } else {
        bail!(
            "{state_dir} is not writable. Make it writable, or run sandbox from a writable copy of the project."
        )
    }
}

/// `[tools] podman` (nested containers) needs rootless Podman: everything it relaxes stays in the user namespace.
fn check_nesting(project: &Project, runtime: &RuntimeInfo) -> Result<()> {
    if !project.settings.podman_tool() {
        return Ok(());
    }
    match (runtime.kind, runtime.rootless) {
        (RuntimeKind::Podman, true) => Ok(()),
        (RuntimeKind::Podman, false) => bail!(
            "podman = true under [tools] needs rootless Podman, but this Podman runs as root. Use rootless Podman \
             (Windows/macOS: podman machine set --rootful=false), or set podman = false under [tools] in sandbox.toml."
        ),
        _ => bail!(
            "podman = true under [tools] needs rootless Podman, not Docker. Set runtime = \"podman\" under [host] in \
             sandbox.local.toml, or podman = false under [tools] in sandbox.toml."
        ),
    }
}

/// Nested: the outer proxy (host of `$HTTPS_PROXY`, `proxy` in the outer group) as an IP address, because the name
/// `proxy` means the inner proxy inside the nested group.
fn upstream_proxy() -> Result<String> {
    let Some((var, url)) = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
        .iter()
        .find_map(|v| {
            std::env::var(v)
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| (*v, u))
        })
    else {
        bail!(
            "HTTPS_PROXY is not set, so the outer sandbox's proxy is unknown (a nested sandbox needs it)."
        );
    };
    let Some(host) = proxy_host(&url) else {
        bail!("{var} is \"{url}\", which has no host name.");
    };
    let addrs: Vec<SocketAddr> = (host.as_str(), UPSTREAM_PROXY_PORT)
        .to_socket_addrs()
        .map_err(|e| anyhow!("The outer proxy {host} ({var}) can't be resolved: {e}"))?
        .collect();
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .ok_or_else(|| anyhow!("The outer proxy {host} ({var}) can't be resolved."))?;
    Ok(addr.ip().to_string())
}

/// The host of a proxy URL: `http://user:pw@proxy:3128/` → `proxy`, `http://[::1]:3128` → `::1`.
fn proxy_host(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, r)| r);
    let rest = rest.split('/').next().unwrap_or_default();
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    let host = if let Some(v6) = rest.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        rest.split(':').next().unwrap_or_default()
    };
    (!host.is_empty()).then(|| host.to_string())
}

/// The outer proxy's URL for nested image builds: `10.0.0.5` → `http://10.0.0.5:3128`, IPv6 in brackets.
fn proxy_url(ip: &str) -> String {
    if ip.contains(':') {
        format!("http://[{ip}]:{UPSTREAM_PROXY_PORT}")
    } else {
        format!("http://{ip}:{UPSTREAM_PROXY_PORT}")
    }
}

/// `base` plus the entries of `extra` it doesn't have yet.
fn with_extra(base: &[String], extra: &[&str]) -> Vec<String> {
    let mut all = base.to_vec();
    for e in extra {
        if !all.iter().any(|a| a == e) {
            all.push((*e).to_string());
        }
    }
    all
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

fn model(project: &Project, runtime: &RuntimeInfo, upstream: Option<String>) -> Model {
    let s = &project.settings;
    let (uid, gid) = user_ids();
    let nesting = s.podman_tool();
    let (devices, security_opt) = if nesting {
        (
            with_extra(&s.devices, NESTING_DEVICES),
            with_extra(&s.security_opt, NESTING_SECURITY_OPT),
        )
    } else {
        (s.devices.clone(), s.security_opt.clone())
    };
    Model {
        host: Host {
            runtime: runtime.kind,
            rootless: runtime.rootless,
            nesting,
            nested: upstream.is_some(),
            build_proxy: upstream.as_deref().map(proxy_url).unwrap_or_default(),
            upstream_proxy: upstream.unwrap_or_default(),
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
            devices,
            security_opt,
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
        llm: s.llm.as_ref().map(|l| Llm {
            model: l.model.clone(),
            context: l.context,
            keep_alive: l.keep_alive.clone(),
            gpu: l.gpu.name().to_string(),
        }),
    }
}

/// Mounts of the desktop: the project, `.sandbox` (hidden or read-only, never writable: `protect_sandbox`), the read-only
/// .git (protect_git), the shares.
pub fn mounts(project: &Project) -> Vec<MountModel> {
    let s = &project.settings;
    let wd = &project.work_dir;
    let mut mounts = vec![MountModel {
        source: project.project_dir.to_string(),
        target: wd.clone(),
        read_only: s.read_only,
    }];
    let sandbox_target = format!("{wd}/{}", project.dir.file_name().unwrap_or(".sandbox"));
    match s.protect_sandbox {
        // An empty folder: the agent can't read the settings, the allowlist or `.state/compose.yml`.
        // (A tmpfs would do too, but Podman copies what is under it into it: `tmpcopyup`.)
        SandboxProtection::Hide => mounts.push(MountModel {
            source: project.state_dir().join(EMPTY_DIR).to_string(),
            target: sandbox_target,
            read_only: true,
        }),
        // Visible but read-only: the agent can't widen its allowlist (already so with `workspace = "ro"`).
        SandboxProtection::ReadOnly if !s.read_only => mounts.push(MountModel {
            source: project.dir.to_string(),
            target: sandbox_target,
            read_only: true,
        }),
        SandboxProtection::ReadOnly => {}
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
    fn proxy_url_brackets_ipv6() {
        assert_eq!(proxy_url("10.89.1.25"), "http://10.89.1.25:3128");
        assert_eq!(proxy_url("fd00::5"), "http://[fd00::5]:3128");
    }

    #[test]
    fn the_proxy_host_is_taken_from_the_url() {
        assert_eq!(proxy_host("http://proxy:3128").as_deref(), Some("proxy"));
        assert_eq!(proxy_host("http://proxy:3128/").as_deref(), Some("proxy"));
        assert_eq!(proxy_host("proxy:3128").as_deref(), Some("proxy"));
        assert_eq!(
            proxy_host(" http://u:p@10.89.0.5:3128 ").as_deref(),
            Some("10.89.0.5")
        );
        assert_eq!(
            proxy_host("http://[fd00::5]:3128").as_deref(),
            Some("fd00::5")
        );
        assert_eq!(proxy_host("http://:3128"), None);
        assert_eq!(proxy_host(""), None);
    }

    #[test]
    fn extra_entries_are_added_once() {
        let base = vec!["/dev/fuse".to_string(), "/dev/dri".to_string()];
        assert_eq!(
            with_extra(&base, NESTING_DEVICES),
            ["/dev/fuse", "/dev/dri", "/dev/net/tun"]
        );
    }

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
                runtime: RuntimeKind::Docker,
                rootless: false,
                nesting: false,
                nested: false,
                upstream_proxy: String::new(),
                build_proxy: String::new(),
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
            llm: None,
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
