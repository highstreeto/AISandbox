using AiSandbox.Processes;
using AiSandbox.Terminal;

namespace AiSandbox.Project;

internal sealed partial record Sandbox
{
    /// <summary>
    /// Runs docker compose with captured output (for spinners, tables and error panels).
    /// </summary>
    public ProcResult Compose(params ReadOnlySpan<string> args) =>
        Proc.Capture(Verbose, "docker", ComposeArgs(["--ansi", "never", .. args]).AsSpan());

    /// <summary>
    /// Runs docker compose attached to this terminal (build output, shell, logs, claude).
    /// </summary>
    public int ComposeInteractive(params ReadOnlySpan<string> args) =>
        Proc.Interactive(Verbose, "docker", ComposeArgs(args).AsSpan());

    /// <summary>
    /// Runs a command in the desktop container as the desktop user, in the project folder, attached to this
    /// terminal.
    /// </summary>
    public int Exec(params ReadOnlySpan<string> command) =>
        ComposeInteractive(["exec", "-u", "abc", "-w", WorkDir, "desktop", .. command]);

    public void NeedDocker()
    {
        if (Proc.Which("docker") == null)
            throw new SandboxException("Docker is not installed.");
        var r = Ui.Spin(
            "Checking Docker…",
            () => Proc.Capture(Verbose, "docker", "info", "--format", "{{.ServerVersion}}")
        );
        if (r.ExitCode != 0)
            throw new SandboxException(
                "Docker is not running. Start Docker Desktop (or the Docker service) and try again."
            );
    }

    /// <summary>
    /// Fails if containers of another project (another <c>sandbox.id</c>) use the same compose project name.
    /// </summary>
    public void CheckCollision()
    {
        var r = Proc.Capture(
            Verbose,
            "docker",
            "ps",
            "-a",
            "--filter",
            $"label=com.docker.compose.project={Name}",
            "--filter",
            "label=sandbox.id",
            "--format",
            "{{.Label \"sandbox.id\"}}"
        );
        if (r.Lines().Any(l => l != Id))
            throw new SandboxException(
                $"Another project already uses the container group '{Name}'. Set name under [project] in sandbox.toml."
            );
    }

    /// <summary>
    /// The desktop's URL, or null if the gateway's port isn't published (not running).
    /// </summary>
    public string? UiUrl()
    {
        var r = Compose("port", "gateway", "3001");
        var p = r.ExitCode == 0 ? r.Lines().FirstOrDefault() : null;
        return string.IsNullOrEmpty(p) || p.EndsWith(":0", StringComparison.Ordinal)
            ? null
            : $"https://localhost:{p[(p.LastIndexOf(':') + 1)..]}";
    }

    /// <summary>
    /// The containers of this project, by service name.
    /// </summary>
    public ImmutableArray<ServiceState> Services()
    {
        var r = Compose("ps", "-a", "--format", "json");
        return r.ExitCode != 0 || string.IsNullOrWhiteSpace(r.StdOut)
            ? []
            : [.. ComposePs.Parse(r.StdOut).OrderBy(s => s.Service, StringComparer.Ordinal)];
    }

    private ImmutableArray<string> ComposeArgs(ReadOnlySpan<string> args) =>
        ["compose", "-p", Name, "--project-directory", Dir, "-f", GenPath, .. args];
}
