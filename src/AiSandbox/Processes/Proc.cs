using System.ComponentModel;
using System.Diagnostics;
using Spectre.Console;

namespace AiSandbox.Processes;

/// <summary>
/// Child processes: captured, or attached to this terminal.
/// </summary>
internal static class Proc
{
    /// <summary>
    /// Runs a process with its output captured and no input. A missing executable gives exit code 127.
    /// </summary>
    public static ProcResult Capture(bool verbose, string file, params ReadOnlySpan<string> args)
    {
        Trace(verbose, file, args);
        var psi = StartInfo(file, args);
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
        catch (Win32Exception e)
        {
            return new ProcResult(127, "", e.Message);
        }
    }

    /// <summary>
    /// Attached to the terminal. Ctrl+C goes to the child (logs -f, shell); we just wait for it to finish.
    /// </summary>
    public static int Interactive(bool verbose, string file, params ReadOnlySpan<string> args)
    {
        Trace(verbose, file, args);
        ConsoleCancelEventHandler keepAlive = (_, e) => e.Cancel = true;
        Console.CancelKeyPress += keepAlive;
        try
        {
            using var p = Process.Start(StartInfo(file, args))!;
            p.WaitForExit();
            return p.ExitCode;
        }
        catch (Win32Exception e)
        {
            throw new SandboxException($"Could not run {file}: {e.Message}");
        }
        finally
        {
            Console.CancelKeyPress -= keepAlive;
        }
    }

    /// <summary>
    /// The full path of an executable on PATH (with PATHEXT on Windows), or null.
    /// </summary>
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

    private static ProcessStartInfo StartInfo(string file, ReadOnlySpan<string> args)
    {
        var psi = new ProcessStartInfo(file) { UseShellExecute = false };
        foreach (var a in args)
            psi.ArgumentList.Add(a);
        return psi;
    }

    private static void Trace(bool verbose, string file, ReadOnlySpan<string> args)
    {
        if (!verbose)
            return;
        var parts = new string[args.Length + 1];
        parts[0] = file;
        for (var i = 0; i < args.Length; i++)
            parts[i + 1] = args[i].Contains(' ', StringComparison.Ordinal) ? $"\"{args[i]}\"" : args[i];
        AnsiConsole.MarkupLine($"[dim]$ {Markup.Escape(string.Join(' ', parts))}[/]");
    }
}
