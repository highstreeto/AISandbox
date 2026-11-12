# Plan: one generated compose file

> **Status:** implemented as `compose-consolidation.patch` (written up after the fact).
> Followed by [plan-sandbox-toml.md](plan-sandbox-toml.md), which replaces the merge with a template.

Every `docker compose` call ran with up to four files layered with `-f`. Merge them inside the app into one
self-contained `.state/compose.yml`, and run every command with only that file.

## Why

- The plan is to turn this into a standalone app/tool that generates the setup. A set of override files that only
  works when Compose stacks them in the right order is awkward for that. One generated file is the whole truth.
- You can read `.state/compose.yml` to see exactly what runs, without mentally merging four files.
- Settings that must not be overridden (UI port, project mounts, the read-only `.sandbox` mount) can be enforced
  by the app. Before, the generated file came *before* `sandbox.yml`, so `sandbox.yml` could override them.

## Before

`ComposeArgs` in `sandbox.project.cs`:

```
docker compose -p <name> --project-directory .sandbox \
  -f engine/compose.yml                # services, networks, x-hardened anchor
  -f .state/compose.generated.yml      # port, sandbox.id label, PUID/PGID/TZ, project mounts
  -f sandbox.yml                       # the project's settings + x-sandbox
  [-f sandbox.local.yml]               # optional, per machine
```

## After

```
docker compose -p <name> --project-directory .sandbox -f .state/compose.yml
```

`Generate()` (still run before every docker command) merges these layers in order, later ones winning:

| # | Layer | Content |
|---|---|---|
| 1 | `engine/compose.yml` | services, networks, hardening |
| 2 | host defaults | `PUID`, `PGID`, `TZ` (can be overridden) |
| 3 | `sandbox.yml` | the project's settings |
| 4 | `sandbox.local.yml` | machine-specific settings (optional) |
| 5 | host settings | UI port (`!override`), `sandbox.id` label, `SANDBOX_WORKDIR`, project / `.sandbox` / `.git` mounts (can't be overridden) |

Output: header comment, LF, no BOM, no `...` end marker, no anchors, top-level `x-*` dropped. Relative paths stay
relative to `--project-directory` (the `.sandbox` folder), and `${VAR}` stays literal for Compose to interpolate.
A leftover `.state/compose.generated.yml` is deleted.

## How to merge: in C#, not `docker compose config`

The other option was `docker compose -f … -f … config > compose.yml`, which gets Compose's exact merge rules for free. Rejected because:

- The tests use a fake `docker` (`tests/FakeDocker`), so generation wouldn't be tested.
- It loses comments, normalizes values (`8g` → bytes), adds defaults and makes paths absolute, so the output is hard to read.
- Generation would need a running Docker daemon.

So `ComposeMerger` in a new `sandbox.compose.cs` uses YamlDotNet and implements the
[Compose merge rules](https://docs.docker.com/reference/compose-file/merge/) that `sandbox.yml` can realistically contain:

| What | Rule |
|---|---|
| mappings | merged key by key, recursively; a later scalar wins |
| `environment`, `labels`, `annotations`, `sysctls`, `build.args`, `build.labels`, `deploy.labels` | list form (`KEY=value`, `KEY`) converted to a mapping, then merged by key |
| `networks`, `depends_on`, `extra_hosts` | either form; converted to a mapping only when the two layers use different forms |
| `build: ./dir` | short form expanded to `{context: ./dir}` when the other layer is a mapping |
| `volumes`, `devices` | merged by container path (target); short and long form; Windows drive letters handled |
| `command`, `entrypoint`, `healthcheck.test` | replaced as a whole |
| other sequences | union, duplicates dropped |
| `!reset` / `!override` | remove the value / replace without merging; tags stripped from the output |
| `<<: *anchor` | expanded when loading (explicit keys win); aliases become independent copies |
| `include`, `extends` | rejected with an error naming the file |
| null scalars (`home:`) | written as `null` (YamlDotNet would write `''`, which Compose reads as a string) |

## Code changes

| File | Change |
|---|---|
| `.sandbox/sandbox.compose.cs` | **New**: `ComposeLayer`, `ComposeMerger` (`Layer`, `Merge`, `Write`) |
| `.sandbox/sandbox.project.cs` | `GenPath` → `.state/compose.yml`; `ComposeArgs` uses only `-f GenPath`; `Generate()` builds the five layers; `HostDefaults()` / `HostSettings()` replace the old one-file generator; `Header` constant |
| `.sandbox/sandbox.config.cs` | `Config.Load` made public (shared YAML loader with line/column errors) |
| `.sandbox/sandbox.cs` | `#:include sandbox.compose.cs`, header comment |
| `.sandbox/engine/compose.yml`, `sandbox.yml` | Comments describe the merge into `.state/compose.yml` |
| `.sandbox/README.md` | Configure section: the one generated file, merge rules, what can and can't be overridden |
| `tests/Sandbox.Tests/SandboxFixture.cs`, `SandboxRunner.cs` | `GeneratedFile` → `compose.yml`; expected compose calls have a single `-f` |
| `tests/Sandbox.Tests/ComposeFileTests.cs` | Exact full-file test; self-contained (no anchors/`x-*`); engine hardening applied; one test per merge rule; host settings can't be overridden; host defaults can; `include`/`extends` rejected; old file removed |
| `tests/Sandbox.Tests/CommandTests.cs` | "every compose call uses only the generated file"; verbose test no longer depends on where the line wraps |
| `tests/README.md` | Generated file name, test description |

## Steps

1. Merger with the rules above, layer loading with merge-key expansion and the unsupported-feature checks.
2. `Generate()` writes the merged file; `ComposeArgs` uses only that file.
3. Parity check against Docker as the reference (`docker compose config` needs no daemon):
   - default settings: old four-file merge vs. new single file;
   - an edge-case `sandbox.local.yml` (list-form env, `!reset`, `!override`, Windows path, map-form networks,
     replaced command, volume at an existing target).
   Both identical except volume order (irrelevant: Docker mounts parents before children) and the dropped `x-*`.
   This found two bugs, both fixed: the desktop build context was lost (`build: ./engine/desktop` vs. a mapping override),
   and `home:` came out as `''`.
4. Tests (95 passing), README and comments.

## Decisions

1. **Merge in C#**, not with `docker compose config`; Docker is only used as a reference to check the result.
2. **Host settings go last** and can't be overridden; **host defaults** (`TZ`, `PUID`, `PGID`) go before `sandbox.yml` and can.
3. **Relative paths stay relative** to `--project-directory`; the generated file isn't meant to move.
4. **No validation call** (`docker compose config --quiet`) before `up`: `up` already validates and reports errors.
5. **The engine file stays on disk** (`engine/compose.yml`). Embedding it and the build contexts in the app is the
   next step towards a standalone tool.

## Known limits

- Mount order in the output can differ from the layer order when a later layer replaces an entry in place.
- Only the listed Compose features are merged specially; anything else follows the generic mapping/union rules.
- Patch had to be applied on the host: `.sandbox/` is read-only inside the sandbox.
