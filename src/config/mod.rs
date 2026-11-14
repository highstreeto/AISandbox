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
/// Personal overrides next to `sandbox.toml`; not committed. Only `[host]` and `[llm]` can be set in it.
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
    ("SANDBOX_LLM_URL", Some("llm.model")),
    ("SANDBOX_LLM_MODEL", Some("llm.model")),
    ("SANDBOX_LLM_CONTEXT", Some("llm.context")),
    ("OLLAMA_HOST", Some("llm.model")),
];

/// The Ollama model used when `[llm]` is on without a `model`.
pub const DEFAULT_LLM_MODEL: &str = "qwen3.6:27b";
/// The context length (tokens) when `llm.context` is not set: Claude Code's system prompt alone is over 20K.
pub const DEFAULT_LLM_CONTEXT: u32 = 65536;
const MIN_LLM_CONTEXT: i64 = 2048;
const MAX_LLM_CONTEXT: i64 = 1_048_576;

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
    pub browser_source: SettingSource,
    /// `[llm]`: the local model; `None` = off.
    pub llm: Option<LlmSettings>,
    /// `[llm]` is set up in `sandbox.toml` but `sandbox.local.toml` turns it off on this machine.
    pub llm_off_locally: bool,
}

/// `[llm]`: a local model in its own container (Ollama behind a filtering gateway).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmSettings {
    /// The model the desktop's tools use (an Ollama tag).
    pub model: String,
    /// More models that `up` pulls too (without the main one, no duplicates).
    pub extra_models: Vec<String>,
    /// Context length in tokens (`OLLAMA_CONTEXT_LENGTH`).
    pub context: u32,
    /// How long a model stays in memory after the last request (an Ollama duration); `None` = Ollama's default.
    pub keep_alive: Option<String>,
    pub gpu: LlmGpu,
    /// Which file `model` is from (`Default`: the built-in model).
    pub model_source: SettingSource,
    /// Which file `gpu` is from.
    pub gpu_source: SettingSource,
}

impl LlmSettings {
    /// The main model first, then the extra ones.
    pub fn all_models(&self) -> Vec<String> {
        std::iter::once(self.model.clone())
            .chain(self.extra_models.iter().cloned())
            .collect()
    }
}

/// `llm.gpu` (`sandbox.local.toml` only): what runs the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmGpu {
    Nvidia,
    /// Linux only: the ROCm image with `/dev/kfd` and `/dev/dri`.
    Amd,
    /// The CPU.
    None,
}

impl LlmGpu {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nvidia => "nvidia",
            Self::Amd => "amd",
            Self::None => "none",
        }
    }
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
pub enum SettingSource {
    /// Not set anywhere.
    Default,
    Main,
    Local,
}

impl SettingSource {
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
/// An Ollama model tag: `name`, `name:tag`, `namespace/name:tag`, `hf.co/org/repo:Q4_K_M`.
static MODEL_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._/-]*(:[A-Za-z0-9._-]+)?$").unwrap());
/// An Ollama duration: `30m`, `1h30m`, `90s`, or `-1` (never unload) / `0` (unload at once).
static KEEP_ALIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(-1|0|(\d+[smh])+)$").unwrap());
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
    let mut llm = r.table("llm")?;
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

    // --- [host] and [llm]; the local file overrides the main one ---
    let mut browser = (BrowserSetting::Default, SettingSource::Default);
    if let Some(b) = read_host(&mut host, FILE_NAME)? {
        browser = (b, SettingSource::Main);
    }
    let main_llm = read_llm(&mut llm, FILE_NAME, false)?;
    let local = read_local(&path.with_file_name(LOCAL_FILE_NAME))?;
    if let Some(b) = local.browser {
        browser = (b, SettingSource::Local);
    }
    let (llm, llm_off_locally) = merge_llm(main_llm, local.llm);

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
        llm,
        llm_off_locally,
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

/// What `sandbox.local.toml` sets.
#[derive(Debug, Default)]
struct LocalSettings {
    browser: Option<BrowserSetting>,
    llm: LlmFile,
}

/// The keys of one `[llm]` section. `models` and `keep_alive` are only read from `sandbox.toml`, `gpu` only from
/// `sandbox.local.toml`.
#[derive(Debug, Default)]
struct LlmFile {
    enabled: Option<bool>,
    model: Option<String>,
    models: Vec<String>,
    context: Option<u32>,
    keep_alive: Option<String>,
    gpu: Option<LlmGpu>,
}

/// `sandbox.local.toml`: its `[host]` and `[llm]` settings, if the file exists. Nothing else is allowed in it.
fn read_local(path: &Utf8Path) -> Result<LocalSettings> {
    if !path.is_file() {
        return Ok(LocalSettings::default());
    }
    local_settings(&parse_file(LOCAL_FILE_NAME, path)?)
}

fn local_settings(root: &Table) -> Result<LocalSettings> {
    if let Some(other) = root.keys().find(|k| *k != "host" && *k != "llm") {
        return Err(error_in(
            LOCAL_FILE_NAME,
            format!("only [host] and [llm] can be set here (found '{other}')."),
        ));
    }
    let mut r = TableReader::new_in(LOCAL_FILE_NAME, root, "");
    let mut host = r.table("host")?;
    let mut llm = r.table("llm")?;
    r.done("table")?;
    Ok(LocalSettings {
        browser: read_host(&mut host, LOCAL_FILE_NAME)?,
        llm: read_llm(&mut llm, LOCAL_FILE_NAME, true)?,
    })
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

/// The `[llm]` section of either file. `local` selects the keys: `gpu` only in `sandbox.local.toml`, `models` and
/// `keep_alive` only in `sandbox.toml`.
fn read_llm(llm: &mut TableReader, file: &'static str, local: bool) -> Result<LlmFile> {
    let fail = |message: String| error_in(file, message);
    let mut out = LlmFile {
        enabled: llm.bool("enabled")?,
        ..LlmFile::default()
    };
    if let Some(m) = llm.str("model")? {
        out.model = Some(model_tag(file, "llm.model", m)?);
    }
    if !local {
        for (i, m) in llm.str_list("models")?.iter().enumerate() {
            let tag = model_tag(file, &format!("llm.models[{}]", i + 1), m)?;
            if !out.models.contains(&tag) {
                out.models.push(tag);
            }
        }
    }
    if let Some(c) = llm.int("context")? {
        if !(MIN_LLM_CONTEXT..=MAX_LLM_CONTEXT).contains(&c) {
            return Err(fail(format!(
                "llm.context must be a whole number from {MIN_LLM_CONTEXT} to {MAX_LLM_CONTEXT} (got {c})."
            )));
        }
        out.context = u32::try_from(c).ok();
    }
    if !local && let Some(k) = llm.str("keep_alive")? {
        let k = k.trim();
        if !KEEP_ALIVE.is_match(k) {
            return Err(fail(format!(
                "llm.keep_alive must be a duration like \"30m\", \"1h\" or \"-1\" for never (got \"{k}\")."
            )));
        }
        out.keep_alive = Some(k.to_string());
    }
    if local && let Some(g) = llm.str("gpu")? {
        out.gpu = Some(match g.trim() {
            "nvidia" => LlmGpu::Nvidia,
            "amd" => LlmGpu::Amd,
            "none" => LlmGpu::None,
            other => {
                return Err(fail(format!(
                    "llm.gpu must be \"nvidia\", \"amd\" or \"none\" (got \"{other}\")."
                )));
            }
        });
    }
    llm.done("key")?;
    Ok(out)
}

/// A model tag typed on the command line (`sandbox llm pull <model>`), trimmed.
pub fn parse_model_tag(value: &str) -> Result<String> {
    let tag = value.trim();
    if !MODEL_TAG.is_match(tag) {
        return Err(anyhow!(
            "\"{tag}\" is not an Ollama model tag like \"{DEFAULT_LLM_MODEL}\"."
        ));
    }
    Ok(tag.to_string())
}

/// A model tag, trimmed; `what` names the setting in the message.
fn model_tag(file: &str, what: &str, value: &str) -> Result<String> {
    let tag = value.trim();
    if !MODEL_TAG.is_match(tag) {
        return Err(error_in(
            file,
            format!(
                "{what} must be an Ollama model tag like \"{DEFAULT_LLM_MODEL}\" (got \"{tag}\")."
            ),
        ));
    }
    Ok(tag.to_string())
}

/// The effective `[llm]`: `sandbox.toml` turns it on (a `model`, or `enabled = true`), `sandbox.local.toml` can
/// turn it off (`enabled = false`), turn it on for this machine only (`enabled = true` or a `model`) and replace the
/// model, the context and the GPU. The flag says that this machine's file switched off what the project turns on.
fn merge_llm(main: LlmFile, local: LlmFile) -> (Option<LlmSettings>, bool) {
    let main_on = main.enabled.unwrap_or(main.model.is_some());
    let local_on = local.enabled.unwrap_or(local.model.is_some() || main_on);
    if !local_on {
        return (None, main_on && local.enabled == Some(false));
    }
    let (model, model_source) = match (&local.model, &main.model) {
        (Some(m), _) => (m.clone(), SettingSource::Local),
        (None, Some(m)) => (m.clone(), SettingSource::Main),
        (None, None) => (DEFAULT_LLM_MODEL.to_string(), SettingSource::Default),
    };
    let extra_models = main.models.into_iter().filter(|m| *m != model).collect();
    let llm = LlmSettings {
        model,
        extra_models,
        context: local
            .context
            .or(main.context)
            .unwrap_or(DEFAULT_LLM_CONTEXT),
        keep_alive: main.keep_alive,
        gpu: local.gpu.unwrap_or(LlmGpu::Nvidia),
        model_source,
        gpu_source: if local.gpu.is_some() {
            SettingSource::Local
        } else {
            SettingSource::Default
        },
    };
    (Some(llm), false)
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
        assert_eq!(s.browser_source, SettingSource::Default);
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
            assert_eq!(s.browser_source, SettingSource::Main, "{value}");
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
        assert_eq!(s.browser_source, SettingSource::Local);
        assert_eq!(s.browser_source.file_name(), Some("sandbox.local.toml"));

        // a local file that sets nothing leaves the main file's value
        let s = read_with_local("[host]\nbrowser = \"firefox\"\n", "[host]\n").unwrap();
        assert_eq!(s.browser, command(&["firefox"]));
        assert_eq!(s.browser_source, SettingSource::Main);
        let s = read_with_local("", "").unwrap();
        assert_eq!(s.browser_source, SettingSource::Default);
    }

    #[test]
    fn the_local_file_may_only_have_host_and_llm() {
        let e = read_with_local("", "[tools]\nnode = true\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "sandbox.local.toml: only [host] and [llm] can be set here (found 'tools')."
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

    fn llm(main: &str, local: &str) -> Option<LlmSettings> {
        read_with_local(main, local).unwrap().llm
    }

    fn llm_error(main: &str, local: &str) -> String {
        read_with_local(main, local).unwrap_err().to_string()
    }

    #[test]
    fn the_llm_is_off_unless_a_file_turns_it_on() {
        assert_eq!(llm("", ""), None);
        assert_eq!(llm("[llm]\ncontext = 8192\n", ""), None);
        assert_eq!(llm("[llm]\nmodel = \"x:1\"\nenabled = false\n", ""), None);
        assert!(
            !read_with_local("[llm]\nmodel = \"x\"\n", "")
                .unwrap()
                .llm_off_locally
        );
    }

    #[test]
    fn a_model_or_enabled_turns_the_llm_on_with_defaults() {
        let l = llm("[llm]\nmodel = \" gpt-oss:20b \"\n", "").unwrap();
        assert_eq!(l.model, "gpt-oss:20b");
        assert_eq!(l.model_source, SettingSource::Main);
        assert_eq!(l.context, DEFAULT_LLM_CONTEXT);
        assert_eq!(
            (l.gpu, l.gpu_source),
            (LlmGpu::Nvidia, SettingSource::Default)
        );
        assert_eq!(l.keep_alive, None);
        assert!(l.extra_models.is_empty());

        let l = llm("[llm]\nenabled = true\n", "").unwrap();
        assert_eq!(l.model, DEFAULT_LLM_MODEL);
        assert_eq!(l.model_source, SettingSource::Default);
    }

    #[test]
    fn every_llm_key_of_the_main_file() {
        let l = llm(
            "[llm]\nmodel = \"a:1\"\nmodels = [\"b:2\", \"a:1\", \"b:2\", \"hf.co/o/r:Q4_K_M\"]\ncontext = 32768\nkeep_alive = \"1h30m\"\n",
            "",
        )
        .unwrap();
        assert_eq!(l.all_models(), vec!["a:1", "b:2", "hf.co/o/r:Q4_K_M"]);
        assert_eq!((l.context, l.keep_alive.as_deref()), (32768, Some("1h30m")));
        for k in ["-1", "0", "90s", "30m"] {
            let l = llm(
                &format!("[llm]\nenabled = true\nkeep_alive = \"{k}\"\n"),
                "",
            );
            assert_eq!(l.unwrap().keep_alive.as_deref(), Some(k));
        }
    }

    #[test]
    fn the_local_file_changes_the_llm_for_this_machine() {
        let main = "[llm]\nmodel = \"a:1\"\nmodels = [\"b:2\"]\ncontext = 8192\n";
        let l = llm(
            main,
            "[llm]\nmodel = \"b:2\"\ncontext = 4096\ngpu = \"amd\"\n",
        )
        .unwrap();
        assert_eq!(
            (l.model.as_str(), l.model_source),
            ("b:2", SettingSource::Local)
        );
        assert_eq!(l.all_models(), vec!["b:2"]);
        assert_eq!(
            (l.context, l.gpu, l.gpu_source),
            (4096, LlmGpu::Amd, SettingSource::Local)
        );

        let s = read_with_local(main, "[llm]\nenabled = false\n").unwrap();
        assert!(s.llm.is_none() && s.llm_off_locally);

        // this machine only
        let l = llm("", "[llm]\nmodel = \"c:3\"\ngpu = \"none\"\n").unwrap();
        assert_eq!((l.model.as_str(), l.gpu), ("c:3", LlmGpu::None));
        assert!(llm("", "[llm]\nenabled = true\n").is_some());
        let s = read_with_local("", "[llm]\nenabled = false\n").unwrap();
        assert!(s.llm.is_none() && !s.llm_off_locally);
    }

    #[test]
    fn invalid_llm_values_are_errors_naming_the_file() {
        let cases = [
            (
                "[llm]\nmodel = 5\n",
                "sandbox.toml: llm.model must be a text in quotes (got 5).",
            ),
            (
                "[llm]\nmodel = \"two words\"\n",
                "sandbox.toml: llm.model must be an Ollama model tag like \"qwen3.6:27b\" (got \"two words\").",
            ),
            (
                "[llm]\nmodel = \"\"\n",
                "sandbox.toml: llm.model must be an Ollama model tag like \"qwen3.6:27b\" (got \"\").",
            ),
            (
                "[llm]\nmodels = [\"ok\", \"a:b:c\"]\n",
                "sandbox.toml: llm.models[2] must be an Ollama model tag like \"qwen3.6:27b\" (got \"a:b:c\").",
            ),
            (
                "[llm]\nmodels = \"x\"\n",
                "sandbox.toml: llm.models must be a list of texts, e.g. [\"a\", \"b\"] (got \"x\").",
            ),
            (
                "[llm]\nenabled = \"yes\"\n",
                "sandbox.toml: llm.enabled must be true or false (got \"yes\").",
            ),
            (
                "[llm]\ncontext = 100\n",
                "sandbox.toml: llm.context must be a whole number from 2048 to 1048576 (got 100).",
            ),
            (
                "[llm]\ncontext = 2000000\n",
                "sandbox.toml: llm.context must be a whole number from 2048 to 1048576 (got 2000000).",
            ),
            (
                "[llm]\ncontext = 4096.5\n",
                "sandbox.toml: llm.context must be a whole number (got 4096.5).",
            ),
            (
                "[llm]\nkeep_alive = \"soon\"\n",
                "sandbox.toml: llm.keep_alive must be a duration like \"30m\", \"1h\" or \"-1\" for never (got \"soon\").",
            ),
            (
                "[llm]\ngpu = \"nvidia\"\n",
                "sandbox.toml: unknown key 'llm.gpu' (known: enabled, model, models, context, keep_alive).",
            ),
            (
                "[llm]\nextra = 1\n",
                "sandbox.toml: unknown key 'llm.extra' (known: enabled, model, models, context, keep_alive).",
            ),
            ("llm = 1\n", "sandbox.toml: llm must be a table (got 1)."),
        ];
        for (toml, message) in cases {
            assert_eq!(llm_error(toml, ""), message, "{toml}");
        }
    }

    #[test]
    fn invalid_local_llm_values_name_the_local_file() {
        let cases = [
            (
                "[llm]\ngpu = \"intel\"\n",
                "sandbox.local.toml: llm.gpu must be \"nvidia\", \"amd\" or \"none\" (got \"intel\").",
            ),
            (
                "[llm]\ngpu = true\n",
                "sandbox.local.toml: llm.gpu must be a text in quotes (got true).",
            ),
            (
                "[llm]\nmodels = [\"x\"]\n",
                "sandbox.local.toml: unknown key 'llm.models' (known: enabled, model, context, gpu).",
            ),
            (
                "[llm]\nkeep_alive = \"1h\"\n",
                "sandbox.local.toml: unknown key 'llm.keep_alive' (known: enabled, model, context, gpu).",
            ),
            (
                "[llm]\nmodel = \"a b\"\n",
                "sandbox.local.toml: llm.model must be an Ollama model tag like \"qwen3.6:27b\" (got \"a b\").",
            ),
            (
                "[llm]\ncontext = 1\n",
                "sandbox.local.toml: llm.context must be a whole number from 2048 to 1048576 (got 1).",
            ),
        ];
        for (toml, message) in cases {
            assert_eq!(llm_error("", toml), message, "{toml}");
        }
    }

    #[test]
    fn the_llm_environment_variables_are_reserved() {
        for name in ["SANDBOX_LLM_URL", "sandbox_llm_model", "OLLAMA_HOST"] {
            let e = read_str(&format!("[env]\n{name} = \"x\"\n"))
                .unwrap_err()
                .to_string();
            assert_eq!(
                e,
                format!("sandbox.toml: env.{name} is set by the sandbox; use llm.model instead.")
            );
        }
        let e = read_str("[env]\nSANDBOX_LLM_CONTEXT = \"x\"\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "sandbox.toml: env.SANDBOX_LLM_CONTEXT is set by the sandbox; use llm.context instead."
        );
    }

    #[test]
    fn model_tags_from_the_command_line() {
        assert_eq!(parse_model_tag(" qwen3.6:27b ").unwrap(), "qwen3.6:27b");
        assert_eq!(parse_model_tag("llama3").unwrap(), "llama3");
        assert!(parse_model_tag("-rf").is_err());
        assert!(parse_model_tag("a b").is_err());
        assert!(parse_model_tag("").is_err());
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
