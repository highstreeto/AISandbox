using AiSandbox.Configuration;
using AiSandbox.Terminal;

namespace AiSandbox.Project;

internal sealed partial record Sandbox
{
    /// <summary>
    /// Set by the launchers to the <c>.sandbox</c> folder next to them.
    /// </summary>
    public const string DirVariable = "SANDBOX_DIR";

    private const string OldConfigFileName = "sandbox.yml";
    private const string OldLocalConfigFileName = "sandbox.local.yml";

    /// <summary>
    /// <c>$SANDBOX_DIR</c> if set; otherwise searching upwards from the current directory for a folder that is
    /// (or has) a <c>.sandbox</c> folder.
    /// </summary>
    private static string LocateSandboxDir()
    {
        if (Environment.GetEnvironmentVariable(DirVariable) is { Length: > 0 } fromEnv)
        {
            var full = Path.GetFullPath(fromEnv);
            return IsSandboxDir(full)
                ? Physical(full)
                : throw new SandboxException(
                    $"{DirVariable} is {full}, which is not a .sandbox folder (with {Config.FileName})."
                );
        }
        for (var d = new DirectoryInfo(Environment.CurrentDirectory); d != null; d = d.Parent)
        {
            if (IsSandboxDir(d.FullName))
                return Physical(d.FullName);
            var sub = Path.Combine(d.FullName, ".sandbox");
            if (IsSandboxDir(sub))
                return Physical(sub);
        }
        throw new SandboxException(
            $"Could not find the .sandbox folder (with {Config.FileName})."
        );
    }

    private static bool IsSandboxDir(string dir) =>
        File.Exists(Path.Combine(dir, Config.FileName))
        || File.Exists(Path.Combine(dir, OldConfigFileName));

    /// <summary>
    /// Like <c>pwd -P</c>: resolves a symlinked folder.
    /// </summary>
    private static string Physical(string path)
    {
        var full = Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
        try
        {
            return new DirectoryInfo(full).ResolveLinkTarget(returnFinalTarget: true)?.FullName
                ?? full;
        }
        catch (IOException)
        {
            return full;
        }
    }

    /// <summary>
    /// <c>.sandbox/engine/</c> from before the engine was built into the app: a warning, nothing is deleted (the
    /// files are committed and may hold the project's own edits).
    /// </summary>
    public static void CheckOldLayout(string dir)
    {
        if (Directory.Exists(Path.Combine(dir, "engine")))
            Ui.Warn(
                ".sandbox/engine/ is no longer used \u2014 the engine is built into the app (see .sandbox/.state/desktop/ and proxy/). "
                    + "Delete it, together with .sandbox/README.md, .gitattributes and .gitignore."
            );
    }

    /// <summary>
    /// sandbox.yml / sandbox.local.yml from before sandbox.toml: an error with the key mapping, or a warning.
    /// </summary>
    private static void CheckOldConfig(string dir)
    {
        var hasToml = File.Exists(Path.Combine(dir, Config.FileName));
        var hasYml = File.Exists(Path.Combine(dir, OldConfigFileName));
        if (hasYml && !hasToml)
            throw new SandboxException(
                """
                sandbox.yml is no longer read. Move its settings to sandbox.toml (see README.md):
                  x-sandbox.project_name / workspace_mode / protect_git / ui_port
                                              → [project] name / workspace / protect_git / ui_port
                  build.args INSTALL_<TOOL>   → [tools] <tool> (lower case, e.g. INSTALL_CLAUDE_CODE → claude_code)
                  environment TITLE / SANDBOX_SUDO / TZ / CUSTOM_USER + PASSWORD
                                              → [desktop] title / sudo / timezone / auth
                  other environment entries   → [env]
                  mem_limit / cpus / devices / security_opt
                                              → [desktop] memory / cpus / devices / security_opt
                  volumes (extra folders)     → [[share]] host / path / readonly
                Then delete sandbox.yml.
                """
            );
        if (hasYml)
            Ui.Warn("sandbox.yml is ignored (settings are read from sandbox.toml). Delete it.");
        if (File.Exists(Path.Combine(dir, OldLocalConfigFileName)))
            Ui.Warn(
                "sandbox.local.yml is no longer read. Move its settings to sandbox.toml and delete it."
            );
    }
}
