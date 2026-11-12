using System.Diagnostics;
using AiSandbox.Processes;
using Spectre.Console;

namespace AiSandbox.Terminal;

/// <summary>
/// Terminal output: messages, spinners, error panels, state colours and opening the browser.
/// </summary>
internal static class Ui
{
    /// <summary>
    /// Errors and warnings go to stderr. Set once by <see cref="Init"/> (it needs the console's final width).
    /// </summary>
    private static IAnsiConsole s_err = AnsiConsole.Console;

    public static bool Interactive =>
        !Console.IsOutputRedirected && AnsiConsole.Profile.Capabilities.Interactive;

    /// <summary>
    /// How to call this app again, for hints in messages: the launcher in the repository root.
    /// </summary>
    public static string Self => OperatingSystem.IsWindows() ? "sandbox.cmd" : "./sandbox";

    /// <summary>
    /// Sane widths when output is redirected (CI, pipes), and errors on stderr.
    /// </summary>
    public static void Init()
    {
        if (Console.IsOutputRedirected || AnsiConsole.Profile.Width < 20)
            AnsiConsole.Profile.Width = FallbackWidth();
        s_err = AnsiConsole.Create(
            new AnsiConsoleSettings { Out = new AnsiConsoleOutput(Console.Error) }
        );
        if (Console.IsErrorRedirected || s_err.Profile.Width < 20)
            s_err.Profile.Width = FallbackWidth();
    }

    public static void Error(string m) =>
        s_err.MarkupLine($"[red bold]ERROR:[/] [red]{Markup.Escape(m)}[/]");

    public static void Warn(string m) =>
        s_err.MarkupLine($"[yellow bold]WARNING:[/] [yellow]{Markup.Escape(m)}[/]");

    public static void Success(string m) =>
        AnsiConsole.MarkupLine($"[green]✓[/] {Markup.Escape(m)}");

    /// <summary>
    /// Runs the action with a spinner (only when interactive).
    /// </summary>
    public static T Spin<T>(string what, Func<T> action) =>
        Interactive
            ? AnsiConsole
                .Status()
                .Spinner(Spinner.Known.Dots)
                .SpinnerStyle(Style.Parse("yellow"))
                .Start(Markup.Escape(what), _ => action())
            : action();

    /// <summary>
    /// The output of a failed command in a red panel (its last 40 lines).
    /// </summary>
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

    /// <summary>
    /// Opens the URL in the default browser, or prints it if that isn't possible.
    /// </summary>
    public static void OpenBrowser(string url)
    {
        try
        {
            if (OperatingSystem.IsWindows())
            {
                Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
                return;
            }
            foreach (var opener in (ReadOnlySpan<string>)["xdg-open", "open", "wslview", "explorer.exe"])
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
        {
            // fall through to the hint
        }
        AnsiConsole.MarkupLine($"Open [link={url}]{url}[/] in your browser.");
    }

    private static int FallbackWidth() =>
        int.TryParse(Environment.GetEnvironmentVariable("COLUMNS"), out var c) && c > 20 ? c : 120;
}
