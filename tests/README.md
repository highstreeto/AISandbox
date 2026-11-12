# Sandbox app tests

Black-box tests for the sandbox control app (`.sandbox/sandbox.cs`). Every command runs against a **fake `docker`**
executable. The tests check the exact docker calls, exit codes and messages, and the generated
`.state/compose.yml`.

```bash
dotnet test tests/Sandbox.Tests
```

Needs the .NET 10 SDK and NuGet access. No Docker needed. Runs on Windows, Linux and macOS, and takes about 50 s.

## How it works

- **`FakeDocker/`** is a tiny console app built as `docker[.exe]`, and also installed as `xdg-open`. Each call appends
  `{"name", "args"}` as a JSON line to `$FAKE_DOCKER_LOG`. Its output and exit code come from the rules in
  `$FAKE_DOCKER_RULES`: the first rule whose `when` is a substring of the joined arguments wins.
- **`SandboxFixture`** copies the repo's `.sandbox/*.cs` and `engine/` into a temporary `demo-project/` (with a `.git/config`
  and `.git/hooks`), builds it once, and resets `sandbox.yml`, `allowlist.txt`, `.state/id` (`0123abcd`) and
  `sandbox.local.yml` to fixed contents before each test.
- **`SandboxTest.Run(...)`** runs `dotnet run --file sandbox.cs --no-build -- <args>`, which is what the launchers run,
  with the fake bin directory first on `PATH`. The default rules answer like a healthy, running sandbox, and a test can
  put its own rules in front of them.

| File | Covers |
|---|---|
| `CommandTests.cs` | Every command and alias, its docker calls in order, `-v`, `sandbox.local.yml`, `project_name` |
| `ErrorTests.cs` | Docker missing or not running, failing docker calls, invalid `sandbox.yml` / hosts |
| `ComposeFileTests.cs` | The generated compose file: exact text, each setting, each merge rule, the settings that can't be overridden |
