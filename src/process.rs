//! Child processes (captured, or attached to this terminal), `docker` lookup, the browser, the host user's ids.

use crate::ui::Ui;
use anyhow::{Result, bail};
use std::process::{Command, Stdio};
use std::sync::Once;

/// The exit code and output of a finished child process.
#[derive(Debug, Clone)]
pub struct ProcResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ProcResult {
    /// The non-empty lines of stdout, trimmed.
    pub fn lines(&self) -> Vec<String> {
        self.stdout
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Runs a process with its output captured and no input. A program that can't be started gives exit code 127.
pub fn capture<S: AsRef<str>>(ui: &Ui, verbose: bool, file: &str, args: &[S]) -> ProcResult {
    trace(ui, verbose, file, args);
    let output = Command::new(file)
        .args(args.iter().map(AsRef::as_ref))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(o) => ProcResult {
            exit_code: exit_code(o.status),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => ProcResult {
            exit_code: 127,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// Attached to the terminal. Ctrl+C goes to the child (`logs -f`, `shell`); this app stays alive and returns the
/// child's exit code.
pub fn interactive<S: AsRef<str>>(ui: &Ui, verbose: bool, file: &str, args: &[S]) -> Result<i32> {
    trace(ui, verbose, file, args);
    keep_alive_on_ctrl_c();
    let status = Command::new(file)
        .args(args.iter().map(AsRef::as_ref))
        .status();
    match status {
        Ok(s) => Ok(exit_code(s)),
        Err(e) => bail!("Could not run {file}: {e}"),
    }
}

/// A no-op Ctrl+C handler, installed once: the foreground child gets the signal too and decides what to do.
fn keep_alive_on_ctrl_c() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ctrlc::set_handler(|| {});
    });
}

/// The exit code; killed by a signal: 128 + the signal number (Unix).
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

/// Whether an executable is on `PATH` (with `PATHEXT` on Windows).
pub fn on_path(name: &str) -> bool {
    which::which(name).is_ok()
}

/// `$ docker …` in dim, with arguments that contain a space in double quotes (`-v`).
fn trace<S: AsRef<str>>(ui: &Ui, verbose: bool, file: &str, args: &[S]) {
    if !verbose {
        return;
    }
    let mut parts = vec![file.to_string()];
    parts.extend(args.iter().map(|a| {
        let a = a.as_ref();
        if a.contains(' ') {
            format!("\"{a}\"")
        } else {
            a.to_string()
        }
    }));
    println!("{}", ui.dim(format!("$ {}", parts.join(" "))));
}

/// Opens the URL in the default browser, or prints it if that isn't possible.
pub fn open_browser(url: &str) {
    if !try_open_browser(url) {
        println!("Open {url} in your browser.");
    }
}

#[cfg(windows)]
fn try_open_browser(url: &str) -> bool {
    Command::new("cmd")
        .args(["/C", "start", "", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// The first of `xdg-open`, `open`, `wslview`, `explorer.exe` on `PATH`; started with the URL as its only argument,
/// output discarded, not waited for.
#[cfg(not(windows))]
fn try_open_browser(url: &str) -> bool {
    for opener in ["xdg-open", "open", "wslview", "explorer.exe"] {
        if on_path(opener) {
            return Command::new(opener)
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok();
        }
    }
    false
}

/// The host user's ids for `PUID`/`PGID` in the desktop container: 1000 on Windows, and never root.
pub fn user_ids() -> (u32, u32) {
    #[cfg(unix)]
    {
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        if uid != 0 {
            return (uid, gid);
        }
    }
    (1000, 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_trimmed_and_non_empty() {
        let r = ProcResult {
            exit_code: 0,
            stdout: " a \r\n\n b\n".into(),
            stderr: String::new(),
        };
        assert_eq!(r.lines(), vec!["a", "b"]);
    }

    #[test]
    fn a_missing_program_is_exit_127() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let r = capture(&ui, false, "definitely-not-a-program-xyz", &["a"]);
        assert_eq!(r.exit_code, 127);
        assert!(!r.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn captured_output_and_exit_code() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let r = capture(&ui, false, "sh", &["-c", "echo out; echo err >&2; exit 3"]);
        assert_eq!(
            (r.exit_code, r.stdout.as_str(), r.stderr.as_str()),
            (3, "out\n", "err\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_is_128_plus_its_number() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let r = capture(&ui, false, "sh", &["-c", "kill -TERM $$"]);
        assert_eq!(r.exit_code, 128 + 15);
    }

    #[test]
    fn user_ids_are_never_root() {
        assert_ne!(user_ids().0, 0);
    }
}
