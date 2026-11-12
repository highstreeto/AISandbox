using System.Collections.Frozen;
using System.Globalization;
using System.Text.RegularExpressions;
using Tomlyn;
using Tomlyn.Model;

namespace AiSandbox.Configuration;

/// <summary>
/// Reads <c>sandbox.toml</c>. Every key is known and typed: unknown keys, wrong types and invalid values are
/// errors.
/// </summary>
internal static partial class Config
{
    public const string FileName = "sandbox.toml";

    /// <summary>
    /// Environment variables the sandbox sets itself, with where to change them instead (null = not at all).
    /// </summary>
    private static readonly FrozenDictionary<string, string?> s_reservedEnv = new Dictionary<
        string,
        string?
    >
    {
        ["HTTP_PROXY"] = null,
        ["HTTPS_PROXY"] = null,
        ["NO_PROXY"] = null,
        ["SANDBOX_WORKDIR"] = null,
        ["PUID"] = null,
        ["PGID"] = null,
        ["TZ"] = "desktop.timezone",
        ["TITLE"] = "desktop.title",
        ["SANDBOX_SUDO"] = "desktop.sudo",
        ["CUSTOM_USER"] = "desktop.auth",
        ["PASSWORD"] = "desktop.auth",
    }.ToFrozenDictionary(StringComparer.OrdinalIgnoreCase);

    /// <param name="path">sandbox.toml</param>
    /// <param name="projectDir">Relative share paths are resolved against it.</param>
    /// <param name="dockerfile">The text of the desktop Dockerfile: its <c>ARG INSTALL_*</c> lines are the known tools.</param>
    public static Settings Read(string path, string projectDir, string dockerfile)
    {
        if (!File.Exists(path))
            throw new SandboxException($"{FileName} not found.");
        TomlTable root;
        try
        {
            root = TomlSerializer.Deserialize<TomlTable>(File.ReadAllText(path)) ?? new TomlTable();
        }
        catch (TomlException e)
        {
            var msg = TomlErrorPrefixRegex().Replace(e.Message, "");
            throw new SandboxException(
                $"{FileName} is not valid TOML (line {e.Line}, column {e.Column}): {msg}"
            );
        }

        var r = new TomlTableReader(root, "");
        var project = r.Table("project");
        var tools = r.Table("tools");
        var desktop = r.Table("desktop");
        var env = r.Table("env");
        var shares = r.TableArray("share");
        r.Done("table");

        // --- [project] ----------------------------------------------------------------
        var name = project.Str("name");
        var workspace = project.Str("workspace") ?? "rw";
        if (workspace is not ("rw" or "ro"))
            throw Error($"project.workspace must be \"rw\" or \"ro\" (got \"{workspace}\").");
        var protectGit = project.Bool("protect_git") ?? true;
        var port = project.Int("ui_port");
        if (port is not null and not (> 0 and < 65536))
            throw Error($"project.ui_port must be a port number from 1 to 65535 (got {port}).");
        project.Done();

        // --- [tools] -------------------------------------------------------------------
        var known = KnownTools(dockerfile);
        var install = ImmutableArray.CreateBuilder<ToolSetting>();
        foreach (var key in tools.Keys)
        {
            var arg = "INSTALL_" + key.ToUpperInvariant();
            if (!ToolKeyRegex().IsMatch(key) || !known.Contains(arg))
                throw Error(
                    $"unknown tool tools.{key} (known: {string.Join(", ", known.Select(ToolName))})."
                );
            install.Add(new ToolSetting(arg, ToolValue(key, tools.Raw(key))));
        }
        tools.Done();

        // --- [desktop] -----------------------------------------------------------------
        var title = desktop.Str("title") ?? "AI Sandbox";
        var sudo = desktop.Bool("sudo") ?? true;
        var memory = desktop.Str("memory") ?? "8g";
        if (!MemorySizeRegex().IsMatch(memory))
            throw Error($"desktop.memory must be a size like \"8g\" or \"512m\" (got \"{memory}\").");
        var cpus = desktop.Number("cpus");
        if (cpus is <= 0)
            throw Error($"desktop.cpus must be greater than 0 (got {FormatNumber(cpus.Value)}).");
        var tz = desktop.Str("timezone");
        var devices = desktop.StrList("devices");
        var securityOpt = desktop.StrList("security_opt");
        BasicAuth? auth = null;
        if (desktop.OptTable("auth") is { } a)
        {
            var user = a.Str("user");
            var password = a.Str("password");
            a.Done();
            if (string.IsNullOrEmpty(user) || string.IsNullOrEmpty(password))
                throw Error("desktop.auth needs both user and password.");
            auth = new BasicAuth(user, password);
        }
        desktop.Done();

        // --- [env] ---------------------------------------------------------------------
        var envVars = ImmutableArray.CreateBuilder<EnvVariable>();
        foreach (var key in env.Keys)
        {
            if (!EnvNameRegex().IsMatch(key))
                throw Error($"env.{key} is not a valid environment variable name.");
            if (s_reservedEnv.TryGetValue(key, out var instead))
                throw Error(
                    instead == null
                        ? $"env.{key} is set by the sandbox and can't be changed."
                        : $"env.{key} is set by the sandbox; use {instead} instead."
                );
            envVars.Add(new EnvVariable(key, env.Scalar(key)));
        }
        env.Done();

        // --- [[share]] -----------------------------------------------------------------
        var shareList = ImmutableArray.CreateBuilder<Share>();
        for (var i = 0; i < shares.Length; i++)
        {
            var s = shares[i];
            var host =
                s.Str("host")
                ?? throw Error($"share #{i + 1} needs host = \"<folder on this machine>\".");
            var full = Path.GetFullPath(Path.Combine(projectDir, host));
            if (!Directory.Exists(full) && !File.Exists(full))
                throw Error($"share #{i + 1}: {host} does not exist ({full}).");
            var target =
                s.Str("path")
                ?? "/shares/" + Path.GetFileName(Path.TrimEndingDirectorySeparator(full));
            if (!target.StartsWith('/'))
                throw Error($"share #{i + 1}: path must be absolute (got \"{target}\").");
            var t = target.TrimEnd('/');
            if (t is "" or "/config" or "/work" || t.StartsWith("/work/", StringComparison.Ordinal))
                throw Error(
                    $"share #{i + 1}: path {target} is reserved (/, /config and /work are used by the sandbox)."
                );
            if (shareList.Any(x => x.Path.TrimEnd('/') == t))
                throw Error($"share #{i + 1}: path {target} is used twice.");
            shareList.Add(new Share(full, target, s.Bool("readonly") ?? true));
            s.Done();
        }

        return new Settings(
            string.IsNullOrWhiteSpace(name) ? null : name.Trim(),
            workspace == "ro",
            protectGit,
            (int?)port,
            install.DrainToImmutable(),
            title,
            sudo,
            memory,
            cpus is { } c ? c.ToString("0.###", CultureInfo.InvariantCulture) : null,
            string.IsNullOrWhiteSpace(tz) ? null : tz.Trim(),
            devices,
            securityOpt,
            auth,
            envVars.DrainToImmutable(),
            shareList.DrainToImmutable()
        );
    }

    /// <summary>
    /// INSTALL_CLAUDE_CODE → claude_code
    /// </summary>
    public static string ToolName(string arg) => arg["INSTALL_".Length..].ToLowerInvariant();

    /// <summary>
    /// The INSTALL_* build args the Dockerfile declares, in its order.
    /// </summary>
    public static ImmutableArray<string> KnownTools(string dockerfile) =>
        [
            .. dockerfile
                .ReplaceLineEndings("\n")
                .Split('\n')
                .Select(l => InstallArgRegex().Match(l))
                .Where(m => m.Success)
                .Select(m => m.Groups[1].Value)
                .Distinct(),
        ];

    /// <summary>
    /// An error about <c>sandbox.toml</c>: the message is prefixed with the file name.
    /// </summary>
    public static SandboxException Error(string message) => new($"{FileName}: {message}");

    /// <summary>
    /// A TOML float as written in messages and environment values: at least one decimal.
    /// </summary>
    public static string FormatNumber(double d) => d.ToString("0.0###", CultureInfo.InvariantCulture);

    /// <summary>
    /// true/false, a version ("10.0", 24) or a list (["10.0", "8.0"]) → the build arg value.
    /// </summary>
    private static string ToolValue(string key, object? v) =>
        v switch
        {
            bool b => b ? "true" : "false",
            long n => n.ToString(CultureInfo.InvariantCulture),
            string s => s.Trim(),
            double d => throw Error(
                $"tools.{key} = {FormatNumber(d)}: write versions as text, e.g. \"{FormatNumber(d)}\" (a number like 10.0 would lose its \".0\")."
            ),
            TomlArray arr => string.Join(
                " ",
                arr.Select(x =>
                    x switch
                    {
                        string s => s.Trim(),
                        long n => n.ToString(CultureInfo.InvariantCulture),
                        _ => throw Error($"tools.{key} must be a list of names or versions."),
                    }
                )
            ),
            _ => throw Error($"tools.{key} must be true, false, a version or a list."),
        };

    [GeneratedRegex(@"^\(\d+,\d+\)\s*:\s*error\s*:\s*")]
    private static partial Regex TomlErrorPrefixRegex();

    [GeneratedRegex("^[a-z0-9_]+$")]
    private static partial Regex ToolKeyRegex();

    [GeneratedRegex(@"^\d+(\.\d+)?\s*[bkmg]?b?$", RegexOptions.IgnoreCase)]
    private static partial Regex MemorySizeRegex();

    [GeneratedRegex("^[A-Za-z_][A-Za-z0-9_]*$")]
    private static partial Regex EnvNameRegex();

    [GeneratedRegex(@"^\s*ARG\s+(INSTALL_[A-Z0-9_]+)")]
    private static partial Regex InstallArgRegex();
}
