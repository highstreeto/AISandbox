namespace Sandbox.Tests;

/// Docker missing or failing, invalid configuration: the right message, exit code 1, and no further docker calls.
public sealed class ErrorTests(SandboxFixture f) : SandboxTest(f)
{
    Call UpWait => Compose("up", "-d", "--wait", "--wait-timeout", "300");

    [Fact]
    public void Docker_not_installed()
    {
        var r = Run(["status"], path: F.EmptyBin);
        AssertExit(r, 1);
        AssertCalls(r);
        AssertOutput(r, "Docker is not installed.");
    }

    [Theory]
    [InlineData("up")]
    [InlineData("status")]
    [InlineData("shell")]
    [InlineData("claude")]
    [InlineData("allow", "example.com")]
    public void Docker_not_running(params string[] args)
    {
        var allowlist = File.ReadAllText(F.AllowlistFile);
        var r = Run(args, [new("info", Stderr: "Cannot connect to the Docker daemon", Exit: 1)]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo);
        AssertOutput(r, "Docker is not running.");
        Assert.Equal(allowlist, File.ReadAllText(F.AllowlistFile));
    }

    [Fact]
    public void Up_refuses_a_container_group_of_another_project()
    {
        var r = Run(["up", "--no-open"], [new("ps -a --filter", "ffffffff\n")]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, CollisionCheck());
        AssertOutput(r, "Another project already uses the container group 'demo-project'.");
    }

    [Fact]
    public void Up_stops_when_the_build_fails()
    {
        var r = Run(["up", "--no-open"], [new(" build", Stderr: "failed to solve", Exit: 1)]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, CollisionCheck(), ComposeInteractive("build"));
        AssertOutput(r, "Building the sandbox images failed");
    }

    [Fact]
    public void Up_tolerates_an_unhealthy_but_running_desktop()
    {
        var r = Run(["up", "--no-open"], [new("up -d --wait", Stderr: "container desktop is unhealthy", Exit: 1)]);
        AssertExit(r, 0);
        AssertCalls(
            r,
            DockerInfo,
            CollisionCheck(),
            ComposeInteractive("build"),
            UpWait,
            Compose("ps", "-a", "--format", "json"),
            Compose("port", "gateway", "3001")
        );
        AssertOutput(r, "The desktop isn't reporting ready yet");
        AssertOutput(r, Url);
    }

    [Fact]
    public void Up_fails_when_the_desktop_is_not_running()
    {
        var ps = """{"Service":"desktop","State":"exited","Health":"","Status":"Exited (1)"}""" + "\n";
        var r = Run(
            ["up", "--no-open"],
            [new("up -d --wait", Stderr: "boom", Exit: 1), new("ps -a --format json", ps)]
        );
        AssertExit(r, 1);
        AssertCalls(
            r,
            DockerInfo,
            CollisionCheck(),
            ComposeInteractive("build"),
            UpWait,
            Compose("ps", "-a", "--format", "json")
        );
        AssertOutput(r, "docker compose up (exit 1)");
        AssertOutput(r, "boom");
        AssertOutput(r, "Starting the sandbox failed");
    }

    [Fact]
    public void Rebuild_does_not_tolerate_an_unhealthy_desktop()
    {
        var r = Run(["rebuild"], [new("up -d --wait", Stderr: "unhealthy", Exit: 1)]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, ComposeInteractive("build"), UpWait);
        AssertOutput(r, "Starting the sandbox failed");
    }

    [Fact]
    public void Up_fails_without_a_ui_port()
    {
        var r = Run(["up", "--no-open"], [new("port gateway 3001", "", Exit: 1)]);
        AssertExit(r, 1);
        AssertOutput(r, "its UI port could not be determined");
    }

    [Theory]
    [InlineData("stop", "Stopping")]
    [InlineData("down", "Removing containers")]
    [InlineData("restart", "Restarting")]
    public void Compose_failure_shows_the_output(string cmd, string what)
    {
        var r = Run([cmd], [new($"never {cmd}", Stderr: "no such service", Exit: 3)]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, Compose(cmd));
        AssertOutput(r, $"docker compose {cmd} (exit 3)");
        AssertOutput(r, "no such service");
        AssertOutput(r, $"{what} failed.");
    }

    [Theory]
    [InlineData("1")] // exit code
    [InlineData("0")] // port 0 = not published
    public void Open_when_not_running(string mode)
    {
        Rule rule = mode == "1" ? new("port gateway 3001", Exit: 1) : new("port gateway 3001", ":0\n");
        var r = Run(["open"], [rule]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, Compose("port", "gateway", "3001"));
        AssertOutput(r, "The sandbox is not running");
    }

    [Fact]
    public void Reset_home_needs_confirmation_when_not_interactive()
    {
        var r = Run("reset-home");
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo);
        AssertOutput(r, "Refusing to delete the home volume without confirmation (use --yes).");
    }

    [Fact]
    public void Reset_home_reports_a_failed_volume_removal()
    {
        var r = Run(["reset-home", "--yes"], [new("volume rm", Stderr: "volume is in use", Exit: 1)]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, Compose("down"), Docker("volume", "rm", "demo-project_home"));
        AssertOutput(r, "Could not remove volume demo-project_home.");
    }

    [Fact]
    public void Reload_failure()
    {
        var r = Run(["reload"], [new("restart proxy", Exit: 1, Stderr: "proxy not found")]);
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo, Compose("restart", "proxy"));
        AssertOutput(r, "Could not restart the proxy.");
    }

    [Fact]
    public void Allow_rejects_an_invalid_host()
    {
        var before = File.ReadAllText(F.AllowlistFile);
        var r = Run("allow", "ok.example.com", "bad host!");
        AssertExit(r, 1);
        AssertCalls(r, DockerInfo);
        AssertOutput(r, "Not a valid host name: 'bad host!'.");
        Assert.Equal(before, File.ReadAllText(F.AllowlistFile));
    }

    [Theory]
    [InlineData("x-sandbox:\n  workspace_mode: rx\n", "x-sandbox.workspace_mode must be 'rw' or 'ro' (got 'rx').")]
    [InlineData("x-sandbox:\n  protect_git: maybe\n", "x-sandbox.protect_git must be true or false (got 'maybe').")]
    [InlineData("x-sandbox:\n  ui_port: abc\n", "x-sandbox.ui_port must be a port number (got 'abc').")]
    [InlineData("x-sandbox:\n  ui_port: 70000\n", "x-sandbox.ui_port must be a port number (got '70000').")]
    [InlineData("x-sandbox: [1, 2]\n", "sandbox.yml: x-sandbox must be a mapping.")]
    [InlineData("x-sandbox:\n  workspace_mode: [rw]\n", "sandbox.yml: x-sandbox.workspace_mode must be a single value.")]
    [InlineData("- just\n- a list\n", "sandbox.yml must be a YAML mapping.")]
    [InlineData("services:\n  desktop: [\n", "sandbox.yml is not valid YAML (line")]
    public void Invalid_sandbox_yml(string yaml, string message)
    {
        WriteConf(yaml);
        foreach (var cmd in new[] { "status", "info" })
        {
            var r = Run(cmd);
            AssertExit(r, 1);
            AssertCalls(r);
            AssertOutput(r, message);
        }
        Assert.False(File.Exists(F.GeneratedFile));
    }

    [Fact]
    public void Invalid_sandbox_local_yml()
    {
        WriteLocalConf("x-sandbox:\n  workspace_mode: rx\n");
        var r = Run("up", "--no-open");
        AssertExit(r, 1);
        AssertCalls(r);
        AssertOutput(r, "x-sandbox.workspace_mode must be 'rw' or 'ro' (got 'rx').");
    }
}
