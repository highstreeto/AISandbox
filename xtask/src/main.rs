//! Development tasks: what `.github/workflows/test.yml` runs (`cargo xtask ci`), runnable locally.
//!
//! No dependencies on purpose, so it builds in a moment and never needs the network.

use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

/// The build fails when total line coverage drops below this (percent). Raise it as coverage grows.
const MIN_LINE_COVERAGE: f64 = 90.0;

/// What `cargo test` and the coverage run cover (xtask itself is left out).
const TEST_ARGS: &[&str] = &[
    "--workspace",
    "--exclude",
    "xtask",
    "--all-targets",
    "--locked",
];

const HELP: &str = "\
Usage: cargo xtask <task>

Tasks:
  test       Run all tests (the `test` job in CI)
  coverage   Run all tests with coverage, write the HTML report and check the minimum (the `coverage` job)
             --open  open the HTML report afterwards
  ci         Everything CI checks: coverage, which runs all tests once
  help       Show this

Coverage needs, once:
  rustup component add llvm-tools-preview
  cargo install cargo-llvm-cov --locked";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["test"] => test(),
        ["coverage"] => coverage(false),
        ["coverage", "--open"] => coverage(true),
        ["ci"] => coverage(false),
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

fn test() -> Result<(), String> {
    cargo(&[&["test"], TEST_ARGS].concat())
}

fn coverage(open: bool) -> Result<(), String> {
    let installed = cargo_command(&["llvm-cov", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        return Err(
            "cargo-llvm-cov is missing. Install it with `rustup component add llvm-tools-preview` \
                    and `cargo install cargo-llvm-cov --locked`."
                .into(),
        );
    }
    cargo(&[&["llvm-cov", "--no-report"], TEST_ARGS].concat())?;
    cargo(&["llvm-cov", "report", "--html"])?;
    let html = root().join("target/llvm-cov/html/index.html");
    println!("HTML report: {}", html.display());

    let min = format!("{MIN_LINE_COVERAGE}");
    let (passed, summary) = cargo_output(&[
        "llvm-cov",
        "report",
        "--summary-only",
        "--fail-under-lines",
        &min,
    ])?;
    println!("{summary}");
    if let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                format!(
                    "Cannot write the job summary {}: {e}",
                    PathBuf::from(&path).display()
                )
            })?;
        writeln!(
            file,
            "## Coverage\n\nMinimum line coverage: {min}%\n\n```\n{summary}```"
        )
        .map_err(|e| format!("Cannot write the job summary: {e}"))?;
    }
    if open {
        cargo(&["llvm-cov", "report", "--open"])?;
    }
    if passed {
        println!("Line coverage is at least {min}%.");
        Ok(())
    } else {
        Err(format!(
            "Line coverage is below {min}%. Add tests, or lower MIN_LINE_COVERAGE in xtask/src/main.rs."
        ))
    }
}

/// The workspace root (the parent of `xtask/`), where every cargo command runs.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ has a parent")
        .to_path_buf()
}

fn cargo_command(args: &[&str]) -> Command {
    let mut cmd = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.args(args).current_dir(root());
    cmd
}

/// Runs cargo with the terminal attached; fails if it fails.
fn cargo(args: &[&str]) -> Result<(), String> {
    println!("$ cargo {}", args.join(" "));
    let status = cargo_command(args)
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
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("Cannot run cargo: {e}"))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}
