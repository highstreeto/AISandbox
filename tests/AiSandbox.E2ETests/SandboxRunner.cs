using System.Diagnostics;
using System.Text.Json;

namespace AiSandbox.E2ETests;

public sealed record Call(string Name, string[] Args)
{
    public override string ToString() => $"{Name} {JsonSerializer.Serialize(Args)}";
}

public sealed record RunResult(int ExitCode, string StdOut, string StdErr, List<Call> Calls)
{
    public string Output => StdOut + "\n" + StdErr;

    /// Stdout split into lines; the app writes \r\n on Windows.
    public string[] Lines => StdOut.ReplaceLineEndings("\n").Split('\n');

    public override string ToString() =>
        $"exit {ExitCode}\n--- stdout\n{StdOut}\n--- stderr\n{StdErr}\n--- calls\n{string.Join('\n', Calls)}";
}

/// One rule for the fake docker: the first rule whose `When` is a substring of the space-joined arguments wins.
public sealed record Rule(string When, string Stdout = "", string Stderr = "", int Exit = 0);

/// Base for the test classes: runs the sandbox app against the fake docker and builds expected calls.
[Collection(SandboxCollection.Name)]
public abstract class SandboxTest
{
    protected SandboxFixture F { get; }

    protected SandboxTest(SandboxFixture fixture)
    {
        F = fixture;
        F.Reset();
    }

    public const string Url = "https://localhost:49153";

    /// Three healthy services, one JSON object per line like `docker compose ps --format json` prints them.
    public static readonly string PsJson = string.Join(
        "\n",
        """{"Service":"desktop","State":"running","Health":"healthy","Status":"Up 2 minutes (healthy)","Publishers":[]}""",
        """{"Service":"gateway","State":"running","Health":"","Status":"Up 2 minutes","Publishers":[{"URL":"127.0.0.1","TargetPort":3001,"PublishedPort":49153,"Protocol":"tcp"}]}""",
        """{"Service":"proxy","State":"running","Health":"healthy","Status":"Up 2 minutes (healthy)","Publishers":[]}"""
    ) + "\n";

    /// What a running sandbox answers.
    protected static Rule[] DefaultRules =>
        [
            new("info --format", "27.0.0\n"),
            new("ps -a --filter", SandboxFixture.Id + "\n"),
            new("port gateway 3001", "127.0.0.1:49153\n"),
            new("ps -a --format json", PsJson),
        ];

    /// Run the built app (`dotnet sandbox.dll args`) with `SANDBOX_DIR` set, like the launchers do.
    /// `rules` are checked before the defaults. `path` replaces the fake bin dir on PATH.
    /// `waitFor`: a call that a detached child makes after the app has exited (the browser opener).
    /// `cwd`: the working directory (default: the temp root, outside the project).
    /// `sandboxDir`: the value of `SANDBOX_DIR` (default: the project's .sandbox; "" leaves it unset).
    protected RunResult Run(
        string[] args,
        Rule[]? rules = null,
        string? path = null,
        string? waitFor = null,
        string? cwd = null,
        string? sandboxDir = null
    )
    {
        // A fresh log per run: a detached child of an earlier run can't write into this one.
        var log = Path.Combine(F.Root, $"calls-{Guid.NewGuid():N}.jsonl");
        var rulesFile = Path.Combine(F.Root, "rules.json");
        File.WriteAllText(
            rulesFile,
            JsonSerializer.Serialize(
                (rules ?? []).Concat(DefaultRules).Select(r => new
                {
                    when = r.When,
                    stdout = r.Stdout,
                    stderr = r.Stderr,
                    exit = r.Exit,
                })
            )
        );

        var psi = new ProcessStartInfo(F.Dotnet)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = true,
            WorkingDirectory = cwd ?? F.Root,
        };
        foreach (var a in args.Prepend(F.AppDll))
            psi.ArgumentList.Add(a);
        psi.Environment["PATH"] =
            path ?? F.FakeBin + Path.PathSeparator + Environment.GetEnvironmentVariable("PATH");
        psi.Environment["FAKE_DOCKER_LOG"] = log;
        psi.Environment["FAKE_DOCKER_RULES"] = rulesFile;
        psi.Environment["TZ"] = "Europe/Vienna";
        psi.Environment["COLUMNS"] = "200";
        psi.Environment["NO_COLOR"] = "1";
        psi.Environment.Remove("DOCKER_HOST");
        sandboxDir ??= F.Dir;
        if (sandboxDir.Length > 0)
            psi.Environment["SANDBOX_DIR"] = sandboxDir;
        else
            psi.Environment.Remove("SANDBOX_DIR");

        using var p = Process.Start(psi)!;
        p.StandardInput.Close();
        var stdout = p.StandardOutput.ReadToEndAsync();
        var stderr = p.StandardError.ReadToEndAsync();
        if (!p.WaitForExit(TimeSpan.FromSeconds(120)))
        {
            p.Kill(entireProcessTree: true);
            throw new TimeoutException($"sandbox {string.Join(' ', args)} did not finish.");
        }

        var calls = ReadCalls(log);
        for (var i = 0; waitFor != null && i < 100 && !calls.Any(c => c.Name == waitFor); i++)
        {
            Thread.Sleep(100);
            calls = ReadCalls(log);
        }
        return new RunResult(p.ExitCode, stdout.Result, stderr.Result, calls);
    }

    static List<Call> ReadCalls(string log) =>
        File.Exists(log)
            ? File.ReadAllLines(log)
                .Where(l => l.Length > 0)
                .Select(l =>
                {
                    using var d = JsonDocument.Parse(l);
                    return new Call(
                        d.RootElement.GetProperty("name").GetString()!,
                        d.RootElement.GetProperty("args")
                            .EnumerateArray()
                            .Select(a => a.GetString()!)
                            .ToArray()
                    );
                })
                .ToList()
            : [];

    protected RunResult Run(params string[] args) => Run(args, null);

    // --- expected calls --------------------------------------------------------

    protected static Call Docker(params string[] args) => new("docker", args);

    protected static Call DockerInfo => Docker("info", "--format", "{{.ServerVersion}}");

    /// `docker compose -p <name> --project-directory <dir> -f .state/compose.yml <args>`
    /// — attached to the terminal (build, exec, logs).
    protected Call ComposeInteractive(params string[] args) =>
        ComposeAs(SandboxFixture.ProjectName, args);

    /// Same, with captured output (`--ansi never`).
    protected Call Compose(params string[] args) => ComposeInteractive(["--ansi", "never", .. args]);

    protected Call ComposeAs(string project, string[] args)
    {
        var a = new List<string>
        {
            "compose",
            "-p",
            project,
            "--project-directory",
            F.Dir,
            "-f",
            F.GeneratedFile,
        };
        a.AddRange(args);
        return Docker([.. a]);
    }

    protected Call CollisionCheck(string project = SandboxFixture.ProjectName) =>
        Docker(
            "ps",
            "-a",
            "--filter",
            $"label=com.docker.compose.project={project}",
            "--filter",
            "label=sandbox.id",
            "--format",
            "{{.Label \"sandbox.id\"}}"
        );

    protected static void AssertCalls(RunResult r, params Call[] expected) =>
        Assert.True(
            expected.Select(c => c.ToString()).SequenceEqual(r.Calls.Select(c => c.ToString())),
            $"Expected calls:\n{string.Join('\n', expected.AsEnumerable())}\n\nActual run:\n{r}"
        );

    protected static void AssertExit(RunResult r, int code) =>
        Assert.True(r.ExitCode == code, $"Expected exit {code}.\n{r}");

    protected static void AssertOutput(RunResult r, string text) =>
        Assert.True(r.Output.Contains(text), $"Expected output to contain '{text}'.\n{r}");

    protected void WriteConf(string toml) => File.WriteAllText(F.ConfFile, toml);
}
