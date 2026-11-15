//! `upgrade`: replaces this binary with a newer release from GitHub (see `.github/workflows/release.yml`).
//!
//! Each release has the raw binaries (`sandbox-<target>[.exe]`), `SHA256SUMS` and a one-line `VERSION`, so no API,
//! JSON or archive handling is needed. Only release builds can upgrade; a build from source is updated with git.

use crate::VERSION;
use crate::ui::Ui;
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

/// Where the releases are published.
pub const DEFAULT_RELEASES_URL: &str = "https://github.com/highstreeto/AISandbox/releases";
/// Overrides [`DEFAULT_RELEASES_URL`] (for tests and forks).
pub const RELEASES_URL_VARIABLE: &str = "SANDBOX_RELEASE_URL";
/// The target triple this binary was built for (set by `build.rs`).
const TARGET: &str = env!("SANDBOX_TARGET");
/// Downloads larger than this are refused.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// Whether this binary was built by the release workflow (`build.rs` sets the flag to `1`, or empty otherwise).
pub fn is_release_build() -> bool {
    !env!("SANDBOX_RELEASE_BUILD").is_empty()
}

/// `sandbox upgrade [VERSION] [--check]`.
pub fn run(ui: &Ui, check: bool, wanted: Option<&str>) -> Result<i32> {
    if !is_release_build() {
        bail!(
            "This sandbox was built from source and can't upgrade itself. Update it with git pull, or install a \
             release with the install script (see README.md)."
        );
    }
    let exe = std::env::current_exe().context("Could not find the path of this program")?;
    let dir = exe
        .parent()
        .context("Could not find the folder of this program")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let releases = Releases {
        base: std::env::var(RELEASES_URL_VARIABLE)
            .unwrap_or_else(|_| DEFAULT_RELEASES_URL.to_string()),
        agent,
    };
    let request = Request {
        current: VERSION,
        target: TARGET,
        check,
        wanted,
    };
    upgrade(ui, &releases, &request, dir, &|new| {
        self_replace::self_replace(new)
    })
}

/// Where to download from.
pub struct Releases {
    /// e.g. `https://github.com/<owner>/<repo>/releases`
    pub base: String,
    pub agent: ureq::Agent,
}

impl Releases {
    fn download_url(&self, tag: &str, file: &str) -> String {
        format!("{}/download/{tag}/{file}", self.base.trim_end_matches('/'))
    }

    fn latest_url(&self, file: &str) -> String {
        format!("{}/latest/download/{file}", self.base.trim_end_matches('/'))
    }

    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self.agent.get(url).call().map_err(|e| match e {
            ureq::Error::StatusCode(404) => anyhow!("Not found: {url}"),
            e => anyhow!(
                "Download of {url} failed: {e}. Check your internet connection (and HTTPS_PROXY)."
            ),
        })?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .with_context(|| format!("Download of {url} failed"))
    }
}

/// What to upgrade from and to.
pub struct Request<'a> {
    /// The running version, e.g. `1.2.3+<commit>`.
    pub current: &'a str,
    /// The target triple, picks the binary.
    pub target: &'a str,
    /// Only report whether a newer version exists.
    pub check: bool,
    /// A specific version (`1.2.3` or `v1.2.3`) instead of the newest; may be older.
    pub wanted: Option<&'a str>,
}

/// Downloads, verifies and installs the release; `replace` puts the verified file (written into `dir`) in place.
///
/// Returns the exit code: 0, or 1 for `--check` when a newer version is available.
pub fn upgrade(
    ui: &Ui,
    releases: &Releases,
    request: &Request,
    dir: &Path,
    replace: &dyn Fn(&Path) -> io::Result<()>,
) -> Result<i32> {
    let current = Version::parse(request.current).with_context(|| {
        format!(
            "The version of this program ({}) is not a release version",
            request.current
        )
    })?;
    let target = match request.wanted {
        Some(wanted) => Version::parse(wanted).with_context(|| {
            format!("{wanted} is not a version. Give one like 1.2.3 (see the releases on GitHub).")
        })?,
        None => {
            let text = releases.get(&releases.latest_url("VERSION"))?;
            let text = String::from_utf8_lossy(&text);
            Version::parse(text.trim()).with_context(|| {
                format!(
                    "The latest release has an invalid VERSION file: {}",
                    text.trim()
                )
            })?
        }
    };

    if target == current {
        ui.success(&format!("Already up to date ({current})."));
        return Ok(0);
    }
    if request.wanted.is_none() && target < current {
        ui.success(&format!(
            "Already up to date ({current}, the latest release is {target})."
        ));
        return Ok(0);
    }
    if request.check {
        println!("sandbox {target} is available (installed: {current}). Run: sandbox upgrade");
        return Ok(1);
    }

    let tag = format!("v{target}");
    let asset = asset_name(request.target);
    let sums = releases
        .get(&releases.download_url(&tag, "SHA256SUMS"))
        .with_context(|| {
            format!("Release {tag} not found (see the releases on GitHub for valid versions)")
        })?;
    let expected = checksum_for(&String::from_utf8_lossy(&sums), &asset)
        .with_context(|| format!("Release {tag} has no {asset} (no build for this system)."))?;
    let binary = ui.spin(&format!("Downloading sandbox {target}"), || {
        releases.get(&releases.download_url(&tag, &asset))
    })?;
    let actual = hex(&Sha256::digest(&binary));
    if actual != expected {
        bail!(
            "The download of {asset} is damaged (checksum mismatch). Nothing was changed; try again."
        );
    }

    let new_file = dir.join(format!(".sandbox-upgrade-{}", std::process::id()));
    write_executable(&new_file, &binary).with_context(|| {
        format!(
            "Could not write to {}. Re-run the install script, or run this with the rights to change that folder.",
            dir.display()
        )
    })?;
    let replaced = replace(&new_file);
    let _ = fs::remove_file(&new_file);
    replaced.with_context(|| format!("Could not replace the program in {}", dir.display()))?;

    ui.success(&format!("Upgraded sandbox {current} \u{2192} {target}."));
    println!(
        "Engine files are refreshed by the next command; see the release notes whether to run: sandbox rebuild"
    );
    Ok(0)
}

/// The release asset for a target triple: `sandbox-<target>`, with `.exe` on Windows.
pub fn asset_name(target: &str) -> String {
    if target.contains("windows") {
        format!("sandbox-{target}.exe")
    } else {
        format!("sandbox-{target}")
    }
}

/// The SHA-256 of `file` from a `sha256sum` listing (`<hex>  <name>`, or `<hex> *<name>` in binary mode).
pub fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let sum = parts.next()?;
        let name = parts.next()?;
        (name.trim_start_matches('*') == file).then(|| sum.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// A release version `major.minor.patch`; a leading `v` and a `+<commit>` suffix are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim().trim_start_matches('v');
        let text = text.split('+').next()?;
        let mut parts = text.split('.').map(|p| p.parse::<u64>().ok());
        let version = Version(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    const TARGET: &str = "x86_64-unknown-linux-musl";

    #[test]
    fn versions_parse_with_v_and_commit() {
        assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("v1.2.3\n"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("0.1.0+d4837d352b29"), Some(Version(0, 1, 0)));
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("latest"), None);
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
    }

    #[test]
    fn asset_names_follow_the_release_workflow() {
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc"),
            "sandbox-x86_64-pc-windows-msvc.exe"
        );
        assert_eq!(
            asset_name("aarch64-apple-darwin"),
            "sandbox-aarch64-apple-darwin"
        );
    }

    #[test]
    fn checksums_are_found_in_text_and_binary_mode() {
        let sums = "AB12  sandbox-a.tar.gz\ncd34 *sandbox-a\n";
        assert_eq!(
            checksum_for(sums, "sandbox-a.tar.gz").as_deref(),
            Some("ab12")
        );
        assert_eq!(checksum_for(sums, "sandbox-a").as_deref(), Some("cd34"));
        assert_eq!(checksum_for(sums, "sandbox-b"), None);
    }

    /// Serves `files` (path → body) over HTTP on localhost; anything else is a 404.
    fn serve(files: HashMap<String, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                let (status, body) = match files.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", b"not found".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{address}/releases")
    }

    fn release_files(version: &str, binary: &[u8], sum: Option<&str>) -> HashMap<String, Vec<u8>> {
        let asset = asset_name(TARGET);
        let sum = sum
            .map(str::to_string)
            .unwrap_or_else(|| hex(&Sha256::digest(binary)));
        HashMap::from([
            (
                "/releases/latest/download/VERSION".to_string(),
                format!("{version}\n").into_bytes(),
            ),
            (
                format!("/releases/download/v{version}/SHA256SUMS"),
                format!("{sum}  {asset}\n").into_bytes(),
            ),
            (
                format!("/releases/download/v{version}/{asset}"),
                binary.to_vec(),
            ),
        ])
    }

    fn releases(base: String) -> Releases {
        let agent = ureq::Agent::config_builder().proxy(None).build().into();
        Releases { base, agent }
    }

    fn quiet_ui() -> Ui {
        Ui {
            interactive: false,
            width: 100,
        }
    }

    /// Runs an upgrade with `current` installed into a temp folder whose `sandbox` file holds "old".
    fn run_upgrade(
        files: HashMap<String, Vec<u8>>,
        current: &str,
        check: bool,
        wanted: Option<&str>,
    ) -> (Result<i32>, Vec<u8>, usize) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("sandbox");
        fs::write(&exe, b"old").unwrap();
        let request = Request {
            current,
            target: TARGET,
            check,
            wanted,
        };
        let replace = |new: &Path| fs::copy(new, &exe).map(|_| ());
        let result = upgrade(
            &quiet_ui(),
            &releases(serve(files)),
            &request,
            dir.path(),
            &replace,
        );
        let installed = fs::read(&exe).unwrap();
        let leftovers = fs::read_dir(dir.path()).unwrap().count() - 1;
        (result, installed, leftovers)
    }

    #[test]
    fn up_to_date_changes_nothing() {
        let (result, installed, _) = run_upgrade(
            release_files("1.2.3", b"new", None),
            "1.2.3+abc",
            false,
            None,
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(installed, b"old");
    }

    #[test]
    fn an_older_latest_release_is_not_a_downgrade() {
        let (result, installed, _) =
            run_upgrade(release_files("1.2.2", b"new", None), "1.2.3", false, None);
        assert_eq!(result.unwrap(), 0);
        assert_eq!(installed, b"old");
    }

    #[test]
    fn check_reports_a_newer_version_with_exit_code_1() {
        let (result, installed, _) =
            run_upgrade(release_files("1.3.0", b"new", None), "1.2.3", true, None);
        assert_eq!(result.unwrap(), 1);
        assert_eq!(installed, b"old");
    }

    #[test]
    fn a_newer_release_is_downloaded_verified_and_installed() {
        let (result, installed, leftovers) = run_upgrade(
            release_files("1.3.0", b"new", None),
            "1.2.3+abc",
            false,
            None,
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(installed, b"new");
        assert_eq!(leftovers, 0, "the downloaded file is cleaned up");
    }

    #[test]
    fn a_specific_older_version_can_be_installed() {
        let (result, installed, _) = run_upgrade(
            release_files("1.0.0", b"older", None),
            "1.2.3",
            false,
            Some("v1.0.0"),
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(installed, b"older");
    }

    #[test]
    fn a_checksum_mismatch_changes_nothing() {
        let files = release_files("1.3.0", b"new", Some(&"0".repeat(64)));
        let (result, installed, leftovers) = run_upgrade(files, "1.2.3", false, None);
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("checksum mismatch"), "{message}");
        assert_eq!(installed, b"old");
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn an_unknown_version_says_where_to_look() {
        let (result, installed, _) = run_upgrade(
            release_files("1.3.0", b"new", None),
            "1.2.3",
            false,
            Some("9.9.9"),
        );
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("Release v9.9.9 not found"), "{message}");
        assert_eq!(installed, b"old");
    }

    #[test]
    fn a_missing_build_for_this_system_is_reported() {
        let mut files = release_files("1.3.0", b"new", None);
        files.insert(
            "/releases/download/v1.3.0/SHA256SUMS".into(),
            b"ab  sandbox-other\n".to_vec(),
        );
        let (result, _, _) = run_upgrade(files, "1.2.3", false, None);
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("no build for this system"), "{message}");
    }
}
