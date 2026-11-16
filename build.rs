//! Build script: the app version (`<version>+<commit>`), the target triple, and a rebuild trigger for the embedded
//! resources.
//!
//! The version is, in order: `SANDBOX_RELEASE_VERSION` (set by the release workflow, which also marks the build as a
//! release so `sandbox upgrade` works), the newest `vX.Y.Z` tag, or the crate version.

use std::process::Command;

/// The folder that is compiled into the binary (see `engine::embedded`).
const RESOURCES: &str = "resources";
/// Set by `.github/workflows/release.yml` to the version being released.
const RELEASE_VERSION: &str = "SANDBOX_RELEASE_VERSION";

fn main() {
    println!("cargo::rerun-if-changed={RESOURCES}");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed={RELEASE_VERSION}");
    for git_path in [".git/HEAD", ".git/refs", ".git/packed-refs"] {
        // A path that doesn't exist would make Cargo rebuild every time.
        if std::path::Path::new(git_path).exists() {
            println!("cargo::rerun-if-changed={git_path}");
        }
    }

    let release = std::env::var(RELEASE_VERSION)
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty());
    let version = release
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let full = match commit() {
        Some(sha) => format!("{version}+{sha}"),
        None => version,
    };
    println!("cargo::rustc-env=SANDBOX_VERSION={full}");
    println!(
        "cargo::rustc-env=SANDBOX_RELEASE_BUILD={}",
        if release.is_some() { "1" } else { "" }
    );
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo::rustc-env=SANDBOX_TARGET={target}");
}

/// The commit of HEAD, or `None` without git (a source tarball, or git refusing the folder).
fn commit() -> Option<String> {
    git(&["rev-parse", "HEAD"])
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}
