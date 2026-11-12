using System.Diagnostics;
using AiSandbox.Project;
using AiSandbox.Terminal;
using Spectre.Console;

namespace AiSandbox.Commands;

/// <summary>
/// Building, starting, stopping and using the sandbox: up, rebuild, stop/down/restart, status, open, shell,
/// claude, logs.
/// </summary>
internal static class LifecycleCommands
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

    /// <summary>
    /// Runs one docker compose command with a spinner (stop, down, restart).
    /// </summary>
    public static int Compose(Sandbox sb, string what, params ImmutableArray<string> args)
    {
        var r = Ui.Spin($"{what} '{sb.Name}'…", () => sb.Compose(args.AsSpan()));
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
        if (ps.IsEmpty)
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

    public static int Shell(Sandbox sb) => sb.Exec("bash", "-l");

    /// <summary>
    /// The Claude Code CLI in the sandbox, with every argument passed on verbatim.
    /// </summary>
    public static int Claude(Sandbox sb, ImmutableArray<string> args)
    {
        sb.NeedDocker();
        sb.Generate();
        const string Script =
            "command -v claude >/dev/null || { echo \"Claude Code CLI not installed: set claude_code under [tools] in sandbox.toml and rebuild.\"; exit 1; }; exec claude \"$@\"";
        return sb.Exec(["sh", "-c", Script, "claude", .. args]);
    }

    public static int Logs(Sandbox sb, ImmutableArray<string> services, int tail, bool follow)
    {
        var args = ImmutableArray.CreateBuilder<string>();
        args.Add("logs");
        args.Add($"--tail={tail}");
        if (follow)
            args.Add("-f");
        args.AddRange(services);
        return sb.ComposeInteractive(args.DrainToImmutable().AsSpan());
    }

    /// <summary>
    /// Image build: docker's own progress output is shown as-is (it can take minutes the first time).
    /// </summary>
    private static void Build(Sandbox sb, bool pull)
    {
        AnsiConsole.Write(
            new Rule(
                pull
                    ? "[yellow]Building images with the newest tools[/]"
                    : "[yellow]Building images[/]"
            ).LeftJustified()
        );
        ImmutableArray<string> args = pull
            ?
            [
                "build",
                "--pull",
                "--build-arg",
                $"CACHE_BUST={DateTimeOffset.UtcNow.ToUnixTimeSeconds()}",
            ]
            : ["build"];
        if (sb.ComposeInteractive(args.AsSpan()) != 0)
            throw new SandboxException(
                "Building the sandbox images failed (see the output above)."
            );
        AnsiConsole.Write(new Rule().RuleStyle("dim"));
    }

    /// <summary>
    /// <c>up -d --wait</c> in the background while a live table shows each service's state and health.
    /// </summary>
    private static void StartAndWait(Sandbox sb, bool tolerateUnhealthy)
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
            if (ps.IsEmpty)
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
}
