# Sandbox app end-to-end tests

End-to-end tests for the sandbox control app (the Cargo package in the repository root). Every command runs the built
`sandbox` binary as a process against a **fake `docker`** executable. The tests check the exact docker calls, exit
codes and messages, and the generated `.state/compose.yml`.

```bash
cargo test
```

Run it from the repository root. Needs a Rust toolchain (1.88 or newer) and access to crates.io for the first build. No
Docker needed. Runs on Windows, Linux and macOS, and takes a few seconds once built (the unit tests inside the modules
run first, then the end-to-end tests in `tests/e2e`).

## How it works

- **`fake_docker/main.rs`** is a tiny program, a second binary of the same package (`fake-docker`, `test = false`).
  The test fixture links it (Unix) or copies it (Windows) as `docker[.exe]` and `xdg-open[.exe]`. Each call appends
  `{"name", "args"}` as a JSON line to `$FAKE_DOCKER_LOG`. Its output and exit code come from the rules in
  `$FAKE_DOCKER_RULES`: the first rule whose `when` is a substring of the joined arguments wins.
- **The app and the fake docker** are built by Cargo along with the tests. Cargo gives the test program their paths as
  `env!("CARGO_BIN_EXE_sandbox")` and `env!("CARGO_BIN_EXE_fake-docker")`.
- **`Fixture`** (`e2e/support.rs`) creates a temporary `demo-project/.sandbox/` (with a `.git/config` and
  `.git/hooks`), with fixed `sandbox.toml`, `allowlist.txt` and `.state/id` (`0123abcd`). **Every test creates its
  own fixture** (a temp folder that is deleted when the test ends), so the tests run in parallel. The engine is not
  copied: the app writes it into `.state/desktop` and `.state/proxy` itself, from the files compiled into the binary.
  `desktop_context()` and `proxy_context()` point at those folders; `init_templates_dir()` is the app's
  `resources/init` source folder.
- **`Fixture::run(...)`** runs `sandbox <args>` with `SANDBOX_DIR` pointing at the test project's `.sandbox`, as the
  launchers do, and with the fake bin directory first on `PATH`. The default rules answer like a healthy, running
  sandbox, and a test can put its own rules in front of them (`RunOpts::rules`). `RunOpts::cwd` and
  `sandbox_dir("")` run it without `SANDBOX_DIR`, to test the search upwards from the current directory.
  `assert_calls`, `assert_exit` and `assert_output` check the result.
- **Unit tests** for pure logic (YAML quoting, `with_header`, `sanitize`, path normalising, the allowlist edits, number
  formatting, the config reader) sit in `#[cfg(test)] mod tests` inside their modules. They are an extra; the end-to-end
  tests are the specification.

| File | Covers |
|---|---|
| `e2e/commands.rs` | Every command and alias, its docker calls in order, `-v`, `[project] name`, finding `.sandbox` |
| `e2e/errors.rs` | Docker missing or not running, failing docker calls, invalid `sandbox.toml` (every check) / hosts, old `sandbox.yml` |
| `e2e/compose_file.rs` | The generated compose file: exact text, each setting, escaping, shares |
| `e2e/engine.rs` | The engine written to `.state/desktop` and `.state/proxy`: headers, manifest, hand edits, leftovers, old `engine/` layout, `info` Engine row |
| `e2e/init.rs` | `sandbox init`: created and kept files, `.state`, parent `.sandbox` warning, missing folder, no Docker |
