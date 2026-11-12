using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using AiSandbox.Configuration;
using AiSandbox.Templating;

namespace AiSandbox.Project;

/// <summary>
/// The project this sandbox belongs to: where its <c>.sandbox</c> folder is, its identity and its settings.
/// </summary>
/// <remarks>
/// Split by area: <c>Sandbox.Locate.cs</c> (finding the folder), <c>Sandbox.Docker.cs</c> (docker and docker
/// compose calls), <c>Sandbox.Generate.cs</c> (the engine files and the generated compose file).
/// </remarks>
/// <param name="Dir">The <c>.sandbox</c> folder.</param>
/// <param name="ProjectDir">The project folder, the parent of <paramref name="Dir"/>.</param>
/// <param name="Id">Created once and stored in <c>.state/id</c>: stable even if the project is moved.</param>
/// <param name="Name">The docker compose project name.</param>
/// <param name="WorkDir">Where the project is mounted in the desktop.</param>
/// <param name="Settings">From <c>sandbox.toml</c>.</param>
/// <param name="Verbose">Print the docker commands that are run.</param>
internal sealed partial record Sandbox(
    string Dir,
    string ProjectDir,
    string Id,
    string Name,
    string WorkDir,
    Settings Settings,
    bool Verbose
)
{
    public string AllowlistPath => Path.Combine(Dir, "allowlist.txt");
    public string StateDir => StateDirOf(Dir);
    public string DesktopContext => Path.Combine(StateDir, "desktop");
    public string ProxyContext => Path.Combine(StateDir, "proxy");
    public string GenPath => Path.Combine(StateDir, "compose.yml");

    public static Sandbox Load(bool verbose = false)
    {
        var dir = LocateSandboxDir();
        var projectDir = Path.GetDirectoryName(dir)!;
        var id = LoadOrCreateId(StateDirOf(dir), projectDir);

        CheckOldConfig(dir);
        CheckOldLayout(dir);
        var settings = Config.Read(
            Path.Combine(dir, Config.FileName),
            projectDir,
            EmbeddedFiles.ReadText("desktop/Dockerfile")
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

    /// <summary>
    /// A docker compose project name: lower case letters, digits, <c>_</c> and <c>-</c>.
    /// </summary>
    public static string Sanitize(string s)
    {
        s = InvalidNameCharsRegex().Replace(s.ToLowerInvariant(), "-");
        s = TrailingDashesRegex().Replace(LeadingSeparatorsRegex().Replace(s, ""), "");
        return s.Length > 0 ? s : "sandbox";
    }

    private static string StateDirOf(string dir) => Path.Combine(dir, ".state");

    /// <summary>
    /// The id stored in <c>.state/id</c>, created from the project folder if it isn't there yet.
    /// </summary>
    public static string LoadOrCreateId(string stateDir, string projectDir)
    {
        Directory.CreateDirectory(stateDir);
        var idFile = Path.Combine(stateDir, "id");
        var id = File.Exists(idFile) ? File.ReadAllText(idFile).Trim() : "";
        if (id.Length == 0)
        {
            id = Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(projectDir)))[..8];
            File.WriteAllText(idFile, id + "\n");
        }
        return id;
    }

    [GeneratedRegex("[^a-z0-9_-]+")]
    private static partial Regex InvalidNameCharsRegex();

    [GeneratedRegex("^[-_]+")]
    private static partial Regex LeadingSeparatorsRegex();

    [GeneratedRegex("-+$")]
    private static partial Regex TrailingDashesRegex();
}
