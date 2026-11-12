using System.Text.RegularExpressions;

namespace Sandbox.Tests;

/// Every command, run against a healthy fake docker: the exact docker calls, in order.
public sealed class CommandTests(SandboxFixture f) : SandboxTest(f)
{
    const string ClaudeScript =
        "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set INSTALL_CLAUDE_CODE in sandbox.yml and rebuild.\"; exit 1; }; exec claude \"$@\"";

    Call UpWait => Compose("up", "-d", "--wait", "--wait-timeout", "300");
    Call Port => Compose("port", "gateway", "3001");
    Call Ps => Compose("ps", "-a", "--format", "json");

    // --- lifecycle --------------------------------------------------------------

    [Theory]
    [InlineData("up")]
    [InlineData("start")]
    public void Up_builds_starts_and_prints_the_url(string cmd)
    {
        var r = Run(cmd, "--no-open");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, CollisionCheck(), ComposeInteractive("build"), UpWait, Port);
        AssertOutput(r, Url);
        AssertOutput(r, F.WorkDir);
        Assert.True(File.Exists(F.GeneratedFile));
    }

    [Fact]
    public void Up_opens_the_browser()
    {
        Assert.SkipWhen(OperatingSystem.IsWindows(), "Windows opens URLs through ShellExecute.");
        var r = Run(["up"], waitFor: "xdg-open");
        AssertExit(r, 0);
        AssertCalls(
            r,
            DockerInfo,
            CollisionCheck(),
            ComposeInteractive("build"),
            UpWait,
            Port,
            new Call("xdg-open", [Url])
        );
    }

    [Theory]
    [InlineData("stop")]
    [InlineData("down")]
    [InlineData("restart")]
    public void Stop_down_restart(string cmd)
    {
        var r = Run(cmd);
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose(cmd));
        AssertOutput(r, ": done.");
    }

    [Theory]
    [InlineData("status")]
    [InlineData("ps")]
    public void Status_lists_services_and_url(string cmd)
    {
        var r = Run(cmd);
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Ps, Port);
        foreach (var s in new[] { "desktop", "gateway", "proxy", "healthy", "127.0.0.1:49153→3001" })
            AssertOutput(r, s);
        AssertOutput(r, $"URL: {Url}");
    }

    [Fact]
    public void Status_without_containers()
    {
        var r = Run(["status"], [new("ps -a --format json", "")]);
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Ps);
        AssertOutput(r, "No containers for 'demo-project'");
    }

    [Fact]
    public void Open_asks_for_the_port_and_opens_the_browser()
    {
        Assert.SkipWhen(OperatingSystem.IsWindows(), "Windows opens URLs through ShellExecute.");
        var r = Run(["open"], waitFor: "xdg-open");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Port, new Call("xdg-open", [Url]));
    }

    [Fact]
    public void Shell_execs_bash_in_the_project_folder()
    {
        var r = Run("shell");
        AssertExit(r, 0);
        AssertCalls(
            r,
            DockerInfo,
            ComposeInteractive("exec", "-u", "abc", "-w", F.WorkDir, "desktop", "bash", "-l")
        );
    }

    [Theory]
    [InlineData]
    [InlineData("--resume", "-p", "hi there")]
    [InlineData("--help")]
    [InlineData("-v", "help", "--", "x y")]
    public void Claude_forwards_all_arguments_verbatim(params string[] args)
    {
        var r = Run(["claude", .. args]);
        AssertExit(r, 0);
        AssertCalls(
            r,
            DockerInfo,
            ComposeInteractive(
                ["exec", "-u", "abc", "-w", F.WorkDir, "desktop", "sh", "-c", ClaudeScript, "claude", .. args]
            )
        );
    }

    [Fact]
    public void Logs_follow_by_default()
    {
        var r = Run("logs");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, ComposeInteractive("logs", "--tail=100", "-f"));
    }

    [Fact]
    public void Logs_with_services_tail_and_no_follow()
    {
        var r = Run("logs", "proxy", "desktop", "-n", "5", "--no-follow");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, ComposeInteractive("logs", "--tail=5", "proxy", "desktop"));
    }

    // --- network ----------------------------------------------------------------

    [Fact]
    public void Allow_uncomments_an_entry_and_reloads_the_proxy()
    {
        Assert.Contains("# api.nuget.org", File.ReadAllLines(F.AllowlistFile));
        var r = Run("allow", "api.nuget.org", "new.example.com", "claude.ai");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("restart", "proxy"));
        var lines = File.ReadAllLines(F.AllowlistFile);
        Assert.Contains("api.nuget.org", lines);
        Assert.DoesNotContain("# api.nuget.org", lines);
        Assert.Equal("new.example.com", lines[^1]);
        Assert.Single(lines, "claude.ai");
        AssertOutput(r, "enabled existing entry");
        AssertOutput(r, "added");
        AssertOutput(r, "already allowed");
        AssertOutput(r, "Proxy reloaded.");
    }

    [Fact]
    public void Deny_comments_an_entry_and_reloads_the_proxy()
    {
        var r = Run("deny", "claude.ai", "not-there.example.com");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("restart", "proxy"));
        var lines = File.ReadAllLines(F.AllowlistFile);
        Assert.Contains("# claude.ai", lines);
        Assert.DoesNotContain("claude.ai", lines);
        AssertOutput(r, "denied");
        AssertOutput(r, "was not allowed");
    }

    [Fact]
    public void Reload_restarts_the_proxy()
    {
        var r = Run("reload");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("restart", "proxy"));
    }

    [Fact]
    public void Blocked_counts_refused_hosts_from_the_proxy_log()
    {
        var log = string.Join(
            "\n",
            "NOTICE    Nov 11 10:00:00 [1]: Proxying refused on filtered domain \"evil.example.com\"",
            "NOTICE    Nov 11 10:00:01 [1]: Proxying refused on filtered domain \"evil.example.com\"",
            "NOTICE    Nov 11 10:00:02 [1]: Proxying refused on filtered url \"api.nuget.org\"",
            "CONNECT   Nov 11 10:00:03 [1]: Connect (file descriptor 7): 172.18.0.3"
        );
        var r = Run(["blocked"], [new("logs --no-log-prefix proxy", log)]);
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("logs", "--no-log-prefix", "proxy"));
        Assert.Matches(@"2\s*│\s*evil\.example\.com\s*│\s*blocked", r.StdOut);
        Assert.Matches(@"1\s*│\s*api\.nuget\.org\s*│\s*blocked", r.StdOut);
    }

    [Fact]
    public void Blocked_with_nothing_blocked()
    {
        var r = Run("blocked");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("logs", "--no-log-prefix", "proxy"));
        AssertOutput(r, "Nothing was blocked");
    }

    [Theory]
    [InlineData]
    [InlineData("-a")]
    [InlineData("--all")]
    public void Allowlist_does_not_call_docker(params string[] args)
    {
        var r = Run(["allowlist", .. args]);
        AssertExit(r, 0);
        AssertCalls(r);
        Assert.False(File.Exists(F.GeneratedFile));
        // non-interactive: one active host per line
        Assert.Contains("api.anthropic.com", r.Lines);
        Assert.DoesNotContain("api.nuget.org", r.Lines);
    }

    // --- maintenance ------------------------------------------------------------

    [Fact]
    public void Rebuild_builds_and_starts()
    {
        var r = Run("rebuild");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, ComposeInteractive("build"), UpWait);
        AssertOutput(r, "Rebuilt and running.");
    }

    [Fact]
    public void Update_pulls_and_busts_the_cache()
    {
        var r = Run("update");
        AssertExit(r, 0);
        Assert.Equal(3, r.Calls.Count);
        Assert.Equal(DockerInfo.ToString(), r.Calls[0].ToString());
        var build = r.Calls[1].Args;
        var prefix = ComposeInteractive("build", "--pull", "--build-arg").Args;
        Assert.Equal(prefix, build[..prefix.Length]);
        Assert.Single(build[prefix.Length..]);
        Assert.Matches(new Regex(@"^CACHE_BUST=\d{10,}$"), build[^1]);
        Assert.Equal(UpWait.ToString(), r.Calls[2].ToString());
    }

    [Fact]
    public void Reset_home_removes_containers_and_the_volume()
    {
        var r = Run("reset-home", "--yes");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Compose("down"), Docker("volume", "rm", "demo-project_home"));
        AssertOutput(r, "Home volume demo-project_home deleted.");
    }

    [Fact]
    public void Info_does_not_call_docker()
    {
        var r = Run("info");
        AssertExit(r, 0);
        AssertCalls(r);
        Assert.False(File.Exists(F.GeneratedFile));
        AssertOutput(r, F.WorkDir);
        AssertOutput(r, SandboxFixture.Id);
        Assert.Matches(@"dotnet\s*│\s*10\.0", r.StdOut);
    }

    [Theory]
    [InlineData("help")]
    [InlineData("--help")]
    [InlineData("help", "up")]
    [InlineData("up", "--help")]
    [InlineData("help", "logs")]
    public void Help_does_not_call_docker(params string[] args)
    {
        var r = Run(args);
        AssertExit(r, 0);
        AssertCalls(r);
        Assert.False(File.Exists(F.GeneratedFile));
        AssertOutput(r, "Usage:");
    }

    /// The shipped sandbox.yml and allowlist.txt (as currently in the repo) load without errors.
    [Fact]
    public void Repo_config_files_are_valid()
    {
        File.Copy(Path.Combine(F.RepoSandboxDir, "sandbox.yml"), F.ConfFile, overwrite: true);
        File.Copy(Path.Combine(F.RepoSandboxDir, "allowlist.txt"), F.AllowlistFile, overwrite: true);
        foreach (var args in new[] { new[] { "info" }, ["allowlist", "-a"], ["status"] })
        {
            var r = Run(args);
            AssertExit(r, 0);
            Assert.DoesNotContain("ERROR", r.Output);
        }
        Assert.Contains("api.anthropic.com", Run("allowlist").Lines);
    }

    // --- options and configuration ---------------------------------------------------

    [Fact]
    public void Verbose_prints_the_docker_commands()
    {
        var r = Run("status", "-v");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, Ps, Port);
        AssertOutput(r, "$ docker info --format {{.ServerVersion}}");
        AssertOutput(r, $"$ docker compose -p demo-project --project-directory {F.Dir} -f ");
        // the line may be wrapped to the terminal width
        Assert.Contains(" ps -a --format json", Regex.Replace(r.Output, @"\s+", " "));
    }

    [Fact]
    public void Every_compose_call_uses_only_the_generated_file()
    {
        WriteLocalConf("services:\n  desktop:\n    mem_limit: 4g\n");
        var r = Run("up", "--no-open");
        AssertExit(r, 0);
        AssertCalls(r, DockerInfo, CollisionCheck(), ComposeInteractive("build"), UpWait, Port);
        Assert.All(
            r.Calls.Where(c => c.Args[0] == "compose"),
            c => Assert.Single(c.Args, a => a == "-f")
        );
        Assert.Contains("mem_limit: 4g", File.ReadAllText(F.GeneratedFile));
    }

    [Fact]
    public void Project_name_sets_the_container_group()
    {
        WriteConf(File.ReadAllText(F.ConfFile).Replace("# project_name: myproject", "project_name: Other Name"));
        var r = Run(["up", "--no-open"], [new("ps -a --filter", "")]);
        AssertExit(r, 0);
        Assert.Equal(CollisionCheck("other-name").ToString(), r.Calls[1].ToString());
        Assert.All(r.Calls.Where(c => c.Args[0] == "compose"), c => Assert.Equal("other-name", c.Args[2]));

        r = Run("reset-home", "-y");
        AssertExit(r, 0);
        Assert.Equal(Docker("volume", "rm", "other-name_home").ToString(), r.Calls[^1].ToString());
    }
}
