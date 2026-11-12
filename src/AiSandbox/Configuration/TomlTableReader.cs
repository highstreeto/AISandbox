using System.Globalization;
using Tomlyn.Model;

namespace AiSandbox.Configuration;

/// <summary>
/// One TOML table: typed getters remember which keys were read; <see cref="Done"/> reports the rest as unknown.
/// </summary>
internal sealed class TomlTableReader
{
    private readonly TomlTable _table;
    private readonly string _path;
    private readonly HashSet<string> _used = [];

    /// <param name="table">The table to read.</param>
    /// <param name="path">Its dotted name in messages ("" for the root).</param>
    public TomlTableReader(TomlTable table, string path)
    {
        _table = table;
        _path = path;
    }

    /// <summary>
    /// All keys, in file order; they all count as read.
    /// </summary>
    public ImmutableArray<string> Keys
    {
        get
        {
            _used.UnionWith(_table.Keys);
            return [.. _table.Keys];
        }
    }

    public object? Raw(string key)
    {
        _used.Add(key);
        return _table.TryGetValue(key, out var v) ? v : null;
    }

    public string? Str(string key) =>
        Raw(key) switch
        {
            null => null,
            string s => s,
            var v => throw Config.Error($"{Name(key)} must be a text in quotes (got {Show(v)})."),
        };

    public bool? Bool(string key) =>
        Raw(key) switch
        {
            null => null,
            bool b => b,
            var v => throw Config.Error($"{Name(key)} must be true or false (got {Show(v)})."),
        };

    public long? Int(string key) =>
        Raw(key) switch
        {
            null => null,
            long n => n,
            var v => throw Config.Error($"{Name(key)} must be a whole number (got {Show(v)})."),
        };

    public double? Number(string key) =>
        Raw(key) switch
        {
            null => null,
            long n => n,
            double d => d,
            var v => throw Config.Error($"{Name(key)} must be a number (got {Show(v)})."),
        };

    /// <summary>
    /// A text, number or true/false, as text (environment values).
    /// </summary>
    public string Scalar(string key) =>
        Raw(key) switch
        {
            string s => s,
            bool b => b ? "true" : "false",
            long n => n.ToString(CultureInfo.InvariantCulture),
            double d => Config.FormatNumber(d),
            var v => throw Config.Error(
                $"{Name(key)} must be a text, number or true/false (got {Show(v)})."
            ),
        };

    public ImmutableArray<string> StrList(string key) =>
        Raw(key) switch
        {
            null => [],
            TomlArray a when a.All(x => x is string) => [.. a.Cast<string>()],
            var v => throw Config.Error(
                $"{Name(key)} must be a list of texts, e.g. [\"a\", \"b\"] (got {Show(v)})."
            ),
        };

    public TomlTableReader? OptTable(string key) =>
        Raw(key) switch
        {
            null => null,
            TomlTable t => new TomlTableReader(t, Name(key)),
            var v => throw Config.Error($"{Name(key)} must be a table (got {Show(v)})."),
        };

    /// <summary>
    /// A table that may be missing (then it is empty).
    /// </summary>
    public TomlTableReader Table(string key) =>
        OptTable(key) ?? new TomlTableReader(new TomlTable(), Name(key));

    public ImmutableArray<TomlTableReader> TableArray(string key) =>
        Raw(key) switch
        {
            null => [],
            TomlTableArray a => [.. a.Select(t => new TomlTableReader(t, Name(key)))],
            var v => throw Config.Error(
                $"{Name(key)} must be written as [[{key}]] blocks (got {Show(v)})."
            ),
        };

    /// <summary>
    /// Unknown keys are errors, with the known ones listed.
    /// </summary>
    /// <param name="what">"key" for a section's keys, "table" for the root's sections.</param>
    public void Done(string what = "key")
    {
        var unknown = _table.Keys.FirstOrDefault(k => !_used.Contains(k));
        if (unknown == null)
            return;
        var known = _used.Count > 0 ? $" (known: {string.Join(", ", _used)})" : "";
        throw Config.Error(
            what == "table"
                ? $"unknown setting '{unknown}'{known}."
                : $"unknown key '{Name(unknown)}'{known}."
        );
    }

    private string Name(string key) => _path.Length == 0 ? key : $"{_path}.{key}";

    private static string Show(object? v) =>
        v switch
        {
            string s => $"\"{s}\"",
            bool b => b ? "true" : "false",
            long n => n.ToString(CultureInfo.InvariantCulture),
            double d => Config.FormatNumber(d),
            TomlArray => "a list",
            TomlTable => "a table",
            TomlTableArray => "[[...]] blocks",
            _ => v?.ToString() ?? "nothing",
        };
}
