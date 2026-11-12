// Merges compose layers into one self-contained file, following the Docker Compose merge rules
// (https://docs.docker.com/reference/compose-file/merge/) for everything sandbox.yml realistically uses:
//
//   mappings              merged key by key, recursively; a scalar from the later layer wins
//   environment, labels,  list form (KEY=value) is turned into a mapping first, then merged by key
//   build.args, ...
//   volumes, devices      merged by container path (target); a later entry replaces an earlier one
//   command, entrypoint,  replaced as a whole
//   healthcheck.test
//   other sequences       union (ports, dns, cap_add, ...), duplicates dropped
//   !reset / !override    remove the value / replace it without merging (Compose's own tags)
//
// YAML merge keys (<<: *anchor) are expanded and top-level x-* extensions dropped, so the output has no
// anchors and docker compose needs no other file. Relative paths stay relative to --project-directory.
using System.Text;
using System.Text.RegularExpressions;
using YamlDotNet.Core;
using YamlDotNet.RepresentationModel;

sealed record ComposeLayer(string Name, YamlMappingNode Root);

static class ComposeMerger
{
    const string Reset = "!reset";
    const string Override = "!override";

    /// Keys whose children are names (services, volumes, ...): their paths use "*" for the name.
    static readonly HashSet<string> NamedSections =
    [
        "services",
        "volumes",
        "networks",
        "configs",
        "secrets",
    ];

    static readonly HashSet<string> Replaced =
    [
        "services.*.command",
        "services.*.entrypoint",
        "services.*.healthcheck.test",
    ];

    /// Always merged as mappings (list form is converted first).
    static readonly HashSet<string> KeyValue =
    [
        "services.*.environment",
        "services.*.labels",
        "services.*.annotations",
        "services.*.sysctls",
        "services.*.build.args",
        "services.*.build.labels",
        "services.*.deploy.labels",
    ];

    /// Either form is fine; converted to a mapping only when the two layers use different forms.
    static readonly HashSet<string> ListOrMap =
    [
        "services.*.networks",
        "services.*.depends_on",
        "services.*.extra_hosts",
    ];

    static readonly HashSet<string> ByTarget = ["services.*.volumes", "services.*.devices"];

    /// Short string forms of mappings: `build: ./dir` is `build: {context: ./dir}`.
    static readonly Dictionary<string, string> ShortForm = new()
    {
        ["services.*.build"] = "context",
    };

    /// A compose file as a layer: merge keys expanded, unsupported features rejected.
    public static ComposeLayer Layer(string name, YamlMappingNode? root)
    {
        var r = root == null ? new YamlMappingNode() : (YamlMappingNode)ExpandCopy(root, name);
        if (r.Children.ContainsKey(Y.Key("include")))
            throw new SandboxException($"{name}: 'include' is not supported.");
        if (r.Children.TryGetValue(Y.Key("services"), out var services))
        {
            if (services is not YamlMappingNode sm)
                throw new SandboxException($"{name}: services must be a mapping.");
            foreach (var (k, v) in sm.Children)
                if (v is YamlMappingNode svc && svc.Children.ContainsKey(Y.Key("extends")))
                    throw new SandboxException(
                        $"{name}: services.{((YamlScalarNode)k).Value}.extends is not supported."
                    );
        }
        return new ComposeLayer(name, r);
    }

    /// All layers merged in order; later layers win.
    public static YamlMappingNode Merge(IEnumerable<ComposeLayer> layers)
    {
        YamlNode result = new YamlMappingNode();
        foreach (var l in layers)
            result =
                MergeNode(result, l.Root, "", l.Name)
                ?? throw new SandboxException($"{l.Name}: the whole file can't be reset.");
        var root = (YamlMappingNode)result;
        foreach (var k in root.Children.Keys.ToList())
            if (k is YamlScalarNode { Value: { } s } && s.StartsWith("x-"))
                root.Children.Remove(k);
        return root;
    }

    /// The merged file as text: header comment, LF line endings, no document-end marker.
    public static string Write(YamlMappingNode root, string header)
    {
        using var w = new StringWriter { NewLine = "\n" };
        foreach (var line in header.Split('\n'))
            w.WriteLine(line.Length == 0 ? "#" : "# " + line);
        WriteNulls(root);
        new YamlStream(new YamlDocument(root)).Save(w, assignAnchors: false);
        return Regex.Replace(w.ToString().Replace("\r\n", "\n"), @"\n\.\.\.\n*$", "\n");
    }

    /// `home:` (null) would otherwise come out as `home: ''`, which Compose reads as a string.
    static void WriteNulls(YamlNode n)
    {
        switch (n)
        {
            case YamlScalarNode s when IsNull(s):
                s.Value = "null";
                s.Style = ScalarStyle.Plain;
                break;
            case YamlMappingNode m:
                foreach (var v in m.Children.Values)
                    WriteNulls(v);
                break;
            case YamlSequenceNode q:
                foreach (var c in q.Children)
                    WriteNulls(c);
                break;
        }
    }

    static bool IsNull(YamlScalarNode s) =>
        s.Tag.IsEmpty
        && (
            s.Value == null
            || (
                s.Style is ScalarStyle.Plain or ScalarStyle.Any
                && s.Value is "" or "~" or "null" or "Null" or "NULL"
            )
        );

    // --- merging ----------------------------------------------------------------

    static YamlNode? MergeNode(YamlNode? b, YamlNode o, string path, string layer)
    {
        switch (TagOf(o))
        {
            case Reset:
                return null;
            case Override:
                return CleanCopy(o);
        }
        if (b == null || Replaced.Contains(path))
            return CleanCopy(o);

        if (ShortForm.TryGetValue(path, out var field) && b.NodeType != o.NodeType)
        {
            b = Expanded(b, field);
            o = Expanded(o, field);
        }

        if (KeyValue.Contains(path) || (ListOrMap.Contains(path) && b.NodeType != o.NodeType))
        {
            b = ToMap(b, path, layer);
            o = ToMap(o, path, layer);
        }

        switch (b, o)
        {
            case (YamlMappingNode bm, YamlMappingNode om):
            {
                var r = (YamlMappingNode)CleanCopy(bm);
                foreach (var (k, v) in om.Children)
                {
                    var key = CleanCopy(k);
                    r.Children.TryGetValue(key, out var existing);
                    var merged = MergeNode(
                        existing,
                        v,
                        Child(path, (k as YamlScalarNode)?.Value ?? ""),
                        layer
                    );
                    if (merged == null)
                        r.Children.Remove(key);
                    else
                        r.Children[key] = merged;
                }
                return r;
            }
            case (YamlSequenceNode bs, YamlSequenceNode os):
                return MergeSequence(bs, os, path);
            default:
                return CleanCopy(o);
        }
    }

    static YamlSequenceNode MergeSequence(YamlSequenceNode b, YamlSequenceNode o, string path)
    {
        Func<YamlNode, string> key = ByTarget.Contains(path) ? TargetOf : Canonical;
        var r = (YamlSequenceNode)CleanCopy(b);
        foreach (var item in o.Children)
        {
            if (TagOf(item) == Reset)
                continue;
            var k = key(item);
            var i = r.Children.ToList().FindIndex(x => key(x) == k);
            if (i >= 0)
                r.Children[i] = CleanCopy(item);
            else
                r.Children.Add(CleanCopy(item));
        }
        return r;
    }

    static YamlNode Expanded(YamlNode n, string field) =>
        n is YamlScalarNode s ? new YamlMappingNode { { field, CleanCopy(s) } } : n;

    static string Child(string path, string key) =>
        path.Length == 0 ? key
        : NamedSections.Contains(path) ? $"{path}.*"
        : $"{path}.{key}";

    /// `KEY=value` / `KEY` (environment, labels, ...), `host:ip` (extra_hosts), `name` (networks, depends_on).
    static YamlNode ToMap(YamlNode n, string path, string layer)
    {
        if (n is not YamlSequenceNode seq)
            return n;
        var m = new YamlMappingNode();
        foreach (var item in seq.Children)
        {
            var s =
                (item as YamlScalarNode)?.Value
                ?? throw new SandboxException($"{layer}: {path} entries must be single values.");
            if (path.EndsWith(".networks"))
                m.Children[Y.Key(s)] = new YamlScalarNode(null);
            else if (path.EndsWith(".depends_on"))
                m.Children[Y.Key(s)] = new YamlMappingNode { { "condition", "service_started" } };
            else
            {
                var sep = path.EndsWith(".extra_hosts") ? s.IndexOfAny(['=', ':']) : s.IndexOf('=');
                m.Children[Y.Key(sep < 0 ? s : s[..sep])] =
                    sep < 0 ? new YamlScalarNode(null) : Y.Str(s[(sep + 1)..]);
            }
        }
        return m;
    }

    /// The container path of a volume or device: `target:` in the long form, the second part of `src:dst[:mode]`.
    static string TargetOf(YamlNode n)
    {
        if (n is YamlMappingNode m)
            return m.Children.TryGetValue(Y.Key("target"), out var t) && t is YamlScalarNode ts
                ? ts.Value ?? ""
                : Canonical(n);
        var parts = SplitVolume((n as YamlScalarNode)?.Value ?? "");
        return parts.Count == 1 ? parts[0] : parts[1];
    }

    /// Splits `src:dst[:mode]`, keeping Windows drive letters (`C:/x`, `C:\x`) in one part.
    static List<string> SplitVolume(string s)
    {
        var parts = new List<string>();
        var i = 0;
        while (true)
        {
            var start = i;
            if (
                i + 2 < s.Length
                && char.IsLetter(s[i])
                && s[i + 1] == ':'
                && s[i + 2] is '/' or '\\'
            )
                i += 2;
            var j = s.IndexOf(':', i);
            if (j < 0)
            {
                parts.Add(s[start..]);
                return parts;
            }
            parts.Add(s[start..j]);
            i = j + 1;
        }
    }

    // --- node helpers -----------------------------------------------------------

    static string? TagOf(YamlNode n) => n.Tag.IsEmpty ? null : n.Tag.Value;

    /// A deep copy without the Compose merge tags (and without children marked !reset).
    static YamlNode CleanCopy(YamlNode n)
    {
        var tag = TagOf(n) is Reset or Override ? default : n.Tag;
        switch (n)
        {
            case YamlMappingNode m:
            {
                var r = new YamlMappingNode { Style = m.Style, Tag = tag };
                foreach (var (k, v) in m.Children)
                    if (TagOf(v) != Reset)
                        r.Children[CleanCopy(k)] = CleanCopy(v);
                return r;
            }
            case YamlSequenceNode s:
            {
                var r = new YamlSequenceNode { Style = s.Style, Tag = tag };
                foreach (var c in s.Children)
                    if (TagOf(c) != Reset)
                        r.Add(CleanCopy(c));
                return r;
            }
            case YamlScalarNode s:
                return new YamlScalarNode(s.Value) { Style = s.Style, Tag = tag };
            default:
                return n;
        }
    }

    /// A deep copy with YAML merge keys (`<<: *anchor` or `<<: [*a, *b]`) applied: explicit keys first,
    /// then the merged-in keys they don't override. Aliases become independent copies.
    static YamlNode ExpandCopy(YamlNode n, string layer)
    {
        switch (n)
        {
            case YamlMappingNode m:
            {
                var r = new YamlMappingNode { Style = m.Style, Tag = m.Tag };
                YamlNode? merge = null;
                foreach (var (k, v) in m.Children)
                    if (k is YamlScalarNode { Value: "<<" })
                        merge = v;
                    else
                        r.Children[ExpandCopy(k, layer)] = ExpandCopy(v, layer);
                if (merge != null)
                    foreach (var src in merge is YamlSequenceNode ms ? ms.Children : [merge])
                    {
                        if (ExpandCopy(src, layer) is not YamlMappingNode sm)
                            throw new SandboxException(
                                $"{layer} (line {src.Start.Line}): '<<' must refer to a mapping."
                            );
                        foreach (var (k, v) in sm.Children)
                            r.Children.TryAdd(k, v);
                    }
                return r;
            }
            case YamlSequenceNode s:
                return new YamlSequenceNode(s.Children.Select(c => ExpandCopy(c, layer)))
                {
                    Style = s.Style,
                    Tag = s.Tag,
                };
            case YamlScalarNode s:
                return new YamlScalarNode(s.Value) { Style = s.Style, Tag = s.Tag };
            default:
                return n;
        }
    }

    /// A text form of a node for comparing sequence entries (style and tags ignored, mapping keys sorted).
    static string Canonical(YamlNode n)
    {
        var sb = new StringBuilder();
        Walk(n);
        return sb.ToString();

        void Walk(YamlNode x)
        {
            switch (x)
            {
                case YamlScalarNode s:
                    sb.Append('"')
                        .Append(s.Value?.Replace("\\", "\\\\").Replace("\"", "\\\""))
                        .Append('"');
                    break;
                case YamlSequenceNode s:
                    sb.Append('[');
                    foreach (var c in s.Children)
                    {
                        Walk(c);
                        sb.Append(',');
                    }
                    sb.Append(']');
                    break;
                case YamlMappingNode m:
                    sb.Append('{');
                    foreach (
                        var (k, v) in m.Children.OrderBy(
                            kv => (kv.Key as YamlScalarNode)?.Value,
                            StringComparer.Ordinal
                        )
                    )
                    {
                        Walk(k);
                        sb.Append(':');
                        Walk(v);
                        sb.Append(',');
                    }
                    sb.Append('}');
                    break;
            }
        }
    }
}
