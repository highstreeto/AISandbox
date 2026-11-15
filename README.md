# AI Sandbox — a Kubuntu desktop for this project

A KDE Plasma desktop (LinuxServer.io WebTop) in your browser, set up for one project. It can only reach the hosts you
allow and only see the folders you share. The settings live in the project's `.sandbox/` folder and are committed, so
anyone who clones the project gets the same environment.

```
 browser ──► 127.0.0.1:<port> ──► gateway ──► desktop (KDE + your tools)
             (localhost only)                    │ internal network: no internet route, no external DNS
                                                 ▼
                                               proxy (tinyproxy + allowlist.txt) ──► internet
```

The control app is a Rust program in this repository (`src/`, engine files in `resources/`). Working on the app itself:
see [AGENTS.md](AGENTS.md).

## Requirements

- **Docker** (Docker Desktop on Windows/macOS)
- **Rust** 1.88 or newer ([rustup.rs](https://rustup.rs)). The first run compiles the app (a minute or two); after
  that it starts instantly.

## Quick start

```bash
./sandbox init [folder]      # create .sandbox/ in a project (default: current directory)
./sandbox up                 # build, start and open the desktop
```

Use `sandbox.ps1` instead of `./sandbox` on Windows. The launchers work from any directory and use the `.sandbox`
folder next to them. For another project, either install the app once and run it inside that project:

```bash
cargo install --path <path to this repository> --bin sandbox
```

or run `cargo run --quiet --locked --manifest-path <path to this repository>/Cargo.toml --bin sandbox -- up` from it.
Without the launcher the app looks for `.sandbox/` in the current directory and its parents.

`init` creates `sandbox.toml`, `allowlist.txt` and a `.gitignore`. Existing files are kept, so it is safe to run again.
The first `up` builds the image (several minutes), waits for KDE and opens the desktop. Accept the self-signed
certificate.

## Commands

| Command | |
|---|---|
| `up` (`start`) | Build if needed, start, open the desktop · `--no-open`, `--no-pull` (skip model downloads) |
| `stop` / `restart` / `down` | Stop (all kept) · restart · remove containers (home and project kept) |
| `status` (`ps`) / `open` | Status and URL · open the desktop |
| `shell` / `claude [args]` | Bash / Claude Code CLI inside the sandbox, in the project folder |
| `logs [service]` | Follow logs (`proxy`, `gateway`, `desktop`, `llm`, `ollama`) · `-n <lines>`, `--no-follow` |
| `allow` / `deny <host>` | Allow or block a host and reload the proxy |
| `allowlist [-a]` / `blocked` / `reload` | Allowed hosts (`-a`: also the available ones) · blocked attempts · reload after editing |
| `rebuild` / `update` | After changing `[tools]` · newest versions of everything |
| `reset-home [-y]` | Wipe the persistent home (logins, IDE settings) |
| `init [folder]` / `info` | Create `.sandbox/` · show what was read from `sandbox.toml` |
| `llm status\|pull\|rm\|usage` | The [local LLM](#local-llm) |

`-v` shows the `docker` commands; `help <command>` or `<command> --help` shows details.

Each project gets its own container group (named after the folder), a random localhost port, and a home volume. The
project is mounted at `/work/<folder>-<id>` (`~/project` links there); the id is kept in `.sandbox/.state/id`, so moving
the folder keeps the same sandbox.

## Configure: `sandbox.toml`

```toml
[project]
# name = "myproject"        # container group name (default: folder name)
workspace = "ro"            # "rw" | "ro"
protect_git = true          # if .git/ should be readonly
# ui_port = 3001            # fixed port instead of a random one

[tools]                     # true = default version, false = skip, "x" = version, [..] = several
claude      = false         # Claude Desktop app
claude_code = false         # Claude Code CLI
dotnet      = false         # e.g. ["10.0", "8.0"]
node        = false         # e.g. 24
bun         = false
rust        = false         # true = stable, "1.88", ["stable", "nightly"]
vscode      = false
jetbrains   = false         # e.g. "rider", ["toolbox", "rider"]
apt         = []            # extra Ubuntu packages

[desktop]
title  = "AI Sandbox"
sudo   = false              # passwordless sudo
memory = "8g"
# cpus = 4
# timezone = "Europe/Vienna"            # default: the host's
# devices = ["/dev/dri:/dev/dri"]       # GPU (Intel/AMD, Linux)
# security_opt = ["seccomp=unconfined"] # only enable if needed!
# auth = { user = "me", password = "secret" }   # basic auth for the web desktop

[host]
# browser = "firefox"       # how up/open show the desktop, see below

[env]                       # extra environment variables
# MY_VAR = "value"

[[share]]                   # an extra folder, one block each
host = "../reference-docs"  # relative to the project folder
path = "/shares/docs"       # default: /shares/<folder name>
readonly = true             # default
```

This is what `init` writes. Keys that are left out default to `workspace = "rw"` and `sudo = true`.
Write versions as text (`"10.0"`, not `10.0`). Mistakes (unknown keys, wrong types, missing share folders, broken TOML)
are reported with the valid values. `sandbox info` shows what was read.

**Tools** are installed when the image is built. Inside the sandbox, `sandbox-install <tool> [version]` installs one
for now (lost on rebuild); without arguments it lists the tools and the hosts each one needs. The image has no Docker
and no C/C++ compiler: `rust = true` brings `build-essential`, otherwise add it under `apt` (e.g. for native npm
modules).

**Browser** (`[host] browser`): not set / `"default"` = the system browser, `false` = just print the URL, `"firefox"` =
a program or full path, `["chromium", "--incognito"]` = with arguments (the URL is added last, or replaces `{url}`).

**Personal settings** go in `.sandbox/sandbox.local.toml`, which is not committed and may only contain `[host]` and
`[llm]`. Its keys override `sandbox.toml`.

**Environment.** Variables the sandbox sets itself (proxy, user ids, `SANDBOX_*`, `OLLAMA_HOST`) can't be set in
`[env]`; `TZ`, `TITLE` and the login are set through `[desktop]`.

**Generated files.** `.sandbox/.state/` holds the generated `compose.yml` and image build folders, so you can see what
runs. It ignores itself in git, and hand edits there are overwritten. After updating the app, the next `up` rebuilds
what changed.

## Network: `allowlist.txt`

One host per line, wildcards like `*.example.com` (does not include `example.com`). Everything else is blocked. The file
has commented-out groups ready to enable: Claude, GitHub, NuGet, npm, pip, Gradle/Maven, Rust, apt, VS Code
marketplace, JetBrains. Use `./sandbox blocked` to see what was refused and `./sandbox allow <host>` to enable it
(takes about a second).

## Local LLM

A model running on your machine through Docker only. Each project that turns it on gets **Ollama** (with the GPU) and a
small gateway; models are stored once per machine in the Docker volume `sandbox-llm-models` and shared by all projects.

```toml
[llm]
model = "qwen3.6:27b"       # setting a model (or enabled = true) turns it on
# models = ["gpt-oss:20b"]  # more models to download
# context = 65536           # tokens (2048..1048576)
# keep_alive = "30m"        # how long a model stays in VRAM ("-1" = always)
```

In `sandbox.local.toml` you can turn it off for your machine (`enabled = false`), pick another `model` or `context`,
and set `gpu = "nvidia"` (default), `"amd"` (Linux, ROCm) or `"none"` (CPU, slow).

`up` downloads missing models (`--no-pull` skips). `down` and `reset-home` never delete models; use
`sandbox llm rm <model>`. `llm status` shows models and GPU use, `llm usage` requests and tokens per day.

**Inside the sandbox** the endpoint is `http://llm:11434` (`SANDBOX_LLM_URL`, model in `SANDBOX_LLM_MODEL`). Run
`sandbox-llm` for setup hints:

- **Claude Code:** `sandbox-llm claude` runs it against the local model; plain `claude` keeps your cloud login.
- **OpenAI-compatible tools** (OpenCode, Continue, …): base URL `http://llm:11434/v1`, any API key.
- **Ollama clients** (VS Code Copilot BYOK, Continue): `http://llm:11434`.
- **Scripts:** `eval "$(sandbox-llm env)"`.

The gateway only allows inference and read-only calls; downloads go through `sandbox llm pull` on the host, so the
model server can't be used to get around the allowlist.

**GPU setup.** Windows: Docker Desktop with WSL2 and a current NVIDIA Windows driver. Linux: NVIDIA driver plus the
NVIDIA Container Toolkit. If the GPU can't be used, `up` says so.

**Models for 24 GB of VRAM:**

| Model | Size (Q4) | Notes |
|---|---|---|
| `qwen3.6:27b` (default) | ~17 GB | Dense, reliable tool calls, room for 64K context |
| `qwen3.6:35b-a3b` | ~22 GB | Mixture of experts, faster, little room for context |
| `gpt-oss:20b` | ~14 GB | Clean tool calls, also fits 16 GB |
| `devstral-small-2` | ~15 GB | Mistral's agentic coding model |
| `qwen3-coder:30b` | ~19 GB | Older, widely supported |

Sizes are rough, and not every tag has been tested. Agent tools need a model with the *tools* capability.

## Isolation

| | |
|---|---|
| Network | Only through the allowlist proxy; no direct route, no external DNS, HTTPS only to port 443 |
| Project | Read-write or read-only (`workspace`) |
| `.sandbox/` | Read-only inside: the agent can't widen its own allowlist or tools |
| `.git/` | Read-only (`protect_git`), so no hooks can be planted that run on your host. And the agent cannot commit any changes. |
| Home | A Docker volume per project |
| UI | `127.0.0.1` only, optional basic auth |
| Proxy / gateway | Run as `nobody`, read-only filesystem, no capabilities |

Files the agent writes in the project (build scripts, `package.json` scripts, `.vscode/tasks.json`) run on your host
if you execute them there. Review changes first.

## Claude Desktop (Linux beta)

Chat and Code work; Cowork needs KVM and doesn't. Sign-in is stored in KDE Wallet, so you may need to sign in again if
the wallet isn't available. If something fails to load, check `./sandbox blocked`.

## Troubleshooting

| Problem | Fix |
|---|---|
| Blank page on first open | Reload; KDE was still starting |
| "Docker is not running" | Start Docker Desktop or the Docker service |
| "cargo not found" / build error | Install Rust 1.88 or newer |
| "Could not find the .sandbox folder" | Use the launchers, or run inside a project with `.sandbox/` (or `init` one) |
| "Another project already uses the container group" | Two projects share a folder name: set `name` under `[project]` |
| "… was edited by hand and has been restored" | Files in `.state/` are generated; change `sandbox.toml` |
| ".sandbox/engine/ is no longer used" | Old layout: check it for your own edits, then delete `.sandbox/engine/`, `.sandbox/README.md` and `.sandbox/.gitattributes` |
| "sandbox.yml is no longer read" | Move the settings to `sandbox.toml`; the message says where each key goes |
