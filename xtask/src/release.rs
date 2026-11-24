//! The release steps of `.github/workflows/release.yml`, runnable locally (`cargo xtask dist` is a dry run of all of
//! them for this machine's target; nothing is published).
//!
//! Everything goes under `target/dist/`:
//!
//! | Path | Written by |
//! |---|---|
//! | `notes.md`, `CHANGELOG.md` | `version` (git-cliff) |
//! | `stage/<target>/` | `package`: what goes into the archive |
//! | `files/` | `package`: `sandbox-<target>[.exe]` and `sandbox-<target>.tar.gz|.zip`, for every target built |
//! | `assets/` | `assets`: everything the GitHub release gets, with `VERSION` and `SHA256SUMS` |

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{
    append, cargo_with_env, copy, file_name, list, mkdir, output, path_str, read, recreate,
    remove_file, root_dir, run_in, size, write,
};

/// Set for the build to the version being released (read by `build.rs`).
const RELEASE_VERSION: &str = "SANDBOX_RELEASE_VERSION";
/// What every archive holds next to the binary, at its top level (no folder, no `./`).
const ARCHIVE_FILES: &[&str] = &["README.md", "LICENSE"];
/// Copied into the release next to the binaries.
const INSTALL_SCRIPTS: &[&str] = &["install/install.sh", "install/install.ps1"];

/// `target/dist/` (under `CARGO_TARGET_DIR` if set).
fn dist_dir() -> PathBuf {
    target_dir().join("dist")
}

fn target_dir() -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => root_dir().join(dir),
        None => root_dir().join("target"),
    }
}

// --- version ---

/// The next version from git-cliff (cliff.toml), and `notes.md` / `CHANGELOG.md` for it if there is a release.
///
/// Prints `tag`, `version` and `release` (and appends them to `$GITHUB_OUTPUT` in GitHub Actions). `release` is false
/// when nothing since the last tag is releasable: git-cliff then returns that tag.
pub fn version() -> Result<(), String> {
    let next = next_version()?;
    let lines = format!(
        "tag={}\nversion={}\nrelease={}\n",
        next.tag,
        next.version(),
        next.release
    );
    print!("{lines}");
    if let Some(path) = env::var_os("GITHUB_OUTPUT") {
        append(Path::new(&path), &lines)?;
    }
    if next.release {
        changelog(&next.tag)?;
    }
    Ok(())
}

struct Next {
    tag: String,
    release: bool,
}

impl Next {
    fn version(&self) -> &str {
        self.tag.trim_start_matches('v')
    }
}

fn next_version() -> Result<Next, String> {
    let last = match output(
        "git",
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
        ],
    ) {
        Ok((true, out)) => out.trim().to_string(),
        _ => String::new(),
    };
    let tag = git_cliff(&["--bumped-version"])?.trim().to_string();
    if tag.is_empty() {
        return Err("`git cliff --bumped-version` printed nothing.".into());
    }
    let release = last.is_empty() || tag != last;
    println!(
        "Last tag: {}, next: {tag}, release: {release}",
        if last.is_empty() { "none" } else { &last }
    );
    Ok(Next { tag, release })
}

/// `target/dist/notes.md` (this release's section) and `target/dist/CHANGELOG.md` (all of it).
fn changelog(tag: &str) -> Result<(), String> {
    let dist = dist_dir();
    mkdir(&dist)?;
    let notes = dist.join("notes.md");
    let changelog = dist.join("CHANGELOG.md");
    let notes_arg = path_str(&notes)?;
    let changelog_arg = path_str(&changelog)?;
    git_cliff(&[
        "--unreleased",
        "--tag",
        tag,
        "--strip",
        "all",
        "-o",
        &notes_arg,
    ])?;
    git_cliff(&["--tag", tag, "-o", &changelog_arg])?;
    println!("{}", read(&notes)?);
    println!("Written: {}, {}", notes.display(), changelog.display());
    Ok(())
}

fn git_cliff(args: &[&str]) -> Result<String, String> {
    let args = [&["cliff"], args].concat();
    match output("git", &args) {
        Ok((true, out)) => Ok(out),
        _ => Err(format!(
            "`git {}` failed. Is git-cliff installed (`cargo install git-cliff --locked`) and are the tags fetched?",
            args.join(" ")
        )),
    }
}

// --- package ---

pub struct PackageOptions {
    pub target: Option<String>,
    pub version: Option<String>,
    pub smoke: bool,
}

/// Builds the release binary for a target, checks its `--version` (unless `smoke` is off, for a target this machine
/// can't run), and writes `sandbox-<target>[.exe]` and the archive to `target/dist/files/`. The archive's entries are
/// checked, so a wrong layout (a folder, `./` entries) fails here and not for the people installing it.
pub fn package(options: &PackageOptions) -> Result<(), String> {
    let version = release_version(options.version.as_deref())?;
    let target = match &options.target {
        Some(target) => target.clone(),
        None => host_target()?,
    };
    let windows = target.contains("windows");
    let exe = if windows { "sandbox.exe" } else { "sandbox" };

    cargo_with_env(
        &[
            "build",
            "--release",
            "--locked",
            "--bin",
            "sandbox",
            "--target",
            &target,
        ],
        &[(RELEASE_VERSION, &version)],
    )?;
    let bin = target_dir().join(&target).join("release").join(exe);

    if options.smoke {
        let (ok, out) = output(&path_str(&bin)?, &["--version"])?;
        let out = out.trim();
        println!("{out}");
        let expected = format!("sandbox {version}+");
        if !ok || !out.starts_with(&expected) {
            return Err(format!(
                "Smoke test: expected `{expected}<commit>` from `{} --version`, got: {out}",
                bin.display()
            ));
        }
    }

    let dist = dist_dir();
    let stage = dist.join("stage").join(&target);
    let files = dist.join("files");
    recreate(&stage)?;
    mkdir(&files)?;
    copy(&bin, &stage.join(exe))?;
    for file in ARCHIVE_FILES {
        copy(&root_dir().join(file), &stage.join(file))?;
    }

    let name = format!("sandbox-{target}");
    let mut entries = vec![exe];
    entries.extend_from_slice(ARCHIVE_FILES);
    let archive = if windows {
        copy(&bin, &files.join(format!("{name}.exe")))?;
        let archive = files.join(format!("{name}.zip"));
        remove_file(&archive)?; // 7z would add to an existing archive
        // The file names, not `.` or `*`: exactly these entries, at the top level.
        let archive_arg = path_str(&archive)?;
        let args = [&["a", "-tzip", archive_arg.as_str()][..], &entries].concat();
        run_in(&stage, "7z", &args)?;
        archive
    } else {
        copy(&bin, &files.join(&name))?;
        let archive = files.join(format!("{name}.tar.gz"));
        // `-C` and the file names, not `.` (which adds `./` and `./sandbox`).
        let archive_arg = path_str(&archive)?;
        let stage_arg = path_str(&stage)?;
        let args = [
            &["-czf", archive_arg.as_str(), "-C", stage_arg.as_str()][..],
            &entries,
        ]
        .concat();
        run_in(&root_dir(), "tar", &args)?;
        archive
    };
    check_archive(&archive, &stage, &entries)?;
    println!("Packaged: {}", files.display());
    Ok(())
}

/// Fails unless the archive holds exactly `expected`, each at its top level. Prints the entries with their sizes
/// (from `stage`, where they were packed from).
fn check_archive(archive: &Path, stage: &Path, expected: &[&str]) -> Result<(), String> {
    let path = path_str(archive)?;
    let zip = path.ends_with(".zip");
    let (ok, out) = if zip {
        output("7z", &["l", "-slt", "-ba", &path])?
    } else {
        output("tar", &["-tzf", &path])?
    };
    if !ok {
        return Err(format!("Cannot list the entries of {path}."));
    }
    let listed: Vec<&str> = if zip {
        out.lines()
            .filter_map(|l| l.strip_prefix("Path = "))
            .filter(|p| *p != path) // the archive itself, should 7z list it
            .collect()
    } else {
        out.lines().collect()
    };
    if !same_entries(&listed, expected) {
        return Err(format!(
            "{path} holds {listed:?}, expected exactly {expected:?} (top level, no folders, no `./`)."
        ));
    }
    println!("{} ({}):", file_name(archive)?, human_size(size(archive)));
    for entry in listed {
        println!("  {}  {entry}", size_column(size(&stage.join(entry))));
    }
    Ok(())
}

/// Exactly the expected names, each once, in any order.
fn same_entries(listed: &[&str], expected: &[&str]) -> bool {
    let set: BTreeSet<&str> = listed.iter().copied().collect();
    set.len() == listed.len() && set == expected.iter().copied().collect()
}

// --- assets ---

/// Collects the release assets in `target/dist/assets/`: everything in `target/dist/files/`, the install scripts,
/// `CHANGELOG.md` (from `version`), `VERSION`, and `SHA256SUMS` over all of them.
pub fn assets(version: Option<&str>) -> Result<(), String> {
    let version = release_version(version)?;
    let dist = dist_dir();
    let files = dist.join("files");
    let assets = dist.join("assets");

    let built = list(&files)?;
    if built.is_empty() {
        return Err(format!(
            "Nothing in {}: run `cargo xtask package` first.",
            files.display()
        ));
    }
    let changelog = dist.join("CHANGELOG.md");
    if !changelog.is_file() {
        return Err(format!(
            "{} is missing: run `cargo xtask version` first.",
            changelog.display()
        ));
    }

    recreate(&assets)?;
    for file in &built {
        copy(file, &assets.join(file_name(file)?))?;
    }
    for script in INSTALL_SCRIPTS {
        let script = root_dir().join(script);
        copy(&script, &assets.join(file_name(&script)?))?;
    }
    copy(&changelog, &assets.join("CHANGELOG.md"))?;
    write(&assets.join("VERSION"), &format!("{version}\n"))?;

    // The `sha256sum` format (`<hex>  <name>`) that install.sh, install.ps1 and `sandbox upgrade` read.
    let mut sums = String::new();
    for file in list(&assets)? {
        let bytes = fs::read(&file).map_err(|e| format!("Cannot read {}: {e}", file.display()))?;
        sums += &format!("{}  {}\n", sha256_hex(&bytes), file_name(&file)?);
    }
    write(&assets.join("SHA256SUMS"), &sums)?;

    for file in list(&assets)? {
        println!("{}  {}", size_column(size(&file)), file_name(&file)?);
    }
    println!("Release assets: {}", assets.display());
    Ok(())
}

// --- dist (local dry run) ---

/// The whole release for this machine's target, without publishing: version and changelog (git-cliff; without it,
/// the given or crate version and a placeholder changelog), package, assets.
pub fn dist_dry_run(version: Option<&str>, smoke: bool) -> Result<(), String> {
    let version = match version.map(str::to_string).or_else(env_version) {
        Some(version) => {
            placeholder_changelog(&version)?;
            version
        }
        None => match next_version() {
            Ok(next) => {
                changelog(&next.tag)?;
                next.version().to_string()
            }
            Err(e) => {
                let version = env!("CARGO_PKG_VERSION").to_string();
                eprintln!(
                    "xtask: {e}\nxtask: Going on with version {version} and a placeholder changelog."
                );
                placeholder_changelog(&version)?;
                version
            }
        },
    };
    let version = version.trim_start_matches('v').to_string();
    package(&PackageOptions {
        target: None,
        version: Some(version.clone()),
        smoke,
    })?;
    assets(Some(&version))
}

fn placeholder_changelog(version: &str) -> Result<(), String> {
    let dist = dist_dir();
    mkdir(&dist)?;
    let note = "(Placeholder written by `cargo xtask dist`.)\n";
    write(
        &dist.join("CHANGELOG.md"),
        &format!("# Changelog\n\n## {version}\n\n{note}"),
    )?;
    write(&dist.join("notes.md"), note)
}

/// `--version`, else `SANDBOX_RELEASE_VERSION`; without a leading `v`.
fn release_version(version: Option<&str>) -> Result<String, String> {
    version
        .map(str::to_string)
        .or_else(env_version)
        .map(|v| v.trim_start_matches('v').to_string())
        .ok_or_else(|| {
            format!(
                "Which version? Pass --version <x.y.z> or set {RELEASE_VERSION} \
                 (`cargo xtask version` prints the next one)."
            )
        })
}

fn env_version() -> Option<String> {
    env::var(RELEASE_VERSION)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn host_target() -> Result<String, String> {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    match output(&rustc, &["-vV"])? {
        (true, out) => out
            .lines()
            .find_map(|l| l.strip_prefix("host: "))
            .map(str::to_string)
            .ok_or_else(|| "No `host:` line in `rustc -vV`.".into()),
        (false, _) => Err("`rustc -vV` failed.".into()),
    }
}

// --- Sizes ---

/// A size like `ls -lh` shows it, in binary units: `512 B`, `1.1 KiB`, `5.9 MiB`.
fn human_size(bytes: u64) -> String {
    let (number, unit) = size_parts(bytes);
    format!("{number} {unit}")
}

/// [`human_size`] for a listing: the number right-aligned, the unit left-aligned, so both line up.
fn size_column(bytes: u64) -> String {
    let (number, unit) = size_parts(bytes);
    format!("{number:>4} {unit:<3}")
}

fn size_parts(bytes: u64) -> (String, &'static str) {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let number = if unit == 0 {
        bytes.to_string()
    } else if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    (number, UNITS[unit])
}

/// The SHA-256 of `bytes` in lowercase hex, as `sha256sum` prints it.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_is_lowercase_like_sha256sum() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn archive_entries_must_match_exactly() {
        let expected = ["sandbox", "README.md", "LICENSE"];
        assert!(same_entries(
            &["LICENSE", "sandbox", "README.md"],
            &expected
        ));
        assert!(!same_entries(
            &["./", "./sandbox", "./README.md", "./LICENSE"],
            &expected
        ));
        assert!(!same_entries(&["sandbox", "README.md"], &expected));
        assert!(!same_entries(
            &["sandbox", "README.md", "LICENSE", "LICENSE"],
            &expected
        ));
    }

    #[test]
    fn human_size_uses_binary_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1126), "1.1 KiB");
        assert_eq!(human_size(52 * 1024), "52 KiB");
        assert_eq!(human_size(6_210_848), "5.9 MiB");
        assert_eq!(human_size(3 << 30), "3.0 GiB");
    }

    #[test]
    fn size_column_aligns_numbers_and_units() {
        assert_eq!(size_column(6), "   6 B  ");
        assert_eq!(size_column(510), " 510 B  ");
        assert_eq!(size_column(2867), " 2.8 KiB");
        assert_eq!(size_column(52 * 1024), "  52 KiB");
        assert_eq!(size_column(1000 * 1024), "1000 KiB");
    }

    #[test]
    fn release_version_drops_the_v() {
        assert_eq!(release_version(Some("v1.2.3")).unwrap(), "1.2.3");
        assert_eq!(release_version(Some("1.2.3")).unwrap(), "1.2.3");
    }
}
