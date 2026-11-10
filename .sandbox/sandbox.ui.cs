// Terminal output: messages, spinners, error panels, state colours and opening the browser.
using System.Diagnostics;
using Spectre.Console;

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
