using System.Reflection;
using System.Text;

namespace AiSandbox.Templating;

/// <summary>
/// The files under <c>src/AiSandbox/Resources</c> that are embedded in the assembly: the engine (compose template,
/// <c>desktop/</c> and <c>proxy/</c> build folders) and the <c>init/</c> templates.
/// </summary>
/// <remarks>
/// Names are the path below <c>Resources/</c> with forward slashes on every OS (<c>desktop/tools/node.sh</c>); the
/// csproj sets them. This is the only place that touches the manifest resource streams.
/// </remarks>
internal static class EmbeddedFiles
{
    private static readonly Assembly s_assembly = typeof(EmbeddedFiles).Assembly;

    /// <summary>
    /// All resource names that start with <paramref name="prefix"/>, sorted ordinally.
    /// </summary>
    public static ImmutableArray<string> Names(string prefix = "") =>
        [
            .. s_assembly
                .GetManifestResourceNames()
                .Where(n => n.StartsWith(prefix, StringComparison.Ordinal))
                .Order(StringComparer.Ordinal),
        ];

    public static byte[] ReadBytes(string name)
    {
        using var stream =
            s_assembly.GetManifestResourceStream(name)
            ?? throw new InvalidOperationException($"Embedded file {name} is missing.");
        using var ms = new MemoryStream();
        stream.CopyTo(ms);
        return ms.ToArray();
    }

    /// <summary>
    /// UTF-8 text with line endings normalised to <c>\n</c>.
    /// </summary>
    public static string ReadText(string name) =>
        new UTF8Encoding(false).GetString(ReadBytes(name)).ReplaceLineEndings("\n");
}
