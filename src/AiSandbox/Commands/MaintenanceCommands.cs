using System.Globalization;
using System.Text;
using AiSandbox.Configuration;
using AiSandbox.Processes;
using AiSandbox.Project;
using AiSandbox.Templating;
using AiSandbox.Terminal;
using Spectre.Console;

namespace AiSandbox.Commands;

/// <summary>
/// init, reset-home and info.
/// </summary>
internal static class MaintenanceCommands
{
    /// <summary>
    /// The files a project owns, created by <c>init</c> from the embedded templates.
    /// </summary>
    private static readonly ImmutableArray<string> s_projectFiles =
    [
        Config.FileName,
        "allowlist.txt",
    ];

    /// <summary>
    /// Creates <c>&lt;folder&gt;/.sandbox/</c> with <c>sandbox.toml</c> and <c>allowlist.txt</c> (files that exist are
    /// kept), the <c>.state</c> folder and the engine. Runs without Docker and ignores <c>SANDBOX_DIR</c>.
    /// </summary>
    public static int Init(string? folder)
    {
        var projectDir = Path.TrimEndingDirectorySeparator(
            Path.GetFullPath(string.IsNullOrWhiteSpace(folder) ? "." : folder)
        );
        if (!Directory.Exists(projectDir))
            throw new SandboxException($"Folder {projectDir} does not exist.");
        var dir = Path.Combine(projectDir, ".sandbox");

        for (var parent = Directory.GetParent(projectDir); parent != null; parent = parent.Parent)
        {
            var outer = Path.Combine(parent.FullName, ".sandbox");
            if (Directory.Exists(outer))
            {
                Ui.Warn($"{outer} already exists; this creates a separate sandbox for {projectDir}");
                break;
            }
        }
        Sandbox.CheckOldLayout(dir);

        Directory.CreateDirectory(dir);
        foreach (var name in s_projectFiles)
        {
            var path = Path.Combine(dir, name);
            if (File.Exists(path))
            {
                AnsiConsole.MarkupLine($"[dim]{Markup.Escape(name)} exists \u2014 kept[/]");
                continue;
            }
            File.WriteAllText(path, EmbeddedFiles.ReadText("init/" + name), new UTF8Encoding(false));
            Ui.Success($"Created .sandbox/{name}");
        }

        var stateDir = Path.Combine(dir, ".state");
        Sandbox.WriteStateIgnore(stateDir);
        Sandbox.LoadOrCreateId(stateDir, projectDir);
        EngineFiles.Sync(stateDir);
        Ui.Success("Wrote .sandbox/.state/ (desktop/ and proxy/ show what the sandbox builds)");

        AnsiConsole.MarkupLine(
            "Next: edit [bold].sandbox/sandbox.toml[/], then \"[bold]sandbox up[/]\"."
        );
        return 0;
    }

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
        LifecycleCommands.Compose(sb, "Removing containers", "down");
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
        var st = sb.Settings;
        var id = new Grid().AddColumn(new GridColumn().NoWrap().PadRight(3)).AddColumn();
        void Row(string k, string v) => id.AddRow($"[bold]{k}[/]", Markup.Escape(v));
        Row("Project", sb.Name);
        Row("Project folder", sb.ProjectDir);
        Row("Mounted at", sb.WorkDir);
        Row("ID", sb.Id);
        Row("Workspace", st.ReadOnly ? "read-only" : "read-write");
        Row("Protect .git", st.ProtectGit ? "yes" : "no");
        Row("UI port", st.UiPort?.ToString(CultureInfo.InvariantCulture) ?? "random free port");
        Row(
            "Engine",
            $"sandbox {EngineFiles.ShortAppVersion}, engine {EngineFiles.ShortEngineHash} \u2192 .sandbox/.state/desktop, proxy (rewritten before each command)"
        );
        AnsiConsole.Write(new Panel(id).Header("[bold] Sandbox [/]").Border(BoxBorder.Rounded));

        var desktop = new Grid().AddColumn(new GridColumn().NoWrap().PadRight(3)).AddColumn();
        void D(string k, string v) => desktop.AddRow($"[bold]{k}[/]", Markup.Escape(v));
        D("Title", st.Title);
        D("Sudo", st.Sudo ? "yes" : "no");
        D("Memory", st.Memory);
        D("CPUs", st.Cpus ?? "no limit");
        D("Time zone", st.TimeZone ?? "from this machine");
        D("Login", st.Auth is { } a ? $"basic auth ({a.User})" : "none");
        if (!st.Devices.IsEmpty)
            D("Devices", string.Join(", ", st.Devices));
        if (!st.SecurityOpt.IsEmpty)
            D("Security options", string.Join(", ", st.SecurityOpt));
        foreach (var (k, v) in st.Env)
            D("$" + k, v);
        AnsiConsole.Write(new Panel(desktop).Header("[bold] Desktop [/]").Border(BoxBorder.Rounded));

        var tools = new Table()
            .Border(TableBorder.Rounded)
            .Title("[bold]Tools [[tools]][/]")
            .AddColumn("Tool")
            .AddColumn("Value");
        foreach (var (arg, value) in st.Install)
        {
            var off = value is "" or "false";
            tools.AddRow(
                Markup.Escape(Config.ToolName(arg)),
                off ? "[dim]not installed[/]"
                    : value == "true" ? "[green]default version[/]"
                    : $"[green]{Markup.Escape(value)}[/]"
            );
        }
        AnsiConsole.Write(tools);

        if (!st.Shares.IsEmpty)
        {
            var shares = new Table()
                .Border(TableBorder.Rounded)
                .Title("[bold]Shared folders [[[[share]]]][/]")
                .AddColumn("On this machine")
                .AddColumn("In the sandbox")
                .AddColumn("Access");
            foreach (var s in st.Shares)
                shares.AddRow(
                    Markup.Escape(s.Host),
                    Markup.Escape(s.Path),
                    s.ReadOnly ? "read-only" : "read-write"
                );
            AnsiConsole.Write(shares);
        }
        return 0;
    }
}
