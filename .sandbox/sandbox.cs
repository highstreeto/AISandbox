// AI Sandbox control app (Windows, Linux, macOS, WSL) — a .NET 10 file-based app.
// Lives in <project>/.sandbox/ and is started through the launchers next to it, from any directory:
//   .sandbox/sandbox up            (sh: Linux, macOS, WSL, Git Bash)
//   .sandbox\sandbox.cmd up        (Windows cmd / PowerShell)
// They run `dotnet run --file sandbox.cs -- <args>`, so every argument reaches this app untouched.
// It never changes your current directory (all paths are absolute).
#:package System.CommandLine@2.0.15
#:package YamlDotNet@18.1.0
#:package Spectre.Console@0.57.2
#:property Nullable=enable
#:property PublishAot=false

using System.CommandLine;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Spectre.Console;
using YamlDotNet.Core;
using YamlDotNet.RepresentationModel;

try
{
    Ui.Init();

    if (args.Length == 1 && args[0] == "help")
        args = [.. args[1..], "--help"]; // `help [command]`

    // `claude` forwards everything verbatim (including --help, --resume, -p, ...) to the CLI in the sandbox.
    if (args.Length == 1 && args[0] == "claude")
        return Cli.Guard(() => Commands.Claude(Sandbox.Load(), args[1..]));

    return await Cli.Build().Parse(args).InvokeAsync();
}
catch (Exception ex)
{
    AnsiConsole.WriteException(ex);
    return 1;
}

// =============================================================================
//  Command line
// =============================================================================
static class Cli
{
    private static readonly Option<bool> Verbose = new("--verbose", "-v")
    {
        Description = "Print the docker commands that are run",
        Recursive = true,
    };

    public static RootCommand Build()
    {
        var root = new RootCommand(
            "AI Sandbox — a desktop for this project, reachable only through an allowlist proxy."
        );
        root.Options.Add(Verbose);

        // --- Lifecycle --------------------------------------------------------
        var up = Cmd("up", "Build (if needed), start and open the desktop in the browser", "start");
        var noOpen = new Option<bool>("--no-open") { Description = "Don't open the browser" };
        up.Options.Add(noOpen);
        up.SetAction(r => Run(r, sb => Commands.Up(sb, !r.GetValue(noOpen))));

        var stop = Cmd("stop", "Stop (everything is kept)");
        stop.SetAction(r => Run(r, sb => Commands.Compose(sb, "Stopping", "stop")));

        var down = Cmd("down", "Remove containers (home volume and project are kept)");
        down.SetAction(r => Run(r, sb => Commands.Compose(sb, "Removing containers", "down")));

        var restart = Cmd("restart", "Restart");
        restart.SetAction(r => Run(r, sb => Commands.Compose(sb, "Restarting", "restart")));

        var status = Cmd("status", "Container status and URL", "ps");
        status.SetAction(r => Run(r, Commands.Status));

        var open = Cmd("open", "Open the desktop in the browser");
        open.SetAction(r => Run(r, Commands.Open));

        var shell = Cmd("shell", "Bash inside the sandbox (in the project folder)");
        shell.SetAction(r =>
            Run(
                r,
                sb =>
                    sb.ComposeInteractive(
                        "exec",
                        "-u",
                        "abc",
                        "-w",
                        sb.WorkDir,
                        "desktop",
                        "bash",
                        "-l"
                    )
            )
        );

        var claude = Cmd(
            "claude",
            "Claude Code CLI inside the sandbox (needs INSTALL_CLAUDE_CODE); all arguments are passed on"
        );
        claude.Arguments.Add(
            new Argument<string[]>("args")
            {
                Arity = ArgumentArity.ZeroOrMore,
                Description = "Arguments for claude",
            }
        );
        claude.TreatUnmatchedTokensAsErrors = false; // handled before parsing; declared for --help

        var logs = Cmd("logs", "Follow logs (proxy, gateway, desktop)");
        var services = new Argument<string[]>("service")
        {
            Arity = ArgumentArity.ZeroOrMore,
            Description = "proxy | gateway | desktop (default: all)",
        };
        var tail = new Option<int>("--tail", "-n")
        {
            Description = "Lines to show from the end",
            DefaultValueFactory = _ => 100,
        };
        var noFollow = new Option<bool>("--no-follow")
        {
            Description = "Print and exit instead of following",
        };
        logs.Arguments.Add(services);
        logs.Options.Add(tail);
        logs.Options.Add(noFollow);
        logs.SetAction(r =>
            Run(
                r,
                sb =>
                    Commands.Logs(
                        sb,
                        r.GetValue(services) ?? [],
                        r.GetValue(tail),
                        !r.GetValue(noFollow)
                    )
            )
        );

        // --- Network ----------------------------------------------------------
        var hosts = new Argument<string[]>("host")
        {
            Arity = ArgumentArity.OneOrMore,
            Description = "Host name, wildcards like *.example.com allowed",
        };

        var allow = Cmd("allow", "Allow host(s) and reload the proxy");
        allow.Arguments.Add(hosts);
        allow.SetAction(r => Run(r, sb => Commands.Allow(sb, r.GetValue(hosts)!)));

        var deny = Cmd("deny", "Remove host(s) and reload the proxy");
        deny.Arguments.Add(hosts);
        deny.SetAction(r => Run(r, sb => Commands.Deny(sb, r.GetValue(hosts)!)));

        var allowlist = Cmd("allowlist", "Show active allowlist entries");
        var all = new Option<bool>("--all", "-a")
        {
            Description = "Also show the commented-out (available) entries",
        };
        allowlist.Options.Add(all);
        allowlist.SetAction(r =>
            Run(r, sb => Commands.ShowAllowlist(sb, r.GetValue(all)), docker: false)
        );

        var blocked = Cmd("blocked", "Show hosts the sandbox tried to reach but were blocked");
        blocked.SetAction(r => Run(r, Commands.Blocked));

        var reload = Cmd("reload", "Reload the proxy after editing allowlist.txt");
        reload.SetAction(r =>
            Run(
                r,
                sb =>
                {
                    Commands.ReloadProxy(sb);
                    return 0;
                }
            )
        );

        // --- Maintenance ------------------------------------------------------
        var rebuild = Cmd("rebuild", "Rebuild after changing INSTALL_* in sandbox.yml");
        rebuild.SetAction(r => Run(r, sb => Commands.Rebuild(sb, pull: false)));

        var update = Cmd("update", "Rebuild with the newest versions of all tools");
        update.SetAction(r => Run(r, sb => Commands.Rebuild(sb, pull: true)));

        var resetHome = Cmd("reset-home", "Delete the persistent home (logins, IDE settings)");
        var yes = new Option<bool>("--yes", "-y") { Description = "Don't ask for confirmation" };
        resetHome.Options.Add(yes);
        resetHome.SetAction(r => Run(r, sb => Commands.ResetHome(sb, r.GetValue(yes))));

        var info = Cmd("info", "Show the project identity, settings and configured tools");
        info.SetAction(r => Run(r, Commands.Info, docker: false));

        var help = Cmd("help", "Show help (for a command: help <command>)");
        help.Arguments.Add(new Argument<string[]>("command") { Arity = ArgumentArity.ZeroOrMore }); // rewritten to --help before parsing

        return root;

        Command Cmd(string name, string description, params string[] aliases)
        {
            var c = new Command(name, description);
            foreach (var a in aliases)
                c.Aliases.Add(a);

            root.Subcommands.Add(c);
            return c;
        }
    }

    /// Load the project, (optionally) check Docker + regenerate the compose file, then run the command.
    static int Run(ParseResult r, Func<Sandbox, int> action, bool docker = true) =>
        Guard(() =>
        {
            var sb = Sandbox.Load(r.GetValue(Verbose));
            if (docker)
            {
                sb.NeedDocker();
                sb.Generate();
            }
            return action(sb);
        });

    public static int Guard(Func<int> action)
    {
        try
        {
            return action();
        }
        catch (SandboxException e)
        {
            Ui.Error(e.Message);
            return 1;
        }
        catch (YamlException e)
        {
            Ui.Error(
                $"sandbox.yml is not valid YAML (line {e.Start.Line}, column {e.Start.Column}): {e.Message}"
            );
            return 1;
        }
    }
}

// =============================================================================
//  Commands
// =============================================================================
static class Commands
{
    public static int Up(Sandbox sb, bool openBrowser)
    {
        sb.CheckCollision();
        Build(sb, pull: false);
        StartAndWait(sb, tolerateUnhealthy: true);
        Thread.Sleep(2000);
        var url =
            sb.UiUrl()
            ?? throw new SandboxException(
                "The sandbox started, but its UI port could not be determined."
            );
        AnsiConsole.Write(
            new Panel(
                new Rows(
                    new Markup($"[bold]Project[/]   {Markup.Escape(sb.Name)}"),
                    new Markup($"[bold]URL[/]       [link={url}]{url}[/]"),
                    new Markup($"[bold]Mounted[/]   {Markup.Escape(sb.WorkDir)}"),
                    new Markup("[dim]Accept the self-signed certificate in the browser.[/]")
                )
            )
                .Header("[green] Sandbox running [/]")
                .Border(BoxBorder.Rounded)
                .BorderColor(Color.Green)
        );
        if (openBrowser)
            Ui.OpenBrowser(url);
        return 0;
    }

    public static int Rebuild(Sandbox sb, bool pull)
    {
        Build(sb, pull);
        StartAndWait(sb, tolerateUnhealthy: false);
        Ui.Success("Rebuilt and running.");
        return 0;
    }

    /// Image build: docker's own progress output is shown as-is (it can take minutes the first time).
    static void Build(Sandbox sb, bool pull)
    {
        AnsiConsole.Write(
            new Rule(
                pull
                    ? "[yellow]Building images with the newest tools[/]"
                    : "[yellow]Building images[/]"
            ).LeftJustified()
        );
        var a = new List<string> { "build" };
        if (pull)
            a.AddRange([
                "--pull",
                "--build-arg",
                $"CACHE_BUST={DateTimeOffset.UtcNow.ToUnixTimeSeconds()}",
            ]);
        if (sb.ComposeInteractive([.. a]) != 0)
            throw new SandboxException(
                "Building the sandbox images failed (see the output above)."
            );
        AnsiConsole.Write(new Rule().RuleStyle("dim"));
    }

    /// `up -d --wait` in the background while a live table shows each service's state and health.
    static void StartAndWait(Sandbox sb, bool tolerateUnhealthy)
    {
        var upTask = Task.Run(() => sb.Compose("up", "-d", "--wait", "--wait-timeout", "300"));
        var started = Stopwatch.StartNew();

        Table Render()
        {
            var t = new Table()
                .Border(TableBorder.Rounded)
                .Title(
                    $"[bold]Starting '{Markup.Escape(sb.Name)}'[/] [dim]({started.Elapsed:mm\\:ss})[/]"
                )
                .AddColumn("Service")
                .AddColumn("State")
                .AddColumn("Health");
            var ps = sb.Services();
            if (ps.Count == 0)
                t.AddRow("[dim]creating containers…[/]", "", "");
            foreach (var s in ps)
                t.AddRow(Markup.Escape(s.Service), Ui.State(s.State), Ui.Health(s.Health, s.State));
            return t;
        }

        if (Ui.Interactive)
            AnsiConsole
                .Live(Render())
                .AutoClear(false)
                .Start(ctx =>
                {
                    while (!upTask.Wait(1000))
                        ctx.UpdateTarget(Render());
                    ctx.UpdateTarget(Render());
                });
        else
            upTask.Wait();

        var up = upTask.Result;
        if (up.ExitCode == 0)
            return;
        if (
            tolerateUnhealthy
            && sb.Services().Any(s => s.Service == "desktop" && s.State == "running")
        )
        {
            Ui.Warn(
                "The desktop isn't reporting ready yet; opening anyway (reload the page if it's blank)."
            );
            return;
        }
        Ui.ShowOutput("docker compose up", up);
        throw new SandboxException("Starting the sandbox failed (see the output above).");
    }

    public static int Compose(Sandbox sb, string what, params string[] args)
    {
        var r = Ui.Spin($"{what} '{sb.Name}'…", () => sb.Compose(args));
        if (r.ExitCode != 0)
        {
            Ui.ShowOutput($"docker compose {string.Join(' ', args)}", r);
            throw new SandboxException($"{what} failed.");
        }
        Ui.Success($"{what} '{sb.Name}': done.");
        return 0;
    }

    public static int Status(Sandbox sb)
    {
        var ps = sb.Services();
        if (ps.Count == 0)
        {
            AnsiConsole.MarkupLine(
                $"[yellow]No containers for '{Markup.Escape(sb.Name)}'.[/] Start the sandbox with: [bold]{Ui.Self} up[/]"
            );
            return 0;
        }
        var t = new Table()
            .Border(TableBorder.Rounded)
            .Title($"[bold]{Markup.Escape(sb.Name)}[/]")
            .AddColumn("Service")
            .AddColumn("State")
            .AddColumn("Health")
            .AddColumn("Status")
            .AddColumn("Ports");
        foreach (var s in ps)
            t.AddRow(
                Markup.Escape(s.Service),
                Ui.State(s.State),
                Ui.Health(s.Health, s.State),
                Markup.Escape(s.Status),
                Markup.Escape(s.Ports)
            );
        AnsiConsole.Write(t);
        if (sb.UiUrl() is { } url)
            AnsiConsole.MarkupLine($"[bold]URL:[/] [link={url}]{url}[/]");
        return 0;
    }

    public static int Open(Sandbox sb)
    {
        var url =
            sb.UiUrl()
            ?? throw new SandboxException(
                $"The sandbox is not running (start it with: {Ui.Self} up)."
            );
        Ui.OpenBrowser(url);
        return 0;
    }

    public static int Claude(Sandbox sb, string[] args)
    {
        sb.NeedDocker();
        sb.Generate();
        const string script =
            "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set INSTALL_CLAUDE_CODE in sandbox.yml and rebuild.\"; exit 1; }; exec claude \"$@\"";
        return sb.ComposeInteractive([
            "exec",
            "-u",
            "abc",
            "-w",
            sb.WorkDir,
            "desktop",
            "sh",
            "-c",
            script,
            "claude",
            .. args,
        ]);
    }

    public static int Logs(Sandbox sb, string[] services, int tail, bool follow)
    {
        var a = new List<string> { "logs", $"--tail={tail}" };
        if (follow)
            a.Add("-f");
        a.AddRange(services);
        return sb.ComposeInteractive([.. a]);
    }

    // --- Network --------------------------------------------------------------
    public static int Allow(Sandbox sb, string[] hosts)
    {
        var list = Allowlist.Load(sb.AllowlistPath);
        var t = ResultTable();
        foreach (var h in hosts.Select(Allowlist.ValidateHost))
            t.AddRow(
                Markup.Escape(h),
                list.Allow(h) switch
                {
                    AllowResult.AlreadyAllowed => "[dim]already allowed[/]",
                    AllowResult.Uncommented => "[green]allowed[/] [dim](enabled existing entry)[/]",
                    _ => "[green]allowed[/] [dim](added)[/]",
                }
            );
        list.Save();
        AnsiConsole.Write(t);
        ReloadProxy(sb);
        return 0;
    }

    public static int Deny(Sandbox sb, string[] hosts)
    {
        var list = Allowlist.Load(sb.AllowlistPath);
        var t = ResultTable();
        foreach (var h in hosts.Select(Allowlist.ValidateHost))
            t.AddRow(Markup.Escape(h), list.Deny(h) ? "[red]denied[/]" : "[dim]was not allowed[/]");
        list.Save();
        AnsiConsole.Write(t);
        ReloadProxy(sb);
        return 0;
    }

    static Table ResultTable() =>
        new Table().Border(TableBorder.Rounded).AddColumn("Host").AddColumn("Result");

    public static int ShowAllowlist(Sandbox sb, bool all)
    {
        var entries = Allowlist
            .Load(sb.AllowlistPath)
            .Entries()
            .Where(e => all || e.Active)
            .ToList();
        if (!Ui.Interactive) // plain output for scripts: one active host per line
        {
            foreach (var e in entries.Where(e => e.Active))
                Console.WriteLine(e.Host);
            return 0;
        }
        var t = new Table().Border(TableBorder.Rounded).AddColumn("Host").AddColumn("Group");
        if (all)
            t.AddColumn("State").AddColumn("Note");
        foreach (var e in entries)
        {
            var host = e.Active
                ? $"[bold]{Markup.Escape(e.Host)}[/]"
                : $"[dim]{Markup.Escape(e.Host)}[/]";
            if (all)
                t.AddRow(
                    host,
                    Markup.Escape(e.Group),
                    e.Active ? "[green]allowed[/]" : "[dim]off[/]",
                    $"[dim]{Markup.Escape(e.Note)}[/]"
                );
            else
                t.AddRow(host, Markup.Escape(e.Group));
        }
        t.Caption(
            $"[dim]{entries.Count(e => e.Active)} allowed{(all ? $", {entries.Count(e => !e.Active)} available" : "")} · {Markup.Escape(sb.AllowlistPath)}[/]"
        );
        AnsiConsole.Write(t);
        return 0;
    }

    public static int Blocked(Sandbox sb)
    {
        var logs = Ui.Spin(
            "Reading proxy logs…",
            () => sb.Compose("logs", "--no-log-prefix", "proxy")
        );
        var rx = new Regex("refused on filtered (?:domain|url) \"([^\"]+)\"");
        var hits = rx.Matches(logs.StdOut + "\n" + logs.StdErr)
            .Select(m => m.Groups[1].Value)
            .GroupBy(h => h, StringComparer.OrdinalIgnoreCase)
            .Select(g => (Host: g.Key, Count: g.Count()))
            .OrderByDescending(x => x.Count)
            .ThenBy(x => x.Host)
            .ToList();
        if (hits.Count == 0)
        {
            Ui.Success("Nothing was blocked (since the proxy last started).");
            return 0;
        }

        var allowed = Allowlist.Load(sb.AllowlistPath);
        var t = new Table()
            .Border(TableBorder.Rounded)
            .AddColumn(new TableColumn("Count").RightAligned())
            .AddColumn("Host")
            .AddColumn("Now");
        foreach (var (host, count) in hits)
            t.AddRow(
                count.ToString(),
                Markup.Escape(host),
                allowed.IsAllowed(host) ? "[green]allowed[/]" : "[red]blocked[/]"
            );
        AnsiConsole.Write(t);
        AnsiConsole.MarkupLine($"[dim](allow one with: {Ui.Self} allow <host>)[/]");
        return 0;
    }

    public static void ReloadProxy(Sandbox sb)
    {
        var r = Ui.Spin("Reloading the proxy…", () => sb.Compose("restart", "proxy"));
        if (r.ExitCode != 0)
        {
            Ui.ShowOutput("docker compose restart proxy", r);
            throw new SandboxException("Could not restart the proxy.");
        }
        Ui.Success("Proxy reloaded.");
    }

    // --- Maintenance ----------------------------------------------------------
    public static int ResetHome(Sandbox sb, bool yes)
    {
        if (!yes)
        {
            if (!Ui.Interactive)
                throw new SandboxException(
                    "Refusing to delete the home volume without confirmation (use --yes)."
                );
            if (
                !AnsiConsole.Confirm(
                    $"Delete the home volume of [bold]{Markup.Escape(sb.Name)}[/] (logins, IDE settings)?",
                    defaultValue: false
                )
            )
                return 0;
        }
        Compose(sb, "Removing containers", "down");
        var vol = $"{sb.Name}_home";
        var r = Ui.Spin(
            $"Removing volume {vol}…",
            () => Proc.Capture(sb.Verbose, "docker", "volume", "rm", vol)
        );
        if (r.ExitCode != 0)
        {
            Ui.ShowOutput("docker volume rm", r);
            throw new SandboxException($"Could not remove volume {vol}.");
        }
        Ui.Success($"Home volume {vol} deleted.");
        return 0;
    }

    public static int Info(Sandbox sb)
    {
        var id = new Grid().AddColumn(new GridColumn().NoWrap().PadRight(3)).AddColumn();
        void Row(string k, string v) => id.AddRow($"[bold]{k}[/]", Markup.Escape(v));
        Row("Project", sb.Name);
        Row("Project folder", sb.ProjectDir);
        Row("Mounted at", sb.WorkDir);
        Row("ID", sb.Id);
        Row("Workspace", sb.Settings.ReadOnly ? "read-only" : "read-write");
        Row("Protect .git", sb.Settings.ProtectGit ? "yes" : "no");
        Row("UI port", sb.Settings.UiPort?.ToString() ?? "random free port");
        Row("Local overrides", File.Exists(sb.LocalConfPath) ? "sandbox.local.yml" : "none");
        AnsiConsole.Write(new Panel(id).Header("[bold] Sandbox [/]").Border(BoxBorder.Rounded));

        var tools = new Table()
            .Border(TableBorder.Rounded)
            .Title("[bold]Tools (INSTALL_*)[/]")
            .AddColumn("Tool")
            .AddColumn("Value");
        foreach (var (k, v) in sb.Settings.Install)
        {
            var off = v is "" or "false";
            tools.AddRow(
                Markup.Escape(k["INSTALL_".Length..].ToLowerInvariant().Replace('_', '-')),
                off ? "[dim]not installed[/]"
                    : v == "true" ? "[green]default version[/]"
                    : $"[green]{Markup.Escape(v)}[/]"
            );
        }
        AnsiConsole.Write(tools);

        if (sb.Settings.Shares.Count > 0)
        {
            var shares = new Table()
                .Border(TableBorder.Rounded)
                .Title("[bold]Extra volumes[/]")
                .AddColumn("Volume");
            foreach (var s in sb.Settings.Shares)
                shares.AddRow(Markup.Escape(s));
            AnsiConsole.Write(shares);
        }
        return 0;
    }
}

// =============================================================================
//  Project identity, configuration and docker compose
// =============================================================================
sealed class SandboxException(string message) : Exception(message);

sealed record Settings(
    string? ProjectName,
    bool ReadOnly,
    bool ProtectGit,
    int? UiPort,
    List<(string Key, string Value)> Install,
    List<string> Shares
);

sealed record ServiceState(
    string Service,
    string State,
    string Health,
    string Status,
    string Ports
);

sealed record Sandbox(
    string Dir,
    string ProjectDir,
    string Id,
    string Name,
    string WorkDir,
    Settings Settings,
    bool Verbose
)
{
    public string ConfPath => Path.Combine(Dir, "sandbox.yml");
    public string LocalConfPath => Path.Combine(Dir, "sandbox.local.yml"); // optional, machine-specific, not committed
    public string AllowlistPath => Path.Combine(Dir, "allowlist.txt");
    string StateDir => Path.Combine(Dir, ".state");
    string GenPath => Path.Combine(StateDir, "compose.generated.yml");

    public static Sandbox Load(bool verbose = false)
    {
        var dir = LocateSandboxDir();
        var projectDir = Path.GetDirectoryName(dir)!;
        var stateDir = Path.Combine(dir, ".state");
        Directory.CreateDirectory(stateDir);

        var idFile = Path.Combine(stateDir, "id"); // created once: stable even if the project is moved
        var id = File.Exists(idFile) ? File.ReadAllText(idFile).Trim() : "";
        if (id.Length == 0)
        {
            id = Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(projectDir)))[..8];
            File.WriteAllText(idFile, id + "\n");
        }

        var settings = Config.Read(
            Path.Combine(dir, "sandbox.yml"),
            Path.Combine(dir, "sandbox.local.yml")
        );
        var baseName = Sanitize(Path.GetFileName(projectDir));
        return new Sandbox(
            Dir: dir,
            ProjectDir: projectDir,
            Id: id,
            Settings: settings,
            Verbose: verbose,
            Name: Sanitize(
                string.IsNullOrWhiteSpace(settings.ProjectName) ? baseName : settings.ProjectName
            ),
            WorkDir: $"/work/{baseName}-{id}"
        );
    }

    /// The folder this .cs file lives in; falls back to searching upwards from the current directory.
    static string LocateSandboxDir()
    {
        static bool IsSandbox(string? d) =>
            d != null
            && File.Exists(Path.Combine(d, "sandbox.yml"))
            && Directory.Exists(Path.Combine(d, "engine"));

        if (AppContext.GetData("EntryPointFileDirectoryPath") is string ep && IsSandbox(ep))
            return Physical(ep);
        for (var d = new DirectoryInfo(Environment.CurrentDirectory); d != null; d = d.Parent)
        {
            if (IsSandbox(d.FullName))
                return Physical(d.FullName);
            var sub = Path.Combine(d.FullName, ".sandbox");
            if (IsSandbox(sub))
                return Physical(sub);
        }
        throw new SandboxException(
            "Could not find the .sandbox folder (with sandbox.yml and engine/)."
        );
    }

    static string Physical(string path) // like `pwd -P`: resolve a symlinked folder
    {
        var full = Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
        try
        {
            return new DirectoryInfo(full).ResolveLinkTarget(returnFinalTarget: true)?.FullName
                ?? full;
        }
        catch (IOException)
        {
            return full;
        }
    }

    public static string Sanitize(string s)
    {
        s = Regex.Replace(s.ToLowerInvariant(), "[^a-z0-9_-]+", "-");
        s = Regex.Replace(Regex.Replace(s, "^[-_]+", ""), "-+$", "");
        return s.Length > 0 ? s : "sandbox";
    }

    // --- docker ---------------------------------------------------------------
    List<string> ComposeArgs(IEnumerable<string> args)
    {
        var a = new List<string>
        {
            "compose",
            "-p",
            Name,
            "--project-directory",
            Dir,
            "-f",
            Path.Combine(Dir, "engine", "compose.yml"),
            "-f",
            GenPath,
            "-f",
            ConfPath,
        };
        if (File.Exists(LocalConfPath))
            a.AddRange(["-f", LocalConfPath]);
        a.AddRange(args);
        return a;
    }

    /// Run docker compose with captured output (for spinners, tables and error panels).
    public ProcResult Compose(params string[] args) =>
        Proc.Capture(Verbose, "docker", [.. ComposeArgs(["--ansi", "never", .. args])]);

    /// Run docker compose attached to this terminal (build output, shell, logs, claude).
    public int ComposeInteractive(params string[] args) =>
        Proc.Interactive(Verbose, "docker", [.. ComposeArgs(args)]);

    public void NeedDocker()
    {
        if (Proc.Which("docker") == null)
            throw new SandboxException("Docker is not installed.");
        var r = Ui.Spin(
            "Checking Docker…",
            () => Proc.Capture(Verbose, "docker", "info", "--format", "{{.ServerVersion}}")
        );
        if (r.ExitCode != 0)
            throw new SandboxException(
                "Docker is not running. Start Docker Desktop (or the Docker service) and try again."
            );
    }

    public void CheckCollision()
    {
        var r = Proc.Capture(
            Verbose,
            "docker",
            "ps",
            "-a",
            "--filter",
            $"label=com.docker.compose.project={Name}",
            "--filter",
            "label=sandbox.id",
            "--format",
            "{{.Label \"sandbox.id\"}}"
        );
        if (r.Lines().Any(l => l != Id))
            throw new SandboxException(
                $"Another project already uses the container group '{Name}'. Set x-sandbox.project_name in sandbox.yml."
            );
    }

    public string? UiUrl()
    {
        var r = Compose("port", "gateway", "3001");
        var p = r.ExitCode == 0 ? r.Lines().FirstOrDefault() : null;
        return string.IsNullOrEmpty(p) || p.EndsWith(":0")
            ? null
            : $"https://localhost:{p[(p.LastIndexOf(':') + 1)..]}";
    }

    public List<ServiceState> Services()
    {
        var r = Compose("ps", "-a", "--format", "json");
        var list = new List<ServiceState>();
        if (r.ExitCode != 0 || string.IsNullOrWhiteSpace(r.StdOut))
            return list;

        // Newer compose prints one JSON object per line, older versions a single array.
        var text = r.StdOut.Trim();
        var docs = text.StartsWith('[')
            ? [text]
            : text.Split(
                '\n',
                StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries
            );
        foreach (var d in docs)
        {
            using var doc = JsonDocument.Parse(d);
            var items =
                doc.RootElement.ValueKind == JsonValueKind.Array
                    ? doc.RootElement.EnumerateArray().ToList()
                    : [doc.RootElement];
            foreach (var e in items)
            {
                string Str(string k) =>
                    e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.String
                        ? v.GetString() ?? ""
                        : "";
                var ports =
                    e.TryGetProperty("Publishers", out var pub)
                    && pub.ValueKind == JsonValueKind.Array
                        ? string.Join(
                            ", ",
                            pub.EnumerateArray()
                                .Where(x =>
                                    x.TryGetProperty("PublishedPort", out var pp)
                                    && pp.GetInt32() > 0
                                )
                                .Select(x =>
                                    $"{x.GetProperty("URL").GetString()}:{x.GetProperty("PublishedPort").GetInt32()}→{x.GetProperty("TargetPort").GetInt32()}"
                                )
                                .Distinct()
                        )
                        : "";
                list.Add(
                    new ServiceState(
                        Str("Service"),
                        Str("State"),
                        Str("Health"),
                        Str("Status"),
                        ports
                    )
                );
            }
        }
        return list.OrderBy(s => s.Service).ToList();
    }

    // --- generated compose override -------------------------------------------
    public void Generate()
    {
        var (uid, gid) = Native.UserIds();
        var tz = TimeZone();

        var env = new YamlMappingNode
        {
            { "SANDBOX_WORKDIR", Y.Str(WorkDir) },
            { "PUID", Y.Str(uid.ToString()) },
            { "PGID", Y.Str(gid.ToString()) },
        };
        if (tz != null)
            env.Add("TZ", Y.Str(tz));

        var volumes = new YamlSequenceNode(
            Y.Bind(ProjectDir, WorkDir, Settings.ReadOnly),
            // The sandbox config is visible but read-only: the agent can't widen its allowlist.
            Y.Bind(Dir, $"{WorkDir}/{Path.GetFileName(Dir)}", true)
        );
        if (Settings.ProtectGit && !Settings.ReadOnly)
            foreach (var p in new[] { "config", "hooks" })
            {
                var src = Path.Combine(ProjectDir, ".git", p);
                if (File.Exists(src) || Directory.Exists(src))
                    volumes.Add(Y.Bind(src, $"{WorkDir}/.git/{p}", true));
            }

        var root = new YamlMappingNode
        {
            {
                "services",
                new YamlMappingNode
                {
                    {
                        "gateway",
                        new YamlMappingNode
                        {
                            {
                                "ports",
                                new YamlSequenceNode(Y.Str($"127.0.0.1:{Settings.UiPort}:3001"))
                            },
                        }
                    },
                    {
                        "desktop",
                        new YamlMappingNode
                        {
                            {
                                "labels",
                                new YamlMappingNode { { "sandbox.id", Y.Str(Id) } }
                            },
                            { "environment", env },
                            { "volumes", volumes },
                        }
                    },
                }
            },
        };

        using var w = new StringWriter { NewLine = "\n" };
        w.WriteLine("# Generated by .sandbox/sandbox — do not edit, change sandbox.yml instead.");
        new YamlStream(new YamlDocument(root)).Save(w, assignAnchors: false);
        var text = Regex.Replace(w.ToString().Replace("\r\n", "\n"), @"\n\.\.\.\n*$", "\n"); // drop the document-end marker
        File.WriteAllText(GenPath, text, new UTF8Encoding(false));
    }

    static string? TimeZone()
    {
        var tz = Environment.GetEnvironmentVariable("TZ");
        if (!string.IsNullOrWhiteSpace(tz))
            return tz;
        var local = TimeZoneInfo.Local;
        if (local.HasIanaId)
            return local.Id is "Local" or "" ? null : local.Id;
        return TimeZoneInfo.TryConvertWindowsIdToIanaId(local.Id, out var iana) ? iana : null;
    }
}

/// Reads the x-sandbox block (and the INSTALL_* build args) from sandbox.yml, then sandbox.local.yml.
static class Config
{
    public static Settings Read(string conf, string local)
    {
        if (!File.Exists(conf))
            throw new SandboxException($"{conf} not found.");
        var x = new Dictionary<string, string>();
        var install = new List<(string, string)>();
        var shares = new List<string>();

        foreach (var (path, isLocal) in new[] { (conf, false), (local, true) })
        {
            if (!File.Exists(path))
                continue;
            var root = Load(path);
            if (root == null)
                continue;
            if (root.Children.TryGetValue(Y.Key("x-sandbox"), out var xs))
            {
                if (xs is not YamlMappingNode map)
                    throw new SandboxException(
                        $"{Path.GetFileName(path)}: x-sandbox must be a mapping."
                    );
                foreach (var (k, v) in map.Children)
                    x[((YamlScalarNode)k).Value ?? ""] = v is YamlScalarNode s
                        ? s.Value ?? ""
                        : throw new SandboxException(
                            $"{Path.GetFileName(path)}: x-sandbox.{k} must be a single value."
                        );
            }
            if (At(root, "services", "desktop", "build", "args") is YamlMappingNode argsMap)
                foreach (var (k, v) in argsMap.Children)
                {
                    var key = ((YamlScalarNode)k).Value ?? "";
                    if (!key.StartsWith("INSTALL_"))
                        continue;
                    install.RemoveAll(t => t.Item1 == key);
                    install.Add((key, (v as YamlScalarNode)?.Value?.Trim() ?? ""));
                }
            if (At(root, "services", "desktop", "volumes") is YamlSequenceNode vols)
                foreach (var v in vols)
                {
                    var text = v switch
                    {
                        YamlScalarNode s => s.Value ?? "",
                        YamlMappingNode m when Get(m, "type") != "tmpfs" =>
                            $"{Get(m, "source")} → {Get(m, "target")}{(Get(m, "read_only") == "true" ? " (ro)" : "")}",
                        _ => "",
                    };
                    if (text.Length > 0)
                        shares.Add(isLocal ? text + "  [local]" : text);
                }
        }

        string? Opt(string k) =>
            x.TryGetValue(k, out var v) && !string.IsNullOrWhiteSpace(v) ? v.Trim() : null;

        var mode = Opt("workspace_mode") ?? "rw";
        if (mode is not ("rw" or "ro"))
            throw new SandboxException(
                $"x-sandbox.workspace_mode must be 'rw' or 'ro' (got '{mode}')."
            );
        var protect = Opt("protect_git")?.ToLowerInvariant() ?? "true";
        if (protect is not ("true" or "false"))
            throw new SandboxException(
                $"x-sandbox.protect_git must be true or false (got '{protect}')."
            );
        int? port = null;
        if (Opt("ui_port") is { } p)
            port =
                int.TryParse(p, out var n) && n is > 0 and < 65536
                    ? n
                    : throw new SandboxException(
                        $"x-sandbox.ui_port must be a port number (got '{p}')."
                    );

        return new Settings(
            Opt("project_name"),
            mode == "ro",
            protect == "true",
            port,
            install,
            shares
        );
    }

    static YamlMappingNode? Load(string path)
    {
        using var reader = new StreamReader(path);
        var ys = new YamlStream();
        try
        {
            ys.Load(reader);
        }
        catch (YamlException e)
        {
            throw new SandboxException(
                $"{Path.GetFileName(path)} is not valid YAML (line {e.Start.Line}, column {e.Start.Column}): {e.InnerException?.Message ?? e.Message}"
            );
        }
        if (ys.Documents.Count == 0)
            return null;
        return ys.Documents[0].RootNode as YamlMappingNode
            ?? throw new SandboxException($"{Path.GetFileName(path)} must be a YAML mapping.");
    }

    static YamlNode? At(YamlNode node, params string[] keys)
    {
        foreach (var k in keys)
            if (node is not YamlMappingNode m || !m.Children.TryGetValue(Y.Key(k), out node!))
                return null;
        return node;
    }

    static string Get(YamlMappingNode m, string k) =>
        m.Children.TryGetValue(Y.Key(k), out var v) && v is YamlScalarNode s ? s.Value ?? "" : "";
}

/// YAML node helpers for the generated file: strings are always quoted, so paths and ports stay strings.
static class Y
{
    public static YamlScalarNode Key(string k) => new(k);

    public static YamlScalarNode Str(string s) => new(s) { Style = ScalarStyle.DoubleQuoted };

    public static YamlScalarNode Bool(bool b) =>
        new(b ? "true" : "false") { Style = ScalarStyle.Plain };

    public static YamlMappingNode Bind(string source, string target, bool readOnly) =>
        new(
            new KeyValuePair<YamlNode, YamlNode>[]
            {
                new(Key("type"), new YamlScalarNode("bind")),
                new(Key("source"), Str(source)),
                new(Key("target"), Str(target)),
                new(Key("read_only"), Bool(readOnly)),
            }
        )
        {
            Style = YamlDotNet.Core.Events.MappingStyle.Flow,
        };
}

// =============================================================================
//  allowlist.txt
// =============================================================================
enum AllowResult
{
    AlreadyAllowed,
    Uncommented,
    Added,
}

sealed record AllowEntry(string Host, string Group, bool Active, string Note);

sealed class Allowlist
{
    static readonly Regex HostRx = new(@"^[A-Za-z0-9*][A-Za-z0-9*._:-]*$");
    static readonly Regex GroupRx = new(@"^#\s*---\s*(.+?)\s*-*\s*$");
    static readonly Regex DisabledRx = new(
        @"^#\s*([A-Za-z0-9*][A-Za-z0-9*._:-]*\.[A-Za-z0-9*._:-]+)\s*(?:#\s*(.*))?$"
    );

    readonly string _path;
    readonly List<string> _lines;

    Allowlist(string path, List<string> lines)
    {
        _path = path;
        _lines = lines;
    }

    public static Allowlist Load(string path)
    {
        if (!File.Exists(path))
            throw new SandboxException($"{path} not found.");
        var lines = File.ReadAllText(path).Replace("\r\n", "\n").Split('\n').ToList();
        if (lines.Count > 0 && lines[^1] == "")
            lines.RemoveAt(lines.Count - 1);
        return new Allowlist(path, lines);
    }

    public void Save() =>
        File.WriteAllText(_path, string.Join("\n", _lines) + "\n", new UTF8Encoding(false)); // LF, no BOM

    public static string ValidateHost(string h) =>
        HostRx.IsMatch(h) ? h : throw new SandboxException($"Not a valid host name: '{h}'.");

    static Regex ActiveLine(string h) => new($"^{Regex.Escape(h)}(\\s|$)", RegexOptions.IgnoreCase);

    public AllowResult Allow(string host)
    {
        var active = ActiveLine(host);
        if (_lines.Any(l => active.IsMatch(l)))
            return AllowResult.AlreadyAllowed;
        var commented = new Regex(
            $"^#\\s*({Regex.Escape(host)}(\\s.*)?)$",
            RegexOptions.IgnoreCase
        );
        var i = _lines.FindIndex(l => commented.IsMatch(l));
        if (i >= 0)
        {
            _lines[i] = commented.Match(_lines[i]).Groups[1].Value;
            return AllowResult.Uncommented;
        }
        _lines.Add(host);
        return AllowResult.Added;
    }

    public bool Deny(string host)
    {
        var active = ActiveLine(host);
        var any = false;
        for (var i = 0; i < _lines.Count; i++)
            if (active.IsMatch(_lines[i]))
            {
                _lines[i] = "# " + _lines[i];
                any = true;
            }
        return any;
    }

    public IEnumerable<AllowEntry> Entries()
    {
        string? group = null; // disabled entries are only recognised below the first "# --- group ---" header
        foreach (var raw in _lines)
        {
            var line = raw.Trim();
            if (line.Length == 0)
                continue;
            if (GroupRx.Match(line) is { Success: true } g)
            {
                group = g.Groups[1].Value;
                continue;
            }
            if (!line.StartsWith('#'))
            {
                var host = Regex.Replace(line, @"\s*#.*$", "");
                if (host.Length > 0)
                    yield return new AllowEntry(host, group ?? "", true, "");
            }
            else if (group != null && DisabledRx.Match(line) is { Success: true } d)
                yield return new AllowEntry(d.Groups[1].Value, group, false, d.Groups[2].Value);
        }
    }

    /// Same matching as tinyproxy's fnmatch filter: exact host or shell-style wildcard, case-insensitive.
    public bool IsAllowed(string host) =>
        Entries()
            .Where(e => e.Active)
            .Any(e =>
                Regex.IsMatch(
                    host,
                    "^" + Regex.Escape(e.Host).Replace("\\*", ".*").Replace("\\?", ".") + "$",
                    RegexOptions.IgnoreCase
                )
            );
}

// =============================================================================
//  Processes, terminal output, native bits
// =============================================================================
sealed record ProcResult(int ExitCode, string StdOut, string StdErr)
{
    public IEnumerable<string> Lines() =>
        StdOut.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
}

static class Proc
{
    static ProcessStartInfo Info(string file, IEnumerable<string> args)
    {
        var psi = new ProcessStartInfo(file) { UseShellExecute = false };
        foreach (var a in args)
            psi.ArgumentList.Add(a);
        return psi;
    }

    static void Trace(bool verbose, string file, string[] args)
    {
        if (verbose)
            AnsiConsole.MarkupLine(
                $"[dim]$ {Markup.Escape(file + " " + string.Join(' ', args.Select(a => a.Contains(' ') ? $"\"{a}\"" : a)))}[/]"
            );
    }

    public static ProcResult Capture(bool verbose, string file, params string[] args)
    {
        Trace(verbose, file, args);
        var psi = Info(file, args);
        psi.RedirectStandardOutput = psi.RedirectStandardError = true;
        psi.RedirectStandardInput = true;
        try
        {
            using var p = Process.Start(psi)!;
            p.StandardInput.Close();
            var stdout = p.StandardOutput.ReadToEndAsync();
            var stderr = p.StandardError.ReadToEndAsync();
            p.WaitForExit();
            return new ProcResult(p.ExitCode, stdout.Result, stderr.Result);
        }
        catch (System.ComponentModel.Win32Exception e)
        {
            return new ProcResult(127, "", e.Message);
        }
    }

    /// Attached to the terminal. Ctrl+C goes to the child (logs -f, shell); we just wait for it to finish.
    public static int Interactive(bool verbose, string file, params string[] args)
    {
        Trace(verbose, file, args);
        ConsoleCancelEventHandler keepAlive = (_, e) => e.Cancel = true;
        Console.CancelKeyPress += keepAlive;
        try
        {
            using var p = Process.Start(Info(file, args))!;
            p.WaitForExit();
            return p.ExitCode;
        }
        catch (System.ComponentModel.Win32Exception e)
        {
            throw new SandboxException($"Could not run {file}: {e.Message}");
        }
        finally
        {
            Console.CancelKeyPress -= keepAlive;
        }
    }

    public static string? Which(string name)
    {
        var exts = OperatingSystem.IsWindows()
            ? (Environment.GetEnvironmentVariable("PATHEXT") ?? ".EXE;.CMD;.BAT")
                .Split(';')
                .Prepend("")
            : [""];
        foreach (
            var dir in (Environment.GetEnvironmentVariable("PATH") ?? "").Split(
                Path.PathSeparator,
                StringSplitOptions.RemoveEmptyEntries
            )
        )
        foreach (var ext in exts)
        {
            var f = Path.Combine(dir, name + ext);
            if (File.Exists(f))
                return f;
        }
        return null;
    }
}

static class Ui
{
    public static bool Interactive =>
        !Console.IsOutputRedirected && AnsiConsole.Profile.Capabilities.Interactive;

    /// How to call this app again, for hints in messages.
    public static string Self =>
        OperatingSystem.IsWindows() ? @".sandbox\sandbox.cmd" : ".sandbox/sandbox";

    static IAnsiConsole _err = AnsiConsole.Console;

    /// Sane widths when output is redirected (CI, pipes), and errors on stderr.
    public static void Init()
    {
        static int Fallback() =>
            int.TryParse(Environment.GetEnvironmentVariable("COLUMNS"), out var c) && c > 20
                ? c
                : 120;
        if (Console.IsOutputRedirected || AnsiConsole.Profile.Width < 20)
            AnsiConsole.Profile.Width = Fallback();
        _err = AnsiConsole.Create(
            new AnsiConsoleSettings { Out = new AnsiConsoleOutput(Console.Error) }
        );
        if (Console.IsErrorRedirected || _err.Profile.Width < 20)
            _err.Profile.Width = Fallback();
    }

    public static void Error(string m) =>
        _err.MarkupLine($"[red bold]ERROR:[/] [red]{Markup.Escape(m)}[/]");

    public static void Warn(string m) =>
        _err.MarkupLine($"[yellow bold]WARNING:[/] [yellow]{Markup.Escape(m)}[/]");

    public static void Success(string m) =>
        AnsiConsole.MarkupLine($"[green]✓[/] {Markup.Escape(m)}");

    public static T Spin<T>(string what, Func<T> action) =>
        Interactive
            ? AnsiConsole
                .Status()
                .Spinner(Spinner.Known.Dots)
                .SpinnerStyle(Style.Parse("yellow"))
                .Start(Markup.Escape(what), _ => action())
            : action();

    public static void ShowOutput(string title, ProcResult r)
    {
        var text = (r.StdOut + "\n" + r.StdErr).Trim();
        if (text.Length == 0)
            return;
        var lines = text.Split('\n');
        if (lines.Length > 40)
            text = $"… ({lines.Length - 40} earlier lines)\n" + string.Join('\n', lines[^40..]);
        AnsiConsole.Write(
            new Panel(new Text(text))
                .Header($"[red] {Markup.Escape(title)} (exit {r.ExitCode}) [/]")
                .BorderColor(Color.Red)
                .Expand()
        );
    }

    public static string State(string s) =>
        s switch
        {
            "running" => "[green]running[/]",
            "exited" or "dead" => $"[red]{s}[/]",
            "" => "[dim]—[/]",
            _ => $"[yellow]{Markup.Escape(s)}[/]",
        };

    public static string Health(string h, string state) =>
        h switch
        {
            "healthy" => "[green]healthy[/]",
            "starting" => "[yellow]starting…[/]",
            "unhealthy" => "[red]unhealthy[/]",
            "" => state == "running" ? "[dim]n/a[/]" : "[dim]—[/]",
            _ => Markup.Escape(h),
        };

    public static void OpenBrowser(string url)
    {
        try
        {
            if (OperatingSystem.IsWindows())
            {
                Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
                return;
            }
            foreach (var opener in new[] { "xdg-open", "open", "wslview", "explorer.exe" })
                if (Proc.Which(opener) != null)
                {
                    var psi = new ProcessStartInfo(opener, url)
                    {
                        UseShellExecute = false,
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                    };
                    Process.Start(psi);
                    return;
                }
        }
        catch (Exception)
        { /* fall through to the hint */
        }
        AnsiConsole.MarkupLine($"Open [link={url}]{url}[/] in your browser.");
    }
}

static class Native
{
    [DllImport("libc", EntryPoint = "getuid")]
    static extern uint getuid();

    [DllImport("libc", EntryPoint = "getgid")]
    static extern uint getgid();

    /// The host user's ids for PUID/PGID (1000 on Windows, and never root).
    public static (uint Uid, uint Gid) UserIds()
    {
        if (OperatingSystem.IsWindows())
            return (1000, 1000);
        try
        {
            var (u, g) = (getuid(), getgid());
            return u == 0 ? (1000, 1000) : (u, g);
        }
        catch (Exception)
        {
            return (1000, 1000);
        }
    }
}
