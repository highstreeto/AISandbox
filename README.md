# AI Sandbox — a Kubuntu desktop for this project

A KDE Plasma desktop (LinuxServer.io WebTop) in your browser, set up for this project.
It can only reach hosts you allow and only see the folders you share.
`.sandbox/` holds the project's two files, `sandbox.toml` (settings) and `allowlist.txt` (network), and is committed
with the project. Anyone who clones it gets the same ready-to-develop environment. The engine (compose template,
Dockerfiles, tool scripts, proxy config) is part of the control app, a native Rust program: its sources are in `src/`
and the engine files in `resources/` of this repository.

```
 browser ──► 127.0.0.1:<port> ──► gateway ──► desktop (KDE + your tools)
             (localhost only)                    │ internal network: no internet route, no external DNS
                                                 ▼
                                               proxy (tinyproxy + allowlist.txt) ──► internet
```

## Set up a project: `sandbox init`

```bash
./sandbox init [folder]      # default: the current directory
```

Creates `<folder>/.sandbox/` with `sandbox.toml`, `allowlist.txt` and `.gitignore` (it keeps `sandbox.local.toml` out of git) from templates, then the generated `.state/`
(see below). Files that already exist are kept and listed (`sandbox.toml exists — kept`), so it is safe to run again and
only fills in what is missing. There is no `--force`: to start over, delete the file. It needs no Docker and ignores
`SANDBOX_DIR`. If a parent folder already has a `.sandbox/`, it warns and creates a separate sandbox for the sub-folder.
Next: edit `.sandbox/sandbox.toml`, then run `up`.

## Start / stop

The launchers are in the repository root. They work from any directory and don't change your current directory.

```bash
./sandbox up                 # Linux / macOS / WSL / Git Bash (sh)
sandbox.ps1 up               # Windows (cmd or PowerShell)
./sandbox help up            # help for one command (or: up --help)
```

Requires Docker and a **Rust toolchain** (1.88 or newer, [rustup.rs](https://rustup.rs)) on the host. The control app is
a normal Cargo package in the repository root (`Cargo.toml`, `src/`, engine files in `resources/`). The launchers run
`cargo run --quiet --locked --manifest-path <repository>/Cargo.toml --bin sandbox -- <args>` with `SANDBOX_DIR` set to
the `.sandbox` folder next to them, so every argument reaches the app unchanged. `--manifest-path` does not change the
current directory. The first run (and the first one after the sources change) downloads the crates and compiles, which
takes a minute or two; after that the app starts in milliseconds.

Without `SANDBOX_DIR` the app searches upwards from the current directory for a `.sandbox` folder. That is how another
project uses it: from inside that project, run `cargo run --quiet --locked --manifest-path <path to this
repository>/Cargo.toml --bin sandbox -- up`, or install the binary once with `cargo install --path <path to this
repository> --bin sandbox` and run `sandbox up` in any project.

`up` first checks that Docker is running. It then builds the image (several minutes the first time), waits until the KDE desktop is
actually ready and opens it in your browser. Accept the self-signed certificate.

| Command | |
|---|---|
| `up` / `stop` / `restart` / `down` | Start · stop (all kept) · restart · remove containers (home + project kept). `up --no-pull` skips model downloads |
| `status` / `open` | Status + URL · open the desktop |
| `shell` / `claude` | Bash / Claude Code CLI inside the sandbox, in the project folder |
| `rebuild` / `update` | After changing tools · newest versions of everything |
| `reset-home` | Wipe the persistent home (logins, IDE settings) · `--yes` skips the question |
| `init [folder]` | Create `.sandbox/` (`sandbox.toml`, `allowlist.txt`, `.gitignore`) in a project folder |
| `info` | Project identity, mount path, engine version, settings, tools and shared folders as read from `sandbox.toml` |
| `logs [service]` | Follow logs · `--tail <n>`, `--no-follow` |
| `llm status` / `pull` / `rm` / `usage` | The local LLM: [see below](#local-llm) |

Add `-v` / `--verbose` to any command to see the `docker` commands it runs.

The container group is named after the project folder (`demo-project` → group `demo-project`).
The project is mounted at `/work/<folder>-<hash>`, for example `/work/demo-project-8a164227`, and `~/project` links there.
The hash is created once and stored in `.sandbox/.state/id`, so moving the folder keeps the same group and home.
Each project gets a random free localhost port, so several sandboxes can run side by side.

## Configure: `sandbox.toml`

All per-project settings are in one committed [TOML](https://toml.io) file:

```toml
[project]
# name = "myproject"        # container group name (default: project folder name)
workspace = "rw"            # "rw" | "ro"
protect_git = true          # .git/config + .git/hooks read-only
# ui_port = 3001            # fixed port instead of a random one

[tools]
claude      = true          # Claude Desktop app
claude_code = false         # Claude Code CLI
dotnet      = "10.0"        # ["10.0", "8.0"] for both
node        = false         # 24
bun         = true
rust        = false         # "1.88", ["stable", "nightly"], ...
vscode      = true
jetbrains   = false         # "rider", ["toolbox", "rider"], ...
apt         = []            # extra Ubuntu packages

[desktop]
title  = "AI Sandbox"
sudo   = true               # false = no passwordless sudo
memory = "8g"
# cpus = 4
# timezone = "Europe/Vienna"            # default: taken from the host
# devices = ["/dev/dri:/dev/dri"]       # GPU (Intel/AMD, Linux)
# security_opt = ["seccomp=unconfined"] # if GUI apps crash
# auth = { user = "me", password = "secret" }   # basic auth for the web desktop

[host]                      # on this machine
# browser = "firefox"       # for up/open: see below

[env]                       # extra environment variables
# MY_VAR = "value"

[[share]]                   # an extra folder; one block per folder
host = "../reference-docs"  # relative to the project folder
path = "/shares/docs"       # in the sandbox (default: /shares/<folder name>)
readonly = true             # default: true
```

**Tools.** Every `[tools]` value works the same way: `true` means the default version, `false` or `[]` means skip,
a text or number is a version, and a list is several. Write versions with dots as text (`"10.0"`, not `10.0`).
Each one runs `sandbox-install <tool> <value>` while the image is built. Inside the sandbox you can run the same command,
e.g. `sandbox-install jetbrains rider`. With no arguments it lists the tools and the hosts each one needs.
Runtime installs are lost on rebuild, so put permanent tools in `sandbox.toml`.
`rust` installs rustup with rustfmt, clippy, rust-src and rust-analyzer; the toolchains live in `/usr/local/rustup`, while
`~/.cargo` (crate registry cache, `cargo install`ed tools) persists in the home volume.

**Browser.** `[host] browser` says how `up` and `open` show the desktop:

| Value | |
|---|---|
| not set, `"default"` or `true` | The system's default browser |
| `false` | Never open a browser (`up` just prints the URL; so does `open`) |
| `"firefox"` | A program on `PATH`, or a full path |
| `["chromium", "--incognito"]` | A program with arguments; the address is added as the last one |
| `["chrome", "--app={url}"]` | `{url}` in an argument is replaced by the address instead (at most once) |

The command is started in the background and its output is discarded. If it can't be started you get a warning with
the address; it does not fall back to the default browser. On Windows a name without a path (`firefox`, `msedge`,
`chrome`) is started through `cmd /C start`, so apps registered with Windows work without being on `PATH`.
`up --no-browser` always wins. `sandbox info` shows the setting and the file it comes from.

**Personal settings: `sandbox.local.toml`.** Your browser is your own choice, so `[host]` can also be set in
`.sandbox/sandbox.local.toml`, next to `sandbox.toml`, and so can your machine's part of `[llm]` (the GPU, a smaller
model). It is optional and **not committed** (`init` creates a `.sandbox/.gitignore` that lists it; the app warns if it
exists but isn't ignored). Only `[host]` and `[llm]` are allowed in it
(`sandbox.local.toml: only [host] and [llm] can be set here (found 'tools').`): everything that shapes the sandbox
stays in the committed file. A key set in it overrides the same key in `sandbox.toml`.

```toml
# .sandbox/sandbox.local.toml
[host]
browser = ["chromium", "--incognito"]
```

**Checked.** Unknown sections, keys and tools are errors that list the valid names, and so are wrong types and invalid
values (`workspace = "rx"`, a share folder that doesn't exist, broken TOML with its line and column).
Environment variables the sandbox sets itself (`HTTP_PROXY`, `NO_PROXY`, `PUID`, `PGID`, `SANDBOX_WORKDIR`, `SANDBOX_LLM_URL`, `SANDBOX_LLM_MODEL`, `SANDBOX_LLM_CONTEXT`, `OLLAMA_HOST`) can't be
set in `[env]`; `TZ`, `TITLE`, `SANDBOX_SUDO`, `CUSTOM_USER`/`PASSWORD` are set through `[desktop]` instead.

**How it's used.** The app renders its embedded `compose.yml.j2` (a [MiniJinja](https://github.com/mitsuhiko/minijinja)
template, Jinja syntax) with these settings and this machine's facts (user ids, time zone, mounts, UI port) into `.state/compose.yml`,
and runs every `docker compose` command with only that file. It's regenerated before each command. Look at it to see
what will actually run. The UI port, the project mounts, the `.sandbox` read-only mount and the proxy settings are
written by the template itself, so nothing in `sandbox.toml` can change them.

## What is in `.sandbox/`

```
.sandbox/
  sandbox.toml          yours: settings                      (commit)
  allowlist.txt         yours: network allowlist             (commit)
  .gitignore            keeps sandbox.local.toml out of git  (commit)
  sandbox.local.toml    yours, optional: [host], [llm]       (not committed)
  .state/               generated, ignores itself (its own .gitignore holds "*")
    id                  stable project id
    .manifest           what the app wrote last time: file hashes, engine hash, app version
    compose.yml         the one file docker compose runs with
    desktop/            build folder of the desktop image, exactly as used
    proxy/              build folder of the proxy/gateway image
    llm/                build folder of the local LLM gateway (nginx)
```

**The engine files are generated.** The Dockerfiles, the tool scripts, the startup script and the proxy config are
embedded in the app and written to `.state/desktop/`, `.state/proxy/` and `.state/llm/` before every command that uses Docker. They
are there so you can see what is built; they can't be edited. A file you change by hand is restored on the next
command, with a warning (`.state/desktop/Dockerfile was edited by hand and has been restored.`). Files an older app
version left behind in these folders are removed; nothing else in `.state/` or outside it is touched. Every file
that allows comments starts with `Generated by sandbox <version> — changes are overwritten. Settings: sandbox.toml.`

To change a sandbox, use the keys in `sandbox.toml`: the built-in `[tools]`, `apt = [...]` for extra Ubuntu packages,
`[env]`, `[desktop]`. If you need a different engine, fork this repository and build the app with your own files in
`resources/` and build the app from it.

**After updating the app.** The engine may have changed. `up` builds the images every time, and Docker's build cache
redoes only the layers whose files or `[tools]` changed, so the next `up` picks the change up. `sandbox info` shows the
app version and the engine hash.

**Migrating from the old layout.** Earlier versions copied the engine into each project as `.sandbox/engine/`. That
folder is no longer used. The app warns about it on every command but never deletes it, because it is committed and may
hold your own edits. Once you have checked that nothing in it is needed, delete `.sandbox/engine/`,
`.sandbox/README.md` and `.sandbox/.gitattributes` (the `.state/` folder ignores itself).

## Network: `allowlist.txt`

One host per line, with wildcards like `*.example.com`. Everything else is blocked.

| | |
|---|---|
| `./sandbox blocked` | Which hosts were blocked (with counts, and whether they're allowed by now) |
| `./sandbox allow api.nuget.org` | Allow (uncomments an existing entry) and reload in about 1 s |
| `./sandbox deny api.nuget.org` | Comment out and reload |
| `./sandbox allowlist -a` | Allowed hosts by group, plus the commented-out ones ready to enable |

The file includes ready-made, commented-out groups for NuGet, npm, pip, Gradle/Maven, Rust (rustup + crates), apt, VS Code marketplace,
JetBrains, and a local LLM on the host (`host.docker.internal`; the built-in [local LLM](#local-llm) needs no entry).

## Local LLM

A model that runs on your machine, without installing anything on it except Docker. Each project that turns it on gets
its own stack next to the desktop: **Ollama** (with the GPU) and a small **nginx gateway** in front of it. The models are
kept in one Docker volume for the whole machine, so a model is downloaded once however many projects use it.

```toml
# .sandbox/sandbox.toml
[llm]
model = "qwen3.6:27b"       # Ollama tag; setting a model (or enabled = true) turns the local LLM on
# enabled = true            # on with the built-in default model (qwen3.6:27b)
# models = ["gpt-oss:20b"]  # more models to download
# context = 65536           # context length in tokens (2048..1048576), the default
# keep_alive = "30m"        # how long a model stays in VRAM after the last request ("-1" = always)
```

```toml
# .sandbox/sandbox.local.toml  (this machine only)
[llm]
# enabled = false           # off here even if the project turns it on (or true: on here only)
# model = "gpt-oss:20b"     # a smaller model for this machine
# context = 32768
# gpu = "nvidia"            # "nvidia" (default) | "amd" (Linux: ROCm, /dev/kfd + /dev/dri) | "none" (CPU, slow)
```

`sandbox up` starts the stack, creates the volume `sandbox-llm-models` and downloads the models that are missing
(Ollama's progress is shown; `up --no-pull` skips it). `down`, `reset-home`, `down -v` and engine updates never touch
the volume. Use `sandbox llm rm <model>` to delete a model; it asks first, because every project shares the volume.

| Command | |
|---|---|
| `sandbox llm status` | Model, source file, GPU, endpoint, installed models (`ollama list`) and what is loaded (`ollama ps`: the GPU/CPU split) |
| `sandbox llm pull [model]` | Download the project's models, or one model |
| `sandbox llm rm <model>` | Delete a model from the shared volume (`--yes` skips the question) |
| `sandbox llm usage` | Requests, errors, input/output tokens and output tokens per second per day and model · `--days N`, `--json`, `--clear` |
| `sandbox info` | An "LLM" row: model, the file it comes from, GPU, endpoint (or `off`) |

**In the sandbox.** The endpoint is `http://llm:11434`, set as `SANDBOX_LLM_URL` (and `OLLAMA_HOST`) with the model in
`SANDBOX_LLM_MODEL` and its context length in `SANDBOX_LLM_CONTEXT`. The host name `llm` bypasses the proxy; nothing else about the allowlist changes. Run
`sandbox-llm` there for the settings to paste into your tools:

- **Claude Code**: `sandbox-llm claude` starts it against the local model (Anthropic API, `/v1/messages`). It sets the
  base URL, a dummy token and the model for the main, small/fast (haiku) and sub-agent roles, so that nothing asks for a
  cloud model. It also sets `CLAUDE_CODE_MAX_CONTEXT_TOKENS` to `llm.context`: Claude Code doesn't know local models
  and would otherwise warn "Model ... isn't recognized · context assumed to be 200k tokens" and compact too late. A plain `claude` keeps your cloud login; both can run side by side.
- **OpenCode**: an OpenAI-compatible provider with `baseURL` `http://llm:11434/v1`; `sandbox-llm` prints the JSON.
  OpenCode can use the cloud models and this one side by side.
- **VS Code, GitHub Copilot Chat**: bring-your-own-model with the Ollama endpoint `http://llm:11434`.
- **Continue and others**: Ollama provider with `apiBase` `http://llm:11434`, or any OpenAI-compatible client with base
  URL `http://llm:11434/v1` and any API key. `sandbox-llm env` prints `export` lines for scripts.

`OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL` are deliberately **not** set for the whole desktop: that would send your cloud
logins to the local model.

**What the desktop can reach.** Only the gateway, which passes inference and read-only requests: `/`,
`/api/version|tags|show|ps|chat|generate|embed|embeddings`, `/v1/chat/completions|completions|embeddings|models`,
`/v1/messages` and `/v1/messages/count_tokens`. Everything else (`/api/pull`, `push`, `create`, `delete`, `copy`,
`/api/blobs`) gets `403` with a hint to use `sandbox llm pull` on the host. Otherwise the agent could have Ollama fetch
from hosts that the allowlist blocks. Ollama itself is on an internal network and has its own route to the model
registry only for the downloads that `sandbox` starts.

**Usage.** The gateway writes one JSON line per request (time, endpoint, status, duration, model, input/output tokens)
to a volume of the project; `sandbox llm usage` adds it up. Tokens come from the answer: Ollama's own API and the
OpenAI API without streaming always report them, the Anthropic API too. A streamed OpenAI answer reports them only if
the client asks for `stream_options.include_usage`; otherwise the counts are empty.

**Windows.** Docker Desktop with the WSL2 backend, a current NVIDIA driver for Windows (it provides the GPU inside WSL2;
don't install a Linux driver in the distribution) and no other setup. **Linux:** the NVIDIA driver and the NVIDIA Container
Toolkit (`nvidia-ctk runtime configure --runtime=docker`). If the GPU can't be used, `up` says so and points at
`gpu = "none"`. On Docker Desktop the models live in the WSL2 virtual disk, which grows with them and does not shrink
by itself after `llm rm`.

**Models for a 24 GB card (RTX 4090).** The default is a dense 27B model with the best tool-call reliability of the group
and room for a 64K context.

| Model (Ollama tag) | Size (Q4) | Notes |
|---|---|---|
| `qwen3.6:27b` (default) | ~17 GB | Dense, ~30–40 tokens/s, room for 64K context |
| `qwen3.6:35b-a3b` | ~22 GB | Mixture of experts, much faster, little VRAM left for context |
| `gpt-oss:20b` | ~14 GB | Clean tool calls, adjustable reasoning, also fits 16 GB cards |
| `devstral-small-2` | ~15 GB | Mistral's agentic coding model |
| `qwen3-coder:30b` | ~19 GB | Older, widely supported coding model |

Sizes and speeds are rough. Agent tools need a model with the *tools* capability: check with `ollama show <tag>` in the
Ollama container (`docker compose exec ollama ollama show <tag>`) before relying on a tag.

**Not verified yet.** Running on a GPU host: `ollama/ollama:0.40.16` and `nginx/nginx-unprivileged:1.31.8-alpine`
(in `compose.yml.j2` and `resources/llm/Dockerfile`, the gateway with the QuickJS engine) with `qwen3.6:27b` and Claude
Code. Not verified: the exact tags `qwen3.6:35b-a3b`, `devstral-small-2` and `qwen3-coder:30b`; the VS Code setting name `github.copilot.chat.byok.ollamaEndpoint` (Copilot's BYOK settings are still changing) and the
OpenCode provider JSON. Anthropic API support needs Ollama 0.14 or newer.

## Isolation

| | |
|---|---|
| Network | Only through the allowlist proxy. No direct route, no external DNS, HTTPS tunnels only to port 443 |
| Project | Mounted read-write (or read-only with `workspace = "ro"`) |
| `.sandbox/` | **Read-only inside the sandbox**, `.state/` included. The agent can read its config but not widen its allowlist or tools |
| `.git/config`, `.git/hooks` | Read-only (`protect_git`). Prevents planting hooks or `core.hooksPath` that would run on your host. Commits still work. Commands that write git config (`git remote add`, `push -u`) must be run on the host |
| Home | A Docker volume per project, not a host folder |
| UI | `127.0.0.1` only. Optional basic auth via `auth` under `[desktop]` |
| Root | Passwordless sudo by default. `sudo = false` under `[desktop]` removes it |
| Proxy / gateway | Run as `nobody`, read-only filesystem, no capabilities |

Still your responsibility: files the agent writes in the project, such as build scripts, `package.json` scripts or
`.vscode/tasks.json`, run on your host if you execute them there. Review changes before running them outside the sandbox.

## Claude Desktop notes (Linux beta)

- It starts with `--no-sandbox` (Electron needs user namespaces, which Docker blocks; the container is the sandbox) and with an explicit proxy flag.
- Cowork (the VM tab) needs KVM and doesn't work in a container. Chat and Code work.
- Sign-in is stored in KDE Wallet. If the wallet isn't available, you may need to sign in again after a restart.
- If Claude Desktop needs a host that isn't listed, check `./sandbox blocked`.

## Troubleshooting

- **Page blank or error on first open** → `up` waits for KDE, but if the desktop is slow it warns and opens anyway. Reload the page.
- **"Docker is not running"** → start Docker Desktop.
- **"cargo not found"** or a build error from the launcher → install a Rust toolchain (`cargo --version` must show 1.88 or newer).
- **"Could not find the .sandbox folder"** → run it through the launchers, or from inside a project that has a `.sandbox` folder.
- **"Another project already uses the container group"** → two projects have the same folder name. Set `name` under `[project]`.
- **GUI apps crash** → under `[desktop]`, set `security_opt = ["seccomp=unconfined"]`.
- **GPU (Intel/AMD, Linux)** → under `[desktop]`, set `devices = ["/dev/dri:/dev/dri"]`.
- **"... was edited by hand and has been restored"** → the files in `.state/desktop/`, `.state/proxy/` and `.state/llm/` are generated. Change `sandbox.toml` instead.
- **".sandbox/engine/ is no longer used"** → see "Migrating from the old layout" above.
- **"sandbox.yml is no longer read"** → the settings moved to `sandbox.toml`; the message shows where each old key goes.
