//! Development tasks: what `.github/workflows/test.yml` (`cargo xtask ci`) and `.github/workflows/release.yml`
//! (`version`, `package`, `assets`) run, runnable locally (`cargo xtask dist`: a release dry run).
//!
//! Few dependencies (only ones the app already has, so they are in Cargo.lock), so it builds in a moment.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

mod release;
mod test;

const HELP: &str = "\
Usage: cargo xtask <task>

Tasks:
  test       Run all tests (the `test` job in CI)
  coverage   Run all tests with coverage, write the HTML report and check the minimum (the `coverage` job)
             --open  open the HTML report afterwards
  ci         Everything CI checks: xtask's own tests, then coverage, which runs all tests once
  help       Show this

Release tasks (.github/workflows/release.yml), output in target/dist/:
  version    Next version from git-cliff (cliff.toml); notes.md and CHANGELOG.md if there is a release
  package    Build the release binary, smoke-test it, write the binary and the archive (checked) to files/
             --target <triple>  instead of this machine's  --version <x.y.z>  else $SANDBOX_RELEASE_VERSION
             --no-smoke         skip `sandbox --version` (a target this machine can't run)
  assets     Collect files/, the install scripts, CHANGELOG.md, VERSION and SHA256SUMS in assets/
             --version <x.y.z>  else $SANDBOX_RELEASE_VERSION
  dist       Dry run of all three for this machine's target; nothing is published
             --version <x.y.z>  (else git-cliff, else the crate version)  --no-smoke

Coverage needs, once:
  rustup component add llvm-tools-preview
  cargo install cargo-llvm-cov --locked

The release tasks need git-cliff for `version` (cargo install git-cliff --locked), tar, and 7z for Windows targets.";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["test"] => test::tests(),
        ["coverage"] => test::coverage(false),
        ["coverage", "--open"] => test::coverage(true),
        ["ci"] => test::ci(),
        ["version"] => release::version(),
        ["package", options @ ..] => parse(options, &["--target", "--version"], &["--no-smoke"])
            .and_then(|o| {
                release::package(&release::PackageOptions {
                    target: o.value("--target"),
                    version: o.value("--version"),
                    smoke: !o.flag("--no-smoke"),
                })
            }),
        ["assets", options @ ..] => parse(options, &["--version"], &[])
            .and_then(|o| release::assets(o.value("--version").as_deref())),
        ["dist", options @ ..] => parse(options, &["--version"], &["--no-smoke"]).and_then(|o| {
            release::dist_dry_run(o.value("--version").as_deref(), !o.flag("--no-smoke"))
        }),
        [] | ["help" | "-h" | "--help"] => {
            println!("{HELP}");
            Ok(())
        }
        _ => Err(format!(
            "Unknown task or option: {}\n\n{HELP}",
            args.join(" ")
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Options of a task: `--name value` for `values`, `--name` for `flags`.
struct Options {
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Options {
    fn value(&self, name: &str) -> Option<String> {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
}

fn parse(args: &[&str], values: &[&str], flags: &[&str]) -> Result<Options, String> {
    let mut options = Options {
        values: Vec::new(),
        flags: Vec::new(),
    };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        if values.contains(&arg) {
            let value = args
                .next()
                .ok_or_else(|| format!("{arg} needs a value.\n\n{HELP}"))?;
            options.values.push((arg.to_string(), value.to_string()));
        } else if flags.contains(&arg) {
            options.flags.push(arg.to_string());
        } else {
            return Err(format!("Unknown option: {arg}\n\n{HELP}"));
        }
    }
    Ok(options)
}

/// The workspace root (the parent of `xtask/`), where every cargo command runs.
fn root_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ has a parent")
        .to_path_buf()
}

fn cargo_command(args: &[&str]) -> Command {
    let mut cmd = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.args(args).current_dir(root_dir());
    cmd
}

/// Runs cargo with the terminal attached; fails if it fails.
fn cargo(args: &[&str]) -> Result<(), String> {
    cargo_with_env(args, &[])
}

/// Runs cargo with extra environment variables and the terminal attached; fails if it fails.
fn cargo_with_env(args: &[&str], vars: &[(&str, &str)]) -> Result<(), String> {
    let shown: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v} ")).collect();
    println!("$ {}cargo {}", shown.concat(), args.join(" "));
    let status = cargo_command(args)
        .envs(vars.iter().copied())
        .status()
        .map_err(|e| format!("Cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed ({status}).", args.join(" ")))
    }
}

/// Runs cargo and returns whether it succeeded and its standard output (standard error goes to the terminal).
fn cargo_output(args: &[&str]) -> Result<(bool, String), String> {
    let output = cargo_command(args)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("Cannot run cargo: {e}"))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

// --- Programs ---

/// Runs a program in `dir` with the terminal attached; fails if it fails.
fn run_in(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    println!("$ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("Cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status}).", args.join(" ")))
    }
}

/// Runs a program in the workspace root and returns whether it succeeded and its standard output (standard error
/// goes to the terminal).
fn output(program: &str, args: &[&str]) -> Result<(bool, String), String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root_dir())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("Cannot run {program}: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

// --- Files ---

fn path_str(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Not a UTF-8 path: {}", path.display()))
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("No file name: {}", path.display()))
}

fn mkdir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))
}

/// Empties `dir`, creating it if needed.
fn recreate(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("Cannot remove {}: {e}", dir.display()))?;
    }
    mkdir(dir)
}

fn remove_file(file: &Path) -> Result<(), String> {
    match fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("Cannot remove {}: {e}", file.display()))
        }
        _ => Ok(()),
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("Cannot copy {} to {}: {e}", from.display(), to.display()))
}

fn read(file: &Path) -> Result<String, String> {
    fs::read_to_string(file).map_err(|e| format!("Cannot read {}: {e}", file.display()))
}

fn write(file: &Path, text: &str) -> Result<(), String> {
    fs::write(file, text).map_err(|e| format!("Cannot write {}: {e}", file.display()))
}

fn append(file: &Path, text: &str) -> Result<(), String> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("Cannot write {}: {e}", file.display()))
}

/// The files directly in `dir`, sorted by name.
fn list(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    Ok(files)
}

/// The size of a file in bytes, 0 if it can't be read (only for display).
fn size(file: &Path) -> u64 {
    fs::metadata(file).map(|m| m.len()).unwrap_or(0)
}
