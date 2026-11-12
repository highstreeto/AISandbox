# Sandbox app end-to-end tests

End-to-end tests for the sandbox control app (`src/AiSandbox`). Every command runs the built app as a process
against a **fake `docker`** executable. The tests check the exact docker calls, exit codes and messages, and the
generated `.state/compose.yml`.

```bash
dotnet test AiSandbox.slnx
```

Run it from the repository root. Needs the .NET 10 SDK and NuGet access. No Docker needed. Runs on Windows, Linux and
macOS, and takes about a minute.

## How it works

- **`FakeDocker/`** is a tiny console app built as `docker[.exe]`, and also installed as `xdg-open`. Each call appends
  `{"name", "args"}` as a JSON line to `$FAKE_DOCKER_LOG`. Its output and exit code come from the rules in
  `$FAKE_DOCKER_RULES`: the first rule whose `when` is a substring of the joined arguments wins.
- **The app** (`src/AiSandbox`) and the fake docker are built along with the tests (`ProjectReference` without
  referencing the assembly), in the same configuration.
- **`SandboxFixture`** copies the repo's `.sandbox/engine/` into a temporary `demo-project/.sandbox/` (with a
  `.git/config` and `.git/hooks`), and resets `sandbox.toml`, `allowlist.txt` and `.state/id` (`0123abcd`) to fixed
  contents (removing any old `sandbox.yml` / `sandbox.local.yml`) before each test.
  Set `SANDBOX_TESTS_DIR` to use another copy of the `.sandbox` folder instead of the repo's.
- **`SandboxTest.Run(...)`** runs `dotnet sandbox.dll <args>` with `SANDBOX_DIR` pointing at the test project's
  `.sandbox`, as the launchers do, and with the fake bin directory first on `PATH`. The default rules answer like a
  healthy, running sandbox, and a test can put its own rules in front of them. `cwd` and `sandboxDir: ""` run it
  without `SANDBOX_DIR`, to test the search upwards from the current directory.

| File | Covers |
|---|---|
| `CommandTests.cs` | Every command and alias, its docker calls in order, `-v`, `[project] name`, finding `.sandbox` |
| `ErrorTests.cs` | Docker missing or not running, failing docker calls, invalid `sandbox.toml` (every check) / hosts, old `sandbox.yml` |
| `ComposeFileTests.cs` | The generated compose file: exact text, each setting, escaping, shares, template errors |
