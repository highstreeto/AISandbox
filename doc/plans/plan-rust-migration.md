# Plan: migrate the control app to Rust

> **Status:** implemented (steps 1 to 8); step 9 (smoke test against real Docker) is left for the host. See *As built*
> at the end. Implementation: Sonnet, in the order of *Steps*.
> Follows [plan-embedded-engine.md](plan-embedded-engine.md). Replaces the .NET console app (`src/AiSandbox`, about
> 3,100 lines of C#) and its xUnit end-to-end tests (about 1,950 lines) with one Cargo package.

Port the `sandbox` control app from C# / .NET 10 to Rust. **Behaviour stays the same**: the same commands, options and
aliases, the same `docker` calls in the same order, the same exit codes, the same message texts, the same files in
`.sandbox/` byte for byte (apart from the three points in *Intentional differences*). The terminal UI may look
different but stays as close as is cheap: rounded tables, a bordered panel, a spinner, a live table while starting.

The code is **not** translated file by file. It is restructured into what a Rust reader expects (see *Design*).

## Why

- One native binary with no runtime: no .NET SDK on the host, no `dotnet run` compile on first use, start-up in
  milliseconds. This is what "later a .NET tool" in the README was working towards, without the tool packaging.
- The app is a thin layer over `docker`, files and text. Rust's standard library and five or six common crates cover it.

## Answers to the open questions

### 1. Embedded resources: is there something similar in Rust?

**Yes. Use the [`include_dir`](https://crates.io/crates/include_dir) crate.** `include_dir!("$CARGO_MANIFEST_DIR/resources")`
compiles the whole folder into the binary as a tree that can be walked at run time. That is the direct equivalent of
`<EmbeddedResource Include="Resources\**">` plus `GetManifestResourceNames()`.

| Option | Verdict |
|---|---|
| `include_str!` / `include_bytes!` (std) | One macro call per file, so the 20 files would have to be listed by hand. A new tool script that is forgotten in the list is silently missing. No. |
| **`include_dir`** | Embeds a folder, files are listed and read by relative path. Small, no run-time cost, no features to switch off. **Chosen.** |
| `rust-embed` | Equally common. In debug builds it reads from disk instead of the binary by default, which is the opposite of what the tests should exercise. Would work with `debug-embed`, but offers nothing extra here. |

Three things the .NET build did for us have to be done explicitly:

- **Rebuild when a resource changes.** A proc macro can't tell Cargo about a folder on stable Rust, so a new or edited
  file would not trigger a rebuild. `build.rs` prints `cargo::rerun-if-changed=resources`. The build script is needed
  anyway for the version (below).
- **Names with forward slashes on every OS, sorted ordinally.** The csproj did this with `LogicalName`. In Rust the one
  module that touches the embedded tree (`engine::embedded`) converts `\` to `/` and sorts by bytes. The engine hash
  depends on this order.
- **LF line endings.** `.gitattributes` keeps forcing them (`resources/** text eol=lf`), and `read_text` still
  normalises `\r\n` to `\n`, as `EmbeddedFiles.ReadText` does.

**The app version** (`0.1.0+<commit sha>`, used in file headers, the manifest and `info`) came from
`AssemblyInformationalVersion`. In Rust, `build.rs` runs `git rev-parse HEAD` and passes
`cargo::rustc-env=SANDBOX_VERSION=<CARGO_PKG_VERSION>+<sha>`; without git (source tarball, or git refusing the folder)
it is just `CARGO_PKG_VERSION`. The code reads it with `env!("SANDBOX_VERSION")`.

**Found while planning:** `src/AiSandbox/Resources/desktop/bin/sandbox-install` is **not in git**. The root
`.gitignore` has `bin/`, which also matches this resource folder. It exists only in the working copy, so a fresh clone
builds an app without it and the desktop image build fails at `COPY bin/sandbox-install`. The move to `resources/` with
a `.gitignore` of `/target` fixes it; step 8 checks that the file is tracked.

### 2. End-to-end tests: `cargo test` or an own app?

**`cargo test`, as Cargo integration tests.** This is the standard Rust way to test a command-line program from the
outside:

- Files under `tests/` are compiled as separate programs that see the package only from the outside. For a package with
  a binary, Cargo builds the binary first and gives the tests its path as `env!("CARGO_BIN_EXE_sandbox")`. No
  `BuiltOutput(...)` path guessing as in `SandboxFixture`.
- The test runs the real binary as a process with a temp project and a fake `docker` first on `PATH`, exactly as today.
- Common helper crates: `assert_cmd` (run the binary with a timeout), `tempfile` (temp folders that delete themselves),
  `rstest` (`#[case(...)]` is xUnit's `[InlineData]`).

No own test app is needed. Snapshot tools (`trycmd`, `insta`) don't fit: the tests check docker call lists and files,
not just output text.

**The fake docker** stays a real executable (a shell script would not run on Windows). It becomes a second binary
target of the same package, `fake-docker`, with its source in `tests/fake_docker/main.rs`. Cargo then also provides
`CARGO_BIN_EXE_fake-docker`, and the fixture copies it to `<tmp>/fakebin/docker` and `xdg-open`.
Considered and rejected: a separate workspace crate (Cargo only passes binary paths within one package; depending on
another package's binary is nightly-only), and a feature-gated binary (`cargo test` would then need `--features`).
The cost is small: `cargo build` also compiles a 50-line program, and `default-run = "sandbox"` keeps `cargo run`
unambiguous. Release packaging uses `--bin sandbox`.

Two changes to how the tests are organised, both because of how `cargo test` works:

- **One fixture per test, not one shared and reset.** `cargo test` runs the tests of one program in parallel threads.
  Each test creates its own `Fixture` (temp project) and drops it at the end. This replaces `Reset()` and the xUnit
  collection, and the suite gets faster (`up` sleeps 2 s per test).
- **One test program** `tests/e2e/main.rs` with the five test modules and a `support` module, instead of five
  top-level files. Each top-level file would be linked as its own executable and could not share the fixture without
  `#[path]` tricks.

In addition, pure logic gets **unit tests** inside its module (`#[cfg(test)] mod tests`), which is where Rust puts
them: the YAML quoting, `with_header`, `sanitize`, path normalising, the allowlist edits, number formatting. They are
an extra; the ported E2E tests remain the specification.

## Design

### Layout

One package in the repository root. No workspace: there is one product.

```
Cargo.toml  Cargo.lock  build.rs
resources/                      moved from src/AiSandbox/Resources (compose.yml.sbn becomes compose.yml.j2)
src/
  main.rs                       parse, run, print the error, exit code. Nothing else.
  lib.rs                        module list, VERSION, run(args) -> exit code
  cli.rs                        clap definitions: Cli, Command (one enum variant per command)
  commands/{mod,lifecycle,network,maintenance}.rs
  project.rs                    Project: locate .sandbox, id, name, workdir, old-layout checks
  config/{mod,reader}.rs        Settings and its parts; reading and validating sandbox.toml
  allowlist.rs                  Allowlist, AllowEntry, AllowOutcome
  docker.rs                     Docker: compose calls, need_docker, collision check, ui_url, services; ServiceState
  process.rs                    capture / interactive child processes, user ids, open_browser
  compose.rs                    the template model (serde structs), render, the `q` filter, mounts
  engine/{mod,embedded,manifest}.rs   sync into .state/, header, hashes, the embedded tree
  paths.rs                      full_path (lexical), physical (final symlink), UTF-8 conversion
  ui/{mod,table,panel}.rs       Ui: messages, styles, spinner, live table, confirm
tests/
  e2e/{main,support,commands,errors,compose_file,engine,init}.rs
  fake_docker/main.rs
  README.md
```

`Cargo.toml`: package `ai-sandbox`, edition 2024, `default-run = "sandbox"`, `[[bin]] name = "sandbox" path =
"src/main.rs"`, `[[bin]] name = "fake-docker" path = "tests/fake_docker/main.rs" test = false`, `[[test]] name = "e2e"
path = "tests/e2e/main.rs"`. `[lints.rust] unsafe_code = "forbid"`, `[lints.clippy] all = "warn"`; `rust-version = "1.88"`. The
equivalent of `TreatWarningsAsErrors` is a build without warnings in the verification of every step (plus
`cargo clippy --all-targets -- -D warnings` where clippy is installed).

### Crates

Checked on 2026-11-13 with Rust 1.93.1: all of them resolve together (the whole build is under 90 packages, no version conflicts
between them), and a probe program that calls every API named in this plan compiles and runs. Use these versions or
newer patch releases (`cargo add` writes them as `^` requirements). `rust-version = "1.88"` in `Cargo.toml`: that is
what the newest of them (`comfy-table`, `sha2`) need.

**In the app**

| Crate | Version | Replaces | What we use it for |
|---|---|---|---|
| `clap` (feature `derive`) | 4.6 | System.CommandLine | The command line: commands, aliases (`start`, `ps`), options, the global `-v`, `--help`, `help <command>`, `--version`. Declared as one `enum`, so adding a command without handling it doesn't compile. |
| `toml` (features `parse`, `serde`, `preserve_order`; no default features) | 1.2 | Tomlyn | Reading `sandbox.toml` into a `toml::Table`. The standard TOML crate of the serde ecosystem. `preserve_order` keeps keys in file order (the "known: …" lists and the order of build args and env vars); a syntax error carries its position. `toml_edit` was considered and dropped: it is made for editing files while keeping their formatting, which we never do. |
| `minijinja` (feature `serde`) | 3.1 | Scriban | Rendering the embedded compose template into `.state/compose.yml`. Keeps the compose file readable as one template instead of code that prints YAML. |
| `serde` (feature `derive`) | 1.0 | hand-built `ScriptObject`s, `[JsonPropertyName]` | Turning our structs into template variables and the manifest into JSON without writing that by hand. |
| `serde_json` | 1.0 | System.Text.Json | Reading `docker compose ps --format json`; reading and writing `.state/.manifest`. |
| `include_dir` | 0.7 | `<EmbeddedResource>` | Compiling `resources/` (the engine and the init templates) into the binary. See question 1. |
| `sha2` | 0.11 | `SHA256` | The project id (hash of the project path), the hash of each engine file (to tell "edited by hand" from "written by an older version") and the engine hash shown by `info`. |
| `regex` | 1 | `Regex` | Host name validation, allowlist lines (group headers, commented-out entries), refused hosts in the proxy log, `ARG INSTALL_*` lines of the Dockerfile, the memory size check, project name sanitising. |
| `anyhow` | 1.0 | `SandboxException`, `Guard` | One error type for the whole app: `bail!("message")` for expected failures, `?` plus `.with_context(…)` for I/O errors. `main` prints either as `ERROR: …`. |
| `camino` | 1.2 | paths as `string` | UTF-8 paths (`Utf8PathBuf`). Project paths are hashed, printed and written into YAML, so they must be text; with `std::path` every such use would need a conversion that can fail. |
| `console` | 0.16 | Spectre.Console (part) | Colours and bold/dim, whether stdout is a terminal, the terminal width, the display width of styled text (for the panel), clearing lines for the live table. Respects `NO_COLOR` (checked). |
| `indicatif` | 0.18 | Spectre `Status()` | The spinner while a docker call runs. Built on `console`. |
| `comfy-table` (feature `custom_styling`) | 8.0 | Spectre `Table` | Tables (`status`, `info`, `allowlist`, `blocked`, `allow`/`deny`): column widths, wrapping to the terminal width. The feature makes it measure cells that already contain colour codes correctly. |
| `which` | 8.0 | `Proc.Which` | Finding `docker` on `PATH` (for "Docker is not installed") and the browser opener; handles `PATHEXT` on Windows. |
| `ctrlc` | 3.5 | `Console.CancelKeyPress` | Keeping the app alive on Ctrl+C while `logs -f`, `shell` or `claude` runs, so the child handles it and its exit code is returned. Works on Unix and Windows. |
| `rustix` (feature `process`, Unix only) | 1.1 | `[LibraryImport] getuid/getgid` | The host user's ids for `PUID`/`PGID`, through a safe API (no `unsafe` in our code). Declared under `[target.'cfg(unix)'.dependencies]`. |
| `iana-time-zone` | 0.1 | `TimeZoneInfo.Local` | The host's time zone name (`Europe/Vienna`) for the desktop's `TZ`, including the Windows-name-to-IANA mapping. |

**In the tests only** (`[dev-dependencies]`)

| Crate | Version | Replaces | What we use it for |
|---|---|---|---|
| `assert_cmd` | 2.2 | `Process` + own timeout | Running the built `sandbox` binary with arguments, environment and a 120 s timeout, and collecting exit code and output. |
| `tempfile` | 3 | temp folder + `Dispose` | One temp project per test, deleted when the test ends. |
| `rstest` | 0.28 | `[Theory]` / `[InlineData]` | One test function run with several inputs (`#[case(...)]`), each reported as its own test. |
| `yaml-rust2` | 0.13 | YamlDotNet | Parsing the generated `compose.yml` to check single values. It keeps key order and leaves the merge key `<<` as an ordinary key, like YamlDotNet's node model, so the test helpers port directly. |

Not needed: an async runtime (the one concurrent spot, `up --wait` while the live table polls, is a scoped thread and
a channel), a prompt crate (the yes/no question is three lines), `libc` (covered by `rustix`), a file-time crate
(`File::set_modified` is in std).

**What the probe showed.** Four things differ from what one would write from memory; the design below already
accounts for them:

- `minijinja` 3: whitespace settings moved to a syntax config,
  `env.set_syntax(SyntaxConfig::builder().trim_blocks(true).lstrip_blocks(true).keep_trailing_newline(true).build()?)`.
  Serde support is the opt-in feature `serde`; render with `template.render(minijinja::value::Serde(&model))`.
- `minijinja` prints a bool as `True` / `False`. Compose needs `true` / `false`, so the template writes it out
  (see *The compose template*).
- `comfy-table` 8: presets are `TableStyle` values loaded with `load_style`, and all of them draw the inner column
  line as `┆`. Two tests match on `│`, so the app defines its own style (see *Terminal output*).
- `toml`: `preserve_order` is needed (without it tables are sorted by key). Indexing a missing key
  (`table["tools"]`) panics; use `get`. The syntax error position for `"[project]\nworkspace = \n"` is line 2,
  column 13, the same as Tomlyn reported.

Also confirmed: `clap` would take `claude --help` for itself, so the pre-parse pass-through for `claude` is needed;
a clap usage error can be mapped to exit code 1 with help staying 0 (`Cli::try_parse`, then `e.use_stderr()`);
`build.rs` `rustc-env` and `CARGO_BIN_EXE_<name>` are visible in integration tests.

**`rustfmt` and `clippy` are not installed in this sandbox** (`cargo fmt` and `cargo clippy` don't exist; there is no
`rustup`). The checks in *Steps* that name them apply where they are available; in here, `cargo build` and
`cargo test` without warnings is the bar. Don't try to install them.

### From C# shapes to Rust shapes

| C# | Rust |
|---|---|
| `Sandbox` partial record with data, docker calls and generation | `Project` (plain data, `Project::load`) and `Docker<'a> { project, verbose, ui }` (everything that runs `docker`). `compose::generate(&project)` and `engine::sync(state_dir)` are free functions. |
| `static class Ui` with a static stderr console | `Ui` value created once in `run()` and passed as `&Ui`. Holds width, interactive, colour on/off. No global state. |
| Spectre markup strings (`"[green]allowed[/]"`, `Markup.Escape`) | Small style functions (`ui.green("allowed")`). There is no markup language, so nothing needs escaping. |
| `Func<Sandbox,int>` lambdas wired in `SandboxCommandLine.Build` | `enum Command` and one `match` in `run()`. Each arm says whether it needs Docker (see *Command pipeline*). |
| `throw new SandboxException(msg)` / `Guard` | `anyhow::bail!(msg)` and `?`. `main` prints `ERROR: <msg>` to stderr and exits 1. Unexpected errors (I/O) get `.with_context(...)` and are printed the same way with their cause chain (`{:#}`). |
| `ImmutableArray<T>`, records | `Vec<T>`, plain structs with `#[derive(Debug, Clone)]`. |
| `string? x` | `Option<String>`. |
| `"rw"`/`"ro"`, `AllowResult` | `bool read_only` (as today), `enum AllowOutcome { AlreadyAllowed, Uncommented, Added }`. Docker's own state and health words stay strings. |
| `EngineFiles.Sync` calling `Ui.Warn` | `engine::sync` returns `SyncReport { restored: Vec<String> }`; the caller prints the warnings. The module does no terminal output and is unit-testable. |
| `TomlTableReader` (typed getters that remember the keys read, `Done()` reports the rest) | `config::reader::TableReader<'a>` over `&toml::Table`, matching on `toml::Value`. Same idea, kept on purpose: the error texts (`unknown key 'project.workspace_mode' (known: name, workspace, protect_git, ui_port)`, `must be true or false (got "maybe")`) can't be produced by `serde` with `deny_unknown_fields`. Keys read are kept in a `Vec` in the order read; that order is the "known" list. |
| Scriban `ScriptObject` model built by hand | `#[derive(Serialize)]` structs (`Model { host, tools, desktop, env, mounts }`) passed to minijinja. `Option` fields serialise as none, which the template tests with `{% if %}`. |

### Command pipeline

`run(args)` keeps the exact order of side effects, because the tests assert it:

1. `Ui::new()`.
2. If the first argument is `claude`: `Project::load(verbose = false)`, `need_docker`, `generate`, then
   `exec sh -c <script> claude <args…>` with **all** remaining arguments verbatim (`--help`, `-v`, `--` included). This
   happens before clap sees anything. clap declares `claude` too (trailing var-arg, hyphen values allowed) so it shows
   in the help and so `sandbox -v claude …` reaches the same function.
3. clap parses. `help [command]` is clap's built-in help subcommand; the hand rewrite to `--help` goes away.
4. `init` runs without loading a project. `allowlist` and `info` load the project only. Every other command: load,
   `need_docker` (`which docker`, then `docker info --format {{.ServerVersion}}`), `generate`, then the command.
5. `Project::load` order: locate the folder, load or create `.state/id` (this creates `.state/` even for `info`), old
   `sandbox.yml` check, old `engine/` check, read `sandbox.toml`.
6. `generate` order: `.state/.gitignore`, `engine::sync` (warnings printed), render and write `compose.yml`, delete
   `compose.generated.yml`.

The process exits with `std::process::exit(code)` so the exit code of `shell`, `claude` and `logs` (docker's own)
passes through unchanged.

### The compose template

`compose.yml.sbn` becomes `compose.yml.j2`. Environment settings: `trim_blocks`, `lstrip_blocks`,
`keep_trailing_newline` (through the syntax config, see *Crates*), undefined behaviour **strict**, and auto-escaping
**off for every template** (`set_auto_escape_callback(|_| AutoEscape::None)`), so nothing but `q` touches the values. The filter `q` is ported from `ComposeTemplate.Quote` character for
character (`\\`, `\"`, `\n`, `\r`, `\t`, `\u00XX` for other control characters, U+007F, U+0085, U+2028, U+2029).

| Scriban | Jinja |
|---|---|
| `{{~ # comment … ~}}` block at the top | `{# … -#}` |
| `{{~ for t in tools ~}}` … `{{~ end ~}}` on their own lines | `{% for t in tools %}` … `{% endfor %}` (the two trim settings remove the line) |
| `{{~ if desktop.auth ~}}`, `{{~ if desktop.devices.size > 0 ~}}` | `{% if desktop.auth %}`, `{% if desktop.devices %}` |
| `{{ "127.0.0.1:" + host.ui_port + ":3001" \| q }}` | `{{ ("127.0.0.1:" ~ host.ui_port ~ ":3001") \| q }}` (a filter binds tighter than `~`) |
| `args:{{ if tools.size == 0 }} {}{{ end }}` | `args:{{ " {}" if not tools else "" }}` (an inline block tag would lose its newline to `trim_blocks`) |
| `read_only: {{ m.read_only }}` | `read_only: {{ "true" if m.read_only else "false" }}` (minijinja prints a bool as `True` / `False`) |

The golden test `default_file_matches_exactly` decides whether the whitespace is right. Fix the template, not the test.

### Terminal output

| Today (Spectre) | Rust |
|---|---|
| `Table().Border(Rounded)` | `comfy-table` with our own `TableStyle` (the presets use `┆` between columns): top `╭─┬╮`, header and content lines `│ │ │`, header separator `├─┼┤`, no row separator, bottom `╰─┴╯`. `ContentArrangement::Dynamic`, width from `Ui`. Columns are separated by `│`; two tests match on it (`2\s*│\s*evil\.example\.com\s*│\s*blocked`, `dotnet\s*│\s*10\.0`). Title above, caption below as plain lines. |
| `Panel` with header (up, info, failed command output) | `ui::panel(title, lines, colour)`: `╭─ title ─…╮`, lines, `╰…╯`, widths with `console::measure_text_width`. The key/value grids inside are padded by hand. |
| `Rule` | A line of `─` with the title at the left. |
| `Status().Spinner(Dots)` | `indicatif` spinner, yellow, cleared when done. Only when interactive; otherwise the action just runs. |
| `Live(table)` during `up` | Print the table, then once a second `Term::clear_last_lines(n)` and print again. Only when interactive; otherwise wait for `up` without calling `ps` (the tests rely on that). |
| `Confirm(…, false)` | Print `… ? [y/N] `, read a line, yes on `y`/`yes`. |
| `[link=url]url[/]` | The plain URL. |

Rules that stay: errors and warnings go to **stderr** as `ERROR: …` / `WARNING: …`, everything else to stdout;
`interactive` = stdout is a terminal; when stdout is redirected or narrower than 20, width = `$COLUMNS` if above 20,
else 120 (same for stderr); `-v` prints `$ docker …` dimmed, an argument containing a space in double quotes;
`allowlist` prints one active host per line when not interactive; no colour when `NO_COLOR` is set or stdout is not a
terminal (`console` does both by itself). Messages are **not** hard-wrapped to the width any more; the terminal wraps.

## Things that must come out the same

These are the places where the obvious Rust call behaves differently from the .NET one. Each needs care (and where it
is pure, a unit test).

1. **`Path.GetFullPath` is lexical.** It removes `.` and `..` without touching the disk. `std::path::absolute` keeps
   `..` on Unix and `fs::canonicalize` resolves symlinks and returns `\\?\` paths on Windows. Write
   `paths::full_path(base, p)`: join, then fold `.`/`..` over the components (on Windows `std::path::absolute` does
   this already). Used for `SANDBOX_DIR`, `init <folder>` and share `host` (`../docs` must render as `<root>/docs`).
2. **`Physical()` resolves only the last component**, and only if it is a symlink (`ResolveLinkTarget(true)`): follow
   `read_link` until the path is no longer a link, relative targets against the link's parent. Parents are not
   resolved. Do not use `canonicalize`.
3. **`File.ReadAllText` drops a UTF-8 BOM.** Strip a leading U+FEFF when reading `sandbox.toml`, `allowlist.txt`,
   `.state/id` and the manifest. All writes are UTF-8 without BOM and with `\n`.
4. **Config.** Port `Config.Read` check by check, in the same order; the 29 cases in `Invalid_sandbox_toml` are the
   list. Details that are easy to lose: tool keys are checked against the `ARG INSTALL_*` lines of the embedded
   Dockerfile, in its order; `[tools]` and `[env]` keep file order; a float tool value is an error that shows the value
   with at least one decimal; the reserved env names are compared case-insensitively; `name` and `timezone` are
   trimmed and empty means not set; a share's default `path` is `/shares/<last component of the full host path>`.
5. **Numbers.** `FormatNumber` is .NET `"0.0###"` (one to four decimals), `cpus` is `"0.###"` (zero to three).
   Format with fixed decimals and trim trailing zeros. `cpus = 4` must render `cpus: 4`, `2.5` → `2.5`,
   `(got 0.0)` for `cpus = 0`.
6. **TOML syntax errors**: `sandbox.toml is not valid TOML (line L, column C): <parser message>`. Line and column are
   1-based and computed from the start of `toml::de::Error::span()` (column counted in characters). Checked: this gives
   line 2, column 13 for `"[project]\nworkspace = \n"`, as the test expects. Use the bare message (`message()`), not
   the multi-line rendering.
7. **Regex.** .NET `$` also matches before a final `\n`; Rust's doesn't. All inputs here are single lines without the
   newline, so the patterns port as they are; `RegexOptions.IgnoreCase` is `(?i)`, `Regex.Escape` is `regex::escape`
   (the `\*` → `.*` and `\?` → `.` replacement in `IsAllowed` still works on its output).
8. **Allowlist.** Load: `\r\n` → `\n`, split on `\n`, drop one trailing empty line. Save: join with `\n` plus a final
   `\n`. `allow` validates **all** hosts before changing anything (a bad host leaves the file untouched). Commented-out
   entries count only below the first `# --- group ---` header.
9. **Engine sync** as in `EngineFiles`: same four cases per file (missing, identical, matches the manifest, edited by
   hand), stale files then empty folders removed (deepest first), manifest written last and only if its text changed,
   no write when nothing changed (a test checks modification times). Header after a shebang line, or first line for
   `Dockerfile`, `*.conf`, `*.sh`, or none. Engine hash: for each name in sorted order, name bytes, `0`, content bytes,
   `0`. **Check:** the hash must equal the one the .NET app writes for the same resources (step 3).
10. **Manifest.** `{"app", "engine", "files"}`, pretty-printed with two spaces, keys sorted (`BTreeMap`), final
    newline. Unreadable or invalid → treated as empty.
11. **Project id and names.** id = first 8 hex characters of SHA-256 over the project path as UTF-8, written as
    `<id>\n`, read back trimmed and used as is. `sanitize`: lower case, runs of other characters → `-`, strip leading
    `-`/`_` and trailing `-`, empty → `sandbox`. Workdir `/work/<sanitised folder name>-<id>` (folder name, not
    `[project] name`).
12. **Child processes.** Captured: stdin closed (`Stdio::null()`), stdout and stderr collected, a program that can't
    be started gives exit code 127 with the OS message as stderr. Interactive: stdio inherited; a no-op Ctrl+C handler
    (`ctrlc`, installed once) keeps the app alive so the child handles Ctrl+C and the app returns the child's code;
    killed by a signal → 128 + signal number on Unix. Every captured compose call starts with `--ansi never`;
    interactive ones don't.
13. **`compose ps` JSON**: one object per line, or one array. A missing or non-text field is `""`. Ports
    `URL:Published→Target` for published ports above 0, distinct, joined with `, `. Services sorted by name.
14. **`up`**: collision check, build (interactive), `up -d --wait --wait-timeout 300`, sleep 2 s, port lookup, panel,
    browser. A failed `up` is tolerated only for `up` and only if `ps` shows `desktop` running. `update` adds
    `--pull --build-arg CACHE_BUST=<unix seconds>`.
15. **Browser.** Unix: the first of `xdg-open`, `open`, `wslview`, `explorer.exe` found on `PATH`, started with the URL
    as its only argument, output discarded, not waited for. Windows: `cmd /C start "" <url>`. On any failure print
    `Open <url> in your browser.`
16. **User ids**: `rustix` `getuid`/`getgid`; root or Windows → `1000`/`1000`. **Time zone**: `$TZ` if not blank, else
    `iana_time_zone::get_timezone()`, else none (the `TZ:` line is left out).
17. **Hints** name the launcher: `./sandbox`, or `sandbox.cmd` on Windows.

## Intentional differences

Visible changes that are accepted. Anything else that differs is a bug in the port.

1. **Help and usage errors** are clap's (layout, wording, typo suggestions). `Usage:` still appears; the exit code for
   a usage error stays **1** (map clap's 2; help and `--version` stay 0). Gone because clap has no equivalent:
   `-?` and `/h`, response files (`@file`), `[directive]` arguments.
2. **Look of tables, panels, spinner, live table and the yes/no question.** Long lines are no longer hard-wrapped.
3. **The template is Jinja**, so the second header line of `.state/compose.yml` says
   `# Rendered from compose.yml.j2 with …`. The engine hash is not affected (it covers `desktop/` and `proxy/` only).
4. **`.state/.manifest`** writes `+` as `+`, not `+`. Old manifests are read fine and rewritten once.
5. **TOML syntax error text** after `(line L, column C):` comes from the Rust parser.
6. **Unexpected failures** print one `ERROR:` line with the cause chain instead of a .NET exception dump. Exit code 1.
7. **A config file that isn't valid UTF-8** is an error (.NET replaced the bad bytes silently).
8. **`sandbox -v claude …`** runs claude with verbose output (before, only a literal first argument `claude` did
   anything).
9. **`share = [{ host = "…" }]`** (an inline list of tables) is accepted like `[[share]]` blocks. Both are the same
   TOML value; Tomlyn kept them apart and rejected the inline form (`must be written as [[share]] blocks`). That error
   remains for every other type (`share = { … }` → `got a table`, which is the tested case).

## Steps

The .NET app stays in place and green until step 8, so there is always a reference to compare against. Rust reads the
resources from `src/AiSandbox/Resources` until then (one path constant and one line in `build.rs`).

The crate registry (`index.crates.io`, `static.crates.io`) is allowed in this sandbox since 2026-11-13; `cargo` can
fetch.

1. **Baseline and scaffold.** Run `dotnet test AiSandbox.slnx` and note the result. Create `Cargo.toml`, `build.rs`
   (version, `rerun-if-changed`), `src/main.rs`, `src/lib.rs`, add the crates, `.gitignore` gets `/target`. Write
   `paths.rs` and `ui/` (messages, styles, table, panel, spinner; the live table comes with `up`), with unit tests for
   the pure parts. *Check:* `cargo build` without warnings; `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` where
   installed (not in this sandbox, see *Crates*).
2. **Test harness.** `tests/fake_docker/main.rs` (port of `FakeDocker/Program.cs`, same two environment variables and
   rule matching) and `tests/e2e/support.rs` (port of `SandboxFixture` + `SandboxRunner`: `Fixture`, `run`, `Rule`,
   `Call`, the `compose` / `compose_interactive` / `collision_check` builders, `assert_calls`, `assert_output`). Same
   environment for the child: `PATH`, `FAKE_DOCKER_LOG`, `FAKE_DOCKER_RULES`, `TZ=Europe/Vienna`, `COLUMNS=200`,
   `NO_COLOR=1`, `DOCKER_HOST` removed, `SANDBOX_DIR`. 120 s timeout. *Check:* a first test, `--help` prints `Usage:`.
3. **Project, config, engine, compose.** `project.rs`, `config/`, `engine/`, `compose.rs`, the template in Jinja, plus
   the commands that need no Docker: `info`, `allowlist`, `init`. Port `init.rs`, `compose_file.rs`, `engine.rs` and
   the config half of `errors.rs` as far as they run (many generate through `status`; do `need_docker` and `status`
   here too). *Check:* those tests pass, and a one-off comparison in a temp project: run the .NET app and the Rust app
   with the fake docker, then `diff -r` the two `.state/` folders. Only the version in the headers, the template name
   line, and the manifest's `app` and `+` escaping may differ; the manifest's `engine` value must be identical.
4. **Docker commands.** `process.rs`, `docker.rs`, `commands/lifecycle.rs` (up with the live table, rebuild, update,
   stop, down, restart, status, open, shell, claude, logs). Port the rest of `commands.rs` and `errors.rs` for them.
5. **Network and maintenance.** `allowlist.rs` edits, `allow`, `deny`, `blocked`, `reload`, `reset-home`. Port the
   remaining tests. *Check:* every xUnit test has a Rust counterpart (count them: each `[Fact]` is one, each
   `[InlineData]` one `#[case]`), `cargo test` is green, run it three times to catch races between parallel tests.
6. **Look at it.** The tests never see the interactive paths. In a real terminal, with the fake docker on `PATH` and a
   rules file that makes `up -d --wait` take a few seconds: `up --no-open`, `status`, `info`, `allowlist -a`,
   `blocked`, a failing `stop` (red panel), `reset-home` (question). Compare side by side with the .NET app and fix
   what looks off. Record what was looked at in *As built*.
7. **Launchers and docs.** `sandbox`: `SANDBOX_DIR="$root/.sandbox" exec cargo run --quiet --locked --manifest-path
   "$root/Cargo.toml" --bin sandbox -- "$@"`; `sandbox.cmd` the same with `%~dp0`. `--manifest-path` does not change
   the current directory, which `init` and the upward search depend on. README: requirements (Rust toolchain instead
   of the .NET SDK), "How it's used" (MiniJinja, `compose.yml.j2`), the `dotnet` troubleshooting line, the fork hint
   (`resources/`), running from another project (`cargo run --manifest-path …` or `cargo install --path . --bin
   sandbox`). Rewrite `tests/README.md`. Add a commented-out `# --- Rust / cargo ---` group (`index.crates.io`,
   `static.crates.io`) to `resources/init/allowlist.txt`.
8. **Remove .NET.** Move `src/AiSandbox/Resources` to `resources/` and switch the two paths. Delete `src/AiSandbox/`,
   `tests/AiSandbox.E2ETests/`, `tests/FakeDocker/`, `AiSandbox.slnx`, `Directory.Build.props`,
   `Directory.Packages.props`. `.gitignore`: `/target` and `*.user` only. `.gitattributes`: `resources/** text
   eol=lf`. `.editorconfig`: drop the `[*.cs]` and csproj parts, add `[*.rs]` (4 spaces; rustfmt decides the rest).
   *Check:* `cargo test` green from a clean `target/`; `git status` lists `resources/desktop/bin/sandbox-install` as a
   new file; `grep -ri "dotnet run\|\.csproj\|Scriban\|\.sbn" --exclude-dir=doc --exclude-dir=target .` finds nothing
   outside the `dotnet` tool script and README's tool list.
9. **Host (not possible here):** smoke test against real Docker on Linux and Windows (`up`, `status`, `shell`,
   `allow`, `blocked`, `down`), as left open in the earlier plans.

## Notes for the implementer

- **The C# sources and the xUnit tests are the specification.** Read the C# method before porting it; copy message
  texts by copy and paste, including `—`, `…`, `→` and `✓`. Don't change an assertion to make a test pass, except
  where *Intentional differences* says so (the template name line in the golden file is the only one).
- **`.sandbox/` is read-only in here** and git may refuse the folder (`dubious ownership`): use
  `git -c safe.directory='*' …` for read-only git commands, plain `mv`/`rm` for file moves, and don't commit.
- No Docker daemon in the sandbox. Everything is tested against the fake.
- Keep modules free of terminal output where the table in *Design* says so (`engine`, `config`, `allowlist`,
  `compose`); they return data or errors, and `commands/` prints.
- Don't add behaviour: no new commands, flags, config keys or environment variables.
- Add an *As built* section to this file at the end, listing any differences from the plan and what step 6 covered.

## Out of scope / later

- A `rust` entry under `[tools]` for the desktop image.
- Release binaries, `cargo install` from a registry, CI.
- Terminal (pty) tests for the interactive paths.
- Project-specific tools and `sandbox engine diff` (see *Later* in plan-embedded-engine.md).

## As built

Steps 1 to 8 are done; step 9 (smoke test against real Docker on Linux and Windows) is not possible in the sandbox and
is left for the host. Everything below differs from, or adds to, the plan above.

**Result.** 58 unit tests in the modules and 149 end-to-end tests in `tests/e2e/` (86 `#[test]` and 63 `#[case]`: the
same count per file as the xUnit suite, which was 149 passing at the baseline). `cargo test` was green three times in a
row after `rm -rf target`, and `cargo build --all-targets` prints no warnings. `cargo fmt` and `cargo clippy` are not
installed here and were not run, so the formatting is by hand.
`.NET`-built and Rust-built `.state/` folders were compared with `diff -r` in a temp project with the fake docker: only
the version in the headers, the template name line, the project path in the mounts (two different temp folders) and
the manifest's `app` value (and so the file hashes that follow from the headers) differ. **The manifest's `engine`
hash is identical** (`8a5a5b1c…`).

**Differences from the plan**

1. **Version without commit in this sandbox.** `build.rs` calls plain `git rev-parse HEAD`; git refuses this folder
   here (dubious ownership), so the version is `0.1.0` and not `0.1.0+<sha>`, while the .NET build had the sha. On a
   host where git works the sha is there. `build.rs` also reruns when `.git/HEAD` or `.git/refs` change, if they exist.
2. **`autotests = false`** in `Cargo.toml`: without it Cargo also made `tests/fake_docker/main.rs` an integration test.
   `lib.rs` is a real library (`[lib] name = "ai_sandbox"`) next to the `sandbox` binary, so the tests read
   `ai_sandbox::VERSION` for the expected headers.
3. **Fake docker is linked, not copied, on Unix.** `Fixture` makes `fakebin/docker` and `xdg-open` symlinks to
   `CARGO_BIN_EXE_fake-docker` (copying an executable while other test threads start processes can fail with "text file
   busy"); on Windows it copies. The fake therefore takes its name from `argv[0]`, not from the executable path.
   Every run writes its own `rules-<n>.json` and `calls-<n>.jsonl`.
4. **Commands take the project, not `Sandbox`.** `Docker { project, verbose, ui }` is as planned; `claude` is the one
   command that loads the project itself (`lifecycle::claude(ui, verbose, args)`), because it also runs before clap
   parses. Its clap definition has `disable_help_flag = true` so `sandbox -v claude --help` reaches claude.
   `process::open_browser(url)` takes no `Ui`.
5. **Engine hash and ids use a small `engine::hex`** instead of the `Display` of the digest.
6. **Extra unit tests** beyond the plan's list: the config reader, `docker::parse_ps`, `engine::sync` against a temp
   folder, the embedded tree, the table and panel drawing, and the CLI definition (`Cli::command().debug_assert()`).
7. **Table titles** (`status`, `info`) are plain lines above the table, left aligned (Spectre centred them); the
   caption of `allowlist` is the line below.
8. The one test assertion that changed is the template name line in `default_file_matches_exactly`
   (`compose.yml.j2`), and the "no template file written" check also accepts `.j2`.

**Step 6, looked at.** Run in a pseudo terminal (`script`, 100 columns) with a wrapper that made `up -d --wait` take
four seconds, on the fake docker with rules for a "starting" desktop, an exited proxy, a failing `stop` and a proxy log:
`up --no-open` (rule, live table redrawn once a second with the elapsed time, the green "Sandbox running" panel),
`status` (rounded table with coloured state and health, URL line), `info` (both panels and the tools table),
`allowlist -a` (table with State and Note, wrapped group names, caption), `blocked` (right-aligned counts), a failing
`stop` (spinner, then the red "docker compose stop (exit 3)" panel and `ERROR: Stopping failed.`) and `reset-home` (the
`[y/N]` question; `n` returns 0). Not compared side by side with the .NET app (the layout was checked against the descriptions in
*Terminal output*). Nothing needed fixing. Not looked at:
Windows and macOS terminals, `shell`, `logs -f` with Ctrl+C.
