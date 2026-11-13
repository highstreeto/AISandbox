//! The `sandbox` command: parse, run, print the error, exit code. Nothing else.

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // `exit` passes the exit code of `shell`, `claude` and `logs` (docker's own) through unchanged.
    std::process::exit(ai_sandbox::run(args));
}
