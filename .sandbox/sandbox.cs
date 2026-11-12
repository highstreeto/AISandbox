// AI Sandbox control app (Windows, Linux, macOS, WSL) — a .NET 10 file-based app.
// Lives in <project>/.sandbox/ and is started through the launchers next to it, from any directory:
//   .sandbox/sandbox up            (sh: Linux, macOS, WSL, Git Bash)
//   .sandbox\sandbox.cmd up        (Windows cmd / PowerShell)
// They run `dotnet run --file sandbox.cs -- <args>`, so every argument reaches this app untouched.
// It never changes your current directory (all paths are absolute).
//
// This file: the entry point, the command line and what each command does.
// The rest lives next to it and is pulled in with #:include:
//   sandbox.project.cs    project identity, docker compose calls, the generated compose file
//   sandbox.compose.cs    merging the compose layers into that one file
//   sandbox.config.cs     sandbox.yml / sandbox.local.yml settings
//   sandbox.allowlist.cs  allowlist.txt
//   sandbox.proc.cs       child processes, native user ids
//   sandbox.ui.cs         terminal output
#:package System.CommandLine@2.0.15
#:package YamlDotNet@18.1.0
#:package Spectre.Console@0.57.2
#:property Nullable=enable
#:property PublishAot=false
#:include sandbox.project.cs
#:include sandbox.compose.cs
#:include sandbox.config.cs
#:include sandbox.allowlist.cs
#:include sandbox.proc.cs
#:include sandbox.ui.cs

using System.CommandLine;
using System.Diagnostics;
using System.Text.RegularExpressions;
using Spectre.Console;
using YamlDotNet.Core;

try
{
    Ui.Init();

    if (args.Length >= 1 && args[0] == "help")
        args = [.. args[1..], "--help"]; // `help [command]`

    // `claude` forwards everything verbatim (including --help, --resume, -p, ...) to the CLI in the sandbox.
    if (args.Length >= 1 && args[0] == "claude")
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
        var up = new Command("up", "Build (if needed), start and open the desktop in the browser");
        up.Aliases.Add("start");
        var noOpen = new Option<bool>("--no-open") { Description = "Don't open the browser" };
        up.Options.Add(noOpen);
        up.SetAction(r => Run(r, sb => Commands.Up(sb, !r.GetValue(noOpen))));
        root.Subcommands.Add(up);

        var stop = new Command("stop", "Stop (everything is kept)");
        stop.SetAction(r => Run(r, sb => Commands.Compose(sb, "Stopping", "stop")));
        root.Subcommands.Add(stop);

        var down = new Command("down", "Remove containers (home volume and project are kept)");
        down.SetAction(r => Run(r, sb => Commands.Compose(sb, "Removing containers", "down")));
        root.Subcommands.Add(down);

        var restart = new Command("restart", "Restart");
        restart.SetAction(r => Run(r, sb => Commands.Compose(sb, "Restarting", "restart")));
        root.Subcommands.Add(restart);

        var status = new Command("status", "Container status and URL");
        status.Aliases.Add("ps");
        status.SetAction(r => Run(r, Commands.Status));
        root.Subcommands.Add(status);

        var open = new Command("open", "Open the desktop in the browser");
        open.SetAction(r => Run(r, Commands.Open));
        root.Subcommands.Add(open);

        var shell = new Command("shell", "Bash inside the sandbox (in the project folder)");
        shell.SetAction(r => Run(r, sb => sb.Exec("bash", "-l")));
        root.Subcommands.Add(shell);

        // Runs before parsing (see the top of this file); declared here for --help only.
        var claude = new Command(
            "claude",
            "Claude Code CLI inside the sandbox (needs INSTALL_CLAUDE_CODE); all arguments are passed on"
        );
        claude.Arguments.Add(Many("args", "Arguments for claude"));
        claude.TreatUnmatchedTokensAsErrors = false;
        root.Subcommands.Add(claude);

        var logs = new Command("logs", "Follow logs (proxy, gateway, desktop)");
        var services = Many("service", "proxy | gateway | desktop (default: all)");
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
        root.Subcommands.Add(logs);

        // --- Network ----------------------------------------------------------
        var hosts = new Argument<string[]>("host")
        {
            Arity = ArgumentArity.OneOrMore,
            Description = "Host name, wildcards like *.example.com allowed",
        };

        var allow = new Command("allow", "Allow host(s) and reload the proxy");
        allow.Arguments.Add(hosts);
        allow.SetAction(r => Run(r, sb => Commands.Allow(sb, r.GetValue(hosts)!)));
        root.Subcommands.Add(allow);

        var deny = new Command("deny", "Remove host(s) and reload the proxy");
        deny.Arguments.Add(hosts);
        deny.SetAction(r => Run(r, sb => Commands.Deny(sb, r.GetValue(hosts)!)));
        root.Subcommands.Add(deny);

        var allowlist = new Command("allowlist", "Show active allowlist entries");
        var all = new Option<bool>("--all", "-a")
        {
            Description = "Also show the commented-out (available) entries",
        };
        allowlist.Options.Add(all);
        allowlist.SetAction(r =>
            Run(r, sb => Commands.ShowAllowlist(sb, r.GetValue(all)), docker: false)
        );
        root.Subcommands.Add(allowlist);

        var blocked = new Command(
            "blocked",
            "Show hosts the sandbox tried to reach but were blocked"
        );
        blocked.SetAction(r => Run(r, Commands.Blocked));
        root.Subcommands.Add(blocked);

        var reload = new Command("reload", "Reload the proxy after editing allowlist.txt");
        reload.SetAction(r => Run(r, Commands.ReloadProxy));
        root.Subcommands.Add(reload);

        // --- Maintenance ------------------------------------------------------
        var rebuild = new Command("rebuild", "Rebuild after changing INSTALL_* in sandbox.yml");
        rebuild.SetAction(r => Run(r, sb => Commands.Rebuild(sb, pull: false)));
        root.Subcommands.Add(rebuild);

        var update = new Command("update", "Rebuild with the newest versions of all tools");
        update.SetAction(r => Run(r, sb => Commands.Rebuild(sb, pull: true)));
        root.Subcommands.Add(update);

        var resetHome = new Command(
            "reset-home",
            "Delete the persistent home (logins, IDE settings)"
        );
        var yes = new Option<bool>("--yes", "-y") { Description = "Don't ask for confirmation" };
        resetHome.Options.Add(yes);
        resetHome.SetAction(r => Run(r, sb => Commands.ResetHome(sb, r.GetValue(yes))));
        root.Subcommands.Add(resetHome);

        var info = new Command("info", "Show the project identity, settings and configured tools");
        info.SetAction(r => Run(r, Commands.Info, docker: false));
        root.Subcommands.Add(info);

        var help = new Command("help", "Show help (for a command: help <command>)");
        help.Arguments.Add(Many("command", "Command to show help for")); // rewritten to --help before parsing
        root.Subcommands.Add(help);

        return root;

        static Argument<string[]> Many(string name, string description) =>
            new(name) { Arity = ArgumentArity.ZeroOrMore, Description = description };
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
        return sb.Exec(["sh", "-c", script, "claude", .. args]);
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

    public static int ReloadProxy(Sandbox sb)
    {
        var r = Ui.Spin("Reloading the proxy…", () => sb.Compose("restart", "proxy"));
        if (r.ExitCode != 0)
        {
            Ui.ShowOutput("docker compose restart proxy", r);
            throw new SandboxException("Could not restart the proxy.");
        }
        Ui.Success("Proxy reloaded.");
        return 0;
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
