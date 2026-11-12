// sandbox.yml / sandbox.local.yml: the x-sandbox settings and the INSTALL_* build args.
using YamlDotNet.Core;
using YamlDotNet.RepresentationModel;

sealed record Settings(
    string? ProjectName,
    bool ReadOnly,
    bool ProtectGit,
    int? UiPort,
    List<(string Key, string Value)> Install,
    List<string> Shares
);

/// Reads the x-sandbox block (and the INSTALL_* build args) from sandbox.yml, then sandbox.local.yml.
static class Config
{
    public static Settings Read(string conf, string local)
    {
        if (!File.Exists(conf))
            throw new SandboxException($"{conf} not found.");
        var x = new Dictionary<string, string>();
        var install = new List<(string, string)>();
        var shares = new List<string>();

        foreach (var (path, isLocal) in new[] { (conf, false), (local, true) })
        {
            if (!File.Exists(path))
                continue;
            var root = Load(path);
            if (root == null)
                continue;
            if (root.Children.TryGetValue(Y.Key("x-sandbox"), out var xs))
            {
                if (xs is not YamlMappingNode map)
                    throw new SandboxException(
                        $"{Path.GetFileName(path)}: x-sandbox must be a mapping."
                    );
                foreach (var (k, v) in map.Children)
                    x[((YamlScalarNode)k).Value ?? ""] = v is YamlScalarNode s
                        ? s.Value ?? ""
                        : throw new SandboxException(
                            $"{Path.GetFileName(path)}: x-sandbox.{k} must be a single value."
                        );
            }
            if (At(root, "services", "desktop", "build", "args") is YamlMappingNode argsMap)
                foreach (var (k, v) in argsMap.Children)
                {
                    var key = ((YamlScalarNode)k).Value ?? "";
                    if (!key.StartsWith("INSTALL_"))
                        continue;
                    install.RemoveAll(t => t.Item1 == key);
                    install.Add((key, (v as YamlScalarNode)?.Value?.Trim() ?? ""));
                }
            if (At(root, "services", "desktop", "volumes") is YamlSequenceNode vols)
                foreach (var v in vols)
                {
                    var text = v switch
                    {
                        YamlScalarNode s => s.Value ?? "",
                        YamlMappingNode m when Get(m, "type") != "tmpfs" =>
                            $"{Get(m, "source")} → {Get(m, "target")}{(Get(m, "read_only") == "true" ? " (ro)" : "")}",
                        _ => "",
                    };
                    if (text.Length > 0)
                        shares.Add(isLocal ? text + "  [local]" : text);
                }
        }

        string? Opt(string k) =>
            x.TryGetValue(k, out var v) && !string.IsNullOrWhiteSpace(v) ? v.Trim() : null;

        var mode = Opt("workspace_mode") ?? "rw";
        if (mode is not ("rw" or "ro"))
            throw new SandboxException(
                $"x-sandbox.workspace_mode must be 'rw' or 'ro' (got '{mode}')."
            );
        var protect = Opt("protect_git")?.ToLowerInvariant() ?? "true";
        if (protect is not ("true" or "false"))
            throw new SandboxException(
                $"x-sandbox.protect_git must be true or false (got '{protect}')."
            );
        int? port = null;
        if (Opt("ui_port") is { } p)
            port =
                int.TryParse(p, out var n) && n is > 0 and < 65536
                    ? n
                    : throw new SandboxException(
                        $"x-sandbox.ui_port must be a port number (got '{p}')."
                    );

        return new Settings(
            Opt("project_name"),
            mode == "ro",
            protect == "true",
            port,
            install,
            shares
        );
    }

    public static YamlMappingNode? Load(string path)
    {
        using var reader = new StreamReader(path);
        var ys = new YamlStream();
        try
        {
            ys.Load(reader);
        }
        catch (YamlException e)
        {
            throw new SandboxException(
                $"{Path.GetFileName(path)} is not valid YAML (line {e.Start.Line}, column {e.Start.Column}): {e.InnerException?.Message ?? e.Message}"
            );
        }
        if (ys.Documents.Count == 0)
            return null;
        return ys.Documents[0].RootNode as YamlMappingNode
            ?? throw new SandboxException($"{Path.GetFileName(path)} must be a YAML mapping.");
    }

    static YamlNode? At(YamlNode node, params string[] keys)
    {
        foreach (var k in keys)
            if (node is not YamlMappingNode m || !m.Children.TryGetValue(Y.Key(k), out node!))
                return null;
        return node;
    }

    static string Get(YamlMappingNode m, string k) =>
        m.Children.TryGetValue(Y.Key(k), out var v) && v is YamlScalarNode s ? s.Value ?? "" : "";
}
