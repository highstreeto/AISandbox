# Plan: `sandbox.toml` + templated compose file

Replace `.sandbox/sandbox.yml` (Compose syntax, merged over `engine/compose.yml`) with a small, purpose-built
`sandbox.toml`. The final `.state/compose.yml` is rendered from a template instead of merged from layers.

## Why

- `sandbox.yml` exposes Compose internals: you have to know that tools live under
  `services.desktop.build.args.INSTALL_*`, that `x-sandbox` is special, that `/shares` needs a tmpfs placeholder.
- The merge engine (`sandbox.compose.cs`, ~400 lines) re-implements Compose's merge rules, plus `!reset`/`!override`,
  anchors and short forms — and the README spends a paragraph explaining them.
- "Can't be overridden" (UI port, project mounts, `.sandbox` ro mount) is enforced by layer order and `!override`
  tricks. With a template it's structural: the user's file simply has no way to reach those keys.

## Target: `.sandbox/sandbox.toml`

```toml
# Sandbox settings for this project — commit this file.
# Apply: .sandbox/sandbox up        Tools changed? .sandbox/sandbox rebuild

[project]
# name = "myproject"        # container group name (default: project folder name)
workspace = "rw"            # "rw" | "ro"  access to the project folder
protect_git = true          # .git/config + .git/hooks read-only
# ui_port = 3001            # fixed localhost port (default: random free port)

[tools]                     # true = default version, false = skip, "x" = version, [..] = list
claude      = true          # Claude Desktop app (beta on Linux)
claude_code = false         # Claude Code CLI: true | "latest"
dotnet      = "10.0"        # ["10.0", "8.0"] for both
node        = false         # 24
bun         = true
vscode      = true
jetbrains   = false         # ["toolbox", "rider", "idea"]
apt         = []            # ["build-essential", "python3-pip"]

[desktop]
title  = "AI Sandbox"
sudo   = true               # false = no root inside
memory = "8g"
# cpus = 4
# timezone = "Europe/Vienna"            # default: taken from the host
# devices = ["/dev/dri:/dev/dri"]       # GPU (Intel/AMD, Linux)
# security_opt = ["seccomp=unconfined"] # if GUI apps crash
# auth = { user = "me", password = "secret" }   # basic auth for the web desktop

[env]                       # extra environment variables in the desktop
# MY_VAR = "value"

# Extra folders. Revoke = remove the block + "sandbox up".
# [[share]]
# host = "../reference-docs"   # relative to the project folder
# path = "/shares/docs"        # default: /shares/<folder name>
# readonly = true              # default: true
```

Rules:

- **Tools** map 1:1 to Dockerfile args: `claude_code` → `INSTALL_CLAUDE_CODE`. `true`/`false` → `"true"`/`"false"`,
  numbers → string, arrays → space-joined. Known tools are read from the `ARG INSTALL_*` lines of
  `engine/desktop/Dockerfile`, so an unknown key (`[tools] nodejs = 24`) is an error listing the valid names, and a
  new tool needs only a `tools/<x>.sh` + `ARG`/`RUN` line — no C# change.
- **Unknown keys/tables anywhere are errors** (`sandbox.toml: unknown key 'desktop.mem' — known: title, sudo, memory, …`).
  Wrong types too (`project.workspace must be "rw" or "ro"`). TOML syntax errors keep line/column.
- **`[env]` can't touch reserved names**: `HTTP(S)_PROXY`, `NO_PROXY` (any case), `SANDBOX_WORKDIR`, `PUID`, `PGID`
  → error. `TZ` goes through `desktop.timezone`. `TITLE`, `SANDBOX_SUDO`, `CUSTOM_USER`, `PASSWORD` go through `[desktop]`.
- **Shares**: host path must exist (clear error otherwise); `path` must be absolute and not inside the project mount.
  Relative `host` paths are resolved against the **project folder** (today: the `.sandbox` folder).
  A `/shares` tmpfs is no longer needed as a placeholder.
- **No local override file.** `sandbox.local.yml` support is dropped; there is no `sandbox.local.toml`. It can be added
  later (same schema, deep-merged before validation) if a need shows up.
- **No escape hatch** for arbitrary Compose keys: only typed keys. New needs get a new typed key.

## Generation: template instead of merge

`engine/compose.yml` becomes **`engine/compose.yml.sbn`**, a [Scriban](https://github.com/scriban/scriban) template
(same author as Tomlyn, both pure .NET, single NuGet package each). It keeps the comments/diagram of today's engine file
and contains every service, network and hardening setting literally; only the variable parts are expressions:

```yaml
  desktop:
    build:
      context: ./engine/desktop
      args:
{{~ for t in tools }}
        {{ t.arg }}: {{ t.value | q }}
{{~ end }}
    environment:
      TITLE: {{ desktop.title | q }}
      SANDBOX_SUDO: {{ desktop.sudo | q }}
      PUID: {{ host.uid | q }}
      ...
{{~ for e in env }}
      {{ e.key | q }}: {{ e.value | q }}
{{~ end }}
    volumes:
      - home:/config
      - { type: bind, source: {{ host.project_dir | q }}, target: {{ host.workdir | q }}, read_only: {{ project.readonly }} }
{{~ for s in shares }}
      - { type: bind, source: {{ s.host | q }}, target: {{ s.path | q }}, read_only: {{ s.readonly }} }
{{~ end }}
    mem_limit: {{ desktop.memory | q }}
{{~ if desktop.cpus }}
    cpus: {{ desktop.cpus }}
{{~ end }}
  gateway:
    ports: [ {{ "127.0.0.1:" + host.ui_port + ":3001" | q }} ]
```

- **One escaping function `q`**: emits a JSON string literal (valid YAML double-quoted scalar). Every user/host value
  goes through it, so a path like `C:\Users\x` or a value containing `: #` can't break or inject YAML.
- The template model is a plain C# object built from validated settings + host facts (uid/gid, tz, workdir, id,
  project/.sandbox/.git mounts, ui port). Scriban runs with strict variables, so a template typo fails loudly.
- Output is still `.state/compose.yml`, regenerated before each docker command, still the only `-f` file.
- Template lives in `engine/` because it's engine-owned; users never edit it.

## Code changes

| File | Change |
|---|---|
| `.sandbox/sandbox.toml` | **New** — replaces `sandbox.yml` (content above) |
| `.sandbox/sandbox.yml` | **Delete** |
| `.sandbox/engine/compose.yml` | **→ `compose.yml.sbn`** (template) |
| `.sandbox/sandbox.config.cs` | Rewrite: Tomlyn parse (syntax errors with line/col) → validate unknown keys/types/reserved env → typed `Settings` (project, tools, desktop, env, shares). Read known tools from the Dockerfile |
| `.sandbox/sandbox.compose.cs` | **Delete** (ComposeMerger) — or reuse the name for the template rendering + `q` function (~60 lines) |
| `.sandbox/sandbox.project.cs` | `Generate()` renders the template; drop layers, `HostDefaults`/`HostSettings` YAML nodes and the `Y` helper; `LocateSandboxDir` looks for `sandbox.toml`; `ConfPath` → `sandbox.toml`, `LocalConfPath` removed |
| `.sandbox/sandbox.cs` | Packages: `-YamlDotNet`, `+Tomlyn`, `+Scriban` (pinned); `#:include` list; `YamlException` catch → `TomlException`; help texts (`rebuild`: "after changing [tools]"); `Info` shows tools/desktop/env/shares from the new model, "Local overrides" row removed |
| `.sandbox/.gitignore` | Drop `sandbox.local.yml` |
| `.sandbox/engine/desktop/Dockerfile` | Comments: "INSTALL_* build args come from [tools] in sandbox.toml" |
| `.sandbox/README.md` | Rewrite "Configure" section (much shorter — no merge rules); troubleshooting GPU/seccomp use `[desktop]` keys |
| `tests/Sandbox.Tests/*` | Fixture writes `sandbox.toml` (no local file). `CommandTests`: drop the `sandbox.local.yml` tests. `ComposeFileTests`: drop merge-rule tests, add one test per setting + exact-text golden file + "output parses as YAML" + escaping (backslashes, quotes, `#`, `:`). `ErrorTests`: TOML syntax error line/col, unknown key, unknown tool, wrong type, reserved env var, missing share folder, leftover `sandbox.yml` |

**Old file detection:** if `sandbox.yml` exists and `sandbox.toml` doesn't, fail with a message that shows the
key mapping (`x-sandbox.*` → `[project]`, `INSTALL_X` → `[tools] x`, `environment` → `[desktop]`/`[env]`,
`volumes` → `[[share]]`). If both exist, warn that `sandbox.yml` is ignored. A leftover `sandbox.local.yml` gets
a warning that it is no longer read (move its settings into `sandbox.toml`).

## Steps

1. Add `sandbox.toml` + new `Settings` model + `Config` reader with validation (tests for parsing/errors first).
2. Write `compose.yml.sbn`, the `q` filter and `Generate()`; golden-file test that the rendered output equals today's
   `.state/compose.yml` for the default settings (modulo the removed `/shares` tmpfs and key order) — this proves parity.
3. Delete `ComposeMerger`, `sandbox.yml`, YamlDotNet; update `Info`, help texts, error handling.
4. README, Dockerfile comments, `.gitignore`, old-file detection.
5. Full `dotnet test tests/Sandbox.Tests`, then a real `sandbox up` / `rebuild` / `info` on Windows and Linux.

## Decisions

1. **Escape hatch for arbitrary Compose keys:** none. Typed keys only.
2. **Share paths:** relative to the project folder.
3. **Migration:** error message with the key mapping; no `migrate` command, YamlDotNet removed.
4. **Local overrides:** dropped for now (no `sandbox.local.toml`); add later if needed.
5. **Later, optional:** CLI edits that keep comments (Tomlyn's syntax tree preserves them), e.g.
   `sandbox tool node 24`, `sandbox share add ../docs`.
