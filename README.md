# AI Sandbox — a Kubuntu desktop for this project

A KDE Plasma desktop (LinuxServer.io WebTop) in your browser, set up for this project.
It can only reach hosts you allow and only see the folders you share.
`.sandbox/` holds the project's two files, `sandbox.toml` (settings) and `allowlist.txt` (network), and is committed
with the project. Anyone who clones it gets the same ready-to-develop environment. The engine (compose template,
Dockerfiles, tool scripts, proxy config) is part of the control app, in `src/AiSandbox` of this repository (later a
.NET tool).

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

Creates `<folder>/.sandbox/` with `sandbox.toml` and `allowlist.txt` from templates, then the generated `.state/`
(see below). Files that already exist are kept and listed (`sandbox.toml exists — kept`), so it is safe to run again and
only fills in what is missing. There is no `--force`: to start over, delete the file. It needs no Docker and ignores
`SANDBOX_DIR`. If a parent folder already has a `.sandbox/`, it warns and creates a separate sandbox for the sub-folder.
Next: edit `.sandbox/sandbox.toml`, then run `up`.

## Start / stop

The launchers are in the repository root. They work from any directory and don't change your current directory.

```bash
./sandbox up                 # Linux / macOS / WSL / Git Bash (sh)
sandbox.cmd up               # Windows (cmd or PowerShell)
./sandbox help up            # help for one command (or: up --help)
```

Requires Docker and the **.NET 10 SDK** on the host. The control app is a normal .NET console project,
`src/AiSandbox` (solution: `AiSandbox.slnx`). The launchers run `dotnet run --project src/AiSandbox -- <args>` with
`SANDBOX_DIR` set to the `.sandbox` folder next to them, so every argument reaches the app unchanged.
The first run (and the first one after the sources change) restores packages and compiles, which takes a few seconds.

Without `SANDBOX_DIR` the app searches upwards from the current directory for a `.sandbox` folder. That is how another
project uses it for now: from inside that project, run `dotnet run --project <path to AiSandbox>/src/AiSandbox -- up`.

`up` first checks that Docker is running. It then builds the image (several minutes the first time), waits until the KDE desktop is
actually ready and opens it in your browser. Accept the self-signed certificate.

| Command | |
|---|---|
| `up` / `stop` / `restart` / `down` | Start · stop (all kept) · restart · remove containers (home + project kept) |
| `status` / `open` | Status + URL · open the desktop |
| `shell` / `claude` | Bash / Claude Code CLI inside the sandbox, in the project folder |
| `rebuild` / `update` | After changing tools · newest versions of everything |
| `reset-home` | Wipe the persistent home (logins, IDE settings) · `--yes` skips the question |
| `init [folder]` | Create `.sandbox/` (`sandbox.toml`, `allowlist.txt`) in a project folder |
| `info` | Project identity, mount path, engine version, settings, tools and shared folders as read from `sandbox.toml` |
| `logs [service]` | Follow logs · `--tail <n>`, `--no-follow` |

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

**Checked.** Unknown sections, keys and tools are errors that list the valid names, and so are wrong types and invalid
values (`workspace = "rx"`, a share folder that doesn't exist, broken TOML with its line and column).
Environment variables the sandbox sets itself (`HTTP_PROXY`, `NO_PROXY`, `PUID`, `PGID`, `SANDBOX_WORKDIR`) can't be
set in `[env]`; `TZ`, `TITLE`, `SANDBOX_SUDO`, `CUSTOM_USER`/`PASSWORD` are set through `[desktop]` instead.

**How it's used.** The app renders its embedded `compose.yml.sbn` (a [Scriban](https://github.com/scriban/scriban)
template) with these settings and this machine's facts (user ids, time zone, mounts, UI port) into `.state/compose.yml`,
and runs every `docker compose` command with only that file. It's regenerated before each command. Look at it to see
what will actually run. The UI port, the project mounts, the `.sandbox` read-only mount and the proxy settings are
written by the template itself, so nothing in `sandbox.toml` can change them.

## What is in `.sandbox/`

```
.sandbox/
  sandbox.toml          yours: settings                      (commit)
  allowlist.txt         yours: network allowlist             (commit)
  .state/               generated, ignores itself (its own .gitignore holds "*")
    id                  stable project id
    .manifest           what the app wrote last time: file hashes, engine hash, app version
    compose.yml         the one file docker compose runs with
    desktop/            build folder of the desktop image, exactly as used
    proxy/              build folder of the proxy/gateway image
```

**The engine files are generated.** The Dockerfiles, the tool scripts, the startup script and the proxy config are
embedded in the app and written to `.state/desktop/` and `.state/proxy/` before every command that uses Docker. They
are there so you can see what is built; they can't be edited. A file you change by hand is restored on the next
command, with a warning (`.state/desktop/Dockerfile was edited by hand and has been restored.`). Files an older app
version left behind in these two folders are removed; nothing else in `.state/` or outside it is touched. Every file
that allows comments starts with `Generated by sandbox <version> — changes are overwritten. Settings: sandbox.toml.`

To change a sandbox, use the keys in `sandbox.toml`: the built-in `[tools]`, `apt = [...]` for extra Ubuntu packages,
`[env]`, `[desktop]`. If you need a different engine, fork this repository and build the app with your own files in
`src/AiSandbox/Resources`.

**After updating the app.** The engine may have changed. `up` builds the images every time, and Docker's build cache
redoes only the layers whose files or `[tools]` changed, so the next `up` picks the change up. `sandbox info` shows the
app version and the engine hash.

**Migrating from the old layout.** Earlier versions copied the engine into each project as `.sandbox/engine/`. That
folder is no longer used. The app warns about it on every command but never deletes it, because it is committed and may
hold your own edits. Once you have checked that nothing in it is needed, delete `.sandbox/engine/`,
`.sandbox/README.md`, `.sandbox/.gitattributes` and `.sandbox/.gitignore` (the `.state/` folder ignores itself).

## Network: `allowlist.txt`

One host per line, with wildcards like `*.example.com`. Everything else is blocked.

| | |
|---|---|
| `./sandbox blocked` | Which hosts were blocked (with counts, and whether they're allowed by now) |
| `./sandbox allow api.nuget.org` | Allow (uncomments an existing entry) and reload in about 1 s |
| `./sandbox deny api.nuget.org` | Comment out and reload |
| `./sandbox allowlist -a` | Allowed hosts by group, plus the commented-out ones ready to enable |

The file includes ready-made, commented-out groups for NuGet, npm, pip, Gradle/Maven, apt, VS Code marketplace,
JetBrains, and a local LLM on the host (`host.docker.internal`).

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
- **"dotnet not found"** or a build error from `src/AiSandbox` → install the .NET 10 SDK (`dotnet --version` must show 10.x).
- **"Could not find the .sandbox folder"** → run it through the launchers, or from inside a project that has a `.sandbox` folder.
- **"Another project already uses the container group"** → two projects have the same folder name. Set `name` under `[project]`.
- **GUI apps crash** → under `[desktop]`, set `security_opt = ["seccomp=unconfined"]`.
- **GPU (Intel/AMD, Linux)** → under `[desktop]`, set `devices = ["/dev/dri:/dev/dri"]`.
- **"... was edited by hand and has been restored"** → the files in `.state/desktop/` and `.state/proxy/` are generated. Change `sandbox.toml` instead.
- **".sandbox/engine/ is no longer used"** → see "Migrating from the old layout" above.
- **"sandbox.yml is no longer read"** → the settings moved to `sandbox.toml`; the message shows where each old key goes.
