# AI Sandbox — a Kubuntu desktop for this project

A KDE Plasma desktop (LinuxServer.io WebTop) in your browser, set up for this project.
It can only reach hosts you allow and only see the folders you share.
Copy this `.sandbox/` folder into any project root and commit it. Anyone who clones the
project gets the same ready-to-develop environment.

```
 browser ──► 127.0.0.1:<port> ──► gateway ──► desktop (KDE + your tools)
             (localhost only)                    │ internal network: no internet route, no external DNS
                                                 ▼
                                               proxy (tinyproxy + allowlist.txt) ──► internet
```

## Start / stop

Run these from anywhere. They don't change your current directory.

```bash
.sandbox/sandbox up          # Linux / macOS / WSL / Git Bash (sh)
.sandbox\sandbox.cmd up      # Windows (cmd or PowerShell)
.sandbox/sandbox help up     # help for one command (or: up --help)
```

Requires Docker and the **.NET 10 SDK** (10.0.400 or newer, for `#:include`) on the host. The control logic is a
[.NET file-based app](https://learn.microsoft.com/dotnet/core/sdk/file-based-apps). The entry point is `sandbox.cs`, which holds the commands.
It pulls in the `sandbox.*.cs` files next to it (project, compose, config, allowlist, proc, ui) with `#:include`. The two files above are
one-line launchers that run `dotnet run --file sandbox.cs -- <args>`, so every argument reaches the app unchanged.
The first run (and the first one after `sandbox.cs` changes) restores packages and compiles, which takes a few seconds. Later runs use the cache.

`up` first checks that Docker is running. It then builds the image (several minutes the first time), waits until the KDE desktop is
actually ready and opens it in your browser. Accept the self-signed certificate.

| Command | |
|---|---|
| `up` / `stop` / `restart` / `down` | Start · stop (all kept) · restart · remove containers (home + project kept) |
| `status` / `open` | Status + URL · open the desktop |
| `shell` / `claude` | Bash / Claude Code CLI inside the sandbox, in the project folder |
| `rebuild` / `update` | After changing tools · newest versions of everything |
| `reset-home` | Wipe the persistent home (logins, IDE settings) · `--yes` skips the question |
| `info` | Project identity, mount path, settings, tools and shared folders as read from `sandbox.toml` |
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

**How it's used.** The app renders `engine/compose.yml.sbn` (a [Scriban](https://github.com/scriban/scriban) template)
with these settings and this machine's facts (user ids, time zone, mounts, UI port) into `.state/compose.yml`, and runs
every `docker compose` command with only that file. It's regenerated before each command. Look at it to see what will
actually run. The UI port, the project mounts, the `.sandbox` read-only mount and the proxy settings are written by
the template itself, so nothing in `sandbox.toml` can change them.

## Network: `allowlist.txt`

One host per line, with wildcards like `*.example.com`. Everything else is blocked.

| | |
|---|---|
| `.sandbox/sandbox blocked` | Which hosts were blocked (with counts, and whether they're allowed by now) |
| `.sandbox/sandbox allow api.nuget.org` | Allow (uncomments an existing entry) and reload in about 1 s |
| `.sandbox/sandbox deny api.nuget.org` | Comment out and reload |
| `.sandbox/sandbox allowlist -a` | Allowed hosts by group, plus the commented-out ones ready to enable |

The file includes ready-made, commented-out groups for NuGet, npm, pip, Gradle/Maven, apt, VS Code marketplace,
JetBrains, and a local LLM on the host (`host.docker.internal`).

## Isolation

| | |
|---|---|
| Network | Only through the allowlist proxy. No direct route, no external DNS, HTTPS tunnels only to port 443 |
| Project | Mounted read-write (or read-only with `workspace = "ro"`) |
| `.sandbox/` | **Read-only inside the sandbox.** The agent can read its config but not widen its allowlist or tools |
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
- If Claude Desktop needs a host that isn't listed, check `.sandbox/sandbox blocked`.

## Troubleshooting

- **Page blank or error on first open** → `up` waits for KDE, but if the desktop is slow it warns and opens anyway. Reload the page.
- **"Docker is not running"** → start Docker Desktop.
- **"dotnet not found"** or a build error from `sandbox.cs` → install the .NET 10 SDK (`dotnet --version` must show 10.x).
- **"Another project already uses the container group"** → two projects have the same folder name. Set `name` under `[project]`.
- **GUI apps crash** → under `[desktop]`, set `security_opt = ["seccomp=unconfined"]`.
- **GPU (Intel/AMD, Linux)** → under `[desktop]`, set `devices = ["/dev/dri:/dev/dri"]`.
- **"sandbox.yml is no longer read"** → the settings moved to `sandbox.toml`; the message shows where each old key goes.
