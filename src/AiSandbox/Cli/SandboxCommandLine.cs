using System.CommandLine;
using AiSandbox.Commands;
using AiSandbox.Project;
using AiSandbox.Terminal;

namespace AiSandbox.Cli;

/// <summary>
/// The command line: every command, its options and which command method it runs.
/// </summary>
internal static class SandboxCommandLine
{
    private static readonly Option<bool> s_verbose = new("--verbose", "-v")
    {
        Description = "Print the docker commands that are run",
        Recursive = true,
    };

    public static RootCommand Build()
    {
        var root = new RootCommand(
            "AI Sandbox — a desktop for this project, reachable only through an allowlist proxy."
        );
        root.Options.Add(s_verbose);

        // --- Lifecycle --------------------------------------------------------
        var up = new Command("up", "Build (if needed), start and open the desktop in the browser");
        up.Aliases.Add("start");
        var noOpen = new Option<bool>("--no-open") { Description = "Don't open the browser" };
        up.Options.Add(noOpen);
        up.SetAction(r => Run(r, sb => LifecycleCommands.Up(sb, !r.GetValue(noOpen))));
        root.Subcommands.Add(up);

        var stop = new Command("stop", "Stop (everything is kept)");
        stop.SetAction(r => Run(r, sb => LifecycleCommands.Compose(sb, "Stopping", "stop")));
        root.Subcommands.Add(stop);

        var down = new Command("down", "Remove containers (home volume and project are kept)");
        down.SetAction(r =>
            Run(r, sb => LifecycleCommands.Compose(sb, "Removing containers", "down"))
        );
        root.Subcommands.Add(down);

        var restart = new Command("restart", "Restart");
        restart.SetAction(r =>
            Run(r, sb => LifecycleCommands.Compose(sb, "Restarting", "restart"))
        );
        root.Subcommands.Add(restart);

        var status = new Command("status", "Container status and URL");
        status.Aliases.Add("ps");
        status.SetAction(r => Run(r, LifecycleCommands.Status));
        root.Subcommands.Add(status);

        var open = new Command("open", "Open the desktop in the browser");
        open.SetAction(r => Run(r, LifecycleCommands.Open));
        root.Subcommands.Add(open);

        var shell = new Command("shell", "Bash inside the sandbox (in the project folder)");
        shell.SetAction(r => Run(r, LifecycleCommands.Shell));
        root.Subcommands.Add(shell);

        // Runs before parsing (see Program.Main); declared here for --help only.
        var claude = new Command(
            "claude",
            "Claude Code CLI inside the sandbox (needs claude_code under [tools]); all arguments are passed on"
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
                    LifecycleCommands.Logs(
                        sb,
                        [.. r.GetValue(services) ?? []],
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
        allow.SetAction(r => Run(r, sb => NetworkCommands.Allow(sb, [.. r.GetValue(hosts)!])));
        root.Subcommands.Add(allow);

        var deny = new Command("deny", "Remove host(s) and reload the proxy");
        deny.Arguments.Add(hosts);
        deny.SetAction(r => Run(r, sb => NetworkCommands.Deny(sb, [.. r.GetValue(hosts)!])));
        root.Subcommands.Add(deny);

        var allowlist = new Command("allowlist", "Show active allowlist entries");
        var all = new Option<bool>("--all", "-a")
        {
            Description = "Also show the commented-out (available) entries",
        };
        allowlist.Options.Add(all);
        allowlist.SetAction(r =>
            Run(r, sb => NetworkCommands.ShowAllowlist(sb, r.GetValue(all)), docker: false)
        );
        root.Subcommands.Add(allowlist);

        var blocked = new Command(
            "blocked",
            "Show hosts the sandbox tried to reach but were blocked"
        );
        blocked.SetAction(r => Run(r, NetworkCommands.Blocked));
        root.Subcommands.Add(blocked);

        var reload = new Command("reload", "Reload the proxy after editing allowlist.txt");
        reload.SetAction(r => Run(r, NetworkCommands.ReloadProxy));
        root.Subcommands.Add(reload);

        // --- Maintenance ------------------------------------------------------
        var rebuild = new Command("rebuild", "Rebuild after changing [tools] in sandbox.toml");
        rebuild.SetAction(r => Run(r, sb => LifecycleCommands.Rebuild(sb, pull: false)));
        root.Subcommands.Add(rebuild);

        var update = new Command("update", "Rebuild with the newest versions of all tools");
        update.SetAction(r => Run(r, sb => LifecycleCommands.Rebuild(sb, pull: true)));
        root.Subcommands.Add(update);

        var resetHome = new Command(
            "reset-home",
            "Delete the persistent home (logins, IDE settings)"
        );
        var yes = new Option<bool>("--yes", "-y") { Description = "Don't ask for confirmation" };
        resetHome.Options.Add(yes);
        resetHome.SetAction(r =>
            Run(r, sb => MaintenanceCommands.ResetHome(sb, r.GetValue(yes)))
        );
        root.Subcommands.Add(resetHome);

        var info = new Command("info", "Show the project identity, settings and configured tools");
        info.SetAction(r => Run(r, MaintenanceCommands.Info, docker: false));
        root.Subcommands.Add(info);

        var help = new Command("help", "Show help (for a command: help <command>)");
        help.Arguments.Add(Many("command", "Command to show help for")); // rewritten to --help before parsing
        root.Subcommands.Add(help);

        return root;

        static Argument<string[]> Many(string name, string description) =>
            new(name) { Arity = ArgumentArity.ZeroOrMore, Description = description };
    }

    /// <summary>
    /// Runs a command and turns a <see cref="SandboxException"/> into an error message and exit code 1.
    /// </summary>
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
    }

    /// <summary>
    /// Loads the project, (optionally) checks Docker and regenerates the compose file, then runs the command.
    /// </summary>
    private static int Run(ParseResult r, Func<Sandbox, int> action, bool docker = true) =>
        Guard(() =>
        {
            var sb = Sandbox.Load(r.GetValue(s_verbose));
            if (docker)
            {
                sb.NeedDocker();
                sb.Generate();
            }
            return action(sb);
        });
}
