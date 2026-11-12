namespace AiSandbox.E2ETests;

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
    [InlineData("[project]\nworkspace = \"rx\"\n", "sandbox.toml: project.workspace must be \"rw\" or \"ro\" (got \"rx\").")]
    [InlineData("[project]\nprotect_git = \"maybe\"\n", "sandbox.toml: project.protect_git must be true or false (got \"maybe\").")]
    [InlineData("[project]\nui_port = \"abc\"\n", "sandbox.toml: project.ui_port must be a whole number (got \"abc\").")]
    [InlineData("[project]\nui_port = 70000\n", "sandbox.toml: project.ui_port must be a port number from 1 to 65535 (got 70000).")]
    [InlineData("[project]\nworkspace_mode = \"ro\"\n", "sandbox.toml: unknown key 'project.workspace_mode' (known: name, workspace, protect_git, ui_port).")]
    [InlineData("[tool]\nnode = 24\n", "sandbox.toml: unknown setting 'tool' (known: project, tools, desktop, env, share).")]
    [InlineData("project = 1\n", "sandbox.toml: project must be a table (got 1).")]
    [InlineData("[tools]\nnodejs = 24\n", "sandbox.toml: unknown tool tools.nodejs (known: apt, claude, claude_code, dotnet, node, bun, vscode, jetbrains).")]
    [InlineData("[tools]\ndotnet = 10.0\n", "sandbox.toml: tools.dotnet = 10.0: write versions as text, e.g. \"10.0\"")]
    [InlineData("[tools]\napt = [true]\n", "sandbox.toml: tools.apt must be a list of names or versions.")]
    [InlineData("[desktop]\nmemory = \"lots\"\n", "sandbox.toml: desktop.memory must be a size like \"8g\" or \"512m\" (got \"lots\").")]
    [InlineData("[desktop]\ncpus = 0\n", "sandbox.toml: desktop.cpus must be greater than 0 (got 0.0).")]
    [InlineData("[desktop]\ndevices = \"/dev/dri\"\n", "sandbox.toml: desktop.devices must be a list of texts, e.g. [\"a\", \"b\"] (got \"/dev/dri\").")]
    [InlineData("[desktop]\nauth = { user = \"me\" }\n", "sandbox.toml: desktop.auth needs both user and password.")]
    [InlineData("[desktop.auth]\nuser = \"me\"\npassword = \"x\"\nemail = \"m\"\n", "sandbox.toml: unknown key 'desktop.auth.email' (known: user, password).")]
    [InlineData("[env]\nHTTP_PROXY = \"http://evil\"\n", "sandbox.toml: env.HTTP_PROXY is set by the sandbox and can't be changed.")]
    [InlineData("[env]\nno_proxy = \"*\"\n", "sandbox.toml: env.no_proxy is set by the sandbox and can't be changed.")]
    [InlineData("[env]\nTZ = \"UTC\"\n", "sandbox.toml: env.TZ is set by the sandbox; use desktop.timezone instead.")]
    [InlineData("[env]\n\"MY-VAR\" = \"x\"\n", "sandbox.toml: env.MY-VAR is not a valid environment variable name.")]
    [InlineData("[env]\nX = [1]\n", "sandbox.toml: env.X must be a text, number or true/false (got a list).")]
    [InlineData("[[share]]\npath = \"/x\"\n", "sandbox.toml: share #1 needs host = \"<folder on this machine>\".")]
    [InlineData("[[share]]\nhost = \"../missing\"\n", "sandbox.toml: share #1: ../missing does not exist")]
    [InlineData("[[share]]\nhost = \".git\"\npath = \"rel\"\n", "sandbox.toml: share #1: path must be absolute (got \"rel\").")]
    [InlineData("[[share]]\nhost = \".git\"\npath = \"/work/x\"\n", "sandbox.toml: share #1: path /work/x is reserved")]
    [InlineData("[[share]]\nhost = \".git\"\npath = \"/config\"\n", "sandbox.toml: share #1: path /config is reserved")]
    [InlineData("[[share]]\nhost = \".git\"\n[[share]]\nhost = \".git/hooks\"\npath = \"/shares/.git\"\n", "sandbox.toml: share #2: path /shares/.git is used twice.")]
    [InlineData("[[share]]\nhost = \".git\"\nro = true\n", "sandbox.toml: unknown key 'share.ro' (known: host, path, readonly).")]
    [InlineData("share = { host = \".git\" }\n", "sandbox.toml: share must be written as [[share]] blocks (got a table).")]
    [InlineData("[project]\nworkspace = \n", "sandbox.toml is not valid TOML (line 2, column 13):")]
    public void Invalid_sandbox_toml(string toml, string message)
    {
        WriteConf(toml);
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
    public void Old_sandbox_yml_is_reported_with_the_key_mapping()
    {
        File.Delete(F.ConfFile);
        File.WriteAllText(F.OldConfFile, "x-sandbox:\n  workspace_mode: rw\n");
        var r = Run("up", "--no-open");
        AssertExit(r, 1);
        AssertCalls(r);
        AssertOutput(r, "sandbox.yml is no longer read. Move its settings to sandbox.toml");
        AssertOutput(r, "→ [tools] <tool>");
    }

    [Fact]
    public void Leftover_yml_files_are_ignored_with_a_warning()
    {
        File.WriteAllText(F.OldConfFile, "x-sandbox:\n  workspace_mode: ro\n");
        File.WriteAllText(F.OldLocalConfFile, "x-sandbox:\n  workspace_mode: ro\n");
        var r = Run("status");
        AssertExit(r, 0);
        AssertOutput(r, "sandbox.yml is ignored");
        AssertOutput(r, "sandbox.local.yml is no longer read");
        Assert.Contains("read_only: false}", File.ReadAllText(F.GeneratedFile));
    }

    [Fact]
    public void Missing_sandbox_toml()
    {
        File.Delete(F.ConfFile);
        var r = Run("status");
        AssertExit(r, 1);
        AssertCalls(r);
        AssertOutput(r, "which is not a .sandbox folder (with sandbox.toml).");

        // Without the launcher's SANDBOX_DIR, the search upwards finds nothing either.
        r = Run(["status"], cwd: F.ProjectDir, sandboxDir: "");
        AssertExit(r, 1);
        AssertCalls(r);
        AssertOutput(r, "Could not find the .sandbox folder (with sandbox.toml).");
    }
}
