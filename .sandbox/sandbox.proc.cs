// Child processes (captured or attached to the terminal) and native user ids.
using System.Diagnostics;
using System.Runtime.InteropServices;
using Spectre.Console;

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
