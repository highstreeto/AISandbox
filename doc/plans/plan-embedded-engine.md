# Plan: the engine inside the app, `sandbox init`

> **Status:** implemented (steps 1-6); step 7 (smoke test against real Docker) and applying `apply-on-host.ps1` are left for the host. See *As built* at the end.
> Follows [plan-console-project.md](plan-console-project.md) (*Out of scope / later*: "engine/ should move into the
> tool as embedded resources"). Prepares packaging as a .NET tool (not part of this plan).
> Project-specific tools (extending the desktop image) are **not** part of this change. They get their own plan later,
> see *Later*.

Today every project carries a copy of the engine in `.sandbox/engine/` (compose template, both Dockerfiles, tool
scripts, proxy config, init scripts) next to its own settings. Move the engine into the app as embedded resources.
The app writes it out on every command into `.sandbox/.state/desktop/` and `.sandbox/.state/proxy/`, the two build
folders, next to the `compose.yml` that is already generated there. A new `sandbox init` creates `.sandbox/` in a
project with only the two files the project owns.

## Why

- A copied engine goes stale: a fix in the engine never reaches projects that already have a copy, and nobody can tell
  which version a project runs.
- `.sandbox/` mixes project settings (edit and commit them) with engine files (don't touch them). People only need to
  see two files: `sandbox.toml` and `allowlist.txt`.
- A .NET tool can't rely on files sitting next to the project. It has to bring its own engine.

## Answers to the open questions

### 1. Can people modify the Dockerfiles?

**No, the engine files can't be edited.**

The engine files hold the security setup: the internal network, the proxy settings for apt/sudo/KDE, hardening, the
sudo policy, the startup script. If a project could edit them, every engine update would have to merge with the edits,
and nobody could say what a given sandbox runs. Projects change the sandbox through the typed keys in `sandbox.toml`,
as before. What's covered today: the built-in `[tools]`, `apt = [...]` for extra Ubuntu packages, `[env]`, `[desktop]`.
A real extension point for the image (project tools) comes in a later change.

If someone edits a generated file by hand anyway, the next command restores it and warns (see question 4).

The escape hatch is a fork: build the app from source with your own engine. The README says so in one sentence.

### 2. Where is the engine written: `.sandbox/` or a temp folder?

**`.sandbox/.state/` (`desktop/`, `proxy/`).** It is inside the project, git-ignored, and rewritten before every command.

| | `.sandbox/.state/` | Temp folder (`/tmp/…`, `%TEMP%`) |
|---|---|---|
| Visible | Yes, next to `.state/compose.yml`, which already shows what runs | Hidden. People have to know the path |
| Protection from the agent | Yes: `.sandbox/` is mounted **read-only** inside the desktop, `.state/` included | Not mounted at all. The same result |
| Protection on the host | None. A process running as you can write anywhere you can, `%TEMP%` too | None |
| Stable while running | Yes | No: `systemd-tmpfiles`, Windows disk cleanup or a reboot can delete it while containers run |
| Docker Desktop file sharing | The project is shared anyway (it's mounted) | macOS/Windows: temp paths may not be shared, so builds and mounts fail |
| Several projects | Separate, one per project | Needs a per-project subfolder and cleanup |

A temp folder only looks safer. The real protection is that the agent can't write to it, and `.state/` already gets
that from the read-only `.sandbox` mount. On the host, regenerating before every command is the protection (see
question 4).

### 3. `.sandbox/` holds only the TOML and the allowlist. How do people see the files that are used?

After this change:

```
.sandbox/
  sandbox.toml          yours: settings                      (committed)
  allowlist.txt         yours: network allowlist             (committed)
  .state/               generated, git-ignored via .state/.gitignore ("*")
    .gitignore
    id
    .manifest           what the app wrote last time: file hashes, engine hash, app version
    compose.yml         the one file docker compose runs with (as today)
    desktop/            build folder of the desktop image, exactly as used
      Dockerfile
      bin/sandbox-install
      tools/…           built-in tool scripts
      root/…            files copied into the image (startup script, proxy settings)
    proxy/              build folder of the proxy/gateway image
      Dockerfile, tinyproxy.conf
```

- There is no `engine/` level: the build folders sit directly in `.state/`, next to the `compose.yml` that uses them.
- `.sandbox/.gitignore`, `.gitattributes`, `README.md` and `engine/` are gone. `.state/` ignores itself with its own
  `.gitignore` containing `*`, as `.venv/` and `.pytest_cache/` do. The project's `.gitignore` needs no entry.
- `compose.yml` points at `./.state/desktop` and `./.state/proxy`. So the files docker compose and docker
  build use are exactly the ones in `.state/`, nothing hidden.
- Every generated file that allows comments starts with a header:
  `Generated by sandbox <version> — changes are overwritten. Settings: sandbox.toml.`
  This covers both Dockerfiles, `compose.yml` and `tinyproxy.conf`. Shell scripts get it as a comment after the shebang.
  The header is added by the sync, not stored in the resources, so the version is always right.
- `sandbox info` gets an **Engine** row: `sandbox 0.1.0+3f2a9c1, engine 9b1e04c7d2aa → .sandbox/.state/desktop, proxy (rewritten before each command)`.
- The compose template (`compose.yml.sbn`) is **not** written out. It is the app's source, not something docker reads.
  The rendered `compose.yml` is what counts.
- The user docs (today `.sandbox/README.md`) move to a root `README.md` in this repo. The `sandbox.toml` header
  links to it.

### 4. What happens to old files: are they overwritten?

**Yes. `.state/desktop/` and `.state/proxy/` belong to the app and are synced before every command.** The steps:

1. Build the expected set: every embedded file under `desktop/` and `proxy/`, with the header added.
2. Read `.state/.manifest` (the hash of each file as the app last wrote it).
3. For each expected file:
   - missing → write it;
   - on disk == expected → do nothing (no write, so the mtime stays);
   - on disk == manifest hash ≠ expected → the app was updated: overwrite it silently;
   - on disk ≠ manifest hash → **edited by hand**: overwrite it and warn
     `.state/desktop/Dockerfile was edited by hand and has been restored. Generated files are overwritten; change sandbox.toml instead.`
4. Delete files and empty folders in `.state/desktop/` and `.state/proxy/` that aren't in the expected set. These are
   left over from an older app version. Only these two folders are cleaned: `id`, `compose.yml`, `built` and anything
   else in `.state/` are never deleted by the sync.
5. Write the new manifest last. If the app crashes halfway, the next run repairs everything, because steps 1–4 are idempotent.

Nothing outside `.state/` is ever overwritten or deleted.

**Image out of date.** Nothing extra needed: `up` runs `docker compose build` every time, and Docker's build cache
redoes only the layers whose files or build args changed. An engine change from an app update is therefore picked up
by the next `up`. (An earlier version of this plan had an "image out of date" hint with a `.state/built` hash; it was
built and then removed, because `up` already rebuilds.)

**Old layout (existing projects).** If `.sandbox/engine/` exists: warn once per command
`.sandbox/engine/ is no longer used — the engine is built into the app (see .sandbox/.state/desktop/ and proxy/). Delete it,
together with .sandbox/README.md, .gitattributes and .gitignore.` Don't delete these files: they are committed and may
contain the project's own edits. Finding `.sandbox` no longer requires `engine/`, only `sandbox.toml` (or the old
`sandbox.yml` for its error message).

### 5. How do people add tools to the desktop image?

**Not in this change.** For now: the built-in `[tools]`, `apt = [...]`, and `sandbox-install` at run time (lost on
rebuild). Project tools get their own plan, see *Later*.

## `sandbox init`

```
sandbox init [folder]      default: the current directory
```

- Target: `<folder>/.sandbox/` (`SANDBOX_DIR` is ignored for `init`, because it names an existing sandbox). The folder
  must exist.
- Creates `sandbox.toml` and `allowlist.txt` from embedded templates. Files that already exist are **skipped** and
  listed (`sandbox.toml exists — kept`), so `init` is safe to run again and only fills in what's missing. No `--force`:
  to start over, delete the file.
- Creates `.state/` (with `.gitignore`, `id`) and writes the engine, so the files can be inspected right away.
- If a parent folder already has a `.sandbox/`: warn (`/repo/.sandbox already exists; this creates a separate sandbox
  for /repo/sub`) and go on. Nested projects in a monorepo are allowed.
- If the target has the old layout (`engine/`), the usual old-layout warning applies.
- It doesn't need Docker (`docker: false`).
- Output: the created files, then `Next: edit .sandbox/sandbox.toml, then "sandbox up".`
- The `sandbox.toml` template is the one from the repo, with fixed comments: the commands in its header become
  `sandbox up` / `sandbox rebuild` / `sandbox info` (today's header says `.sandbox/sandbox up`, which no longer
  exists), and it gets a link to the README.

## Resources in the app

```
src/AiSandbox/Resources/
  compose.yml.sbn                 → rendered to .state/compose.yml (not written out)
  desktop/Dockerfile              → copied to .state/desktop/
  desktop/bin/sandbox-install     → copied to .state/desktop/bin/
  desktop/tools/*.sh              → copied to .state/desktop/tools/
  desktop/root/**                 → copied to .state/desktop/root/
  proxy/Dockerfile                → copied to .state/proxy/
  proxy/tinyproxy.conf            → copied to .state/proxy/
  init/
    sandbox.toml                  → sandbox init
    allowlist.txt                 → sandbox init
```

- `AiSandbox.csproj`: `<EmbeddedResource Include="Resources\**" LogicalName="$([System.String]::Copy('%(RecursiveDir)%(Filename)%(Extension)').Replace('\','/'))" />`
  (or a plain `Resources/` prefix strip in code; it must give the same names on Windows and Linux). One class,
  `Templating/EmbeddedFiles`, lists and reads them (`Names(prefix)`, `ReadText(name)`, `ReadBytes(name)`). Nothing else
  touches `GetManifestResourceStream`.
- Everything under `desktop/` and `proxy/` is copied as-is (plus the header). The desktop Dockerfile stays a plain
  file, not a template. The root `.gitattributes` gets `src/AiSandbox/Resources/** text eol=lf`, so the files are LF in
  the assembly no matter how the repo was checked out.
- **Known tools** are still the `ARG INSTALL_*` lines of the desktop Dockerfile, now read from the embedded resource
  instead of a file path. `Config.Read` takes the Dockerfile text (or the list of names) instead of a path.
- `ComposeTemplate.Render` takes the template **text** and a name for error messages, not a path.
- **Versions: no hand-maintained engine counter.** The sync works on hashes only; a version is just a label.
  `.state/.manifest` (JSON) holds:
  - `files`: SHA-256 of every written file (path → hash). This drives the sync: hand edits and leftovers.
  - `engine`: one SHA-256 over all embedded `desktop/` + `proxy/` resources (sorted names + contents), computed at
    startup. Changes exactly when an engine file changes. Shown in `sandbox info` (first 12 chars).
  - `app`: `AssemblyInformationalVersion`, for people (header, `info`, bug reports). `<Version>` in
    `Directory.Build.props` (starting at `0.1.0`) is bumped by hand only for releases; the SDK appends
    `+<commit sha>` automatically for builds from git, so dev builds are traceable without bumping anything.

### Proxy config baked into the proxy image

`tinyproxy.conf` is engine-owned and static. `proxy/Dockerfile` copies it to `/etc/tinyproxy/tinyproxy.conf`, and
the compose `command` uses that path. The `.:/sandbox:ro` mount stays, because the allowlist must be live for
`sandbox reload`. Then the running proxy no longer depends on the files in `.state/proxy/`, and `tinyproxy.conf`
changes the same way as the other engine files (rebuild).

## Code changes

| File | Change |
|---|---|
| `src/AiSandbox/Resources/**` | **New**: engine files moved from `.sandbox/engine/` (`compose.yml.sbn`, `desktop/`, `proxy/`), `init/` templates |
| `src/AiSandbox/AiSandbox.csproj` | `EmbeddedResource` item |
| `Directory.Build.props` | `<Version>0.1.0</Version>` |
| `.gitattributes` | `src/AiSandbox/Resources/** text eol=lf` |
| `Templating/EmbeddedFiles.cs` | **New**: list/read resources |
| `Templating/ComposeTemplate.cs` | Render from text and a name |
| `Project/EngineFiles.cs` | **New**: `Sync(stateDir)`: the steps in question 4, header, manifest read/write, hand-edit warnings, stale-file removal |
| `Project/Sandbox.cs` | `TemplatePath` gone; `DesktopContext => .state/desktop`, `ProxyContext => .state/proxy`; `Load` reads known tools from the embedded Dockerfile |
| `Project/Sandbox.Generate.cs` | `Generate()` = write `.state/.gitignore` → `EngineFiles.Sync` → render `compose.yml`. Build contexts: `./.state/desktop`, `./.state/proxy` |
| `Project/Sandbox.Locate.cs` | `IsSandboxDir`: `sandbox.toml` or `sandbox.yml`, no `engine/`; old-layout warning |
| `Configuration/Config.cs` | `KnownTools` from the Dockerfile text instead of a path |
| `Commands/MaintenanceCommands.cs` | `Init`; `Info` shows the Engine row |
| `Cli/SandboxCommandLine.cs` | `init [folder]` command (`docker: false`, doesn't load a `Sandbox`) |
| `Resources/compose.yml.sbn` | `build:` contexts → `./.state/…`; proxy `command` → `/etc/tinyproxy/tinyproxy.conf`; header comments |
| `Resources/proxy/Dockerfile` | `COPY tinyproxy.conf /etc/tinyproxy/tinyproxy.conf` |
| `Resources/desktop/bin/sandbox-install` | Comment: `sandbox.yml` → `sandbox.toml` |
| `README.md` (root) | **New**: moved from `.sandbox/README.md` and updated. Layout of `.sandbox/`, `init`, "engine files are generated", migration from the old layout |
| `.sandbox/engine/`, `.sandbox/README.md`, `.sandbox/.gitignore`, `.sandbox/.gitattributes` | **Delete** (host patch, see below) |
| `.sandbox/sandbox.toml` | Header from the new init template (host patch) |

### Tests (`tests/AiSandbox.E2ETests`)

- `SandboxFixture`: stop copying `engine/`; `FindRepoSandboxDir` / `RepoSandboxDir` go away (`SANDBOX_TESTS_DIR`
  goes with them; update `tests/README.md`). Helpers: `DesktopContext`, `ProxyContext`.
- `ComposeFileTests`: the golden file changes (build contexts, proxy command). Update it once and check the diff by
  hand: only those lines change.
- **New `EngineTests`:** the engine is written on the first command (Dockerfile, sandbox-install, tools, root, proxy);
  `.state/.gitignore` is `*`; a second run writes nothing (mtimes stay the same); a file edited by hand is restored
  and the warning names it; a leftover file is removed; the manifest has the app version; the Dockerfiles have the
  header; an unknown `[tools]` key still lists the built-in tools.
- **New `InitTests`:** an empty folder → `sandbox.toml`, `allowlist.txt`, `.state/` created, exit 0, and `info` works
  afterwards; running it again → "kept", content unchanged; only `allowlist.txt` missing → only that one created;
  parent has `.sandbox` → warning, still created; a folder that doesn't exist → error; no docker on PATH → works.
- `ErrorTests`/`CommandTests`: `.sandbox` without `engine/` is found; old `engine/` folder → warning, command still
  works.
- `FakeDocker`: no change expected. Check that build calls still record their context path.

## Steps

1. **Resources + loader.** Copy `.sandbox/engine/desktop` and `proxy` to `src/AiSandbox/Resources/desktop` and
   `proxy`, and the template to `Resources/compose.yml.sbn`. Add `EmbeddedFiles`, the csproj item, `.gitattributes`,
   `<Version>`. Render compose from the embedded text; known tools from the embedded Dockerfile. The fixture still
   copies `engine/` here. Suite green.
2. **Sync to `.state/desktop` + `.state/proxy`.** `EngineFiles.Sync` with the header, manifest, hand-edit warning and
   stale removal. Build contexts and the proxy command change. `IsSandboxDir` without `engine/`, old-layout warning.
   Fixture stops copying `engine/`. Golden file updated. Add `EngineTests`. Suite green.
3. **`sandbox init`** + init templates + `InitTests`.
4. ~~Stale-image hint~~ (built, then removed: `up` always builds).
5. **Docs:** root `README.md`, `tests/README.md`, header comments, `sandbox-install` comment.
6. **Host patch** for `.sandbox/` (see below).
7. **Smoke test against real Docker, on the host:** `sandbox init` in an empty folder, `up`, edit
   `.state/desktop/Dockerfile` by hand → restored with a warning, `allow`/`reload` still work (allowlist mount),
   change `[tools]` → `up` rebuilds the changed layers, `down`. Linux and Windows.

Run `dotnet build` and `dotnet test` from the root after each step. Warnings are errors.

## Limits for whoever implements this

- **`.sandbox/` is read-only inside the sandbox.** Steps 1–5 only write under `src/`, `tests/`, `doc/` and the root.
  For step 6, write `embedded-engine-host.patch` (or a short `apply-on-host.sh`) in the repo root. It deletes
  `.sandbox/engine/`, `README.md`, `.gitignore`, `.gitattributes` and replaces the `sandbox.toml` header. Tell the
  user to apply it on the host, as was done for the earlier plans.
- Copy, don't `git mv`, in step 1 (`.sandbox/` can't be changed here). The host patch does the delete, so git sees the
  move as delete + add. That is fine.
- Don't change behaviour that isn't listed here. The existing assertions stay as they are, except the golden file and
  the tests that check the fixture layout.
- Add an *As built* section to this file at the end, listing any differences from the plan.

## Decisions

1. **Engine files can't be edited.** Hand edits are restored with a warning.
2. **Written to `.sandbox/.state/desktop/` and `proxy/`** (no `engine/` level), not a temp folder: visible, stable,
   already read-only for the agent.
3. **`.sandbox/` = `sandbox.toml` + `allowlist.txt`.** `.state/` ignores itself.
4. **Sync before every command**, with a manifest: changes from app updates are applied silently, hand edits with a
   warning, stale files are removed. Nothing outside `.state/` is touched.
5. **No extra rebuild logic.** `up` already builds every time; Docker's cache keeps that cheap.
6. **Old `.sandbox/engine/`: warn, don't delete.**
7. **`init` never overwrites** and has no `--force`.
8. **`tinyproxy.conf` goes into the proxy image.** Only the allowlist stays mounted.
9. **Project tools are a separate change.**

## Later

- **Project tools** (own plan): extending the desktop image per project. The idea so far is scripts in
  `.sandbox/tools/<name>.sh`, switched on in `[tools]` like the built-in ones and installed through `sandbox-install`.
  That would make the desktop Dockerfile a template and `_lib.sh` an interface projects depend on. This change keeps
  that path open: the name `.sandbox/tools/` isn't used for anything else, and the sync and manifest work on a list of
  expected files, so extra files from the project can be added to that list later.
- A `sandbox engine diff` command that compares a project's old `.sandbox/engine/` with the embedded one, to find
  local edits before deleting it.
- Packaging as a .NET tool (`PackAsTool`). After this plan, nothing on disk is needed besides the project's `.sandbox/`.

## As built

Steps 1-6 are done, in order, with `dotnet build` and `dotnet test` green after each. Test count: **115 before, 154 after**
(1 removed, 40 added: 28 in `EngineTests` incl. the stale-image hint and `info` row, 12 in `InitTests`; the
existing tests are unchanged apart from the points below). Step 7 (smoke test against real Docker, Linux and Windows) was not
done: no Docker here. `apply-on-host.ps1` (step 6) was written but not applied; it couldn't be run here (no PowerShell), so it was checked by reading only.

Differences from the plan:

- **`apply-on-host.ps1`** instead of a patch file (first written as a POSIX sh script, rewritten for PowerShell 5.1/7
  because the host is Windows). Run from the repo root, idempotent, prints what it does; writes UTF-8 without BOM and LF. It
  deletes `.sandbox/engine/`, `README.md`, `.gitignore`, `.gitattributes`, replaces only the header comment block of
  `.sandbox/sandbox.toml` (everything above `[project]`) with the init template's, and creates
  `.sandbox/.state/.gitignore` (`*`) if `.state/` exists without one, so nothing shows up in `git status` before the
  next app run.
- **Removed test `Template_errors_are_reported`** (`ComposeFileTests`): it edited `.sandbox/engine/compose.yml.sbn` on
  disk, and the template is embedded now. A black-box test can no longer break the template. The two error paths in
  `ComposeTemplate` (`<name> is not a valid template`, `<name>: <message>`) are unchanged apart from dropping the
  `engine/` prefix.
- **Error texts without `engine/`**: `... (with sandbox.toml)` instead of `(with sandbox.toml and engine/)`, because
  `IsSandboxDir` no longer needs `engine/`. The four assertions in `CommandTests`/`ErrorTests` that check these texts
  were updated; nothing else in the existing tests changed except the fixture-layout ones the plan names.
- **Compose golden file**: verified by diff that only the header comment lines, `build: ./.state/proxy` (twice),
  `context: ./.state/desktop` and the proxy `command` changed. The header is now `# Generated by sandbox <version> —
  changes are overwritten. Settings: sandbox.toml.` (prepended by `Sandbox.Generate`, so the version is always right)
  followed by the template's own two comment lines (`# Rendered from compose.yml.sbn ...`). The comment on the
  `.:/sandbox:ro` volume now says `allowlist.txt (live: "sandbox reload")` (fixed after review; golden file updated).
- **Fixture**: `RepoSandboxDir`/`SANDBOX_TESTS_DIR` are gone. The two tests that used the shipped files
  (`Repo_config_files_are_valid`, `Shipped_sandbox_toml_renders_all_tools`) now read the init templates through a new
  `InitTemplatesDir` (`src/AiSandbox/Resources/init`). `AppVersion` (read from the built `sandbox.dll`),
  `OldEngineDir` were added; `Reset()` also removes an old-layout `engine/`.
- **Header rules** (`EngineFiles.WithHeader`): the header (`# ` comment) goes after the shebang if the file has one,
  else at the top for `Dockerfile`, `*.conf` and `*.sh`. `apt.conf.d/95sandbox-proxy` (`//` comments), `sudoers.d/...`
  and `kioslaverc` get none. Hashes in `.manifest` are of the file as written (with header). Text is normalised to LF
  and written as UTF-8 without BOM, so a CRLF checkout gives the same files and the same engine hash.
- **Hand-edit detection without a manifest entry**: a file that differs from the expected content and has no manifest
  entry counts as edited by hand (warning), as the plan's literal rule says. A missing or unreadable manifest is treated
  as empty.
- **`info` Engine row** shows the commit shortened to 7 characters (the SDK appends the full 40-character sha);
  the headers and the manifest hold the full `AssemblyInformationalVersion`.
- **`proxy/Dockerfile`** uses `COPY --chmod=0644 tinyproxy.conf /etc/tinyproxy/tinyproxy.conf` so the config stays readable
  by `nobody` even when the host's umask wrote `.state/proxy/tinyproxy.conf` as `0600`. Needs BuildKit (default in current
  Docker). Not tested against real Docker.
- **Stale-image hint removed after review.** It was built as planned, but `up` always runs `docker compose build`, so
  the hint asked for a rebuild that the same `up` was already doing. Removed with `.state/built` and its 5 tests
  (`Sandbox.Build.cs` deleted). Final count: **149 tests**.
- **`sandbox init` output**: `Created .sandbox/<file>`, `<file> exists — kept`, `Wrote .sandbox/.state/ ...`,
  `Next: edit .sandbox/sandbox.toml, then "sandbox up".`. A missing folder gives `Folder <path> does not exist.`
- **Init templates**: `sandbox.toml` got the new header (`sandbox up` / `rebuild` / `info`, link to the GitHub README).
  `allowlist.txt` header fixed after review (`sandbox reload` etc. instead of `.sandbox/sandbox …`). The repo's own
  `.sandbox/allowlist.txt` still has the old header (read-only here; edit on the host if wanted).
- Launchers `sandbox` and `sandbox.ps1`: comment now points to the root `README.md`.
- `plan-console-project.md` and `plan-sandbox-toml.md` were not edited (they describe earlier states).
- Not done: step 7; applying `apply-on-host.ps1`; `sandbox engine diff` and project tools (out of scope, see *Later*).
