//! The `sandbox` command: parse, run, print the error, exit code. Nothing else.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // The exit code of `shell`, `claude` and `logs` (docker's own) is passed through unchanged.
    let code = ai_sandbox::run(args);
    // Returning (rather than `process::exit`) lets the process end normally: on Windows `process::exit` is
    // `ExitProcess`, which skips the exit handlers, so a coverage build would write no profile. Codes that don't
    // fit an `ExitCode` (Windows: above 255 or negative) still go through `process::exit`.
    match u8::try_from(code) {
        Ok(code) => ExitCode::from(code),
        Err(_) => std::process::exit(code),
    }
}
