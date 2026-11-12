using System.Collections.Immutable;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace AiSandbox.Project;

/// <summary>
/// <c>.state/.manifest</c>: what the engine sync wrote last time.
/// </summary>
/// <param name="App">The app's version (for people: bug reports, <c>info</c>).</param>
/// <param name="Engine">One SHA-256 over the embedded <c>desktop/</c> and <c>proxy/</c> files.</param>
/// <param name="Files">SHA-256 of every written file as it is on disk, keyed by path below <c>.state/</c>.</param>
internal sealed record EngineManifest(
    [property: JsonPropertyName("app")] string App,
    [property: JsonPropertyName("engine")] string Engine,
    [property: JsonPropertyName("files")] ImmutableSortedDictionary<string, string> Files
)
{
    private static readonly JsonSerializerOptions s_options = new() { WriteIndented = true };

    public static EngineManifest Empty { get; } =
        new("", "", ImmutableSortedDictionary<string, string>.Empty.WithComparers(StringComparer.Ordinal));

    /// <summary>
    /// The manifest at <paramref name="path"/>; an empty one if it is missing or unreadable (the next sync repairs it).
    /// </summary>
    public static EngineManifest Read(string path)
    {
        try
        {
            if (!File.Exists(path))
                return Empty;
            var json = JsonSerializer.Deserialize<Json>(File.ReadAllText(path));
            return json is null
                ? Empty
                : new EngineManifest(
                    json.App ?? "",
                    json.Engine ?? "",
                    (json.Files ?? []).ToImmutableSortedDictionary(StringComparer.Ordinal)
                );
        }
        catch (Exception e) when (e is JsonException or IOException or UnauthorizedAccessException)
        {
            return Empty;
        }
    }

    /// <summary>
    /// The manifest as written to disk: indented JSON with sorted keys and a final newline.
    /// </summary>
    public string ToJson() =>
        JsonSerializer.Serialize(new Json(App, Engine, Files.ToDictionary()), s_options) + "\n";

    private sealed record Json(
        [property: JsonPropertyName("app")] string? App,
        [property: JsonPropertyName("engine")] string? Engine,
        [property: JsonPropertyName("files")] Dictionary<string, string>? Files
    );
}
