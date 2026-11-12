namespace AiSandbox.Processes;

/// <summary>
/// The exit code and output of a finished child process.
/// </summary>
internal sealed record ProcResult(int ExitCode, string StdOut, string StdErr)
{
    /// <summary>
    /// The non-empty lines of <see cref="StdOut"/>, trimmed.
    /// </summary>
    public ImmutableArray<string> Lines() =>
        [.. StdOut.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)];
}
