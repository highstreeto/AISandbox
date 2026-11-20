# Nested sandboxes: Podman in Podman

> **Status:** implemented; spike done on a Windows host (Podman 6.2 machine); the built image and a nested `up` not yet tried. See *As built*.

## Context
This repo is developed from inside the sandbox it builds, but the sandbox can't start a sandbox: the desktop image
strips Docker, has no Podman, and the desktop container lacks what a container engine needs. Goal: `./sandbox up`
works inside the desktop, so the app can be tested in itself.

Decisions (agreed):
- **Opt-in with one key, `[tools] podman = true`.** It installs Podman in the image and, because Podman is useless
  without them, makes the app add the nesting bits to the desktop container.
- **Rootless Podman only** on the outer host. Docker (seccomp/AppArmor would need `unconfined`) and rootful Podman
  are rejected with an error. Everything we relax stays inside the rootless user namespace.
- **No level limit**: every level gets the nesting bits again (proxy chain, devices, storage all recurse). Only
  host → desktop → nested sandbox is verified by hand. Depth is bounded by disk/RAM and the host's subuid range
  (each level loses ~1 id; document a larger `/etc/subuid` range, e.g. 1000000, for deeper nesting).
  Inside, `[llm]` is ignored with a note.

Why rootless Podman can do this without `--privileged`: Podman's default seccomp profile allows `unshare`/`mount` in
a user namespace (Docker's does not). The desktop needs `/dev/fuse`, `/dev/net/tun`, a non-overlay storage
directory, subuids for `abc`, and an unmasked `/proc` so nested containers can mount their own procfs.

## Design

### 1. The `podman` tool (image)
- `resources/desktop/Dockerfile`: `ARG INSTALL_PODMAN=false` + `RUN sandbox-install podman "${INSTALL_PODMAN}"`.
- `resources/desktop/tools/podman.sh` (pattern: `tools/rust.sh`, helpers in `_lib.sh`): apt install `podman`,
  `docker-compose-v2` (Ubuntu's package, `/usr/libexec/docker/cli-plugins/docker-compose`), `crun`, `passt`, `netavark`, `aardvark-dns`, `uidmap`, `fuse-overlayfs`, `catatonit`. Writes
  `/etc/containers/containers.conf.d/50-sandbox.conf` with the nested defaults:
  `[containers] default_sysctls = []` (ping_group_range can't be set nested), `[engine] cgroup_manager = "cgroupfs"`,
  `events_logger = "file"`. Storage stays the default (native overlay; works because storage is on a volume, see 2).
- **newuidmap/newgidmap get file capabilities, not setuid** (spike finding): `chmod 0755` + `setcap cap_setuid=ep
  newuidmap`, `setcap cap_setgid=ep newgidmap`. Ubuntu's setuid build stays euid 0, and writing `uid_map` of a
  namespace owned by the user then needs CAP_SYS_ADMIN, which the desktop lacks (EPERM). Check that the image build
  keeps the `security.capability` xattr (`getcap` in the built desktop).
- **`utsns = "host"`** in the containers.conf drop-in (spike finding): Podman's default seccomp profile only allows
  `sethostname` with CAP_SYS_ADMIN, so crun can't set a hostname in a nested UTS namespace. Nested mode therefore also
  omits `hostname: sandbox` from the template.
- **docker-compose as provider, through Podman's API socket** (spike finding: podman-compose 1.5 rejects
  `--project-directory`, which the app passes on every call, plus `up --wait` and `ps -a`; so the app's Podman path
  only works with docker-compose, as on the host). The drop-in pins `[engine] compose_providers =
  ["/usr/libexec/docker/cli-plugins/docker-compose"]`, `compose_warning_logs = false`. An s6 service runs
  `podman system service --time=0 unix://$XDG_RUNTIME_DIR/podman/podman.sock` as `abc`; `podman compose` finds that
  socket by itself. No app changes needed for compose.
- **Healthcheck runner** (spike finding: without systemd, health stays `starting` forever, so `up --wait` and
  `depends_on: service_healthy` would hang): an s6 service started by the tool (user `abc`, its `XDG_RUNTIME_DIR`)
  that every 3 s runs `podman healthcheck run` for each container whose status shows `(healthy|unhealthy|starting)`.
- `resources/init/allowlist.txt`: a commented **podman** group with what a nested *build* needs through the outer
  proxy: `lscr.io`, `ghcr.io`, `pkg-containers.githubusercontent.com`, `registry-1.docker.io`, `auth.docker.io`,
  `production.cloudfront.docker.com`, `dl-cdn.alpinelinux.org`, `archive.ubuntu.com`, `security.ubuntu.com`. Note in
  the group comment: the tools' download hosts must be allowed too (the inner build runs through the outer proxy).
- `resources/init/sandbox.toml` + README: `podman = false  # Podman in the sandbox (rootless Podman host only): containers and nested sandboxes`.

### 2. Outer desktop container (`resources/compose.yml.j2`, `src/engine/compose.rs`)
New model field `host.nesting: bool` (= tools has podman enabled, also in nested mode). When true, the
`desktop` service gets:
- `devices: ["/dev/fuse", "/dev/net/tun"]` (merged with `[desktop] devices`).
- `security_opt: ["label=disable", "unmask=ALL"]` (merged with `[desktop] security_opt`). `unmask=ALL` is what lets
  nested containers mount a fresh `/proc`; the alternative (`/proc:/proc` into nested containers) would show the
  outer processes and break s6 and the `pgrep plasmashell` healthcheck inside.
- `pids_limit: 32768` instead of 4096 (a whole inner desktop runs in it).
- Volume `containers:/config/.local/share/containers` (new project volume `containers`; on the host fs, so native
  overlay works and the home volume stays small; not touched by `reset-home`, removed with the project's volumes).
- env `SANDBOX_INSIDE: "1"`, added to `RESERVED_ENV` in `src/config/mod.rs`.

Validation in `src/config/mod.rs` / at `up`: `podman = true` with runtime Docker → "Nested containers need rootless
Podman. Set runtime = \"podman\" under `[host]` in sandbox.local.toml, or podman = false under `[tools]`."; with rootful
Podman (from `RuntimeInfo.rootless`) → similar message. The check belongs where `RuntimeInfo` is known
(`engine::compose::generate` or `need_runtime` in `src/runtime/mod.rs`).

### 3. Desktop start (`resources/desktop/root/custom-cont-init.d/`)
New `60-podman` (no-op when `podman` isn't installed):
- **subuids/subgids for `abc`** from `/proc/self/uid_map` / `gid_map`: total ids N (65537 with keep-id); write
  `abc:1:PUID-1` and `abc:PUID+1:N-PUID-1` to `/etc/subuid` and `/etc/subgid` (PUID = abc's uid after the
  linuxserver init renamed it). Works at any depth from whatever N it gets. Warn if N ≤ 65535 (a further level
  would lack ids such as `nobody`; hint: larger subuid range on the host).
- `chown abc:abc /config/.local/share/containers` (the 50-sandbox chown loop skips mounts).
- `install -d -m 0700 -o abc /run/user/$PUID` and export `XDG_RUNTIME_DIR` via `/etc/environment` (50-sandbox writes
  it; append there, or write it in 50-sandbox when podman is present).

### 4. The app running inside (nested mode, `SANDBOX_INSIDE=1`)
- **State stays in `.sandbox/.state`; a read-only `.sandbox` is an error.** The project's own `.sandbox` (and with
  `workspace = "ro"` the whole project) is read-only inside, so a nested sandbox runs from a writable copy (e.g. a
  checkout under `/config/...`). `Project::load` (`src/project.rs`) checks that the state dir is writable (create it /
  write a probe file) before anything else and fails with: "<dir> is read-only. Copy the project to a writable folder
  (e.g. under /config) and run sandbox there." (the "copy" hint only with `SANDBOX_INSIDE`; otherwise the plain
  read-only message). No template path changes: `./.state/...` stays as is.
- **Inner proxy chains to the outer proxy** (the inner egress network only reaches the outer `sandbox` network):
  `resources/proxy/Dockerfile` bakes a second static config `tinyproxy-nested.conf` = `tinyproxy.conf` +
  `Upstream http sandbox-upstream:3128`. In nested mode the template runs the proxy with that config and adds
  `extra_hosts: ["sandbox-upstream:<ip>"]`, where the app resolves the host of `$HTTPS_PROXY` (outer `proxy`) to an
  IP at render time (the name `proxy` means the inner proxy in there). Result: inner allowlist ∩ outer allowlist.
- **No `hostname:`** on the desktop (nested containers share the UTS namespace, see 1).
- **cgroup limits stay** (spike: nested Podman accepts `--memory`/`--pids-limit` with cgroupfs; enforcement is not
  guaranteed, the outer desktop's limits apply anyway). Drop `host.docker.internal:host-gateway` (meaningless there,
  and needs Podman ≥ 5).
- **`[llm]` ignored** with a note ("The local LLM is off in a nested sandbox."). `host.nesting` stays as in 2, so
  the inner desktop can nest again; its proxy chains to its parent, giving the intersection of all allowlists.
- Runtime: `auto` finds `podman` inside; no change. Docker inside → same error as in 2.

### 4b. Not in this plan: podman-compose on hosts
The app passes `--project-directory`, `up --wait` and `ps -a`, which podman-compose 1.5 rejects, so a host whose
`podman compose` provider is podman-compose can't run the sandbox today. Separate follow-up: detect it and say
"Install docker-compose (podman compose uses it)", and fix README/AGENTS, which claim podman-compose works.

### 5. Docs
README (Podman section: nested sandboxes, rootless only, memory ≥ 16g recommended, allowlist group), AGENTS.md
("Developing from inside the sandbox": how to test with a nested sandbox; nested mode in *How it works*), plan file.

## Spike (Windows host, Podman 6.2 machine; inner Ubuntu 26.04, Podman 5.7, podman-compose 1.5)
Done so far: rootless Podman runs nested with native overlay, netavark, 1000001 ids, `nobody` mapped, builds with
network, limits accepted — after the newuidmap file-caps and `utsns = "host"` fixes. Healthchecks don't run without
systemd (confirmed); the runner loop makes them healthy (confirmed). podman-compose is unusable for the app (4b), so
the desktop uses docker-compose + `podman system service` (needs `mkdir -p $XDG_RUNTIME_DIR/podman` first). With
that, confirmed: `compose up -d --wait --wait-timeout` (what the app calls) incl. `depends_on: service_healthy`,
`ps --format json`, `port`, a published `127.0.0.1` port, service DNS across an internal network. Not covered by
the spike: the internal network having no internet (result not reported), the proxy chain to an outer proxy, and
file capabilities surviving an image build (all checked during implementation).


## Tests (e2e are the spec; `tests/e2e/`, fixture in `tests/e2e/support.rs`, fake in `tests/fake_docker/`)
- `compose_file.rs` / `runtime_select.rs`: rootless Podman + `podman = true` → devices, security_opt, pids_limit,
  `containers` volume, `SANDBOX_INSIDE`; Docker and rootful Podman with `podman = true` → the errors; without the
  tool → byte-identical to today.
- New `tests/e2e/nested.rs`: `SANDBOX_INSIDE=1`, `HTTPS_PROXY=http://127.0.0.1:3128`. A read-only `.sandbox` →
  the error with the copy hint, no docker calls (Unix only). Writable → proxy command with
  `tinyproxy-nested.conf` + `sandbox-upstream:127.0.0.1`, no limits, no llm services + note, and with
  `podman = true` the nesting bits are still rendered (multi-level).
- Unit tests: upstream host parsing from `HTTPS_PROXY`, `RESERVED_ENV` rejects `SANDBOX_INSIDE`.

## Verification
1. `cargo test --workspace --all-targets --locked`, `cargo clippy --all-targets`.
2. Host (rootless Podman, WSL2 here): `podman = true` + podman allowlist group in this repo, `sandbox rebuild`.
   In the desktop: `podman info` (rootless, overlay), `podman run --rm alpine echo ok`.
3. In the desktop: `./sandbox up` in the read-only project → the read-only error. Then in a writable copy under
   `/config`: `./sandbox up` → inner desktop healthy; `./sandbox open` shows it in the outer
   desktop's browser; inner `curl` to an allowed and a blocked host (inner and outer `sandbox blocked`); `./sandbox down`.

## Critical files
`resources/desktop/Dockerfile`, `resources/desktop/tools/podman.sh` (new), `resources/desktop/root/custom-cont-init.d/60-podman` (new),
`resources/compose.yml.j2`, `resources/proxy/Dockerfile`, `resources/init/{sandbox.toml,allowlist.txt}`,
`src/engine/compose.rs`, `src/project.rs`, `src/config/mod.rs`, `src/runtime/mod.rs`, `tests/e2e/`, README.md, AGENTS.md.
Note: the repo's own `.sandbox` is read-only from inside, so implementation from inside the sandbox happens in a copy
and is handed over as a patch on the Desktop.

## As built

Implemented as planned, with these details and differences:

- **Runtime dir.** The desktop already sets `XDG_RUNTIME_DIR=/config/.XDG` (in the home volume), so no `/run/user`:
  `60-podman` deletes Podman's state there (`containers`, `libpod`, `podman`, `netns`, …) at every start, because it
  is not a tmpfs and would be stale after a restart.
- **Services.** `tools/podman.sh` writes two s6-rc longruns (`svc-podman-api`, `svc-podman-health`, depending on
  `init-services`, in the `user` bundle), both `s6-setuidgid abc` under `with-contenv` (so they get the proxy and
  `XDG_RUNTIME_DIR`). The runner is `/usr/local/bin/sandbox-podman-health`.
- **Provider.** Ubuntu's `docker-compose-v2` (the spike used the Docker repo's plugin from the webtop base image, at
  the same path). The drop-in pins it and turns off the provider banner.
- **Limits kept.** `mem_limit`, `cpus`, `pids_limit` stay in nested mode (accepted by nested Podman).
- **Where the app checks.** All in `engine::compose::generate`, before anything is written: `check_writable`
  (probe file in `.state/`; the "copy to /config" hint only with `SANDBOX_INSIDE`), `check_nesting` (Docker /
  rootful Podman errors), `upstream_proxy` (first of `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`; host
  parsed by `proxy_host`, resolved on port 3128, IPv4 preferred). The upstream port is fixed at 3128, the template's
  own proxy port. `[llm]` is dropped in `Project::load` (`Settings::llm_off_nested`); `up` says so, `info` shows
  "off (nested sandbox)", `llm …` commands explain it.
- **`SANDBOX_INSIDE`** is only set in a desktop with the tool (reserved in `[env]`, pointing to `tools.podman`).
  The e2e fixture unsets it and the proxy variables, so the tests also pass inside such a desktop.
- **Docker output unchanged**, and so is Podman's without the tool (existing compose-file tests untouched).
- **Tests.** `tests/e2e/nested.rs` (11 tests): the nesting parts and their merge with `[desktop]` lists, no parts
  without the tool, the Docker and rootful errors, the nested proxy chain, nesting again inside, `[llm]` off,
  missing/unresolvable outer proxy, read-only `.state/`. Unit tests: `proxy_host`, `with_extra`, `podman_tool`,
  `SANDBOX_INSIDE` reserved. Tool lists in `engine.rs`, `errors.rs`, `compose_file.rs` include `podman`.
- **Not verified on a real host yet:** that the image build keeps the file capabilities (`getcap` in the desktop),
  that `docker-compose-v2` installs on the webtop's Ubuntu, the s6 services starting, the proxy chain, and a full
  nested `sandbox up` (build through the outer proxy, the inner desktop healthy, `open`).
