# Plan: `rust` tool (rustup, rustfmt, clippy, rust-analyzer)

> **Status:** implemented (patch). Unit/e2e tests pass; the real image build (`rust = true` + `./sandbox up`) is not
> verified yet, because there is no Docker daemon in the sandbox where it was written.

Add `[tools] rust = ...` so a project gets a full Rust toolchain through rustup instead of the outdated apt
`rustc`/`cargo` (no rustup, no `cargo fmt`, no `cargo clippy`).

## Target

```toml
[tools]
rust = true                    # stable
rust = "1.88"                  # a pinned toolchain
rust = ["stable", "nightly"]   # several; the first one is the default
```

`sandbox-install rust nightly` works at runtime too (same script, lost on rebuild like every runtime install).

What gets installed:

| What | How |
|---|---|
| rustup + toolchain(s) | `rustup-init -y --no-modify-path --profile minimal --default-toolchain <first>`, further toolchains with `rustup toolchain install --profile minimal` |
| Components, for **every** toolchain | `rustfmt`, `clippy`, `rust-src`, `rust-analyzer` (`rustup component add ... --toolchain <t>`). Gives `cargo fmt`, `cargo clippy`, IDE support (VS Code rust-analyzer extension, RustRover use `rust-src`) |
| Native build deps (apt) | `build-essential` (the `cc` linker — without it nothing links), `pkg-config`, `libssl-dev` (openssl-sys is the most common native dep), `gdb` (for `rust-gdb`) |

Not included (decision 4): `cargo install`-compiled helpers such as cargo-nextest, cargo-watch, cargo-audit.

## Layout inside the image

- `RUSTUP_HOME=/usr/local/rustup` — toolchains, shared, in the image.
- rustup is installed with `CARGO_HOME=/usr/local/cargo` only for the install step; its proxies
  (`cargo`, `rustc`, `rustup`, `rustfmt`, `cargo-fmt`, `cargo-clippy`, `clippy-driver`, `rust-analyzer`, `rust-gdb`,
  `rustdoc`, …) are symlinked into `/usr/local/bin` (rustup proxies dispatch on argv[0], so symlinks work). No PATH
  change in the Dockerfile needed.
- `ENV RUSTUP_HOME=/usr/local/rustup` in the Dockerfile `ENV` block next to `DOTNET_ROOT` (harmless when rust is off;
  the proxies need it at runtime, and `sudo -E` in `sandbox-install` carries it).
- `chmod -R a+rwX /usr/local/rustup` so the desktop user (`abc`) can `rustup component add` / `rustup toolchain
  install` / `rustup update` without sudo (same approach as the official `rust` Docker image).
- At runtime `CARGO_HOME` is **not** set, so it defaults to `~/.cargo` = `/config/.cargo`, inside the persistent home
  volume: the crate registry cache and `cargo install`ed binaries survive rebuilds. The script writes
  `/etc/profile.d/sandbox-rust.sh` that adds `$HOME/.cargo/bin` to `PATH` (if not already there), and makes sure
  `/etc/bash.bashrc` sources it (idempotent: only append the line if missing), because Konsole starts non-login shells.
- Remove `/usr/local/cargo/registry` and other caches after install to keep the layer small.
- If the apt `rustc`/`cargo` packages are present (user added them under `apt`), the `/usr/local/bin` symlinks win
  over `/usr/bin` in PATH order. Do not uninstall anything.

Architecture: rustup-init via `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- ...` picks the
host triple itself (x86_64 and aarch64 both fine).

## Steps

1. **`resources/desktop/tools/rust.sh`**, following the existing scripts (`source "$(dirname "$0")/_lib.sh"`,
   `ABOUT:`/`HOSTS:` header, defaults via `[ $# -gt 0 ] || set -- stable`, ends by printing versions:
   `rustc --version; cargo --version; cargo fmt --version; cargo clippy --version`).
   - `# ABOUT: Rust via rustup + rustfmt, clippy, rust-src, rust-analyzer, build-essential. Option: toolchain(s), e.g. stable nightly 1.88 (default stable; first = default)`
   - `# HOSTS: sh.rustup.rs static.rust-lang.org (install, rustup) index.crates.io static.crates.io (crates)`
   - Must be re-runnable (runtime `sandbox-install rust nightly` after the image already has stable): if
     `/usr/local/rustup` already has rustup, skip rustup-init and only `rustup toolchain install`; do not change the
     existing default toolchain in that case.
   - Use `apt_install` from `_lib.sh` for the apt packages.
2. **`resources/desktop/Dockerfile`**: `ENV RUSTUP_HOME=/usr/local/rustup` in the ENV block; `ARG INSTALL_RUST=false`
   + `RUN sandbox-install rust "${INSTALL_RUST}"` placed after the `BUN` pair and before `VSCODE` (languages before
   IDEs). The known-tools list is read from these ARG lines, so order there = order in error messages.
3. **`resources/init/sandbox.toml`**: `rust        = false         # Rust via rustup: true = stable, "1.88", ["stable", "nightly"]`
   after `bun`.
4. **`resources/init/allowlist.txt`**: rename the `--- Rust crates ---` group to `--- Rust (rustup + crates) ---` and
   add commented `# sh.rustup.rs` and `# static.rust-lang.org` with the same style of trailing comment as other
   entries (`# toolchain install, rustup update`).
5. **README.md**: add `rust` to the `[tools]` example in *Configure*; mention in one sentence that the toolchain lives
   in `/usr/local/rustup` and `~/.cargo` (registry cache, `cargo install`) persists in the home volume. Mention the
   Rust allowlist group in the *Network* paragraph that lists the ready-made groups.
6. **Tests**: update every place that lists the tools/engine files:
   - `tests/e2e/engine.rs`: embedded tool scripts list (add `tools/rust.sh`, alphabetical), and the
     `unknown tool tools.nope (known: ...)` message (`..., bun, rust, vscode, jetbrains`).
   - `tests/e2e/errors.rs`: same `known:` message.
   - grep `tests/` and `src/` for other hard-coded tool lists (e.g. `jetbrains`, `vscode`) and update them.
   - Add one e2e test in `tests/e2e/compose_file.rs` style: `rust = ["stable", "nightly"]` renders
     `INSTALL_RUST: "stable nightly"`, and `rust = 1.88` is rejected with the existing "write versions as text" error
     (only if not already covered generically — don't duplicate).
   - `cargo test --locked` must pass. Also run `bash -n` on `rust.sh`, and `shellcheck` if it is available.
7. **This plan** goes to `doc/plans/plan-rust-tool.md` with the *Status* line updated and an *As built* section at the
   end listing any differences.

## Decisions

1. **rustup, not apt.** apt's rustc lags far behind and has no rustup, so no `cargo fmt`/`clippy` component management
   or pinned toolchains (`rust-toolchain.toml` needs rustup).
2. **Toolchains in the image, `~/.cargo` in the home volume.** The image stays reproducible; downloaded crates and
   user-installed cargo binaries are not lost on `rebuild`.
3. **`--profile minimal` + explicit components.** Skips `rust-docs` (~600 MB unpacked, offline HTML docs nobody opens in
   a sandbox) but has everything for fmt/clippy/IDEs.
4. **No compiled cargo extras by default.** `cargo install cargo-nextest` etc. compiles for minutes per tool at every
   image build. Users run `cargo install <x>` once at runtime; it lands in `~/.cargo/bin`, persists and is on PATH.
5. **World-writable `RUSTUP_HOME`.** Lets the agent/user add components and toolchains without sudo (also with
   `sudo = false`). It's a single-user container; the risk is nil.
6. **`build-essential` etc. are part of the tool**, not left to `apt = [...]`: Rust without a linker is useless.

## Host steps (user)

- `git apply` the patch on the host.
- Allow the hosts: `./sandbox allow sh.rustup.rs` and `./sandbox allow static.rust-lang.org` (plus `index.crates.io`
  `static.crates.io`, already allowed in this repo).
- Optionally set `rust = true` in this repo's own `.sandbox/sandbox.toml` and run `./sandbox up` (this is the smoke test
  against real Docker; it can't run inside the sandbox).

## As built

As planned, with these notes:

- `rust.sh` calls `apt_install` first, then rustup. Re-run detection is `[ -x /usr/local/cargo/bin/rustup ]`; in that case
  the existing default toolchain is kept and all requested toolchains are only installed and given the components.
- The script exports `RUSTUP_HOME`/`CARGO_HOME` itself (`CARGO_HOME=/usr/local/cargo` only during the script);
  `/usr/local/bin` symlinks are made for every file in `/usr/local/cargo/bin`.
- `rust = 1.88` rejection is already covered generically (`dotnet = 10.0` test in `errors.rs`), so no extra test. Added
  `rust_toolchains_become_one_build_arg` and updated the tool lists in `engine.rs`, `errors.rs`, `compose_file.rs`
  (`INSTALL_RUST` between `INSTALL_BUN` and `INSTALL_VSCODE`).
- Not verified: the actual install (rustup hosts are blocked in the sandbox); only `bash -n` was run (shellcheck is not
  installed). Pinned toolchains older than 1.64 have no `rust-analyzer` component and would fail the install.
- Fixed after the first real build: `rustup` was not found after `rustup-init`, because `/usr/local/cargo/bin` was not
  on PATH yet (the symlinks come later). The script now puts it on PATH, sets `RUSTUP_INIT_SKIP_PATH_CHECK=yes` (an
  apt `rustc` on PATH made `rustup-init` stop) and `HOME=/root` (rustup complains when `HOME` is the desktop user's
  `/config` under sudo).
