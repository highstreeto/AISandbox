using System.Text;
using System.Text.RegularExpressions;

namespace AiSandbox.Network;

/// <summary>
/// <c>allowlist.txt</c>: the proxy's host allowlist, edited line by line so comments and groups are kept.
/// </summary>
internal sealed partial class Allowlist
{
    private readonly string _path;
    private readonly List<string> _lines; // the edit buffer, written back by Save()

    private Allowlist(string path, List<string> lines)
    {
        _path = path;
        _lines = lines;
    }

    public static Allowlist Load(string path)
    {
        if (!File.Exists(path))
            throw new SandboxException($"{path} not found.");
        var lines = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n').ToList();
        if (lines.Count > 0 && lines[^1].Length == 0)
            lines.RemoveAt(lines.Count - 1);
        return new Allowlist(path, lines);
    }

    /// <summary>
    /// Writes the file back with LF line endings and no BOM.
    /// </summary>
    public void Save() =>
        File.WriteAllText(_path, string.Join("\n", _lines) + "\n", new UTF8Encoding(false));

    public static string ValidateHost(string h) =>
        HostRegex().IsMatch(h) ? h : throw new SandboxException($"Not a valid host name: '{h}'.");

    public AllowResult Allow(string host)
    {
        var active = ActiveLine(host);
        if (_lines.Any(active.IsMatch))
            return AllowResult.AlreadyAllowed;
        var commented = new Regex($"^#\\s*({Regex.Escape(host)}(\\s.*)?)$", RegexOptions.IgnoreCase);
        var i = _lines.FindIndex(commented.IsMatch);
        if (i >= 0)
        {
            _lines[i] = commented.Match(_lines[i]).Groups[1].Value;
            return AllowResult.Uncommented;
        }
        _lines.Add(host);
        return AllowResult.Added;
    }

    /// <summary>
    /// Comments out every active line for the host; false if there was none.
    /// </summary>
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

    /// <summary>
    /// Active entries, and commented-out ones below the first <c># --- group ---</c> header.
    /// </summary>
    public ImmutableArray<AllowEntry> Entries()
    {
        var entries = ImmutableArray.CreateBuilder<AllowEntry>();
        string? group = null; // disabled entries are only recognised below the first group header
        foreach (var raw in _lines)
        {
            var line = raw.Trim();
            if (line.Length == 0)
                continue;
            if (GroupRegex().Match(line) is { Success: true } g)
            {
                group = g.Groups[1].Value;
                continue;
            }
            if (!line.StartsWith('#'))
            {
                var host = TrailingCommentRegex().Replace(line, "");
                if (host.Length > 0)
                    entries.Add(new AllowEntry(host, group ?? "", Active: true, ""));
            }
            else if (group != null && DisabledRegex().Match(line) is { Success: true } d)
                entries.Add(new AllowEntry(d.Groups[1].Value, group, Active: false, d.Groups[2].Value));
        }
        return entries.DrainToImmutable();
    }

    /// <summary>
    /// Same matching as tinyproxy's fnmatch filter: exact host or shell-style wildcard, case-insensitive.
    /// </summary>
    public bool IsAllowed(string host) =>
        Entries()
            .Where(e => e.Active)
            .Any(e =>
                Regex.IsMatch(
                    host,
                    "^" + Regex.Escape(e.Host).Replace("\\*", ".*", StringComparison.Ordinal).Replace("\\?", ".", StringComparison.Ordinal) + "$",
                    RegexOptions.IgnoreCase
                )
            );

    private static Regex ActiveLine(string host) =>
        new($"^{Regex.Escape(host)}(\\s|$)", RegexOptions.IgnoreCase);

    [GeneratedRegex(@"^[A-Za-z0-9*][A-Za-z0-9*._:-]*$")]
    private static partial Regex HostRegex();

    [GeneratedRegex(@"^#\s*---\s*(.+?)\s*-*\s*$")]
    private static partial Regex GroupRegex();

    [GeneratedRegex(@"^#\s*([A-Za-z0-9*][A-Za-z0-9*._:-]*\.[A-Za-z0-9*._:-]+)\s*(?:#\s*(.*))?$")]
    private static partial Regex DisabledRegex();

    [GeneratedRegex(@"\s*#.*$")]
    private static partial Regex TrailingCommentRegex();
}
