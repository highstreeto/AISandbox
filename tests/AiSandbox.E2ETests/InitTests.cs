namespace AiSandbox.E2ETests;

/// `sandbox init [folder]`: creates .sandbox/ with the two files a project owns, without Docker.
public sealed class InitTests(SandboxFixture f) : SandboxTest(f)
{
    string NewFolder(string name = "fresh")
    {
        var dir = Path.Combine(F.Root, name);
        if (Directory.Exists(dir))
            Directory.Delete(dir, recursive: true);
        Directory.CreateDirectory(dir);
        return dir;
    }

    static string Template(SandboxFixture f, string name) =>
        File.ReadAllText(Path.Combine(f.InitTemplatesDir, name)).ReplaceLineEndings("\n");

    [Fact]
    public void An_empty_folder_gets_the_files_and_the_engine()
    {
        var dir = NewFolder();
        var r = Run(["init", dir]);
        AssertExit(r, 0);
        AssertCalls(r);
        var sb = Path.Combine(dir, ".sandbox");
        Assert.Equal(Template(F, "sandbox.toml"), File.ReadAllText(Path.Combine(sb, "sandbox.toml")));
        Assert.Equal(Template(F, "allowlist.txt"), File.ReadAllText(Path.Combine(sb, "allowlist.txt")));
        Assert.Equal("*\n", File.ReadAllText(Path.Combine(sb, ".state", ".gitignore")));
        Assert.Matches("^[0-9a-f]{8}\n$", File.ReadAllText(Path.Combine(sb, ".state", "id")));
        Assert.True(File.Exists(Path.Combine(sb, ".state", "desktop", "Dockerfile")));
        Assert.True(File.Exists(Path.Combine(sb, ".state", "proxy", "tinyproxy.conf")));
        Assert.True(File.Exists(Path.Combine(sb, ".state", ".manifest")));
        Assert.False(Directory.Exists(Path.Combine(sb, "engine")));
        Assert.Equal(["allowlist.txt", "sandbox.toml"], Directory.GetFiles(sb).Select(p => Path.GetFileName(p)!).Order().ToArray());
        AssertOutput(r, "Created .sandbox/sandbox.toml");
        AssertOutput(r, "Created .sandbox/allowlist.txt");
        AssertOutput(r, "Next: edit .sandbox/sandbox.toml, then \"sandbox up\".");
        Assert.DoesNotContain("WARNING", r.Output);
    }

    [Fact]
    public void The_new_sandbox_works_with_info()
    {
        var dir = NewFolder();
        AssertExit(Run(["init", dir]), 0);
        var r = Run(["info"], sandboxDir: Path.Combine(dir, ".sandbox"));
        AssertExit(r, 0);
        AssertOutput(r, "Project folder");
        AssertOutput(r, dir);
        Assert.Contains("api.anthropic.com", Run(["allowlist"], sandboxDir: Path.Combine(dir, ".sandbox")).Lines);
    }

    [Fact]
    public void The_new_sandbox_header_names_the_real_commands()
    {
        var dir = NewFolder();
        AssertExit(Run(["init", dir]), 0);
        var toml = File.ReadAllText(Path.Combine(dir, ".sandbox", "sandbox.toml"));
        Assert.Contains("sandbox up", toml);
        Assert.Contains("sandbox rebuild", toml);
        Assert.DoesNotContain(".sandbox/sandbox ", toml);
    }

    [Fact]
    public void The_default_folder_is_the_current_directory()
    {
        var dir = NewFolder();
        AssertExit(Run(["init"], cwd: dir), 0);
        Assert.True(File.Exists(Path.Combine(dir, ".sandbox", "sandbox.toml")));
    }

    [Fact]
    public void SANDBOX_DIR_is_ignored()
    {
        var dir = NewFolder();
        var before = File.ReadAllText(F.ConfFile);
        AssertExit(Run(["init", dir], sandboxDir: F.Dir), 0);
        Assert.True(File.Exists(Path.Combine(dir, ".sandbox", "sandbox.toml")));
        Assert.Equal(before, File.ReadAllText(F.ConfFile));
        Assert.False(Directory.Exists(F.DesktopContext));
    }

    [Fact]
    public void Running_it_again_keeps_the_files()
    {
        var dir = NewFolder();
        AssertExit(Run(["init", dir]), 0);
        var toml = Path.Combine(dir, ".sandbox", "sandbox.toml");
        var allow = Path.Combine(dir, ".sandbox", "allowlist.txt");
        File.WriteAllText(toml, "[project]\nname = \"mine\"\n");
        File.AppendAllText(allow, "example.org\n");
        var allowBefore = File.ReadAllText(allow);
        var id = File.ReadAllText(Path.Combine(dir, ".sandbox", ".state", "id"));

        var r = Run(["init", dir]);
        AssertExit(r, 0);
        AssertOutput(r, "sandbox.toml exists — kept");
        AssertOutput(r, "allowlist.txt exists — kept");
        Assert.DoesNotContain("Created", r.Output);
        Assert.Equal("[project]\nname = \"mine\"\n", File.ReadAllText(toml));
        Assert.Equal(allowBefore, File.ReadAllText(allow));
        Assert.Equal(id, File.ReadAllText(Path.Combine(dir, ".sandbox", ".state", "id")));
    }

    [Fact]
    public void Only_the_missing_file_is_created()
    {
        var dir = NewFolder();
        AssertExit(Run(["init", dir]), 0);
        var toml = Path.Combine(dir, ".sandbox", "sandbox.toml");
        File.WriteAllText(toml, "[desktop]\ntitle = \"x\"\n");
        File.Delete(Path.Combine(dir, ".sandbox", "allowlist.txt"));

        var r = Run(["init", dir]);
        AssertExit(r, 0);
        AssertOutput(r, "Created .sandbox/allowlist.txt");
        AssertOutput(r, "sandbox.toml exists — kept");
        Assert.Equal("[desktop]\ntitle = \"x\"\n", File.ReadAllText(toml));
        Assert.Equal(Template(F, "allowlist.txt"), File.ReadAllText(Path.Combine(dir, ".sandbox", "allowlist.txt")));
    }

    [Fact]
    public void A_sandbox_in_a_parent_folder_is_a_warning()
    {
        var sub = Path.Combine(F.ProjectDir, "sub");
        Directory.CreateDirectory(sub);
        var r = Run(["init", sub]);
        AssertExit(r, 0);
        AssertOutput(r, $"{F.Dir} already exists; this creates a separate sandbox for {sub}");
        Assert.True(File.Exists(Path.Combine(sub, ".sandbox", "sandbox.toml")));
    }

    [Fact]
    public void A_folder_that_does_not_exist_is_an_error()
    {
        var dir = Path.Combine(F.Root, "nope");
        var r = Run(["init", dir]);
        AssertExit(r, 1);
        AssertOutput(r, $"Folder {dir} does not exist.");
        Assert.False(Directory.Exists(dir));
    }

    [Fact]
    public void It_works_without_docker()
    {
        var dir = NewFolder();
        var r = Run(["init", dir], path: F.EmptyBin);
        AssertExit(r, 0);
        Assert.True(File.Exists(Path.Combine(dir, ".sandbox", "sandbox.toml")));
    }

    [Fact]
    public void An_old_engine_folder_gets_the_usual_warning()
    {
        var dir = NewFolder();
        Directory.CreateDirectory(Path.Combine(dir, ".sandbox", "engine"));
        var r = Run(["init", dir]);
        AssertExit(r, 0);
        AssertOutput(r, ".sandbox/engine/ is no longer used");
        Assert.True(Directory.Exists(Path.Combine(dir, ".sandbox", "engine")));
    }

    [Fact]
    public void Init_is_listed_in_the_help()
    {
        var r = Run("--help");
        AssertExit(r, 0);
        AssertOutput(r, "init");
        r = Run("init", "--help");
        AssertExit(r, 0);
        AssertOutput(r, "Project folder (default: the current directory)");
    }
}
