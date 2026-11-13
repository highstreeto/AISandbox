//! Reads `sandbox.toml` (and the optional `sandbox.local.toml`). Every key is known and typed: unknown keys, wrong types and invalid values are errors.

pub mod reader;

use crate::paths::{self, full_path};
use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use reader::TableReader;
use regex::Regex;
use std::sync::LazyLock;
use toml::{Table, Value};

pub const FILE_NAME: &str = "sandbox.toml";
/// Personal overrides next to `sandbox.toml`; not committed. Only `[host]` can be set in it.
pub const LOCAL_FILE_NAME: &str = "sandbox.local.toml";

/// Environment variables the sandbox sets itself, with where to change them instead (`None` = not at all).
const RESERVED_ENV: &[(&str, Option<&str>)] = &[
    ("HTTP_PROXY", None),
    ("HTTPS_PROXY", None),
    ("NO_PROXY", None),
    ("SANDBOX_WORKDIR", None),
    ("PUID", None),
    ("PGID", None),
    ("TZ", Some("desktop.timezone")),
    ("TITLE", Some("desktop.title")),
    ("SANDBOX_SUDO", Some("desktop.sudo")),
    ("CUSTOM_USER", Some("desktop.auth")),
    ("PASSWORD", Some("desktop.auth")),
];

/// The validated settings from `sandbox.toml`.
#[derive(Debug, Clone)]
pub struct Settings {
    pub project_name: Option<String>,
    pub read_only: bool,
    pub protect_git: bool,
    pub ui_port: Option<u16>,
    pub install: Vec<ToolSetting>,
    pub title: String,
    pub sudo: bool,
    pub memory: String,
    pub cpus: Option<String>,
    pub time_zone: Option<String>,
    pub devices: Vec<String>,
    pub security_opt: Vec<String>,
    pub auth: Option<BasicAuth>,
    pub env: Vec<EnvVariable>,
    pub shares: Vec<Share>,
    pub browser: BrowserSetting,
    /// Which file `browser` is from.
    pub browser_source: BrowserSource,
}

/// `[host] browser`: how `up` and `open` show the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserSetting {
    /// The system's default browser.
    Default,
    /// Never open a browser.
    Off,
    /// A program and its arguments; the URL is added as the last one, or where `{url}` is.
    Command(Vec<String>),
}

/// Where a setting that can come from either file was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSource {
    /// Not set anywhere.
    Default,
    Main,
    Local,
}

impl BrowserSource {
    /// The file name for messages; `None` when the setting was not set.
    pub fn file_name(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Main => Some(FILE_NAME),
            Self::Local => Some(LOCAL_FILE_NAME),
        }
    }
}

/// One `[tools]` entry as a Dockerfile build arg, e.g. `INSTALL_DOTNET = "10.0 8.0"`.
#[derive(Debug, Clone)]
pub struct ToolSetting {
    pub arg: String,
    pub value: String,
}

/// One `[env]` entry.
#[derive(Debug, Clone)]
pub struct EnvVariable {
    pub key: String,
    pub value: String,
}

/// Basic auth for the web desktop (`desktop.auth`).
#[derive(Debug, Clone)]
pub struct BasicAuth {
    pub user: String,
    pub password: String,
}

/// One `[[share]]` block: an extra folder mounted into the desktop.
#[derive(Debug, Clone)]
pub struct Share {
    /// Full path on this machine.
    pub host: String,
    /// Absolute path in the sandbox.
    pub path: String,
    pub read_only: bool,
}

static TOOL_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z0-9_]+$").unwrap());
static MEMORY_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\d+(\.\d+)?\s*[bkmg]?b?$").unwrap());
static ENV_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").unwrap());
static INSTALL_ARG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*ARG\s+(INSTALL_[A-Z0-9_]+)").unwrap());

/// An error about `sandbox.toml`: the message is prefixed with the file name.
pub fn error(message: impl AsRef<str>) -> anyhow::Error {
    error_in(FILE_NAME, message)
}

/// An error about the given settings file: the message is prefixed with its name.
pub fn error_in(file: &str, message: impl AsRef<str>) -> anyhow::Error {
    anyhow!("{file}: {}", message.as_ref())
}

/// A TOML float as written in messages and environment values: at least one decimal, at most four (`0.0###`).
pub fn format_number(d: f64) -> String {
    let fixed = format!("{d:.4}");
    match fixed.split_once('.') {
        Some((int, frac)) => {
            let frac = frac.trim_end_matches('0');
            format!("{int}.{}", if frac.is_empty() { "0" } else { frac })
        }
        None => fixed,
    }
}

/// `cpus` as written to the compose file: up to three decimals (`0.###`): `4`, `2.5`.
pub fn format_cpus(d: f64) -> String {
    let fixed = format!("{d:.3}");
    match fixed.split_once('.') {
        Some((int, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() {
                int.to_string()
            } else {
                format!("{int}.{frac}")
            }
        }
        None => fixed,
    }
}

/// INSTALL_CLAUDE_CODE → claude_code
pub fn tool_name(arg: &str) -> String {
    arg.trim_start_matches("INSTALL_").to_lowercase()
}

/// The INSTALL_* build args the Dockerfile declares, in its order.
pub fn known_tools(dockerfile: &str) -> Vec<String> {
    let mut known: Vec<String> = Vec::new();
    for line in dockerfile.replace("\r\n", "\n").split('\n') {
        if let Some(m) = INSTALL_ARG.captures(line) {
            let arg = m[1].to_string();
            if !known.contains(&arg) {
                known.push(arg);
            }
        }
    }
    known
}

/// Reads and validates `sandbox.toml`.
///
/// * `project_dir`: relative share paths are resolved against it.
/// * `dockerfile`: the text of the desktop Dockerfile; its `ARG INSTALL_*` lines are the known tools.
pub fn read(path: &Utf8Path, project_dir: &Utf8Path, dockerfile: &str) -> Result<Settings> {
    if !path.is_file() {
        return Err(anyhow!("{FILE_NAME} not found."));
    }
    let root = parse_file(FILE_NAME, path)?;

    let mut r = TableReader::new(&root, "");
    let mut project = r.table("project")?;
    let mut tools = r.table("tools")?;
    let mut desktop = r.table("desktop")?;
    let mut env = r.table("env")?;
    let mut shares = r.table_array("share")?;
    let mut host = r.table("host")?;
    r.done("table")?;

    // --- [project] ---
    let name = project.str("name")?;
    let workspace = project.str("workspace")?.unwrap_or("rw");
    if workspace != "rw" && workspace != "ro" {
        return Err(error(format!(
            "project.workspace must be \"rw\" or \"ro\" (got \"{workspace}\")."
        )));
    }
    let protect_git = project.bool("protect_git")?.unwrap_or(true);
    let port = project.int("ui_port")?;
    let ui_port = match port {
        None => None,
        Some(p) => Some(u16::try_from(p).ok().filter(|p| *p > 0).ok_or_else(|| {
            error(format!(
                "project.ui_port must be a port number from 1 to 65535 (got {p})."
            ))
        })?),
    };
    project.done("key")?;

    // --- [tools] ---
    let known = known_tools(dockerfile);
    let mut install = Vec::new();
    for key in tools.keys() {
        let arg = format!("INSTALL_{}", key.to_uppercase());
        if !TOOL_KEY.is_match(&key) || !known.contains(&arg) {
            let names: Vec<String> = known.iter().map(|a| tool_name(a)).collect();
            return Err(error(format!(
                "unknown tool tools.{key} (known: {}).",
                names.join(", ")
            )));
        }
        let value = tool_value(&key, tools.raw(&key))?;
        install.push(ToolSetting { arg, value });
    }
    tools.done("key")?;

    // --- [desktop] ---
    let title = desktop.str("title")?.unwrap_or("AI Sandbox").to_string();
    let sudo = desktop.bool("sudo")?.unwrap_or(true);
    let memory = desktop.str("memory")?.unwrap_or("8g").to_string();
    if !MEMORY_SIZE.is_match(&memory) {
        return Err(error(format!(
            "desktop.memory must be a size like \"8g\" or \"512m\" (got \"{memory}\")."
        )));
    }
    let cpus = desktop.number("cpus")?;
    if let Some(c) = cpus
        && c <= 0.0
    {
        return Err(error(format!(
            "desktop.cpus must be greater than 0 (got {}).",
            format_number(c)
        )));
    }
    let tz = desktop.str("timezone")?;
    let devices = desktop.str_list("devices")?;
    let security_opt = desktop.str_list("security_opt")?;
    let mut auth = None;
    if let Some(mut a) = desktop.opt_table("auth")? {
        let user = a.str("user")?.unwrap_or("");
        let password = a.str("password")?.unwrap_or("");
        a.done("key")?;
        if user.is_empty() || password.is_empty() {
            return Err(error("desktop.auth needs both user and password."));
        }
        auth = Some(BasicAuth {
            user: user.to_string(),
            password: password.to_string(),
        });
    }
    desktop.done("key")?;

    // --- [env] ---
    let mut env_vars = Vec::new();
    for key in env.keys() {
        if !ENV_NAME.is_match(&key) {
            return Err(error(format!(
                "env.{key} is not a valid environment variable name."
            )));
        }
        if let Some((_, instead)) = RESERVED_ENV
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&key))
        {
            return Err(error(match instead {
                None => format!("env.{key} is set by the sandbox and can't be changed."),
                Some(instead) => format!("env.{key} is set by the sandbox; use {instead} instead."),
            }));
        }
        let value = env.scalar(&key)?;
        env_vars.push(EnvVariable { key, value });
    }
    env.done("key")?;

    // --- [[share]] ---
    let mut share_list: Vec<Share> = Vec::new();
    for (i, s) in shares.iter_mut().enumerate() {
        let n = i + 1;
        let host = s.str("host")?.ok_or_else(|| {
            error(format!(
                "share #{n} needs host = \"<folder on this machine>\"."
            ))
        })?;
        let full = full_path(project_dir, host);
        if !full.exists() {
            return Err(error(format!(
                "share #{n}: {host} does not exist ({full})."
            )));
        }
        let target = match s.str("path")? {
            Some(p) => p.to_string(),
            None => format!("/shares/{}", file_name(&full)),
        };
        if !target.starts_with('/') {
            return Err(error(format!(
                "share #{n}: path must be absolute (got \"{target}\")."
            )));
        }
        let t = target.trim_end_matches('/');
        if t.is_empty() || t == "/config" || t == "/work" || t.starts_with("/work/") {
            return Err(error(format!(
                "share #{n}: path {target} is reserved (/, /config and /work are used by the sandbox)."
            )));
        }
        if share_list.iter().any(|x| x.path.trim_end_matches('/') == t) {
            return Err(error(format!("share #{n}: path {target} is used twice.")));
        }
        let read_only = s.bool("readonly")?.unwrap_or(true);
        share_list.push(Share {
            host: full.into_string(),
            path: target,
            read_only,
        });
        s.done("key")?;
    }

    // --- [host] ---
    let mut browser = (BrowserSetting::Default, BrowserSource::Default);
    if let Some(b) = read_host(&mut host, FILE_NAME)? {
        browser = (b, BrowserSource::Main);
    }
    if let Some(b) = read_local(&path.with_file_name(LOCAL_FILE_NAME))? {
        browser = (b, BrowserSource::Local);
    }

    Ok(Settings {
        project_name: name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string),
        read_only: workspace == "ro",
        protect_git,
        ui_port,
        install,
        title,
        sudo,
        memory,
        cpus: cpus.map(format_cpus),
        time_zone: tz
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        devices,
        security_opt,
        auth,
        env: env_vars,
        shares: share_list,
        browser: browser.0,
        browser_source: browser.1,
    })
}

/// Reads and parses a settings file; a syntax error names the file, line and column.
fn parse_file(name: &str, path: &Utf8Path) -> Result<Table> {
    let text = paths::read_text(path)?;
    parse_text(name, &text)
}

fn parse_text(name: &str, text: &str) -> Result<Table> {
    text.parse::<Table>().map_err(|e| {
        let (line, column) = e.span().map_or((1, 1), |s| line_and_column(text, s.start));
        anyhow!(
            "{name} is not valid TOML (line {line}, column {column}): {}",
            e.message()
        )
    })
}

/// `sandbox.local.toml`: its `[host] browser`, if the file exists and sets it. Nothing but `[host]` is allowed in it.
fn read_local(path: &Utf8Path) -> Result<Option<BrowserSetting>> {
    if !path.is_file() {
        return Ok(None);
    }
    local_host(&parse_file(LOCAL_FILE_NAME, path)?)
}

fn local_host(root: &Table) -> Result<Option<BrowserSetting>> {
    if let Some(other) = root.keys().find(|k| *k != "host") {
        return Err(error_in(
            LOCAL_FILE_NAME,
            format!("only [host] can be set here (found '{other}')."),
        ));
    }
    let mut r = TableReader::new_in(LOCAL_FILE_NAME, root, "");
    let mut host = r.table("host")?;
    r.done("table")?;
    read_host(&mut host, LOCAL_FILE_NAME)
}

/// The `[host]` section of either file: `Some` if `browser` is set.
fn read_host(host: &mut TableReader, file: &str) -> Result<Option<BrowserSetting>> {
    let setting = match host.raw("browser") {
        None => None,
        Some(v) => Some(browser_value(file, v)?),
    };
    host.done("key")?;
    Ok(setting)
}

/// `"default"` / true, false, a command (`"firefox"`) or a command with arguments (`["chromium", "--incognito"]`).
fn browser_value(file: &str, v: &Value) -> Result<BrowserSetting> {
    const URL: &str = "{url}";
    let fail = |message: String| error_in(file, message);
    match v {
        Value::Boolean(true) => Ok(BrowserSetting::Default),
        Value::Boolean(false) => Ok(BrowserSetting::Off),
        Value::String(s) => match s.trim() {
            "" => Err(fail(
                "host.browser must not be empty (use \"default\", or false for no browser).".into(),
            )),
            "default" => Ok(BrowserSetting::Default),
            program => Ok(BrowserSetting::Command(vec![program.to_string()])),
        },
        Value::Array(items) => {
            let mut parts = Vec::new();
            for (i, x) in items.iter().enumerate() {
                match x {
                    Value::String(s) => parts.push(s.clone()),
                    other => {
                        return Err(fail(format!(
                            "host.browser must be a list of texts, e.g. [\"chromium\", \"--incognito\"] (item {} is {}).",
                            i + 1,
                            reader::show(other)
                        )));
                    }
                }
            }
            if parts.is_empty() {
                return Err(fail(
                    "host.browser must not be an empty list; start it with the program, e.g. [\"chromium\", \"--incognito\"]."
                        .into(),
                ));
            }
            if parts[0].trim().is_empty() {
                return Err(fail(
                    "host.browser: the first item must be the program, not an empty text.".into(),
                ));
            }
            if parts[0].contains(URL) {
                return Err(fail(format!(
                    "host.browser: the first item is the program and can't contain {URL}."
                )));
            }
            if parts.iter().map(|p| p.matches(URL).count()).sum::<usize>() > 1 {
                return Err(fail(format!(
                    "host.browser can contain {URL} only once (it is replaced by the address)."
                )));
            }
            Ok(BrowserSetting::Command(parts))
        }
        other => Err(fail(format!(
            "host.browser must be true, false, a text or a list of texts (got {}).",
            reader::show(other)
        ))),
    }
}

/// The last component of a path (`Path.GetFileName`).
fn file_name(p: &Utf8PathBuf) -> String {
    p.file_name().unwrap_or("").to_string()
}

/// 1-based line and column (in characters) of a byte offset.
fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

/// true/false, a version ("10.0", 24) or a list (["10.0", "8.0"]) → the build arg value.
fn tool_value(key: &str, v: Option<&Value>) -> Result<String> {
    match v {
        Some(Value::Boolean(b)) => Ok(b.to_string()),
        Some(Value::Integer(n)) => Ok(n.to_string()),
        Some(Value::String(s)) => Ok(s.trim().to_string()),
        Some(Value::Float(d)) => {
            let n = format_number(*d);
            Err(error(format!(
                "tools.{key} = {n}: write versions as text, e.g. \"{n}\" (a number like 10.0 would lose its \".0\")."
            )))
        }
        Some(Value::Array(items)) => {
            let mut parts = Vec::new();
            for x in items {
                match x {
                    Value::String(s) => parts.push(s.trim().to_string()),
                    Value::Integer(n) => parts.push(n.to_string()),
                    _ => {
                        return Err(error(format!(
                            "tools.{key} must be a list of names or versions."
                        )));
                    }
                }
            }
            Ok(parts.join(" "))
        }
        _ => Err(error(format!(
            "tools.{key} must be true, false, a version or a list."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKERFILE: &str = "FROM x\nARG INSTALL_APT=\nARG INSTALL_CLAUDE=true\n  ARG INSTALL_NODE=false\nARG INSTALL_APT=again\nARG OTHER=1\n";

    fn read_str(toml: &str) -> Result<Settings> {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(dir.join(FILE_NAME), toml).unwrap();
        read(&dir.join(FILE_NAME), dir, DOCKERFILE)
    }

    #[test]
    fn numbers() {
        assert_eq!(format_number(10.0), "10.0");
        assert_eq!(format_number(0.0), "0.0");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1.23456), "1.2346");
        assert_eq!(format_cpus(4.0), "4");
        assert_eq!(format_cpus(2.5), "2.5");
        assert_eq!(format_cpus(0.125), "0.125");
        assert_eq!(format_cpus(0.3333), "0.333");
    }

    #[test]
    fn known_tools_come_from_the_dockerfile_in_order() {
        assert_eq!(
            known_tools(DOCKERFILE),
            vec!["INSTALL_APT", "INSTALL_CLAUDE", "INSTALL_NODE"]
        );
        assert_eq!(tool_name("INSTALL_CLAUDE_CODE"), "claude_code");
    }

    #[test]
    fn line_and_column_count_characters() {
        assert_eq!(line_and_column("[project]\nworkspace = \n", 22), (2, 13));
        assert_eq!(line_and_column("äö = \n", 7), (1, 6));
    }

    #[test]
    fn defaults() {
        let s = read_str("").unwrap();
        assert!(!s.read_only && s.protect_git && s.sudo);
        assert_eq!((s.title.as_str(), s.memory.as_str()), ("AI Sandbox", "8g"));
        assert!(s.install.is_empty() && s.cpus.is_none() && s.auth.is_none());
    }

    #[test]
    fn tools_keep_file_order_and_values() {
        let s =
            read_str("[tools]\nnode = 24\nclaude = true\napt = [\"git\", \" curl \"]\n").unwrap();
        let got: Vec<(&str, &str)> = s
            .install
            .iter()
            .map(|t| (t.arg.as_str(), t.value.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("INSTALL_NODE", "24"),
                ("INSTALL_CLAUDE", "true"),
                ("INSTALL_APT", "git curl")
            ]
        );
    }

    #[test]
    fn name_and_timezone_are_trimmed_and_blank_means_unset() {
        let s =
            read_str("[project]\nname = \"  \"\n[desktop]\ntimezone = \" Asia/Tokyo \"\n").unwrap();
        assert_eq!(s.project_name, None);
        assert_eq!(s.time_zone.as_deref(), Some("Asia/Tokyo"));
    }

    #[test]
    fn reserved_env_names_are_compared_case_insensitively() {
        let e = read_str("[env]\nhttp_proxy = \"x\"\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "sandbox.toml: env.http_proxy is set by the sandbox and can't be changed."
        );
    }

    #[test]
    fn a_share_without_a_path_goes_to_shares_and_its_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::create_dir_all(dir.join("proj/docs")).unwrap();
        std::fs::write(
            dir.join("proj/sandbox.toml"),
            "[[share]]\nhost = \"../proj/./docs/\"\n",
        )
        .unwrap();
        let s = read(
            &dir.join("proj/sandbox.toml"),
            &dir.join("proj"),
            DOCKERFILE,
        )
        .unwrap();
        assert_eq!(s.shares[0].path, "/shares/docs");
        assert_eq!(s.shares[0].host, dir.join("proj").join("docs").as_str());
        assert!(s.shares[0].read_only);
    }

    fn read_with_local(main: &str, local: &str) -> Result<Settings> {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(dir.join(FILE_NAME), main).unwrap();
        std::fs::write(dir.join(LOCAL_FILE_NAME), local).unwrap();
        read(&dir.join(FILE_NAME), dir, DOCKERFILE)
    }

    fn command(items: &[&str]) -> BrowserSetting {
        BrowserSetting::Command(items.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn the_browser_is_the_default_unless_set() {
        let s = read_str("").unwrap();
        assert_eq!(s.browser, BrowserSetting::Default);
        assert_eq!(s.browser_source, BrowserSource::Default);
        assert_eq!(s.browser_source.file_name(), None);
    }

    #[test]
    fn every_form_of_the_browser() {
        let cases: [(&str, BrowserSetting); 6] = [
            ("\"default\"", BrowserSetting::Default),
            ("true", BrowserSetting::Default),
            ("false", BrowserSetting::Off),
            ("\" firefox \"", command(&["firefox"])),
            (
                "[\"chromium\", \"--incognito\"]",
                command(&["chromium", "--incognito"]),
            ),
            (
                "[\"chrome\", \"--app={url}\"]",
                command(&["chrome", "--app={url}"]),
            ),
        ];
        for (value, expected) in cases {
            let s = read_str(&format!("[host]\nbrowser = {value}\n")).unwrap();
            assert_eq!(s.browser, expected, "{value}");
            assert_eq!(s.browser_source, BrowserSource::Main, "{value}");
        }
    }

    #[test]
    fn invalid_browsers_are_errors_naming_the_file() {
        let cases = [
            ("\"\"", "host.browser must not be empty"),
            ("\"  \"", "host.browser must not be empty"),
            ("[]", "host.browser must not be an empty list"),
            ("[\"\"]", "host.browser: the first item must be the program"),
            (
                "[\"a\", 5]",
                "host.browser must be a list of texts, e.g. [\"chromium\", \"--incognito\"] (item 2 is 5)",
            ),
            (
                "[\"a{url}\"]",
                "the first item is the program and can't contain {url}",
            ),
            (
                "[\"a\", \"{url}\", \"x={url}\"]",
                "host.browser can contain {url} only once",
            ),
            (
                "[\"a\", \"{url}{url}\"]",
                "host.browser can contain {url} only once",
            ),
            (
                "5",
                "host.browser must be true, false, a text or a list of texts (got 5)",
            ),
        ];
        for (value, message) in cases {
            let e = read_str(&format!("[host]\nbrowser = {value}\n"))
                .unwrap_err()
                .to_string();
            assert!(e.starts_with("sandbox.toml: "), "{e}");
            assert!(e.contains(message), "{value}: {e}");
        }
    }

    #[test]
    fn unknown_host_keys_are_errors() {
        assert_eq!(
            read_str("[host]\nbrowzer = 1\n").unwrap_err().to_string(),
            "sandbox.toml: unknown key 'host.browzer' (known: browser)."
        );
    }

    #[test]
    fn the_local_file_overrides_the_main_file() {
        let s = read_with_local(
            "[host]\nbrowser = \"firefox\"\n",
            "[host]\nbrowser = false\n",
        )
        .unwrap();
        assert_eq!(s.browser, BrowserSetting::Off);
        assert_eq!(s.browser_source, BrowserSource::Local);
        assert_eq!(s.browser_source.file_name(), Some("sandbox.local.toml"));

        // a local file that sets nothing leaves the main file's value
        let s = read_with_local("[host]\nbrowser = \"firefox\"\n", "[host]\n").unwrap();
        assert_eq!(s.browser, command(&["firefox"]));
        assert_eq!(s.browser_source, BrowserSource::Main);
        let s = read_with_local("", "").unwrap();
        assert_eq!(s.browser_source, BrowserSource::Default);
    }

    #[test]
    fn the_local_file_may_only_have_host() {
        let e = read_with_local("", "[tools]\nnode = true\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "sandbox.local.toml: only [host] can be set here (found 'tools')."
        );
        let e = read_with_local("", "[host]\nbrowser = false\n[env]\n")
            .unwrap_err()
            .to_string();
        assert!(e.ends_with("(found 'env')."), "{e}");
    }

    #[test]
    fn errors_in_the_local_file_name_it() {
        let e = read_with_local("", "[host]\nbrowser = 5\n")
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with("sandbox.local.toml: host.browser must be"),
            "{e}"
        );
        let e = read_with_local("", "[host]\nother = 5\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "sandbox.local.toml: unknown key 'host.other' (known: browser)."
        );
        let e = read_with_local("", "[host]\nbrowser = \n")
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with("sandbox.local.toml is not valid TOML (line 2, column 11): "),
            "{e}"
        );
        let e = read_with_local("", "host = 1\n").unwrap_err().to_string();
        assert_eq!(e, "sandbox.local.toml: host must be a table (got 1).");
    }

    #[test]
    fn syntax_errors_carry_the_position() {
        let e = read_str("[project]\nworkspace = \n")
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with("sandbox.toml is not valid TOML (line 2, column 13): "),
            "{e}"
        );
    }
}
