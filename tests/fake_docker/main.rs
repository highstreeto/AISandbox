//! Fake docker CLI for the sandbox tests.
//!
//!   FAKE_DOCKER_LOG    file; every call appends one JSON line {"name": "docker", "args": [...]}
//!   FAKE_DOCKER_RULES  file with a JSON array of {"when", "stdout", "stderr", "exit"}; the first rule whose
//!                      "when" is a substring of the space-joined arguments decides the output and exit code.
//!                      No match: no output, exit 0.
//!
//! The same binary is also installed as "xdg-open"; "name" (the program name it was called by) tells the two apart.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Serialize)]
struct Call<'a> {
    name: &'a str,
    args: &'a [String],
}

#[derive(Deserialize)]
struct Rule {
    when: Option<String>,
    stdout: Option<String>,
    stderr: Option<String>,
    #[serde(default)]
    exit: i32,
}

fn main() -> ExitCode {
    let mut argv = std::env::args_os();
    let program = argv.next().unwrap_or_default();
    let name = Path::new(&program)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "docker".to_string());
    let args: Vec<String> = argv.map(|a| a.to_string_lossy().into_owned()).collect();
    let joined = args.join(" ");

    if let Some(log) = std::env::var_os("FAKE_DOCKER_LOG").filter(|l| !l.is_empty()) {
        let mut line = serde_json::to_string(&Call {
            name: &name,
            args: &args,
        })
        .unwrap_or_default();
        line.push('\n');
        // Calls can overlap (the app polls `compose ps` while `up` runs): retry on a sharing violation.
        for attempt in 0.. {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .and_then(|mut f| f.write_all(line.as_bytes()));
            if result.is_ok() || attempt >= 50 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    if let Some(rules_file) = std::env::var_os("FAKE_DOCKER_RULES").filter(|r| !r.is_empty())
        && let Ok(text) = std::fs::read_to_string(&rules_file)
    {
        let rules: Vec<Rule> = serde_json::from_str(&text).unwrap_or_default();
        for r in rules {
            if !joined.contains(r.when.as_deref().unwrap_or("")) {
                continue;
            }
            print!("{}", r.stdout.unwrap_or_default());
            eprint!("{}", r.stderr.unwrap_or_default());
            let _ = std::io::stdout().flush();
            return ExitCode::from(u8::try_from(r.exit & 0xff).unwrap_or(1));
        }
    }
    ExitCode::SUCCESS
}
