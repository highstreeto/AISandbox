//! Build script: the app version (`<crate version>+<commit>`) and a rebuild trigger for the embedded resources.

use std::process::Command;

/// The folder that is compiled into the binary (see `engine::embedded`).
const RESOURCES: &str = "resources";

fn main() {
    println!("cargo::rerun-if-changed={RESOURCES}");
    println!("cargo::rerun-if-changed=build.rs");
    for git_path in [".git/HEAD", ".git/refs"] {
        // A path that doesn't exist would make Cargo rebuild every time.
        if std::path::Path::new(git_path).exists() {
            println!("cargo::rerun-if-changed={git_path}");
        }
    }

    let version = env!("CARGO_PKG_VERSION");
    let full = match commit() {
        Some(sha) => format!("{version}+{sha}"),
        None => version.to_string(),
    };
    println!("cargo::rustc-env=SANDBOX_VERSION={full}");
}

/// The commit of HEAD, or `None` without git (a source tarball, or git refusing the folder).
fn commit() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}
