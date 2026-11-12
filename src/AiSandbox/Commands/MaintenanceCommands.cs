using System.Globalization;
using AiSandbox.Configuration;
using AiSandbox.Processes;
using AiSandbox.Project;
using AiSandbox.Terminal;
using Spectre.Console;

namespace AiSandbox.Commands;

/// <summary>
/// reset-home and info.
/// </summary>
internal static class MaintenanceCommands
{
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
