// The project: identity and paths, docker compose calls, and the generated compose override.
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using YamlDotNet.Core;
using YamlDotNet.RepresentationModel;

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
    public string ConfPath => Path.Combine(Dir, "sandbox.yml");
    public string LocalConfPath => Path.Combine(Dir, "sandbox.local.yml"); // optional, machine-specific, not committed
    public string AllowlistPath => Path.Combine(Dir, "allowlist.txt");
    string StateDir => Path.Combine(Dir, ".state");
    string GenPath => Path.Combine(StateDir, "compose.generated.yml");

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

        var settings = Config.Read(
            Path.Combine(dir, "sandbox.yml"),
            Path.Combine(dir, "sandbox.local.yml")
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

    /// The folder this .cs file lives in; falls back to searching upwards from the current directory.
    static string LocateSandboxDir()
    {
        static bool IsSandbox(string? d) =>
            d != null
            && File.Exists(Path.Combine(d, "sandbox.yml"))
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
            "Could not find the .sandbox folder (with sandbox.yml and engine/)."
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
            Path.Combine(Dir, "engine", "compose.yml"),
            "-f",
            GenPath,
            "-f",
            ConfPath,
        };
        if (File.Exists(LocalConfPath))
            a.AddRange(["-f", LocalConfPath]);
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
                $"Another project already uses the container group '{Name}'. Set x-sandbox.project_name in sandbox.yml."
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
            : text.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
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

    // --- generated compose override -------------------------------------------
    /// Writes .state/compose.generated.yml: the host-specific part of the compose setup.
    public void Generate()
    {
        var root = new YamlMappingNode
        {
            {
                "services",
                new YamlMappingNode
                {
                    {
                        "gateway",
                        new YamlMappingNode
                        {
                            {
                                "ports",
                                new YamlSequenceNode(Y.Str($"127.0.0.1:{Settings.UiPort}:3001"))
                            },
                        }
                    },
                    {
                        "desktop",
                        new YamlMappingNode
                        {
                            {
                                "labels",
                                new YamlMappingNode { { "sandbox.id", Y.Str(Id) } }
                            },
                            { "environment", DesktopEnvironment() },
                            { "volumes", DesktopVolumes() },
                        }
                    },
                }
            },
        };

        using var w = new StringWriter { NewLine = "\n" };
        w.WriteLine("# Generated by .sandbox/sandbox — do not edit, change sandbox.yml instead.");
        new YamlStream(new YamlDocument(root)).Save(w, assignAnchors: false);
        var text = Regex.Replace(w.ToString().Replace("\r\n", "\n"), @"\n\.\.\.\n*$", "\n"); // drop the document-end marker
        File.WriteAllText(GenPath, text, new UTF8Encoding(false));
    }

    YamlMappingNode DesktopEnvironment()
    {
        var (uid, gid) = Native.UserIds();
        var env = new YamlMappingNode
        {
            { "SANDBOX_WORKDIR", Y.Str(WorkDir) },
            { "PUID", Y.Str(uid.ToString()) },
            { "PGID", Y.Str(gid.ToString()) },
        };
        if (TimeZone() is { } tz)
            env.Add("TZ", Y.Str(tz));
        return env;
    }

    YamlSequenceNode DesktopVolumes()
    {
        var volumes = new YamlSequenceNode(
            Y.Bind(ProjectDir, WorkDir, Settings.ReadOnly),
            // The sandbox config is visible but read-only: the agent can't widen its allowlist.
            Y.Bind(Dir, $"{WorkDir}/{Path.GetFileName(Dir)}", true)
        );
        if (Settings.ProtectGit && !Settings.ReadOnly)
            foreach (var p in new[] { "config", "hooks" })
            {
                var src = Path.Combine(ProjectDir, ".git", p);
                if (File.Exists(src) || Directory.Exists(src))
                    volumes.Add(Y.Bind(src, $"{WorkDir}/.git/{p}", true));
            }
        return volumes;
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

/// YAML node helpers for the generated file: strings are always quoted, so paths and ports stay strings.
static class Y
{
    public static YamlScalarNode Key(string k) => new(k);

    public static YamlScalarNode Str(string s) => new(s) { Style = ScalarStyle.DoubleQuoted };

    public static YamlScalarNode Bool(bool b) =>
        new(b ? "true" : "false") { Style = ScalarStyle.Plain };

    public static YamlMappingNode Bind(string source, string target, bool readOnly) =>
        new(
            new KeyValuePair<YamlNode, YamlNode>[]
            {
                new(Key("type"), new YamlScalarNode("bind")),
                new(Key("source"), Str(source)),
                new(Key("target"), Str(target)),
                new(Key("read_only"), Bool(readOnly)),
            }
        )
        {
            Style = YamlDotNet.Core.Events.MappingStyle.Flow,
        };
}
