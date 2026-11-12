using System.Text.Json;

namespace AiSandbox.Project;

/// <summary>
/// Reads the output of <c>docker compose ps --format json</c>.
/// </summary>
internal static class ComposePs
{
    /// <summary>
    /// Newer compose prints one JSON object per line, older versions a single array.
    /// </summary>
    public static ImmutableArray<ServiceState> Parse(string json)
    {
        var text = json.Trim();
        string[] docs = text.StartsWith('[')
            ? [text]
            : text.Split(
                '\n',
                StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries
            );
        var result = ImmutableArray.CreateBuilder<ServiceState>();
        foreach (var d in docs)
        {
            using var doc = JsonDocument.Parse(d);
            if (doc.RootElement.ValueKind == JsonValueKind.Array)
                foreach (var e in doc.RootElement.EnumerateArray())
                    result.Add(ToState(e));
            else
                result.Add(ToState(doc.RootElement));
        }
        return result.DrainToImmutable();
    }

    private static ServiceState ToState(JsonElement e) =>
        new(Str(e, "Service"), Str(e, "State"), Str(e, "Health"), Str(e, "Status"), FormatPorts(e));

    private static string Str(JsonElement e, string key) =>
        e.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String
            ? v.GetString() ?? ""
            : "";

    /// <summary>
    /// The published ports of one entry, e.g. "127.0.0.1:49153→3001".
    /// </summary>
    private static string FormatPorts(JsonElement e)
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
}
