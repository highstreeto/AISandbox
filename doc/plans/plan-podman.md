# Podman support (default when both are installed)

> **Status:** implemented; not yet tried on a real Podman host (no Podman where it was written). See *As built*.

## Context
`sandbox` drives everything through the `docker` CLI: `docker compose`, `docker info`, `docker ps`, and `docker volume`. Most of that is in `src/docker.rs`; two `volume` calls are in `src/commands/maintenance.rs:95` and `lifecycle.rs`. We want Podman to work as well, and to be chosen automatically when it is installed (even if Docker is too).

Decisions made:
- Compose runs through **`podman compose`**. That delegates to docker-compose (through the Podman socket) when available, otherwise to podman-compose.
- Override: env var **`SANDBOX_RUNTIME=auto|podman|docker`** (it wins), then **`[host] runtime = "auto"|"podman"|"docker"`** in `sandbox.local.toml` (machine-specific). It is also accepted in `sandbox.toml` because `[host]` is allowed in both files.

## Design

### 0. Introduce runtime (engine stays as described)
- **`src/engine/` stays as it is.** It is already independent of the runtime: Podman builds the same Dockerfiles (Buildah reads the Dockerfile format), and `podman compose` reads the same `compose.yml`. The only change there is that `engine/compose.rs` passes `host.runtime` into the template (section 3).
- **New module `src/runtime/`** for everything that talks to Docker/Podman, replacing `src/docker.rs`. **No `containers.rs`**: the runtimes themselves are the objects the commands use.
  - `runtime/mod.rs`: `trait Runtime`, `struct Ctx { project, verbose, ui }` (what today's `Docker` struct holds), `RuntimeSetting`, `select()`, and `ServiceState`.
  - `runtime/docker.rs`: `pub struct Docker<'a> { ctx: Ctx<'a> }`, `impl Runtime for Docker`.
  - `runtime/podman.rs`: `pub struct Podman<'a> { ctx: Ctx<'a> }`, `impl Runtime for Podman`.
  - `runtime/ps.rs`: `parse_ps` for compose JSON (today's code) plus the podman-compose shape, with unit tests.
- **The trait is the operations the commands need**, one method per thing the app does, so commands never build docker arguments themselves:
  ```rust
  pub trait Runtime {
      fn ctx(&self) -> &Ctx;
      fn name(&self) -> &'static str;                              // "Docker" / "Podman"
      fn program(&self) -> &'static str;                           // "docker" / "podman"

      // host level
      fn check_running(&self) -> Result<String>;                    // version; "not running" error with hint
      fn create_volume(&self, name: &str) -> ProcResult;
      fn remove_volume(&self, name: &str) -> ProcResult;
      fn project_owner_ids(&self) -> Vec<String>;                   // sandbox.id labels for the collision check

      // the project's compose group
      fn compose_up(&self, args: &[&str]) -> ProcResult;
      fn compose_build(&self, pull: bool) -> Result<i32>;          // attached
      fn compose_stop(&self) -> ProcResult;
      fn compose_down(&self) -> ProcResult;
      fn compose_restart(&self, services: &[&str]) -> ProcResult;
      fn compose_ps(&self) -> Result<Vec<ServiceState>>;
      fn compose_port(&self, service: &str, port: u16) -> Option<String>;
      fn compose_logs(&self, services: &[String], tail: i64, follow: bool) -> Result<i32>;   // attached
      fn compose_logs_captured(&self, service: &str) -> ProcResult;  // `blocked` reads the proxy log
      fn compose_exec(&self, service: &str, cmd: &[&str]) -> ProcResult;   // -T, captured (llm)
      fn exec_desktop(&self, cmd: &[String]) -> Result<i32>;      // attached, as abc in work_dir (shell, claude)

      // facts for the compose template and messages
      fn template_name(&self) -> &'static str;                      // host.runtime in compose.yml.j2
      fn gpu_hint(&self) -> &'static str;
  }
  ```
  Most methods have **default implementations in `mod.rs`** built on two small hooks, `compose_base() -> Vec<String>` (`compose --ansi never -p … --project-directory … -f …` for Docker; `compose -p …` with `COMPOSE_ANSI=never` for Podman) and `compose_env()`. So `docker.rs` is short, and `podman.rs` overrides only what really differs: `check_running` (other `info` format, `podman machine start` hint), `compose_ps` (podman-compose JSON), `template_name`, `gpu_hint`, and the NVIDIA check in `compose_up` (section 3). Adding another runtime later is one file.
- `ui_url()` stays a free function on top of `compose_port`, and `need_runtime` (Docker installed + running + collision) is a free function on `&dyn Runtime`. Commands take `&dyn Runtime` (today `&Docker`), named `rt`.
- So `engine/` = what gets generated, `runtime/` = what runs it. AGENTS.md layout table updated to match.
- The setting is **`[host] runtime`** and the env var **`SANDBOX_RUNTIME`**, so neither clashes with the existing "Engine" wording.
- Commits: first a refactor (`refactor: move docker.rs into runtime/ behind a Runtime trait`) with only the Docker impl and no behaviour change (all e2e tests unchanged and green), then `feat: support Podman as container runtime` on top.

### 1. Runtime selection (`src/runtime/mod.rs`)
- `pub fn select<'a>(ctx: Ctx<'a>) -> Result<Box<dyn Runtime + 'a>>`, from `SANDBOX_RUNTIME`, then `project.settings.runtime`:
  - Explicit `podman`/`docker` → that runtime, and an error if it is not on PATH ("Podman is not installed (runtime = \"podman\" in sandbox.local.toml).").
  - `auto` → Podman if `process::on_path("podman")`, else Docker, else bail "Neither Podman nor Docker is installed."
- The two `volume` calls outside today's `docker.rs` (`maintenance.rs:95`, `lifecycle.rs` via `create_volume`) go through `remove_volume`/`create_volume`.

### 2. Runtime-specific differences (all inside `src/runtime/`)
| Call | Docker | Podman |
|---|---|---|
| Daemon check | `docker info --format {{.ServerVersion}}` | `podman info --format {{.Version.Version}}`; error text: "Podman is not running. Start the Podman machine (`podman machine start`) and try again." on macOS/Windows |
| Compose | `docker compose --ansi never …` | `podman compose …`. Keep `--ansi never` only if the provider accepts it: probe once and drop the flag on "unknown flag" (or set `COMPOSE_ANSI=never`/`NO_COLOR=1` in env, which both providers honor; preferred, no probe). |
| Collision check | `ps -a --filter label=com.docker.compose.project=…` | same (`podman ps` supports it; docker-compose via socket and podman-compose both set this label) |
| `compose ps --format json` | as today | `parse_ps` already accepts both shapes; add a fallback for podman-compose's `podman ps` JSON (`Labels["com.docker.compose.service"]`, `State`, `Ports[]` with `host_ip/host_port/container_port`) |
| `compose port gateway 3001` | as today | same output format (`0.0.0.0:PORT`/`127.0.0.1:PORT`); `ui_url` already takes the part after `:` |

### 3. Compose template (`resources/compose.yml.j2`, `src/engine/compose.rs`)
Add `host.runtime` (from `Runtime::template_name()`) to the template context and branch only where needed:
- **Rootless user mapping**: for Podman add `userns_mode: keep-id` to `desktop` so bind-mounted project files keep the host user's ownership (PUID/PGID stay the host ids). This is the important one; without it, files written in the sandbox end up owned by subuids.
- **SELinux**: bind mounts get `selinux: z` (long-syntax `bind: {selinux: z}`) for Podman on Linux, so Fedora/RHEL hosts work.
- **host-gateway**: `host.docker.internal:host-gateway` works with Podman ≥ 5; add `host.containers.internal` too, or skip for Podman < 5 (check the version from the `info` call).
- **NVIDIA GPU**: Podman uses CDI: `devices: ["nvidia.com/gpu=all"]` instead of the `deploy.resources.reservations` block. AMD (`/dev/kfd`, `/dev/dri`) is unchanged.
  - What is needed on the host is the **NVIDIA Container Toolkit** (`nvidia-container-toolkit`) plus a generated CDI spec, not the CUDA Toolkit. CUDA comes inside the `ollama/ollama` image; the host only needs the driver. Docker needs the same toolkit today (registered as a Docker runtime instead of CDI).
  - **Check, don't install.** Installing needs root, a distro-specific package repo and a driver match, so the app should not do it. Instead `Podman::compose_up`, when `llm.gpu = "nvidia"` (the default) and on Linux, checks for the CDI device first: `nvidia-ctk cdi list` contains `nvidia.com/gpu=all`, or as a fallback, a spec file in `/etc/cdi/` or `/var/run/cdi/`. If it's missing, it stops before starting with: "The NVIDIA GPU is not available to Podman. Install nvidia-container-toolkit and run `sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml`, or set gpu = \"none\" under `[llm]` in sandbox.local.toml."
  - In a `podman machine` (macOS/Windows) the spec lives inside the VM, so there is no pre-check; the existing after-failure `gpu_hint` (Podman variant) covers it.
- `restart: unless-stopped` only survives reboots with `podman-restart.service`. Document it; no code needed.

### 4. Settings (`src/config/mod.rs`)
- `read_host` reads `runtime` in addition to `browser`; values `auto|docker|podman`, invalid values list the valid ones (existing error style). The local file overrides the main one, as with `browser`.
- `Settings.runtime: RuntimeSetting` (default `Auto`).
- `resources/init/sandbox.toml` `[host]` comment: `# runtime = "auto"  # "podman" (preferred if installed) or "docker"`.

### 5. User-facing text
- `need_docker` → `need_runtime`; messages use `rt.name()`. Spinner "Checking Podman…".
- `ui.show_output("docker compose …")` labels in `lifecycle.rs`, `network.rs`, `maintenance.rs` use the program name.
- `info` gets a **Runtime** row ("Podman 5.4.1 (auto)") next to the existing Engine row, using the same selection, without failing if nothing is installed.
- `llm::gpu_hint` gains a Podman variant (nvidia-container-toolkit + CDI: `nvidia-ctk cdi generate`).
- README + AGENTS.md: "Docker or Podman", selection rules, and Podman notes (rootless, `podman machine` on macOS/Windows, docker-compose recommended as provider).

## Tests (the e2e tests are the specification)
- `tests/e2e/support.rs`: the fixture also links `fake-docker` as `podman[.exe]` in a **separate** dir, and **every run sets `SANDBOX_RUNTIME=docker` by default** so existing tests stay exact and a real Podman on the developer's PATH can't leak in. `RunOpts` gets `runtime`/`with_podman` options.
- `fake_docker/main.rs`: log `name` already distinguishes `docker`/`podman`; add default rules for `podman info` and the podman-compose ps JSON.
- New `tests/e2e/runtime_select.rs`:
  - both on PATH, auto → podman calls; only docker → docker; only podman → podman; neither → "Neither Podman nor Docker is installed."
  - `SANDBOX_RUNTIME` beats `[host] runtime`; local file beats main; invalid value error.
  - explicit runtime missing → error naming the setting.
  - Podman not running → Podman message.
- NVIDIA check: Podman + gpu nvidia + fake `nvidia-ctk` with/without the CDI device → starts / stops with the message; `gpu = "none"` skips the check.
- `compose_file.rs`: Podman render has `userns_mode: keep-id`, `selinux: z`, CDI GPU; Docker render byte-identical to today.
- Unit tests: `parse_ps` with podman-compose JSON; runtime setting parsing.

## Verification
1. `cargo test --workspace --all-targets --locked` and `cargo clippy --all-targets`.
2. Manual, on a Linux host with Podman 5 (+ docker-compose installed, then removed to test podman-compose): `sandbox -v up` shows `$ podman compose …`; `status`, `open`, `shell`, `logs`, `allow`, `reload`, `down`; check that files created in the sandbox are owned by the host user; `SANDBOX_RUNTIME=docker sandbox status` still uses Docker.

## Critical files
`src/docker.rs` → `src/runtime/{mod,docker,podman,ps}.rs`, `src/lib.rs`, `src/config/mod.rs`, `src/engine/compose.rs`, `resources/compose.yml.j2`, `src/commands/{lifecycle,maintenance,network,llm}.rs`, `resources/init/sandbox.toml`, `tests/e2e/support.rs`, `tests/fake_docker/main.rs`, README.md, AGENTS.md.

## As built

Everything above is implemented: first `refactor: move docker.rs into runtime/ behind a Runtime trait` (Docker only,
every existing e2e test unchanged and green), then `feat: support Podman as container runtime`, then follow-up fixes
and refactors from trying it. Differences and details worth knowing:

- **Trait.** Besides the operations listed above, it has `compose_exec_attached` (`llm pull` with Ollama's progress),
  `volume_exists`, and the plumbing the defaults are built on: `run` (`<program> <args>`), `compose` (captured; `echo`
  also shows the output as it arrives), `compose_attached`, the free function `compose_base`, and the hooks
  `captured_flags`, `env`, `captured_env`, `not_running_message`. An implementation names its `RuntimeKind`
  (`Docker` / `Podman`: `name()`, `program()`, serialized as `docker` / `podman`), and `name()` / `program()` default
  to it. `select` returns a `Box<dyn Runtime + 'a>`; `available` (also used by `info`) returns the `RuntimeKind` a
  setting resolves to. `need_runtime` returns the `RuntimeInfo` (`kind`, `version`, `rootless`) that
  `engine::compose::generate` takes.
- **`Ctx` is `{ project, verbose }`.** The `Ui` is not stored: every operation that runs a program takes `ui: &Ui`.
- **NVIDIA check.** Not inside `compose_up` but a separate hook `check_before_up`, called at the start of `up` and
  `rebuild`, so a missing CDI spec stops the command before a long image build. Linux only: `nvidia-ctk cdi list` must
  list `nvidia.com/gpu=all`; without `nvidia-ctk`, any spec under `/etc/cdi` or `/var/run/cdi` naming
  `nvidia.com/gpu` counts. Nothing is installed.
- **Colours / banner.** Podman gets no `--ansi never` (podman-compose doesn't know it). Captured calls set
  `COMPOSE_ANSI=never` and `NO_COLOR=1` instead, and every call sets `PODMAN_COMPOSE_WARNING_LOGS=false` (no
  "Executing external compose provider" banner). `process::capture` and `process::interactive` take the extra
  environment variables for this; `capture` also takes `echo`.
- **`-v` and `up`.** With `-v` there is no live table: `compose up` is captured and echoed as it runs (`echo`), and
  its output isn't shown a second time when it fails.
- **Models volume.** `up` / `rebuild` run `volume inspect sandbox-llm-models` and create the volume only if it is
  missing: `podman volume create` fails for an existing volume, where Docker's is idempotent. This adds one call
  for Docker too.
- **Rootless.** `podman info --format "{{.Version.Version}} {{.Host.Security.Rootless}}"`. Only rootless Podman gets
  `userns_mode: "keep-id"`, together with `user: "0:0"`, because keep-id would otherwise start the container as the
  host user and the image's init must start as root (it then switches to PUID/PGID, the host ids). Rootful Podman
  rejects keep-id.
- **SELinux.** `.:/sandbox:ro,z` and `bind: {selinux: z}` on every desktop bind mount, for Podman only. This relabels
  the project folder and every `[[share]]` on SELinux hosts, so don't share system folders or the whole home.
- **host-gateway.** Unchanged (`host.docker.internal:host-gateway`); it needs a Podman that understands
  `host-gateway` (5.x). No version check.
- **Docker output is unchanged** (the compose file, the calls and the messages), except: the `volume inspect` call
  above; "Docker is not installed." now names where Docker was chosen (`(SANDBOX_RUNTIME=docker)`,
  `(runtime = "docker" in sandbox.local.toml)`); with `auto` and neither installed: "Neither Podman nor Docker is
  installed."; failed-call panels and errors name the program (`docker compose …`, `podman compose …`).
- **`info`.** A **Runtime** row with the runtime and why (`Podman (auto)`, `Docker (SANDBOX_RUNTIME=docker)`,
  `… : not installed`). No version: `info` runs no container commands.
- **Tests.** The fixture links the fake as `podman` and `nvidia-ctk` in a second folder and sets
  `SANDBOX_RUNTIME=docker` unless a test chooses (`RunOpts::runtime`). `tests/e2e/runtime_select.rs` (20 tests) covers
  the selection rules, Podman calls (`status`, `up`, `reset-home`), podman-compose's `ps` JSON, the rootless and
  rootful compose file, the CDI device and check (Linux only), and the `info` row. Unit tests: `[host] runtime`
  parsing, podman `ps` entries.
- **Home ownership (not Podman-specific, found while testing).** `sandbox-install` runs the tool installers with
  `HOME=/root`, so they no longer leave root-owned `~/.cache`, `~/.gnupg`, `~/.config/opencode` in the home volume
  (KDE's app menu stays empty without a writable `~/.cache`). `50-sandbox` gives everything in `/config` not owned
  by `abc` back to `abc`, skipping mounts under `/config`, to repair homes from older images.
- **Not verified on a real host.** Worth checking first: `podman compose` with docker-compose as provider (`up --wait`,
  `port`, `exec -T`), then with podman-compose, which may not support `--project-directory`, `--wait` or
  `port`/`ps --format json` the way docker-compose does; file ownership in the project after the agent writes files;
  the CDI GPU in Ollama.
