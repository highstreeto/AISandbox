using System.Diagnostics;

namespace Sandbox.Tests;

/// One throw-away project with a copy of the repo's .sandbox folder, built once and reset before each test:
///
///   <tmp>/demo-project/.git/{config,hooks/}
///   <tmp>/demo-project/.sandbox/...        copy of the repo's .sandbox (without .state)
///   <tmp>/fakebin/docker, xdg-open          the fake docker (see tests/FakeDocker)
///   <tmp>/empty/                            a PATH with nothing on it
public sealed class SandboxFixture : IDisposable
{
    public const string ProjectName = "demo-project";
    public const string Id = "0123abcd";

    public string RepoSandboxDir { get; }
    public string Root { get; }
    public string ProjectDir { get; }
    public string Dir { get; }
    public string EntryFile => Path.Combine(Dir, "sandbox.cs");
    public string FakeBin { get; }
    public string EmptyBin { get; }
    public string Dotnet { get; }

    public string StateDir => Path.Combine(Dir, ".state");
    public string IdFile => Path.Combine(StateDir, "id");
    public string GeneratedFile => Path.Combine(StateDir, "compose.yml");
    public string ConfFile => Path.Combine(Dir, "sandbox.toml");
    public string OldConfFile => Path.Combine(Dir, "sandbox.yml");
    public string OldLocalConfFile => Path.Combine(Dir, "sandbox.local.yml");
    public string AllowlistFile => Path.Combine(Dir, "allowlist.txt");
    public string WorkDir => $"/work/{ProjectName}-{Id}";

    public SandboxFixture()
    {
        RepoSandboxDir = FindRepoSandboxDir();
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
        foreach (var f in Directory.GetFiles(RepoSandboxDir, "*.cs"))
            File.Copy(f, Path.Combine(Dir, Path.GetFileName(f)));
        CopyDir(Path.Combine(RepoSandboxDir, "engine"), Path.Combine(Dir, "engine"));
        InstallFakeDocker();

        Reset();
        Build();
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

    void Build()
    {
        var psi = new ProcessStartInfo(Dotnet)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            WorkingDirectory = Root,
        };
        foreach (var a in new[] { "build", EntryFile, "--nologo", "-v", "q" })
            psi.ArgumentList.Add(a);
        using var p = Process.Start(psi)!;
        var stdout = p.StandardOutput.ReadToEndAsync();
        var stderr = p.StandardError.ReadToEndAsync();
        p.WaitForExit();
        if (p.ExitCode != 0)
            throw new InvalidOperationException(
                $"Building {EntryFile} failed:\n{stdout.Result}\n{stderr.Result}"
            );
    }

    /// The fake docker is built with this project (ProjectReference); copy it as `docker` and `xdg-open`.
    void InstallFakeDocker()
    {
        var testBin = new DirectoryInfo(AppContext.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar));
        var tfm = testBin.Name; // net10.0
        var config = testBin.Parent!.Name; // Debug / Release
        var src = Path.Combine(RepoRoot, "tests", "FakeDocker", "bin", config, tfm);
        if (!File.Exists(Path.Combine(src, "docker.dll")))
            throw new InvalidOperationException($"Fake docker not built: {src}");

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

    string RepoRoot => Path.GetDirectoryName(RepoSandboxDir)!;

    /// The repo's .sandbox folder, or the one in $SANDBOX_TESTS_DIR (e.g. a staged copy).
    static string FindRepoSandboxDir()
    {
        if (Environment.GetEnvironmentVariable("SANDBOX_TESTS_DIR") is { Length: > 0 } dir)
            return Path.GetFullPath(dir);
        for (var d = new DirectoryInfo(AppContext.BaseDirectory); d != null; d = d.Parent)
        {
            var s = Path.Combine(d.FullName, ".sandbox");
            if (File.Exists(Path.Combine(s, "sandbox.cs")))
                return s;
        }
        throw new InvalidOperationException("Could not find the repo's .sandbox folder.");
    }

    static void CopyDir(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (var f in Directory.GetFiles(from))
            File.Copy(f, Path.Combine(to, Path.GetFileName(f)));
        foreach (var d in Directory.GetDirectories(from))
            CopyDir(d, Path.Combine(to, Path.GetFileName(d)));
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
