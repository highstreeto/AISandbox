# Plan: the sandbox app as a normal C# console project

> **Status:** implemented. The `.sandbox/` part (deleting the old `*.cs` files, README, template comment) has to be
> applied on the host, because `.sandbox/` is read-only inside the sandbox: `sandbox-dir.patch` in the repo root.
> Differences from the plan as first written are noted in *As built* at the end.
> Follows [plan-sandbox-toml.md](plan-sandbox-toml.md). Prepares packaging as a .NET tool (not part of this plan).

Today the control app is a .NET file-based app: `.sandbox/sandbox.cs` plus six `sandbox.*.cs` files pulled in with
`#:include`, started with `dotnet run --file`. Turn it into a regular SDK-style console project under `src/`, with a
root `.slnx` that holds the app and the tests, conventional file names, explicit visibility, `readonly` state and
`ImmutableArray<T>` for collections that don't change after they are built.

Behaviour stays the same. Every existing test must pass unchanged in what it asserts; only the harness changes.

## Why

- A `.csproj` is the starting point for `PackAsTool` (`dotnet tool install -g …`). A file-based app can't be packed.
- Normal tooling: IDE navigation, analyzers, `dotnet build`/`dotnet test` from the root, one solution.
- `#:include` needs SDK 10.0.400+; a project builds on any .NET 10 SDK.
- The tests stop compiling a copy of the sources into a temp folder and run the built app directly (faster, and the
  fixture no longer depends on `EntryPointFileDirectoryPath`).

## Target layout

```
AiSandbox.slnx                       new: src + tests
Directory.Build.props                new: net10.0, Nullable, ImplicitUsings, warnings
Directory.Packages.props             new: central package versions (app + tests)
sandbox, sandbox.ps1                 launchers: dotnet run --project src/AiSandbox
src/AiSandbox/
  AiSandbox.csproj
  Program.cs                         Main: help rewrite, `claude` passthrough, parse + invoke
  SandboxException.cs
  Cli/
    SandboxCommandLine.cs            was Cli.Build + Run + Guard
  Commands/
    LifecycleCommands.cs             Up, Rebuild, Compose, Status, Open, Claude, Logs (+ private Build, StartAndWait)
    NetworkCommands.cs               Allow, Deny, ShowAllowlist, Blocked, ReloadProxy
    MaintenanceCommands.cs           ResetHome, Info
  Project/
    Sandbox.cs                       identity, paths, Load, Sanitize           (partial)
    Sandbox.Locate.cs                LocateSandboxDir, Physical, CheckOldConfig (partial)
    Sandbox.Docker.cs                Compose*, Exec, NeedDocker, CheckCollision, UiUrl, Services (partial)
    Sandbox.Generate.cs              Generate, Model, Mounts, TimeZone         (partial)
    ComposePs.cs                     ParsePs, FormatPorts (from `compose ps --format json`)
    ServiceState.cs
    Mount.cs
  Configuration/
    Config.cs                        Read, KnownTools, ToolName, ToolValue
    TomlTableReader.cs               was the nested Config.Reader
    Settings.cs                      Settings + Share, ToolSetting, EnvVariable, BasicAuth
  Templating/
    ComposeTemplate.cs
  Network/
    Allowlist.cs
    AllowEntry.cs
    AllowResult.cs
  Processes/
    Proc.cs
    ProcResult.cs
    NativeUser.cs                    was Native
  Terminal/
    Ui.cs
tests/
  FakeDocker/                        unchanged
  AiSandbox.E2ETests/                was Sandbox.Tests; harness changes only (see Tests)
  Sandbox.Tests.slnx                 deleted (replaced by AiSandbox.slnx)
.sandbox/                            data only: sandbox.toml, allowlist.txt, engine/, .state/, README.md
```

Removed from `.sandbox/`: all `*.cs` files. `engine/` stays in `.sandbox/` for now (see *Later*).

### Names

- Project, assembly and root namespace: **`AiSandbox`**. Not `Sandbox`: the main type is `Sandbox`, and a type with
  the same name as its namespace forces `Sandbox.Sandbox` qualifications. Sub-namespaces follow the folders
  (`AiSandbox.Commands`, `AiSandbox.Configuration`, …), file-scoped.
- One top-level type per file, file name = type name. Small records that only exist for one type
  (`Share`, `ToolSetting`, `EnvVariable`, `BasicAuth` for `Settings`) may share `Settings.cs`.
- `Sandbox` is the one class split with `partial` (`Sandbox.<Area>.cs`), because its parts share the identity and
  paths. Splitting it into separate classes (e.g. a `DockerCompose` service) is a later refactor, not this one.
- `Commands` is split into three static classes by the existing comment sections. `MaintenanceCommands.ResetHome`
  calls `LifecycleCommands.Compose`, which becomes `internal`.
- `Cli` → `SandboxCommandLine`; `Native` → `NativeUser`. Everything else keeps its name.
- Assembly name **`sandbox`**: it is the command name System.CommandLine shows in the help (`Usage: sandbox up`), as
  the file-based app did, and later the tool command.
- Private static fields `s_name`, private instance fields `_name` (as `Allowlist` already does); the `.editorconfig`
  below enforces it.

## Project files

`Directory.Build.props`:

```xml
<Project>
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
    <AnalysisLevel>latest-recommended</AnalysisLevel>
  </PropertyGroup>
</Project>
```

The test and FakeDocker projects drop the properties that move here. `AnalysisLevel` may need a few rule
suppressions in `.editorconfig` (e.g. CA1303 literals, CA1031 the catch-all in `Main`); decide while implementing,
but don't leave warnings.

`Directory.Packages.props` (`ManagePackageVersionsCentrally`): System.CommandLine 2.0.15, Tomlyn 2.11.1,
Scriban 7.6.1, Spectre.Console 0.57.2, and the test packages (Microsoft.NET.Test.Sdk 18.12.0, xunit.v3 3.2.2,
xunit.runner.visualstudio 3.1.5, YamlDotNet 18.1.0). Same versions as today.

`src/AiSandbox/AiSandbox.csproj`:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <RootNamespace>AiSandbox</RootNamespace>
    <AssemblyName>sandbox</AssemblyName>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks> <!-- needed by [LibraryImport] -->
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="System.CommandLine" />
    <PackageReference Include="Tomlyn" />
    <PackageReference Include="Scriban" />
    <PackageReference Include="Spectre.Console" />
  </ItemGroup>
</Project>
```

No `InvariantGlobalization`: `TimeZoneInfo.TryConvertWindowsIdToIanaId` needs ICU, so on Windows the host time zone
would be lost. `PublishAot=false` goes away (it is the default).

`AiSandbox.slnx`:

```xml
<Solution>
  <Folder Name="/src/">
    <Project Path="src/AiSandbox/AiSandbox.csproj" />
  </Folder>
  <Folder Name="/tests/">
    <Project Path="tests/FakeDocker/FakeDocker.csproj" />
    <Project Path="tests/AiSandbox.E2ETests/AiSandbox.E2ETests.csproj" />
  </Folder>
  <Folder Name="/Solution Items/">
    <File Path="Directory.Build.props" />
    <File Path="Directory.Packages.props" />
  </Folder>
</Solution>
```

Plus a root `.gitignore` (`bin/`, `obj/`, `TestResults/`, `*.user`) replacing `tests/.gitignore`, and a minimal
`.editorconfig` (naming rules, file-scoped namespaces, the CSharpier-compatible formatting already in use).

## Finding the `.sandbox` folder

The file-based app found it through `AppContext.GetData("EntryPointFileDirectoryPath")` (the folder of
`sandbox.cs`). A compiled app has no such thing. New order in `Sandbox.LocateSandboxDir`:

1. `$SANDBOX_DIR`, if set. Must contain `sandbox.toml` (or old `sandbox.yml`) and `engine/`, otherwise an error that
   names the variable. The launchers set it, so "run from any directory" keeps working.
2. Upwards from the current directory: `<dir>` itself or `<dir>/.sandbox` (unchanged). This is what a future
   `dotnet tool` relies on.

Both go through `Physical()` as today.

## Launchers

`sandbox` (sh):

```sh
#!/bin/sh
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
SANDBOX_DIR="$root/.sandbox" exec dotnet run --project "$root/src/AiSandbox" --no-launch-profile -v q -- "$@"
```

`sandbox.ps1`:

```bat
@echo off
setlocal
set "SANDBOX_DIR=%~dp0.sandbox"
dotnet run --project "%~dp0src\AiSandbox" --no-launch-profile -v q -- %*
```

Both now resolve paths from their own location instead of the current directory (today's launchers only work from
the repo root). `Ui.Self` (the hint text) becomes `./sandbox` / `sandbox.ps1`, matching where the launchers are.

**Consequence:** copying only `.sandbox/` into another project no longer brings the app with it. Until the tool
exists, other projects run it with `dotnet run --project <aisandbox>/src/AiSandbox -- up` from inside the project
(the upward search finds their `.sandbox`). The README says so.

## Code changes

### Visibility

- Every type `internal`; `sealed` on every class and record that isn't `static` (most already are).
- Members default to `private`; `public` only where another type calls them. Helpers used only inside a type become
  `private` (e.g. `Sandbox.ComposeArgs`, `Config.ToolValue`, `Ui.Fallback`).
- `Program` is `internal static class Program` with `private static async Task<int> Main(string[] args)` (no
  top-level statements, so the entry point is a nameable type and `args` is not a hidden global).

### readonly / immutability

| Where | Change |
|---|---|
| `Config.ReservedEnv` | `FrozenDictionary<string, string?>` (case-insensitive), `private static readonly` |
| `Allowlist` regexes, `Config` patterns (`^[a-z0-9_]+$`, memory size, env name, `ARG INSTALL_*`), `Sandbox.Sanitize`, `ComposeTemplate` error prefix, `Blocked` log pattern | `[GeneratedRegex]` on `partial` methods instead of `new Regex` / `Regex.IsMatch(string, pattern)`; the per-host regexes in `Allowlist.Allow/Deny/IsAllowed` stay dynamic |
| `Allowlist._path`, `_lines` | already `readonly`; `_lines` stays a `List<string>` (it is the edit buffer) |
| `TomlTableReader` | primary-constructor parameters copied to `private readonly` fields (`_table`, `_path`); `_used` already readonly |
| `Ui._err` | `private static IAnsiConsole s_err` stays assignable (set once in `Init`), documented; everything else in `Ui` is get-only |
| `SandboxCommandLine.Verbose` | already `static readonly` → `s_verbose` |
| `NativeUser` | `[LibraryImport]` (source-generated, class becomes `partial`) instead of `[DllImport]` |
| Records | stay positional (`init`-only); no settable properties anywhere |

### ImmutableArray

Tuples in public shapes become named records at the same time.

| Today | After |
|---|---|
| `Settings.Install: List<(string Arg, string Value)>` | `ImmutableArray<ToolSetting>` (`record ToolSetting(string Arg, string Value)`) |
| `Settings.Devices`, `SecurityOpt: List<string>` | `ImmutableArray<string>` |
| `Settings.Auth: (string User, string Password)?` | `BasicAuth?` |
| `Settings.Env: List<(string Key, string Value)>` | `ImmutableArray<EnvVariable>` |
| `Settings.Shares: List<Share>` | `ImmutableArray<Share>` |
| `Config.KnownTools(): List<string>` | `ImmutableArray<string>` |
| `TomlTableReader.StrList`, `TableArray` | `ImmutableArray<string>`, `ImmutableArray<TomlTableReader>` |
| `Sandbox.Services(): List<ServiceState>` | `ImmutableArray<ServiceState>` |
| `Sandbox.Mounts(): List<(…)>` | `ImmutableArray<Mount>` (`record Mount(string Source, string Target, bool ReadOnly)`) |
| `ProcResult.Lines(): IEnumerable<string>` | `ImmutableArray<string>` |
| `Allowlist.Entries(): IEnumerable<AllowEntry>` (iterator) | `ImmutableArray<AllowEntry>` (built once per call) |
| `Commands.Build` / `Logs` arg lists (`new List<string>` → `[.. a]`) | `ImmutableArray.CreateBuilder<string>()` or a collection expression |
| `Proc.Capture/Interactive(…, params string[] args)` | `params ReadOnlySpan<string>` (they only forward to `ArgumentList`); `Sandbox.Compose`, `ComposeInteractive`, `Exec` likewise |

Notes:

- Builders use `ImmutableArray.CreateBuilder<T>()` + `ToImmutable()` (or `MoveToImmutable()` when the size is known);
  short fixed lists use collection expressions (`ImmutableArray<string> x = ["a", "b"];`).
- `ImmutableArray<T>` in a record compares by reference. Nothing compares `Settings` today; leave a comment on the
  record so nobody starts relying on value equality.
- `default(ImmutableArray<T>)` throws on enumeration: every path that builds `Settings` assigns all arrays (empty =
  `[]`), and `Info` keeps using `.Length`/`.IsEmpty` instead of `Count`.
- Scriban's `ScriptArray` takes `IEnumerable<object>`; `Sandbox.Model()` maps the immutable arrays as today.
- System.CommandLine values (`string[]`) stay `string[]` at the boundary in `SandboxCommandLine` and are passed on
  as `ReadOnlySpan<string>` / converted with `ImmutableArray.Create`.

### Other small things that fall out of the move

- `Program.Main`: the `help` rewrite and the `claude` passthrough stay before parsing (unchanged behaviour).
- File header comments that describe `#:include`, `dotnet run --file` and `sandbox.*.cs` are rewritten or dropped;
  XML doc comments (`///`) on types become proper `<summary>` blocks.
- `engine/compose.yml.sbn` header comment (a Scriban comment, not in the output): `../sandbox.cs` → "the sandbox app".
  The generated `# Generated by .sandbox/sandbox` line stays as is, since `ComposeFileTests` checks the exact text.

## Tests

The tests stay black-box (they run the app as a process). Besides the rename below, harness changes only.

### Rename: `Sandbox.Tests` → `AiSandbox.E2ETests`

The suite runs the real executable end to end against a fake `docker`; it never references the app's code. The name
should say so, and leave `AiSandbox.Tests` free for unit tests against the assembly later.

- `git mv tests/Sandbox.Tests tests/AiSandbox.E2ETests` and the `.csproj` inside it (keeps history).
- Namespace `Sandbox.Tests` → `AiSandbox.E2ETests` in all test files.
- Delete `tests/Sandbox.Tests.sln.DotSettings.user` (a per-user file for the old solution; `*.user` is ignored from now on).
- `FakeDocker` keeps its name: it is a test helper, not a test project.

### Harness

| File | Change |
|---|---|
| `AiSandbox.E2ETests.csproj` | Drop properties now in `Directory.Build.props`; versions from `Directory.Packages.props`. Add `<ProjectReference Include="..\..\src\AiSandbox\AiSandbox.csproj" ReferenceOutputAssembly="false" />` so the app is built with the tests, like FakeDocker |
| `SandboxFixture` | No longer copies `*.cs` and no longer runs `dotnet build sandbox.cs`. Copies only `engine/` into `demo-project/.sandbox/`. Finds the built app at `src/AiSandbox/bin/<config>/<tfm>/AiSandbox.dll` (same lookup as FakeDocker). `FindRepoSandboxDir` looks for `.sandbox/sandbox.toml` + `engine/` instead of `sandbox.cs`. `EntryFile` → `AppDll` |
| `SandboxRunner.Run` | Runs `dotnet <AppDll> <args>` instead of `dotnet run --file … --no-build --`, with `SANDBOX_DIR=<F.Dir>` (as the launchers do). New optional `cwd` parameter (default `F.Root`) and a way to leave `SANDBOX_DIR` unset |
| `CommandTests` | New: run from `demo-project/src/` without `SANDBOX_DIR` → finds `.sandbox` by searching upwards. New: `SANDBOX_DIR` pointing at a folder without `sandbox.toml` → error, exit 1. New: run from `Root` without `SANDBOX_DIR` → "Could not find the .sandbox folder" |
| `tests/README.md` | Describe the new harness; run with `dotnet test` from the root |

`SANDBOX_TESTS_DIR` keeps meaning "use this `.sandbox` folder's `engine/` and config files"; the app under test is
always the one built from `src/`.

The fixture no longer compiles the app itself (it is built with the tests). Each run still starts `dotnet`, so the
suite takes about as long as before (~55 s).

## Docs

- `.sandbox/README.md`: requirements (.NET 10 SDK, no 10.0.400 minimum), the "Start / stop" block (`./sandbox up`,
  `sandbox.ps1 up`), the paragraph about the file-based app, troubleshooting entry about build errors, and the note
  on using it from another project (see *Launchers*).
- `tests/README.md`: as above.
- Root `README.md` doesn't exist; not adding one in this change.

## Order of work

1. Add `Directory.Build.props`, `Directory.Packages.props`, `.editorconfig`, root `.gitignore`, `AiSandbox.slnx`;
   move the test projects to central versions. Delete `tests/Sandbox.Tests.slnx`, `tests/.gitignore`. Rename
   `Sandbox.Tests` → `AiSandbox.E2ETests` (folder, project, namespace). Run the suite: still green on the old layout.
2. `git mv` the sources to `src/AiSandbox/` with their new names (keeps history), add the `.csproj`, split the files
   into one type per file. Build. No code changes yet beyond what the split needs (`#:` directives out, namespaces in).
3. Replace `EntryPointFileDirectoryPath` with `SANDBOX_DIR` + upward search; update the launchers.
4. Switch the test harness; run the full suite → green. This is the checkpoint: same behaviour, new layout.
5. Visibility, `readonly`, `[GeneratedRegex]`, `FrozenDictionary`, `LibraryImport`.
6. `ImmutableArray` and the named records. Suite green after each of 5 and 6.
7. Docs, the template comment, `.idea`/`.vscode` paths if they point at `.sandbox/*.cs`.
8. Smoke test against real Docker: `./sandbox up`, `status`, `allow`, `down` from the repo root and from a subfolder,
   on Linux and Windows (`sandbox.ps1`).

## Out of scope / later

- **.NET tool:** `PackAsTool`, `ToolCommandName=sandbox`, publishing. Then `engine/` should move into the tool as
  embedded resources (rendered/extracted at run time), and `.sandbox/` in a project would hold only `sandbox.toml`,
  `allowlist.txt` and `.state/`. That is a behaviour change (which engine version a project gets) and needs its own
  plan.
- `sandbox init` to create `.sandbox/` in a new project (natural once the tool exists).
- Splitting `Sandbox` into separate services, or dependency injection.
- Unit tests against the assembly (`InternalsVisibleTo`) in a future `tests/AiSandbox.Tests`; the E2E tests stay the
  main suite.

## As built

- Steps 2–6 were done in one pass (the code was written in its final form), then the suite was run: 115 tests pass
  (the 112 existing ones and 3 new ones for finding `.sandbox`). `./sandbox info` / `help` / `status` work from a
  subfolder.
- Names: `SandboxCommandLine` (not `CommandLineBuilder`, which reads like the old System.CommandLine type),
  namespace `AiSandbox.Templating` (not `.Compose`, which clashes with the `Compose` methods), assembly `sandbox`.
- `ErrorTests.Missing_sandbox_toml`: with `SANDBOX_DIR` set (as the launchers do) the error now names the variable
  ("SANDBOX_DIR is …, which is not a .sandbox folder (with sandbox.toml and engine/)."); the old message is still
  checked for a run without `SANDBOX_DIR`. A set but wrong `SANDBOX_DIR` is an error and doesn't fall back to the
  search, so a broken launcher can't silently pick up another project's `.sandbox`.
- Analyzers (`latest-recommended`, warnings as errors): CA1303, CA1515, CA1031, CA1305, CA1307, CA1310, CA1311, CA1707
  are off everywhere (see `.editorconfig` for why); CA1711, CA1716, CA1822, CA1861 are off for test code only.
- `NativeUser` catches only `DllNotFoundException` / `EntryPointNotFoundException` instead of every exception.
- `.gitattributes` in the root also covers `*.cmd` (CRLF), since `sandbox.ps1` now lives there.

