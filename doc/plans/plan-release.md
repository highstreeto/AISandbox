# Releases: binaries for Windows, Linux, macOS on every push to main

> **Status:** implemented; the workflow has not run on GitHub yet (written inside the sandbox, no push). See *As built*.

## Context

Today `sandbox` is run through `./sandbox` / `sandbox.ps1` (`cargo run` from the repo) or `cargo install --path`, so
every user needs Rust and a checkout. The binary is already self-contained (the engine is embedded via
`include_dir!` in `src/engine/embedded.rs`), so it can be shipped as a single file. Goal: every push to `main` runs the
tests, computes the next version from the commit messages, builds binaries for all three OSes, and publishes a GitHub
release with a changelog and install scripts that put `sandbox` on PATH.

Decisions taken: **git tags `vX.Y.Z` are the source of truth** (nothing is committed back to main; Cargo.lock pins the
crate version, so CI never touches Cargo.toml); **install scripts** (`install.sh`, `install.ps1`) are the install
channel; **Conventional Commits** drive both the version and the changelog (git-cliff).

## Version rules

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org); git-cliff computes both the
changelog and the next version from them (`git cliff --bumped-version`), so all rules live in `cliff.toml`.

- Base: latest tag matching `vX.Y.Z`. No tag yet → first release is `v0.1.0` (`initial_tag`).
- `feat` → minor, `fix`/`refactor`/`perf` → patch, `!` or a `BREAKING CHANGE:` footer → major (minor in 0.x:
  `breaking_always_bump_major = false`). Highest wins.
- `docs`/`test`/`ci`/`build`/`chore`/`style` are skipped: a push with only those makes no release (the bumped version
  equals the last tag).
- Unconventional messages (all commits before the switch) go to an *Other* section and count as a patch, so the
  first release lists the whole history and a slip is visible rather than lost.

## Files

### New: `cliff.toml` (git-cliff config)
Sections Added (`feat`), Fixed (`fix`), Changed (`refactor`, `perf`), Other (unconventional); scope in bold and
`[breaking]` in the entry. `[bump]` as above. Used three times by the workflow: `--bumped-version`, `--unreleased
--tag vX.Y.Z --strip all` for the release notes and `--tag vX.Y.Z` for the full `CHANGELOG.md` asset.

### Changed: `build.rs`
Version selection, in order:
1. `SANDBOX_RELEASE_VERSION` env (set by the release job) — add `cargo::rerun-if-env-changed=SANDBOX_RELEASE_VERSION`.
2. Nearest tag via `git describe --tags --abbrev=0 --match 'v[0-9]*'` (strip `v`), so dev builds show a real version.
3. `CARGO_PKG_VERSION`.
Then `+<commit>` as today. `.git/refs` is already watched (also watch `.git/packed-refs` if it exists). Nothing else
changes: `VERSION` in `src/lib.rs`, the engine header shortening in `src/engine/mod.rs:61`, and the e2e tests'
`f.app_version()` all read the same constant.

### Changed: `Cargo.toml`
`[profile.release]` with `strip = true`, `lto = true`, `codegen-units = 1` (small single binaries).

### Changed: `.github/workflows/test.yml`
Add `workflow_call:` so the release workflow can reuse it; change `push:` to `branches-ignore: [main]` so main isn't
tested twice (release.yml runs it).

### New: `.github/workflows/release.yml`
```
on: push: branches: [main]
concurrency: { group: release, cancel-in-progress: false }   # releases are serialized, never cancelled
permissions: { contents: write }
```
Jobs:
1. **test** — `uses: ./.github/workflows/test.yml`.
2. **version** (needs test) — checkout with `fetch-depth: 0`, install git-cliff (`taiki-e/install-action`),
   `git cliff --bumped-version` (release = no tag yet, or the result differs from the last tag), generate `notes.md`
   and `CHANGELOG.md`; outputs `tag`, `version`, `release`; upload both md files as an artifact.
3. **build** (needs version, `if: release == 'true'`) — matrix, `cargo build --release --locked --bin sandbox
   --target <t>` with `SANDBOX_RELEASE_VERSION`:

   | target | runner | notes |
   |---|---|---|
   | x86_64-unknown-linux-musl | ubuntu-latest | `musl-tools`; static, runs on any distro/WSL |
   | aarch64-unknown-linux-musl | ubuntu-24.04-arm | static |
   | x86_64-pc-windows-msvc | windows-latest | |
   | aarch64-pc-windows-msvc | windows-latest | cross target via rustup, no smoke run |
   | aarch64-apple-darwin | macos-latest | |
   | x86_64-apple-darwin | macos-latest | cross target via rustup, no smoke run |

   Smoke test on native targets: `sandbox --version` must contain the version. Package as
   **version-less names** so `releases/latest/download/…` URLs are stable: `sandbox-<target>.tar.gz` (Unix) /
   `sandbox-<target>.zip` (Windows), each holding the binary, `README.md`, `LICENSE`. Upload as artifacts.
4. **release** (needs build) — download artifacts, write `SHA256SUMS`, then
   `gh release create vX.Y.Z --target $GITHUB_SHA --title vX.Y.Z --notes-file notes.md <archives> SHA256SUMS
   CHANGELOG.md install/install.sh install/install.ps1`. The tag is created only here, so a failed build doesn't burn
   a version; tags made with `GITHUB_TOKEN` don't trigger workflows, so there is no loop.

Pin actions to major versions like `test.yml` does (`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
`Swatinem/rust-cache@v2` keyed per target, `actions/upload-artifact@v4`, `actions/download-artifact@v4`).

### New: `install/install.sh` (Linux, macOS, WSL)
`curl -fsSL https://github.com/highstreeto/AISandbox/releases/latest/download/install.sh | sh`
- Map `uname -s`/`uname -m` to a target (x86_64/amd64, aarch64/arm64; Linux → musl, Darwin → apple-darwin).
- Download `releases/latest/download/sandbox-<target>.tar.gz` (or `releases/download/$SANDBOX_VERSION/…` when that env
  is set) plus `SHA256SUMS`, verify with `sha256sum`/`shasum -a 256`.
- Install to `${SANDBOX_INSTALL_DIR:-$HOME/.local/bin}`; on macOS drop the quarantine attribute (best effort).
- If the dir isn't on PATH, print the exact line to add to the shell profile (don't edit rc files silently).
- Re-running = update. Add it to `.gitattributes` as LF.

### New: `install/install.ps1` (Windows)
`irm https://github.com/highstreeto/AISandbox/releases/latest/download/install.ps1 | iex`
- Pick x64/arm64 from `$env:PROCESSOR_ARCHITECTURE`, download zip + `SHA256SUMS`, verify with `Get-FileHash`.
- Extract to `$env:LOCALAPPDATA\Programs\sandbox`, add it to the **user** PATH
  (`[Environment]::SetEnvironmentVariable('Path', …, 'User')`) if missing, and to the current session.

### New command: `sandbox upgrade`
Named `upgrade` because `update` already exists (rebuild images with newest tools, `src/cli.rs:103`). Help text:
"Upgrade this app to the newest release (`update` refreshes the tools in the image)".

```
sandbox upgrade            # download and replace the running binary if a newer release exists
sandbox upgrade --check    # only report; exit 0 = up to date, 1 = newer version available
sandbox upgrade --version v1.2.3   # install a specific release (also allows downgrades)
```

- **Release side:** the release job also uploads the raw binaries (`sandbox-<target>` / `sandbox-<target>.exe`) and a
  one-line `VERSION` asset next to the archives. Upgrade then needs no archive or JSON handling and no GitHub API
  (no rate limits): it reads `releases/latest/download/VERSION`, then fetches the raw binary + `SHA256SUMS`
  (`releases/download/vX.Y.Z/…` for `--version`).
- **Flow** (`src/commands/upgrade.rs`, new): compare the current version (`VERSION` before `+`) with the remote one
  (simple `major.minor.patch` compare) → "Already up to date (1.2.3)." or download to a temp file next to the exe,
  verify SHA-256 with the existing `sha2` dep (mismatch → error, nothing replaced), replace via the `self-replace`
  crate (handles the running-exe lock on Windows), print `Upgraded 1.2.3 → 1.3.0. Engine files are refreshed on the next
  command; run sandbox rebuild if the release notes say so.`
- **Target:** the build target triple is baked in by `build.rs` (`cargo::rustc-env=SANDBOX_TARGET={TARGET}`) so the
  right asset is chosen.
- **Source builds refuse:** `build.rs` sets `SANDBOX_RELEASE_BUILD=1` only when `SANDBOX_RELEASE_VERSION` is set. Dev
  builds (`./sandbox`, `cargo install --path`) error with `This sandbox was built from source. Update it with git pull
  (or reinstall with the install script).`
- **HTTP:** new dep `ureq` (rustls, no OpenSSL → musl and cross builds stay simple) with proxy from the
  environment (`HTTPS_PROXY`), plus `self-replace`. If the cargo proxy inside the sandbox doesn't allow fetching them,
  the user adds them on the host.
- **Testability:** base URL overridable through a hidden env var `SANDBOX_RELEASE_URL` (default
  `https://github.com/highstreeto/AISandbox/releases`).
- **Order of side effects:** like `init`/`allowlist`/`info`, it needs no project and no Docker — dispatch it in
  `run_inner` (`src/lib.rs:60`) before `Project::load`, and add it to the unreachable arm in `run_docker_command`.
- **Tests:** the core takes the current version, target and destination path as parameters, so it is testable
  without a release build. Unit tests in `upgrade.rs`: version parse/compare, checksum lookup in `SHA256SUMS`, and the
  flow against a tiny `std::net::TcpListener` HTTP server (up to date / newer for `--check`, checksum mismatch leaves
  the destination untouched, successful replace of a temp file standing in for the exe). E2E
  (`tests/e2e/commands.rs`): the test binary is a source build, so `sandbox upgrade` prints the "built from source"
  message with exit code 1 and runs no docker call.
- The install scripts stay the first-install path; README points to `sandbox upgrade` for updates.

### Docs (per AGENTS.md "keep README, templates and code in step")
- `README.md` commands table: `upgrade` next to `update`, with the distinction spelled out.
- `README.md`: new **Install** section with both one-liners first; Requirements: Rust only for working on the app;
  Quick start uses `sandbox`; keep the repo launchers / `cargo install --path` as the from-source path; update the
  "cargo not found" troubleshooting row.
- `AGENTS.md`: a **Releases** section (commit convention, what release.yml does, `git cliff` dry run, assets naming)
  and the new paths in the Layout table.
- `doc/plans/plan-release.md`: this plan with a `> **Status:**` line and an *As built* section.

## Verification

Inside the sandbox (no push, no Docker; `.git` is read-only, so build in a scratchpad clone and hand over a patch to
`/config/Desktop`):
- `cargo test --workspace --all-targets --locked` and `cargo clippy --all-targets` still pass.
- `cargo build` then `target/debug/sandbox --version`: shows `0.1.0+<sha>` (no tags yet); with
  `SANDBOX_RELEASE_VERSION=1.2.3 cargo build` → `1.2.3+<sha>`.
- `git cliff --bumped-version` against a throwaway clone: no tags → `v0.1.0`; after `v0.1.0` only `chore`/`docs` →
  `v0.1.0` (no release); unconventional or `fix` → `v0.1.1`; `feat` → `v0.2.0`; `refactor!` in 0.x → minor;
  `feat!` after `v1.0.0` → `v2.0.0`.
- `shellcheck` on the shell scripts and `actionlint` on the workflows if the tools are available.

On the host / GitHub (user):
- Push to a branch first: only Tests run. Merge to main: release `v0.1.0` appears with 6 archives, `SHA256SUMS`,
  `CHANGELOG.md`, both install scripts; notes grouped Added/Fixed/Changed/Removed.
- Run both install one-liners on a clean machine/VM, open a new shell, `sandbox --version` and `sandbox info` work from
  any directory.
- After a second release, `sandbox upgrade --check` reports it, `sandbox upgrade` replaces the binary (on Windows
  too, while running), `sandbox --version` shows the new version; `sandbox upgrade --version v0.1.0` downgrades.
- Check repo settings allow Actions to create releases (Settings → Actions → Workflow permissions), in case the org
  restricts the `permissions:` block.

## As built

- **Windows on ARM dropped.** `ureq`'s rustls uses `ring`, which needs clang to cross-compile for
  `aarch64-pc-windows-msvc`; Windows on ARM runs the x64 build through emulation, so `install.ps1` always installs
  `x86_64-pc-windows-msvc`. Five targets instead of six.
- **`sandbox upgrade [VERSION]`** takes the version as an argument instead of `--version`, which reads like the
  global `--version` flag.
- **Conventional Commits instead of `+bump:` markers.** The first version grouped free-form commits by their first
  word and computed the version in a shell script; the result was a catch-all *Changed* section and two places for
  the rules. git-cliff now does both from `cliff.toml`; the script is gone.
- Each release also has a `VERSION` asset and the raw binaries; `SHA256SUMS` covers every asset.
- `Ui::self_name` (hints like "run ./sandbox rebuild") says `sandbox` in a release build, the repo launcher otherwise.
- `upgrade` core takes the version, target, folder and replace function as parameters; the unit tests run it against
  a small HTTP server on localhost (`proxy(None)`, so a proxy in the environment doesn't interfere). A release build
  was smoke-tested by hand: `--check`, an unknown version, upgrade 0.1.5 → 0.2.0, then "Already up to date".
- Not verified here: the GitHub workflow itself, `install.ps1` (no
  PowerShell), real downloads from GitHub, and self-replace on Windows.
