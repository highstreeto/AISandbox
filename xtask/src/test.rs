//! The test tasks: what `.github/workflows/test.yml` runs (`cargo xtask ci`).

use std::env;
use std::fs;
use std::path::Path;

use crate::{append, cargo, cargo_command, cargo_output, root_dir};

/// The build fails when total line coverage drops below this (percent). Raise it as coverage grows.
const MIN_LINE_COVERAGE: f64 = 90.0;

/// The coverage summary as markdown, relative to the repository root (posted by `.github/workflows/test.yml`).
const SUMMARY_FILE: &str = "target/llvm-cov/summary.md";

/// What `cargo test` and the coverage run cover (xtask itself is left out).
const TEST_ARGS: &[&str] = &[
    "--workspace",
    "--exclude",
    "xtask",
    "--all-targets",
    "--locked",
];

/// Everything CI checks: xtask's own tests, then coverage, which runs all tests once.
pub fn ci() -> Result<(), String> {
    cargo(&["test", "--package", "xtask", "--locked"])?;
    coverage(false)
}

/// All tests.
pub fn tests() -> Result<(), String> {
    cargo(&[&["test"], TEST_ARGS].concat())
}

/// All tests with coverage, the HTML report, and the minimum check.
pub fn coverage(open: bool) -> Result<(), String> {
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
    let html = root_dir().join("target/llvm-cov/html/index.html");
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
    let markdown =
        format!("## Coverage\n\nMinimum line coverage: {min}%\n\n```\n{summary}```\n");
    // Also a file, which the CI workflow posts as a pull request comment.
    let file = root_dir().join(SUMMARY_FILE);
    fs::write(&file, &markdown).map_err(|e| format!("Cannot write {}: {e}", file.display()))?;
    if let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") {
        append(Path::new(&path), &markdown)?;
    }
    if open {
        cargo(&["llvm-cov", "report", "--open"])?;
    }
    if passed {
        println!("Line coverage is at least {min}%.");
        Ok(())
    } else {
        Err(format!(
            "Line coverage is below {min}%. Add tests, or lower MIN_LINE_COVERAGE in xtask/src/test.rs."
        ))
    }
}
