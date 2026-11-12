using AiSandbox.Cli;
using AiSandbox.Commands;
using AiSandbox.Project;
using AiSandbox.Terminal;
using Spectre.Console;

try
{
    Ui.Init();

    // `help [command]` is `[command] --help`.
    if (args is ["help", .. var helpArgs])
        args = [.. helpArgs, "--help"];

    // `claude` forwards everything verbatim (including --help, --resume, -p, ...) to the CLI in the sandbox.
    if (args is ["claude", .. var claudeArgs])
        return SandboxCommandLine.Guard(() =>
            LifecycleCommands.Claude(Sandbox.Load(), [.. claudeArgs])
        );

    return await SandboxCommandLine.Build().Parse(args).InvokeAsync().ConfigureAwait(false);
}
catch (Exception ex)
{
    AnsiConsole.WriteException(ex);
    return 1;
}
