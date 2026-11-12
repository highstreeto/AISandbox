using System.Globalization;
using System.Text.RegularExpressions;
using AiSandbox.Network;
using AiSandbox.Project;
using AiSandbox.Terminal;
using Spectre.Console;

namespace AiSandbox.Commands;

/// <summary>
/// The proxy's allowlist: allow, deny, allowlist, blocked, reload.
/// </summary>
internal static partial class NetworkCommands
{
    public static int Allow(Sandbox sb, ImmutableArray<string> hosts)
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

    public static int Deny(Sandbox sb, ImmutableArray<string> hosts)
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

    public static int ShowAllowlist(Sandbox sb, bool all)
    {
        var entries = Allowlist
            .Load(sb.AllowlistPath)
            .Entries()
            .Where(e => all || e.Active)
            .ToImmutableArray();
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
        var hits = RefusedRegex()
            .Matches(logs.StdOut + "\n" + logs.StdErr)
            .Select(m => m.Groups[1].Value)
            .GroupBy(h => h, StringComparer.OrdinalIgnoreCase)
            .Select(g => (Host: g.Key, Count: g.Count()))
            .OrderByDescending(x => x.Count)
            .ThenBy(x => x.Host, StringComparer.Ordinal)
            .ToImmutableArray();
        if (hits.IsEmpty)
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
                count.ToString(CultureInfo.InvariantCulture),
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

    private static Table ResultTable() =>
        new Table().Border(TableBorder.Rounded).AddColumn("Host").AddColumn("Result");

    /// <summary>tinyproxy's log line for a host that isn't on the allowlist.</summary>
    [GeneratedRegex("refused on filtered (?:domain|url) \"([^\"]+)\"")]
    private static partial Regex RefusedRegex();
}
