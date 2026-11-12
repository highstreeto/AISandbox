using System.Diagnostics;

namespace AiSandbox.E2ETests;

/// One throw-away project, reset before each test:
///
///   <tmp>/demo-project/.git/{config,hooks/}
///   <tmp>/demo-project/.sandbox/            sandbox.toml, allowlist.txt, .state/id written by Reset();
///                                           the engine is written to .state/desktop and .state/proxy by the app
///   <tmp>/fakebin/docker, xdg-open          the fake docker (see tests/FakeDocker)
///   <tmp>/empty/                            a PATH with nothing on it
public sealed class SandboxFixture : IDisposable
{
    public const string ProjectName = "demo-project";
    public const string Id = "0123abcd";

    public string Root { get; }
    public string ProjectDir { get; }
    public string Dir { get; }
    /// The app under test, built from src/AiSandbox along with the tests (ProjectReference).
    public string AppDll { get; }
    /// The app's version as it writes it into headers and the manifest (AssemblyInformationalVersion).
    public string AppVersion { get; }
    public string FakeBin { get; }
    public string EmptyBin { get; }
    public string Dotnet { get; }

    public string StateDir => Path.Combine(Dir, ".state");
    public string IdFile => Path.Combine(StateDir, "id");
    public string DesktopContext => Path.Combine(StateDir, "desktop");
    public string ProxyContext => Path.Combine(StateDir, "proxy");
    public string GeneratedFile => Path.Combine(StateDir, "compose.yml");
    public string ConfFile => Path.Combine(Dir, "sandbox.toml");
    /// The engine folder of the old layout; tests that need it create it, Reset() removes it.
    public string OldEngineDir => Path.Combine(Dir, "engine");
    public string OldConfFile => Path.Combine(Dir, "sandbox.yml");
    public string OldLocalConfFile => Path.Combine(Dir, "sandbox.local.yml");
    /// The templates `sandbox init` writes (the shipped sandbox.toml and allowlist.txt), in the app's sources.
    public string InitTemplatesDir => Path.Combine(RepoRoot, "src", "AiSandbox", "Resources", "init");
    public string AllowlistFile => Path.Combine(Dir, "allowlist.txt");
    public string WorkDir => $"/work/{ProjectName}-{Id}";

    public SandboxFixture()
    {
        Root = Path.GetFullPath(
            Path.Combine(Path.GetTempPath(), "sandbox-tests-" + Guid.NewGuid().ToString("N")[..8])
        );
        ProjectDir = Path.Combine(Root, ProjectName);
        Dir = Path.Combine(ProjectDir, ".sandbox");
        FakeBin = Path.Combine(Root, "fakebin");
        EmptyBin = Path.Combine(Root, "empty");
        Dotnet = Environment.GetEnvironmentVariable("DOTNET_HOST_PATH") is { Length: > 0 } h
            ? h
            : "dotnet";

        Directory.CreateDirectory(Dir);
        Directory.CreateDirectory(EmptyBin);
        AppDll = BuiltOutput("src/AiSandbox", "sandbox.dll");
        AppVersion = FileVersionInfo.GetVersionInfo(AppDll).ProductVersion!;
        InstallFakeDocker();

        Reset();
    }

    /// A fixed sandbox.toml (not the repo's, which people edit) in the shipped format.
    public const string DefaultConf = """
        [project]
        # name = "myproject"
        workspace = "rw"
        protect_git = true
        # ui_port = 3001

        [tools]
        claude = true
        claude_code = false
        dotnet = "10.0"
        node = false

        [desktop]
        title = "AI Sandbox"

        """;

    /// A fixed allowlist.txt: active entries, a group with commented-out entries, header comments.
    public const string DefaultAllowlist = """
        # =====================================================
        #  Sandbox network allowlist — one host per line.
        # =====================================================
        #    example.com        exactly example.com

        # --- Claude (required) -------------------------------
        api.anthropic.com
        claude.ai
        *.claudeusercontent.com

        # --- .NET / NuGet ------------------------------------
        # api.nuget.org
        # globalcdn.nuget.org

        """;

    /// Back to the default state: fixed sandbox.toml + allowlist.txt, no old config files, a fixed id, .git present.
    public void Reset()
    {
        File.WriteAllText(ConfFile, DefaultConf.ReplaceLineEndings("\n"));
        File.WriteAllText(AllowlistFile, DefaultAllowlist.ReplaceLineEndings("\n"));
        if (Directory.Exists(OldEngineDir))
            Directory.Delete(OldEngineDir, recursive: true);
        File.Delete(OldConfFile);
        File.Delete(OldLocalConfFile);
        if (Directory.Exists(StateDir))
            Directory.Delete(StateDir, recursive: true);
        Directory.CreateDirectory(StateDir);
        File.WriteAllText(IdFile, Id + "\n");

        var git = Path.Combine(ProjectDir, ".git");
        Directory.CreateDirectory(Path.Combine(git, "hooks"));
        File.WriteAllText(Path.Combine(git, "config"), "[core]\n\tbare = false\n");
    }

    /// A file in the build output of another project of the repo, built with this one (ProjectReference),
    /// in the same configuration and target framework as the tests.
    static string BuiltOutput(string project, string file)
    {
        var testBin = new DirectoryInfo(AppContext.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar));
        var tfm = testBin.Name; // net10.0
        var config = testBin.Parent!.Name; // Debug / Release
        var path = Path.Combine(RepoRoot, project, "bin", config, tfm, file);
        return File.Exists(path)
            ? path
            : throw new InvalidOperationException($"{project} not built: {path} is missing.");
    }

    /// The fake docker, copied as `docker` and `xdg-open`.
    void InstallFakeDocker()
    {
        var src = Path.GetDirectoryName(BuiltOutput("tests/FakeDocker", "docker.dll"))!;

        Directory.CreateDirectory(FakeBin);
        var exe = OperatingSystem.IsWindows() ? ".exe" : "";
        foreach (var name in new[] { "docker", "xdg-open" })
        {
            File.Copy(Path.Combine(src, "docker" + exe), Path.Combine(FakeBin, name + exe));
            File.Copy(Path.Combine(src, "docker.dll"), Path.Combine(FakeBin, name + ".dll"));
            File.Copy(
                Path.Combine(src, "docker.runtimeconfig.json"),
                Path.Combine(FakeBin, name + ".runtimeconfig.json")
            );
        }
    }

    static string RepoRoot { get; } = FindRepoRoot();

    /// The folder with AiSandbox.slnx.
    static string FindRepoRoot()
    {
        for (var d = new DirectoryInfo(AppContext.BaseDirectory); d != null; d = d.Parent)
            if (File.Exists(Path.Combine(d.FullName, "AiSandbox.slnx")))
                return d.FullName;
        throw new InvalidOperationException("Could not find the repo root (with AiSandbox.slnx).");
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(Root, recursive: true);
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }
}

[CollectionDefinition(Name)]
public sealed class SandboxCollection : ICollectionFixture<SandboxFixture>
{
    public const string Name = "sandbox";
}
