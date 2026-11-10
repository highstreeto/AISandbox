// allowlist.txt: the proxy's host allowlist, edited line by line so comments and groups are kept.
using System.Text;
using System.Text.RegularExpressions;

enum AllowResult
{
    AlreadyAllowed,
    Uncommented,
    Added,
}

sealed record AllowEntry(string Host, string Group, bool Active, string Note);

sealed class Allowlist
{
    static readonly Regex HostRx = new(@"^[A-Za-z0-9*][A-Za-z0-9*._:-]*$");
    static readonly Regex GroupRx = new(@"^#\s*---\s*(.+?)\s*-*\s*$");
    static readonly Regex DisabledRx = new(
        @"^#\s*([A-Za-z0-9*][A-Za-z0-9*._:-]*\.[A-Za-z0-9*._:-]+)\s*(?:#\s*(.*))?$"
    );

    readonly string _path;
    readonly List<string> _lines;

    Allowlist(string path, List<string> lines)
    {
        _path = path;
        _lines = lines;
    }

    public static Allowlist Load(string path)
    {
        if (!File.Exists(path))
            throw new SandboxException($"{path} not found.");
        var lines = File.ReadAllText(path).Replace("\r\n", "\n").Split('\n').ToList();
        if (lines.Count > 0 && lines[^1] == "")
            lines.RemoveAt(lines.Count - 1);
        return new Allowlist(path, lines);
    }

    public void Save() =>
        File.WriteAllText(_path, string.Join("\n", _lines) + "\n", new UTF8Encoding(false)); // LF, no BOM

    public static string ValidateHost(string h) =>
        HostRx.IsMatch(h) ? h : throw new SandboxException($"Not a valid host name: '{h}'.");

    static Regex ActiveLine(string h) => new($"^{Regex.Escape(h)}(\\s|$)", RegexOptions.IgnoreCase);

    public AllowResult Allow(string host)
    {
        var active = ActiveLine(host);
        if (_lines.Any(l => active.IsMatch(l)))
            return AllowResult.AlreadyAllowed;
        var commented = new Regex(
            $"^#\\s*({Regex.Escape(host)}(\\s.*)?)$",
            RegexOptions.IgnoreCase
        );
        var i = _lines.FindIndex(l => commented.IsMatch(l));
        if (i >= 0)
        {
            _lines[i] = commented.Match(_lines[i]).Groups[1].Value;
            return AllowResult.Uncommented;
        }
        _lines.Add(host);
        return AllowResult.Added;
    }

    public bool Deny(string host)
    {
        var active = ActiveLine(host);
        var any = false;
        for (var i = 0; i < _lines.Count; i++)
            if (active.IsMatch(_lines[i]))
            {
                _lines[i] = "# " + _lines[i];
                any = true;
            }
        return any;
    }

    public IEnumerable<AllowEntry> Entries()
    {
        string? group = null; // disabled entries are only recognised below the first "# --- group ---" header
        foreach (var raw in _lines)
        {
            var line = raw.Trim();
            if (line.Length == 0)
                continue;
            if (GroupRx.Match(line) is { Success: true } g)
            {
                group = g.Groups[1].Value;
                continue;
            }
            if (!line.StartsWith('#'))
            {
                var host = Regex.Replace(line, @"\s*#.*$", "");
                if (host.Length > 0)
                    yield return new AllowEntry(host, group ?? "", true, "");
            }
            else if (group != null && DisabledRx.Match(line) is { Success: true } d)
                yield return new AllowEntry(d.Groups[1].Value, group, false, d.Groups[2].Value);
        }
    }

    /// Same matching as tinyproxy's fnmatch filter: exact host or shell-style wildcard, case-insensitive.
    public bool IsAllowed(string host) =>
        Entries()
            .Where(e => e.Active)
            .Any(e =>
                Regex.IsMatch(
                    host,
                    "^" + Regex.Escape(e.Host).Replace("\\*", ".*").Replace("\\?", ".") + "$",
                    RegexOptions.IgnoreCase
                )
            );
}
