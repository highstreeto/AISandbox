//! Child processes (captured, or attached to this terminal), program lookup, the browser, the host user's ids.

use crate::config::BrowserSetting;
use crate::ui::Ui;
use anyhow::{Result, bail};
use std::io::{self, Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::Once;
use std::thread::JoinHandle;

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

/// Runs a process with its output captured, extra environment variables and no input. `echo` also copies its
/// output to this terminal as it arrives. A program that can't be started gives exit code 127.
pub fn capture<S: AsRef<str>>(
    ui: &Ui,
    verbose: bool,
    file: &str,
    args: &[S],
    envs: &[(&str, &str)],
    echo: bool,
) -> ProcResult {
    trace(ui, verbose, file, args);
    let mut cmd = Command::new(file);
    cmd.args(args.iter().map(AsRef::as_ref))
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = if echo {
        output_echoed(&mut cmd)
    } else {
        cmd.output()
    };
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

/// [`Command::output`], with stdout and stderr also copied to this process's stdout and stderr as they arrive.
fn output_echoed(cmd: &mut Command) -> io::Result<Output> {
    let mut child = cmd.spawn()?;
    let stdout = tee(child.stdout.take().expect("stdout is piped"), io::stdout());
    let stderr = tee(child.stderr.take().expect("stderr is piped"), io::stderr());
    let status = child.wait()?;
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Reads `src` to the end on a new thread, copying it to `sink` and returning all of it. When `sink` fails (a
/// closed stdout: `sandbox up -v | head`) the copying stops but the reading goes on, so the child never blocks on a
/// full pipe and the output is still captured.
fn tee(
    mut src: impl Read + Send + 'static,
    mut sink: impl Write + Send + 'static,
) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut copying = true;
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if copying {
                        copying = sink
                            .write_all(&buf[..n])
                            .and_then(|()| sink.flush())
                            .is_ok();
                    }
                    captured.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        captured
    })
}

/// Attached to the terminal, with extra environment variables. Ctrl+C goes to the child (`logs -f`, `shell`); this
/// app stays alive and returns the child's exit code.
pub fn interactive<S: AsRef<str>>(
    ui: &Ui,
    verbose: bool,
    file: &str,
    args: &[S],
    envs: &[(&str, &str)],
) -> Result<i32> {
    trace(ui, verbose, file, args);
    keep_alive_on_ctrl_c();
    let status = Command::new(file)
        .args(args.iter().map(AsRef::as_ref))
        .envs(envs.iter().copied())
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

/// Opens the URL as `[host] browser` says: the default browser (or the URL is printed if there is none), the
/// configured command, or nothing. A configured command that can't be started is a warning, not a fallback.
pub fn open_browser(ui: &Ui, url: &str, browser: &BrowserSetting) {
    match browser {
        BrowserSetting::Off => {}
        BrowserSetting::Default => {
            if !try_open_browser(url) {
                println!("Open {url} in your browser.");
            }
        }
        BrowserSetting::Command(cmd) => {
            let (program, args) = browser_command(cmd, url);
            let (program, args) = if cfg!(windows) {
                start_command(&program, args)
            } else {
                (program, args)
            };
            if let Err(e) = spawn_detached(&program, &args) {
                ui.warn(&format!(
                    "Could not start the browser \"{program}\": {e}. Open {url} in your browser."
                ));
            }
        }
    }
}

/// The program and arguments for a configured browser: the URL is the last argument, or replaces `{url}` where an
/// argument contains it.
pub fn browser_command(cmd: &[String], url: &str) -> (String, Vec<String>) {
    let (program, rest) = cmd.split_first().map_or(("", &[][..]), |(p, r)| (p, r));
    let args = if rest.iter().any(|a| a.contains("{url}")) {
        rest.iter().map(|a| a.replace("{url}", url)).collect()
    } else {
        rest.iter().cloned().chain([url.to_string()]).collect()
    };
    (program.to_string(), args)
}

/// Windows: a bare program name goes through `cmd /C start`, which also finds apps registered with Windows
/// (`firefox`, `msedge`, `chrome`) that are not on `PATH`; a path is started directly.
fn start_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    if program.contains(['/', '\\']) {
        return (program.to_string(), args);
    }
    let mut all: Vec<String> = ["/C", "start", "", program]
        .iter()
        .map(|s| s.to_string())
        .collect();
    all.extend(args);
    ("cmd".to_string(), all)
}

/// Started with its output discarded and not waited for.
fn spawn_detached(program: &str, args: &[String]) -> std::io::Result<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
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
        let r = capture(
            &ui,
            false,
            "definitely-not-a-program-xyz",
            &["a"],
            &[],
            false,
        );
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
        let r = capture(
            &ui,
            false,
            "sh",
            &["-c", "echo out; echo err >&2; exit 3"],
            &[],
            false,
        );
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
        let r = capture(&ui, false, "sh", &["-c", "kill -TERM $$"], &[], false);
        assert_eq!(r.exit_code, 128 + 15);
    }

    #[cfg(unix)]
    #[test]
    fn echoed_output_is_captured_too() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let r = capture(
            &ui,
            false,
            "sh",
            &["-c", "echo \"$X\"; echo err >&2; exit 3"],
            &[("X", "out")],
            true,
        );
        assert_eq!(
            (r.exit_code, r.stdout.as_str(), r.stderr.as_str()),
            (3, "out\n", "err\n")
        );
    }

    #[test]
    fn a_missing_program_is_exit_127_when_echoed() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let r = capture(
            &ui,
            false,
            "definitely-not-a-program-xyz",
            &["a"],
            &[],
            true,
        );
        assert_eq!(r.exit_code, 127);
        assert!(!r.stderr.is_empty());
    }

    /// A sink that keeps what it gets, shared with the test.
    #[derive(Clone, Default)]
    struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A sink like a closed stdout.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn tee_copies_and_captures_everything() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let sink = Shared::default();
        let captured = tee(io::Cursor::new(data.clone()), sink.clone())
            .join()
            .unwrap();
        assert_eq!(captured, data);
        assert_eq!(*sink.0.lock().unwrap(), data);
    }

    #[test]
    fn tee_keeps_reading_when_the_sink_is_closed() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let captured = tee(io::Cursor::new(data.clone()), Closed).join().unwrap();
        assert_eq!(captured, data);
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_url_is_the_last_argument() {
        let (program, args) = browser_command(&texts(&["chromium", "--incognito"]), "https://x");
        assert_eq!(program, "chromium");
        assert_eq!(args, texts(&["--incognito", "https://x"]));
        let (program, args) = browser_command(&texts(&["firefox"]), "https://x");
        assert_eq!((program.as_str(), args), ("firefox", texts(&["https://x"])));
    }

    #[test]
    fn url_placeholder_replaces_the_last_argument() {
        let (program, args) = browser_command(
            &texts(&["chrome", "--app={url}", "--new-window"]),
            "https://x",
        );
        assert_eq!(program, "chrome");
        assert_eq!(args, texts(&["--app=https://x", "--new-window"]));
    }

    #[test]
    fn windows_starts_a_bare_name_through_cmd_and_a_path_directly() {
        let (program, args) = start_command("firefox", texts(&["-private", "https://x"]));
        assert_eq!(program, "cmd");
        assert_eq!(
            args,
            texts(&["/C", "start", "", "firefox", "-private", "https://x"])
        );
        let (program, args) = start_command("C:\\Apps\\ff.exe", texts(&["https://x"]));
        assert_eq!(
            (program.as_str(), args),
            ("C:\\Apps\\ff.exe", texts(&["https://x"]))
        );
        assert_eq!(start_command("./ff", vec![]).0, "./ff");
    }

    #[test]
    fn a_browser_that_cannot_start_is_an_error() {
        assert!(spawn_detached("definitely-not-a-program-xyz", &[]).is_err());
    }

    #[test]
    fn user_ids_are_never_root() {
        assert_ne!(user_ids().0, 0);
    }
}
