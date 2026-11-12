// The project: identity and paths, docker compose calls, and the generated compose file.
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Scriban.Runtime;

sealed class SandboxException(string message) : Exception(message);

sealed record ServiceState(
    string Service,
    string State,
    string Health,
    string Status,
    string Ports
);

sealed record Sandbox(
    string Dir,
    string ProjectDir,
    string Id,
    string Name,
    string WorkDir,
    Settings Settings,
    bool Verbose
)
{
    // --- identity and paths ---------------------------------------------------
    public string ConfPath => Path.Combine(Dir, Config.FileName);
    public string AllowlistPath => Path.Combine(Dir, "allowlist.txt");
    public string StateDir => Path.Combine(Dir, ".state");
    public string TemplatePath => Path.Combine(Dir, "engine", "compose.yml.sbn");
    public string DockerfilePath => Path.Combine(Dir, "engine", "desktop", "Dockerfile");
    public string GenPath => Path.Combine(StateDir, "compose.yml");

    public static Sandbox Load(bool verbose = false)
    {
        var dir = LocateSandboxDir();
        var projectDir = Path.GetDirectoryName(dir)!;
        var stateDir = Path.Combine(dir, ".state");
        Directory.CreateDirectory(stateDir);

        var idFile = Path.Combine(stateDir, "id"); // created once: stable even if the project is moved
        var id = File.Exists(idFile) ? File.ReadAllText(idFile).Trim() : "";
        if (id.Length == 0)
        {
            id = Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(projectDir)))[..8];
            File.WriteAllText(idFile, id + "\n");
        }

        CheckOldConfig(dir);
        var settings = Config.Read(
            Path.Combine(dir, Config.FileName),
            projectDir,
            Path.Combine(dir, "engine", "desktop", "Dockerfile")
        );
        var baseName = Sanitize(Path.GetFileName(projectDir));
        return new Sandbox(
            Dir: dir,
            ProjectDir: projectDir,
            Id: id,
            Settings: settings,
            Verbose: verbose,
            Name: Sanitize(
                string.IsNullOrWhiteSpace(settings.ProjectName) ? baseName : settings.ProjectName
            ),
            WorkDir: $"/work/{baseName}-{id}"
        );
    }

    /// sandbox.yml / sandbox.local.yml from before sandbox.toml: an error with the key mapping, or a warning.
    static void CheckOldConfig(string dir)
    {
        var hasToml = File.Exists(Path.Combine(dir, Config.FileName));
        var hasYml = File.Exists(Path.Combine(dir, "sandbox.yml"));
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
        if (File.Exists(Path.Combine(dir, "sandbox.local.yml")))
            Ui.Warn(
                "sandbox.local.yml is no longer read. Move its settings to sandbox.toml and delete it."
            );
    }

    /// The folder this .cs file lives in; falls back to searching upwards from the current directory.
    static string LocateSandboxDir()
    {
        static bool IsSandbox(string? d) =>
            d != null
            && (
                File.Exists(Path.Combine(d, Config.FileName))
                || File.Exists(Path.Combine(d, "sandbox.yml"))
            )
            && Directory.Exists(Path.Combine(d, "engine"));

        if (AppContext.GetData("EntryPointFileDirectoryPath") is string ep && IsSandbox(ep))
            return Physical(ep);
        for (var d = new DirectoryInfo(Environment.CurrentDirectory); d != null; d = d.Parent)
        {
            if (IsSandbox(d.FullName))
                return Physical(d.FullName);
            var sub = Path.Combine(d.FullName, ".sandbox");
            if (IsSandbox(sub))
                return Physical(sub);
        }
        throw new SandboxException(
            "Could not find the .sandbox folder (with sandbox.toml and engine/)."
        );
    }

    static string Physical(string path) // like `pwd -P`: resolve a symlinked folder
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

    public static string Sanitize(string s)
    {
        s = Regex.Replace(s.ToLowerInvariant(), "[^a-z0-9_-]+", "-");
        s = Regex.Replace(Regex.Replace(s, "^[-_]+", ""), "-+$", "");
        return s.Length > 0 ? s : "sandbox";
    }

    // --- docker compose -------------------------------------------------------
    List<string> ComposeArgs(IEnumerable<string> args)
    {
        var a = new List<string>
        {
            "compose",
            "-p",
            Name,
            "--project-directory",
            Dir,
            "-f",
            GenPath,
        };
        a.AddRange(args);
        return a;
    }

    /// Run docker compose with captured output (for spinners, tables and error panels).
    public ProcResult Compose(params string[] args) =>
        Proc.Capture(Verbose, "docker", [.. ComposeArgs(["--ansi", "never", .. args])]);

    /// Run docker compose attached to this terminal (build output, shell, logs, claude).
    public int ComposeInteractive(params string[] args) =>
        Proc.Interactive(Verbose, "docker", [.. ComposeArgs(args)]);

    /// Run a command in the desktop container as the desktop user, in the project folder, attached to this terminal.
    public int Exec(params string[] command) =>
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

    public string? UiUrl()
    {
        var r = Compose("port", "gateway", "3001");
        var p = r.ExitCode == 0 ? r.Lines().FirstOrDefault() : null;
        return string.IsNullOrEmpty(p) || p.EndsWith(":0")
            ? null
            : $"https://localhost:{p[(p.LastIndexOf(':') + 1)..]}";
    }

    public List<ServiceState> Services()
    {
        var r = Compose("ps", "-a", "--format", "json");
        return r.ExitCode != 0 || string.IsNullOrWhiteSpace(r.StdOut)
            ? []
            : ParsePs(r.StdOut).OrderBy(s => s.Service).ToList();
    }

    /// Newer compose prints one JSON object per line, older versions a single array.
    static IEnumerable<ServiceState> ParsePs(string json)
    {
        var text = json.Trim();
        var docs = text.StartsWith('[')
            ? [text]
            : text.Split(
                '\n',
                StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries
            );
        foreach (var d in docs)
        {
            using var doc = JsonDocument.Parse(d);
            var items =
                doc.RootElement.ValueKind == JsonValueKind.Array
                    ? doc.RootElement.EnumerateArray().ToList()
                    : [doc.RootElement];
            foreach (var e in items)
                yield return new ServiceState(
                    Str(e, "Service"),
                    Str(e, "State"),
                    Str(e, "Health"),
                    Str(e, "Status"),
                    FormatPorts(e)
                );
        }

        static string Str(JsonElement e, string k) =>
            e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.String
                ? v.GetString() ?? ""
                : "";
    }

    /// The published ports of one `compose ps` entry, e.g. "127.0.0.1:49153→3001".
    static string FormatPorts(JsonElement e)
    {
        if (!e.TryGetProperty("Publishers", out var pub) || pub.ValueKind != JsonValueKind.Array)
            return "";
        return string.Join(
            ", ",
            pub.EnumerateArray()
                .Where(x => x.TryGetProperty("PublishedPort", out var pp) && pp.GetInt32() > 0)
                .Select(x =>
                    $"{x.GetProperty("URL").GetString()}:{x.GetProperty("PublishedPort").GetInt32()}→{x.GetProperty("TargetPort").GetInt32()}"
                )
                .Distinct()
        );
    }

    // --- generated compose file ----------------------------------------------
    /// Writes .state/compose.yml, the only file docker compose runs with: engine/compose.yml.sbn rendered
    /// with the settings from sandbox.toml and this machine's facts (user ids, time zone, mounts, UI port).
    /// The UI port, the project mounts and the proxy settings are written by the template itself, so
    /// sandbox.toml has no way to change them.
    public void Generate()
    {
        var text = ComposeTemplate.Render(TemplatePath, Model());
        File.WriteAllText(GenPath, text, new UTF8Encoding(false));
        File.Delete(Path.Combine(StateDir, "compose.generated.yml")); // left over from the multi-file setup
    }

    /// The template's variables: host, tools, desktop, env, mounts.
    public ScriptObject Model()
    {
        var s = Settings;
        var (uid, gid) = Native.UserIds();

        var auth = s.Auth is { } a ? Obj(("user", a.User), ("password", a.Password)) : null;
        return Obj(
            (
                "host",
                Obj(
                    ("id", Id),
                    ("workdir", WorkDir),
                    ("uid", uid.ToString()),
                    ("gid", gid.ToString()),
                    ("ui_port", s.UiPort?.ToString() ?? "")
                )
            ),
            ("tools", Arr(s.Install.Select(t => Obj(("arg", t.Arg), ("value", t.Value))))),
            (
                "desktop",
                Obj(
                    ("title", s.Title),
                    ("sudo", s.Sudo ? "true" : "false"),
                    ("auth", auth),
                    ("timezone", s.TimeZone ?? TimeZone()),
                    ("memory", s.Memory),
                    ("cpus", s.Cpus),
                    ("devices", Arr(s.Devices)),
                    ("security_opt", Arr(s.SecurityOpt))
                )
            ),
            ("env", Arr(s.Env.Select(e => Obj(("key", e.Key), ("value", e.Value))))),
            ("mounts", Arr(Mounts().Select(m => Obj(
                ("source", m.Source),
                ("target", m.Target),
                ("read_only", m.ReadOnly)
            )))));
    }

    static ScriptObject Obj(params (string Key, object? Value)[] members)
    {
        var o = new ScriptObject();
        foreach (var (k, v) in members)
            o[k] = v;
        return o;
    }

    static ScriptArray Arr(IEnumerable<object> items) => new(items);

    /// Bind mounts of the desktop: the project, the read-only .sandbox, the protected .git paths, the shares.
    List<(string Source, string Target, bool ReadOnly)> Mounts()
    {
        var mounts = new List<(string, string, bool)>
        {
            (ProjectDir, WorkDir, Settings.ReadOnly),
            // The sandbox config is visible but read-only: the agent can't widen its allowlist.
            (Dir, $"{WorkDir}/{Path.GetFileName(Dir)}", true),
        };
        if (Settings.ProtectGit && !Settings.ReadOnly)
            foreach (var p in new[] { "config", "hooks" })
            {
                var src = Path.Combine(ProjectDir, ".git", p);
                if (File.Exists(src) || Directory.Exists(src))
                    mounts.Add((src, $"{WorkDir}/.git/{p}", true));
            }
        foreach (var sh in Settings.Shares)
            mounts.Add((sh.Host, sh.Path, sh.ReadOnly));
        return mounts;
    }

    static string? TimeZone()
    {
        var tz = Environment.GetEnvironmentVariable("TZ");
        if (!string.IsNullOrWhiteSpace(tz))
            return tz;
        var local = TimeZoneInfo.Local;
        if (local.HasIanaId)
            return local.Id is "Local" or "" ? null : local.Id;
        return TimeZoneInfo.TryConvertWindowsIdToIanaId(local.Id, out var iana) ? iana : null;
    }
}
