//! Test support: a throw-away project (`Fixture`), running the built app against the fake docker, expected calls.

#![allow(dead_code)]

use ai_sandbox::VERSION;
use assert_cmd::Command;
use camino::{Utf8Path, Utf8PathBuf};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

pub const PROJECT_NAME: &str = "demo-project";
pub const ID: &str = "0123abcd";
pub const URL: &str = "https://localhost:49153";

/// A fixed sandbox.toml (not the repo's, which people edit) in the shipped format.
pub const DEFAULT_CONF: &str = r#"[project]
# name = "myproject"
workspace = "rw"
protect_git = true
# ui_port = 3001

[tools]
claude = true
claude_code = false
dotnet = "10.0"
node = false

[desktop]
title = "AI Sandbox"
"#;

/// A fixed allowlist.txt: active entries, a group with commented-out entries, header comments.
pub const DEFAULT_ALLOWLIST: &str = r#"# =====================================================
#  Sandbox network allowlist — one host per line.
# =====================================================
#    example.com        exactly example.com

# --- Claude (required) -------------------------------
api.anthropic.com
claude.ai
*.claudeusercontent.com

# --- .NET / NuGet ------------------------------------
# api.nuget.org
# globalcdn.nuget.org
"#;

/// Three healthy services, one JSON object per line like `docker compose ps --format json` prints them.
pub fn ps_json() -> String {
    [
        r#"{"Service":"desktop","State":"running","Health":"healthy","Status":"Up 2 minutes (healthy)","Publishers":[]}"#,
        r#"{"Service":"gateway","State":"running","Health":"","Status":"Up 2 minutes","Publishers":[{"URL":"127.0.0.1","TargetPort":3001,"PublishedPort":49153,"Protocol":"tcp"}]}"#,
        r#"{"Service":"proxy","State":"running","Health":"healthy","Status":"Up 2 minutes (healthy)","Publishers":[]}"#,
    ]
    .join("\n")
        + "\n"
}

/// One throw-away project (every test has its own, deleted when the test ends):
///
/// ```text
/// <tmp>/demo-project/.git/{config,hooks/}
/// <tmp>/demo-project/.sandbox/   sandbox.toml, allowlist.txt, .state/id written here;
///                                the engine is written to .state/desktop and .state/proxy by the app
/// <tmp>/fakebin/docker, xdg-open the fake docker (tests/fake_docker)
/// <tmp>/empty/                   a PATH with nothing on it
/// ```
pub struct Fixture {
    _tmp: TempDir,
    pub root: Utf8PathBuf,
    pub project_dir: Utf8PathBuf,
    pub dir: Utf8PathBuf,
    pub fake_bin: Utf8PathBuf,
    pub empty_bin: Utf8PathBuf,
    runs: AtomicUsize,
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("sandbox-tests-")
            .tempdir()
            .expect("temp folder");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8 temp path");
        let project_dir = root.join(PROJECT_NAME);
        let dir = project_dir.join(".sandbox");
        let f = Self {
            fake_bin: root.join("fakebin"),
            empty_bin: root.join("empty"),
            root,
            project_dir,
            dir,
            _tmp: tmp,
            runs: AtomicUsize::new(0),
        };
        fs::create_dir_all(f.state_dir()).unwrap();
        fs::create_dir_all(&f.empty_bin).unwrap();
        f.install_fake_docker();

        fs::write(f.conf_file(), DEFAULT_CONF).unwrap();
        fs::write(f.allowlist_file(), DEFAULT_ALLOWLIST).unwrap();
        fs::write(f.id_file(), format!("{ID}\n")).unwrap();
        let git = f.project_dir.join(".git");
        fs::create_dir_all(git.join("hooks")).unwrap();
        fs::write(git.join("config"), "[core]\n\tbare = false\n").unwrap();
        f
    }

    pub fn state_dir(&self) -> Utf8PathBuf {
        self.dir.join(".state")
    }
    pub fn id_file(&self) -> Utf8PathBuf {
        self.state_dir().join("id")
    }
    pub fn desktop_context(&self) -> Utf8PathBuf {
        self.state_dir().join("desktop")
    }
    pub fn proxy_context(&self) -> Utf8PathBuf {
        self.state_dir().join("proxy")
    }
    pub fn generated_file(&self) -> Utf8PathBuf {
        self.state_dir().join("compose.yml")
    }
    pub fn conf_file(&self) -> Utf8PathBuf {
        self.dir.join("sandbox.toml")
    }
    /// The engine folder of the old layout; tests that need it create it.
    pub fn old_engine_dir(&self) -> Utf8PathBuf {
        self.dir.join("engine")
    }
    pub fn old_conf_file(&self) -> Utf8PathBuf {
        self.dir.join("sandbox.yml")
    }
    pub fn old_local_conf_file(&self) -> Utf8PathBuf {
        self.dir.join("sandbox.local.yml")
    }
    /// The templates `sandbox init` writes (the shipped sandbox.toml and allowlist.txt), in the app's sources.
    pub fn init_templates_dir(&self) -> Utf8PathBuf {
        Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(RESOURCES)
            .join("init")
    }
    pub fn allowlist_file(&self) -> Utf8PathBuf {
        self.dir.join("allowlist.txt")
    }
    pub fn work_dir(&self) -> String {
        format!("/work/{PROJECT_NAME}-{ID}")
    }
    /// The app's version as it writes it into headers and the manifest.
    pub fn app_version(&self) -> &'static str {
        VERSION
    }

    pub fn write_conf(&self, toml: &str) {
        fs::write(self.conf_file(), toml).unwrap();
    }
    pub fn read_conf(&self) -> String {
        fs::read_to_string(self.conf_file()).unwrap()
    }

    /// The fake docker as `docker` and `xdg-open`: symlinks on Unix (copying an executable while other tests start
    /// processes can fail with "text file busy"), copies elsewhere.
    fn install_fake_docker(&self) {
        fs::create_dir_all(&self.fake_bin).unwrap();
        let exe = env!("CARGO_BIN_EXE_fake-docker");
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        for name in ["docker", "xdg-open"] {
            let target = self.fake_bin.join(format!("{name}{suffix}"));
            link_or_copy(Path::new(exe), target.as_std_path());
        }
    }

    /// Runs the built app with `SANDBOX_DIR` set, like the launchers do.
    pub fn run<S: AsRef<str>>(&self, args: &[S]) -> RunResult {
        self.run_with(args, RunOpts::default())
    }

    /// `opts.rules` are checked before the defaults. `opts.path` replaces the fake bin dir on PATH.
    pub fn run_with<S: AsRef<str>>(&self, args: &[S], opts: RunOpts) -> RunResult {
        let n = self.runs.fetch_add(1, Ordering::SeqCst);
        // A fresh log per run: a detached child of an earlier run can't write into this one.
        let log = self.root.join(format!("calls-{n}.jsonl"));
        let rules_file = self.root.join(format!("rules-{n}.json"));
        let mut all_rules = opts.rules;
        all_rules.extend(default_rules());
        let json: Vec<serde_json::Value> = all_rules
            .iter()
            .map(|r| serde_json::json!({"when": r.when, "stdout": r.stdout, "stderr": r.stderr, "exit": r.exit}))
            .collect();
        fs::write(&rules_file, serde_json::to_string(&json).unwrap()).unwrap();

        let path = opts.path.unwrap_or_else(|| {
            let mut dirs = vec![self.fake_bin.clone().into_std_path_buf()];
            dirs.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            std::env::join_paths(dirs).unwrap()
        });

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sandbox"));
        cmd.args(args.iter().map(|a| a.as_ref()))
            .current_dir(opts.cwd.as_deref().unwrap_or(&self.root))
            .env("PATH", path)
            .env("FAKE_DOCKER_LOG", &log)
            .env("FAKE_DOCKER_RULES", &rules_file)
            .env("TZ", "Europe/Vienna")
            .env("COLUMNS", "200")
            .env("NO_COLOR", "1")
            .env_remove("DOCKER_HOST")
            .write_stdin("")
            .timeout(Duration::from_secs(120));
        match opts.sandbox_dir {
            Some(d) if d.is_empty() => {
                cmd.env_remove("SANDBOX_DIR");
            }
            Some(d) => {
                cmd.env("SANDBOX_DIR", d);
            }
            None => {
                cmd.env("SANDBOX_DIR", &self.dir);
            }
        }
        let shown: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
        let out = cmd
            .output()
            .unwrap_or_else(|e| panic!("sandbox {} did not finish: {e}", shown.join(" ")));

        let mut calls = read_calls(&log);
        if let Some(wait) = opts.wait_for {
            for _ in 0..100 {
                if calls.iter().any(|c| c.name == wait) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
                calls = read_calls(&log);
            }
        }
        RunResult {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            calls,
        }
    }

    // --- expected calls ---------------------------------------------------------

    /// `docker compose -p <name> --project-directory <dir> -f .state/compose.yml <args>` — attached to the
    /// terminal (build, exec, logs).
    pub fn compose_interactive<S: AsRef<str>>(&self, args: &[S]) -> Call {
        self.compose_as(PROJECT_NAME, args)
    }

    /// Same, with captured output (`--ansi never`).
    pub fn compose<S: AsRef<str>>(&self, args: &[S]) -> Call {
        let mut a = vec!["--ansi".to_string(), "never".to_string()];
        a.extend(args.iter().map(|s| s.as_ref().to_string()));
        self.compose_as(PROJECT_NAME, &a)
    }

    pub fn compose_as<S: AsRef<str>>(&self, project: &str, args: &[S]) -> Call {
        let mut a: Vec<String> = [
            "compose",
            "-p",
            project,
            "--project-directory",
            self.dir.as_str(),
            "-f",
            self.generated_file().as_str(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        a.extend(args.iter().map(|s| s.as_ref().to_string()));
        Call {
            name: "docker".into(),
            args: a,
        }
    }

    pub fn collision_check(&self, project: &str) -> Call {
        docker(&[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--filter",
            "label=sandbox.id",
            "--format",
            "{{.Label \"sandbox.id\"}}",
        ])
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

/// The folder the templates and the engine come from (see `build.rs`).
pub const RESOURCES: &str = "resources";

#[cfg(unix)]
fn link_or_copy(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}

#[cfg(not(unix))]
fn link_or_copy(from: &Path, to: &Path) {
    fs::copy(from, to).unwrap();
}

/// One call the fake docker logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    pub args: Vec<String>,
}

impl std::fmt::Display for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}",
            self.name,
            serde_json::to_string(&self.args).unwrap()
        )
    }
}

pub fn docker<S: AsRef<str>>(args: &[S]) -> Call {
    Call {
        name: "docker".into(),
        args: args.iter().map(|a| a.as_ref().to_string()).collect(),
    }
}

/// `docker info --format {{.ServerVersion}}`
pub fn docker_info() -> Call {
    docker(&["info", "--format", "{{.ServerVersion}}"])
}

/// A call of a program other than docker (the browser opener).
pub fn call(name: &str, args: &[&str]) -> Call {
    Call {
        name: name.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
    }
}

pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub calls: Vec<Call>,
}

impl RunResult {
    pub fn output(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    /// Stdout split into lines.
    pub fn lines(&self) -> Vec<String> {
        self.stdout
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect()
    }
}

impl std::fmt::Display for RunResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let calls: Vec<String> = self.calls.iter().map(|c| c.to_string()).collect();
        write!(
            f,
            "exit {}\n--- stdout\n{}\n--- stderr\n{}\n--- calls\n{}",
            self.exit_code,
            self.stdout,
            self.stderr,
            calls.join("\n")
        )
    }
}

/// One rule for the fake docker: the first rule whose `when` is a substring of the space-joined arguments wins.
#[derive(Debug, Clone)]
pub struct Rule {
    pub when: String,
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
}

pub fn rule(when: &str) -> Rule {
    Rule {
        when: when.into(),
        stdout: String::new(),
        stderr: String::new(),
        exit: 0,
    }
}

impl Rule {
    pub fn stdout(mut self, s: impl Into<String>) -> Self {
        self.stdout = s.into();
        self
    }
    pub fn stderr(mut self, s: impl Into<String>) -> Self {
        self.stderr = s.into();
        self
    }
    pub fn exit(mut self, code: i32) -> Self {
        self.exit = code;
        self
    }
}

/// What a running sandbox answers.
pub fn default_rules() -> Vec<Rule> {
    vec![
        rule("info --format").stdout("27.0.0\n"),
        rule("ps -a --filter").stdout(format!("{ID}\n")),
        rule("port gateway 3001").stdout("127.0.0.1:49153\n"),
        rule("ps -a --format json").stdout(ps_json()),
    ]
}

/// Options of [`Fixture::run_with`].
#[derive(Default)]
pub struct RunOpts {
    /// Rules checked before the defaults.
    pub rules: Vec<Rule>,
    /// Replaces the fake bin dir on PATH.
    pub path: Option<OsString>,
    /// A call that a detached child makes after the app has exited (the browser opener).
    pub wait_for: Option<&'static str>,
    /// The working directory (default: the temp root, outside the project).
    pub cwd: Option<Utf8PathBuf>,
    /// The value of `SANDBOX_DIR` (default: the project's .sandbox; "" leaves it unset).
    pub sandbox_dir: Option<String>,
}

impl RunOpts {
    pub fn rules(mut self, rules: Vec<Rule>) -> Self {
        self.rules = rules;
        self
    }
    pub fn path(mut self, path: impl Into<OsString>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn wait_for(mut self, call: &'static str) -> Self {
        self.wait_for = Some(call);
        self
    }
    pub fn cwd(mut self, cwd: impl Into<Utf8PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
    pub fn sandbox_dir(mut self, dir: impl Into<String>) -> Self {
        self.sandbox_dir = Some(dir.into());
        self
    }
}

fn read_calls(log: &Utf8Path) -> Vec<Call> {
    let Ok(text) = fs::read_to_string(log) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: serde_json::Value =
                serde_json::from_str(l).expect("a JSON line in the call log");
            Call {
                name: v["name"].as_str().unwrap_or_default().to_string(),
                args: v["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|x| x.as_str().unwrap_or_default().to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

// --- assertions -----------------------------------------------------------------

pub fn assert_calls(r: &RunResult, expected: &[Call]) {
    let want: Vec<String> = expected.iter().map(|c| c.to_string()).collect();
    let got: Vec<String> = r.calls.iter().map(|c| c.to_string()).collect();
    assert!(
        want == got,
        "Expected calls:\n{}\n\nActual run:\n{r}",
        want.join("\n")
    );
}

pub fn assert_exit(r: &RunResult, code: i32) {
    assert!(r.exit_code == code, "Expected exit {code}.\n{r}");
}

pub fn assert_output(r: &RunResult, text: &str) {
    assert!(
        r.output().contains(text),
        "Expected output to contain '{text}'.\n{r}"
    );
}

/// A shorthand for building `&[&str]` argument lists from a mixed list.
pub fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|i| i.to_string()).collect()
}
