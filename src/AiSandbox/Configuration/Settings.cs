namespace AiSandbox.Configuration;

/// <summary>
/// The validated settings from <c>sandbox.toml</c>.
/// </summary>
/// <remarks>
/// <see cref="ImmutableArray{T}"/> compares by reference, so two <see cref="Settings"/> with the same content are
/// not equal. Nothing compares them; don't start relying on value equality.
/// </remarks>
internal sealed record Settings(
    string? ProjectName,
    bool ReadOnly,
    bool ProtectGit,
    int? UiPort,
    ImmutableArray<ToolSetting> Install,
    string Title,
    bool Sudo,
    string Memory,
    string? Cpus,
    string? TimeZone,
    ImmutableArray<string> Devices,
    ImmutableArray<string> SecurityOpt,
    BasicAuth? Auth,
    ImmutableArray<EnvVariable> Env,
    ImmutableArray<Share> Shares
);

/// <summary>
/// One <c>[tools]</c> entry as a Dockerfile build arg, e.g. <c>INSTALL_DOTNET = "10.0 8.0"</c>.
/// </summary>
internal sealed record ToolSetting(string Arg, string Value);

/// <summary>
/// One <c>[env]</c> entry.
/// </summary>
internal sealed record EnvVariable(string Key, string Value);

/// <summary>
/// Basic auth for the web desktop (<c>desktop.auth</c>).
/// </summary>
internal sealed record BasicAuth(string User, string Password);

/// <summary>
/// One <c>[[share]]</c> block: an extra folder mounted into the desktop.
/// </summary>
/// <param name="Host">Full path on this machine.</param>
/// <param name="Path">Absolute path in the sandbox.</param>
/// <param name="ReadOnly">Mounted read-only.</param>
internal sealed record Share(string Host, string Path, bool ReadOnly);
