// sandbox.toml: reading and validating the project settings.
using System.Globalization;
using System.Text.RegularExpressions;
using Tomlyn;
using Tomlyn.Model;

sealed record Share(string Host, string Path, bool ReadOnly);

sealed record Settings(
    string? ProjectName,
    bool ReadOnly,
    bool ProtectGit,
    int? UiPort,
    List<(string Arg, string Value)> Install, // INSTALL_* build args, in file order
    string Title,
    bool Sudo,
    string Memory,
    string? Cpus,
    string? TimeZone,
    List<string> Devices,
    List<string> SecurityOpt,
    (string User, string Password)? Auth,
    List<(string Key, string Value)> Env,
    List<Share> Shares
);

/// Reads sandbox.toml. Every key is known and typed: unknown keys, wrong types and invalid values are errors.
static class Config
{
    public const string FileName = "sandbox.toml";

    /// Environment variables the sandbox sets itself, with where to change them instead (null = not at all).
    static readonly Dictionary<string, string?> ReservedEnv = new(StringComparer.OrdinalIgnoreCase)
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
    };

    /// <param name="path">sandbox.toml</param>
    /// <param name="projectDir">relative share paths are resolved against it</param>
    /// <param name="dockerfile">engine/desktop/Dockerfile: its ARG INSTALL_* lines are the known tools</param>
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
            var msg = Regex.Replace(e.Message, @"^\(\d+,\d+\)\s*:\s*error\s*:\s*", "");
            throw new SandboxException(
                $"{FileName} is not valid TOML (line {e.Line}, column {e.Column}): {msg}"
            );
        }

        var r = new Reader(root, "");
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
            throw Err($"project.workspace must be \"rw\" or \"ro\" (got \"{workspace}\").");
        var protectGit = project.Bool("protect_git") ?? true;
        var port = project.Int("ui_port");
        if (port is not null and not (> 0 and < 65536))
            throw Err($"project.ui_port must be a port number from 1 to 65535 (got {port}).");
        project.Done();

        // --- [tools] -------------------------------------------------------------------
        var known = KnownTools(dockerfile);
        var install = new List<(string, string)>();
        foreach (var key in tools.Keys)
        {
            var arg = "INSTALL_" + key.ToUpperInvariant();
            if (!Regex.IsMatch(key, "^[a-z0-9_]+$") || !known.Contains(arg))
                throw Err(
                    $"unknown tool tools.{key} (known: {string.Join(", ", known.Select(ToolName))})."
                );
            install.Add((arg, ToolValue(key, tools.Raw(key))));
        }
        tools.Done();

        // --- [desktop] -----------------------------------------------------------------
        var title = desktop.Str("title") ?? "AI Sandbox";
        var sudo = desktop.Bool("sudo") ?? true;
        var memory = desktop.Str("memory") ?? "8g";
        if (!Regex.IsMatch(memory, @"^\d+(\.\d+)?\s*[bkmg]?b?$", RegexOptions.IgnoreCase))
            throw Err($"desktop.memory must be a size like \"8g\" or \"512m\" (got \"{memory}\").");
        var cpus = desktop.Number("cpus");
        if (cpus is <= 0)
            throw Err($"desktop.cpus must be greater than 0 (got {Fmt(cpus.Value)}).");
        var tz = desktop.Str("timezone");
        var devices = desktop.StrList("devices");
        var securityOpt = desktop.StrList("security_opt");
        (string, string)? auth = null;
        if (desktop.OptTable("auth") is { } a)
        {
            var user = a.Str("user");
            var password = a.Str("password");
            a.Done();
            if (string.IsNullOrEmpty(user) || string.IsNullOrEmpty(password))
                throw Err("desktop.auth needs both user and password.");
            auth = (user, password);
        }
        desktop.Done();

        // --- [env] ---------------------------------------------------------------------
        var envVars = new List<(string, string)>();
        foreach (var key in env.Keys)
        {
            if (!Regex.IsMatch(key, "^[A-Za-z_][A-Za-z0-9_]*$"))
                throw Err($"env.{key} is not a valid environment variable name.");
            if (ReservedEnv.TryGetValue(key, out var instead))
                throw Err(
                    instead == null
                        ? $"env.{key} is set by the sandbox and can't be changed."
                        : $"env.{key} is set by the sandbox; use {instead} instead."
                );
            envVars.Add((key, env.Scalar(key)));
        }
        env.Done();

        // --- [[share]] -----------------------------------------------------------------
        var shareList = new List<Share>();
        for (var i = 0; i < shares.Count; i++)
        {
            var s = shares[i];
            var host =
                s.Str("host")
                ?? throw Err($"share #{i + 1} needs host = \"<folder on this machine>\".");
            var full = Path.GetFullPath(Path.Combine(projectDir, host));
            if (!Directory.Exists(full) && !File.Exists(full))
                throw Err($"share #{i + 1}: {host} does not exist ({full}).");
            var target =
                s.Str("path")
                ?? "/shares/" + Path.GetFileName(Path.TrimEndingDirectorySeparator(full));
            if (!target.StartsWith('/'))
                throw Err($"share #{i + 1}: path must be absolute (got \"{target}\").");
            var t = target.TrimEnd('/');
            if (t is "" or "/config" or "/work" || t.StartsWith("/work/"))
                throw Err(
                    $"share #{i + 1}: path {target} is reserved (/, /config and /work are used by the sandbox)."
                );
            if (shareList.Any(x => x.Path.TrimEnd('/') == t))
                throw Err($"share #{i + 1}: path {target} is used twice.");
            shareList.Add(new Share(full, target, s.Bool("readonly") ?? true));
            s.Done();
        }

        return new Settings(
            string.IsNullOrWhiteSpace(name) ? null : name.Trim(),
            workspace == "ro",
            protectGit,
            (int?)port,
            install,
            title,
            sudo,
            memory,
            cpus is { } c ? c.ToString("0.###", CultureInfo.InvariantCulture) : null,
            string.IsNullOrWhiteSpace(tz) ? null : tz.Trim(),
            devices,
            securityOpt,
            auth,
            envVars,
            shareList
        );
    }

    /// INSTALL_CLAUDE_CODE → claude_code
    public static string ToolName(string arg) => arg["INSTALL_".Length..].ToLowerInvariant();

    /// The INSTALL_* build args the Dockerfile declares, in its order.
    public static List<string> KnownTools(string dockerfile) =>
        File.Exists(dockerfile)
            ? File.ReadLines(dockerfile)
                .Select(l => Regex.Match(l, @"^\s*ARG\s+(INSTALL_[A-Z0-9_]+)"))
                .Where(m => m.Success)
                .Select(m => m.Groups[1].Value)
                .Distinct()
                .ToList()
            : [];

    /// true/false, a version ("10.0", 24) or a list (["10.0", "8.0"]) → the build arg value.
    static string ToolValue(string key, object? v) =>
        v switch
        {
            bool b => b ? "true" : "false",
            long n => n.ToString(CultureInfo.InvariantCulture),
            string s => s.Trim(),
            double d => throw Err(
                $"tools.{key} = {Fmt(d)}: write versions as text, e.g. \"{Fmt(d)}\" (a number like 10.0 would lose its \".0\")."
            ),
            TomlArray arr => string.Join(
                " ",
                arr.Select(x =>
                    x switch
                    {
                        string s => s.Trim(),
                        long n => n.ToString(CultureInfo.InvariantCulture),
                        _ => throw Err($"tools.{key} must be a list of names or versions."),
                    }
                )
            ),
            _ => throw Err($"tools.{key} must be true, false, a version or a list."),
        };

    static string Fmt(double d) => d.ToString("0.0###", CultureInfo.InvariantCulture);

    static SandboxException Err(string message) => new($"{FileName}: {message}");

    /// One table: typed getters remember which keys were read; Done() reports the rest as unknown.
    sealed class Reader(TomlTable table, string path)
    {
        readonly HashSet<string> used = [];

        public IEnumerable<string> Keys
        {
            get
            {
                foreach (var k in table.Keys)
                    used.Add(k);
                return table.Keys;
            }
        }

        string Name(string key) => path.Length == 0 ? key : $"{path}.{key}";

        public object? Raw(string key)
        {
            used.Add(key);
            return table.TryGetValue(key, out var v) ? v : null;
        }

        public string? Str(string key) =>
            Raw(key) switch
            {
                null => null,
                string s => s,
                var v => throw Err($"{Name(key)} must be a text in quotes (got {Show(v)})."),
            };

        public bool? Bool(string key) =>
            Raw(key) switch
            {
                null => null,
                bool b => b,
                var v => throw Err($"{Name(key)} must be true or false (got {Show(v)})."),
            };

        public long? Int(string key) =>
            Raw(key) switch
            {
                null => null,
                long n => n,
                var v => throw Err($"{Name(key)} must be a whole number (got {Show(v)})."),
            };

        public double? Number(string key) =>
            Raw(key) switch
            {
                null => null,
                long n => n,
                double d => d,
                var v => throw Err($"{Name(key)} must be a number (got {Show(v)})."),
            };

        /// A text, number or true/false, as text (environment values).
        public string Scalar(string key) =>
            Raw(key) switch
            {
                string s => s,
                bool b => b ? "true" : "false",
                long n => n.ToString(CultureInfo.InvariantCulture),
                double d => Fmt(d),
                var v => throw Err($"{Name(key)} must be a text, number or true/false (got {Show(v)})."),
            };

        public List<string> StrList(string key) =>
            Raw(key) switch
            {
                null => [],
                TomlArray a when a.All(x => x is string) => a.Cast<string>().ToList(),
                var v => throw Err(
                    $"{Name(key)} must be a list of texts, e.g. [\"a\", \"b\"] (got {Show(v)})."
                ),
            };

        public Reader? OptTable(string key) =>
            Raw(key) switch
            {
                null => null,
                TomlTable t => new Reader(t, Name(key)),
                var v => throw Err($"{Name(key)} must be a table (got {Show(v)})."),
            };

        public Reader Table(string key) => OptTable(key) ?? new Reader(new TomlTable(), Name(key));

        public List<Reader> TableArray(string key) =>
            Raw(key) switch
            {
                null => [],
                TomlTableArray a => a.Select(t => new Reader(t, Name(key))).ToList(),
                var v => throw Err(
                    $"{Name(key)} must be written as [[{key}]] blocks (got {Show(v)})."
                ),
            };

        /// Unknown keys are errors, with the known ones listed.
        public void Done(string what = "key")
        {
            var unknown = table.Keys.FirstOrDefault(k => !used.Contains(k));
            if (unknown == null)
                return;
            var known = used.Count > 0 ? $" (known: {string.Join(", ", used)})" : "";
            throw Err(
                what == "table"
                    ? $"unknown setting '{unknown}'{known}."
                    : $"unknown key '{Name(unknown)}'{known}."
            );
        }

        static string Show(object? v) =>
            v switch
            {
                string s => $"\"{s}\"",
                bool b => b ? "true" : "false",
                long n => n.ToString(CultureInfo.InvariantCulture),
                double d => Fmt(d),
                TomlArray => "a list",
                TomlTable => "a table",
                TomlTableArray => "[[...]] blocks",
                _ => v?.ToString() ?? "nothing",
            };
    }
}
