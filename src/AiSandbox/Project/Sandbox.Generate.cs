using System.Globalization;
using System.Text;
using AiSandbox.Processes;
using AiSandbox.Templating;
using Scriban.Runtime;

namespace AiSandbox.Project;

internal sealed partial record Sandbox
{
    /// <summary>
    /// Writes <c>.state/compose.yml</c>, the only file docker compose runs with: <c>engine/compose.yml.sbn</c>
    /// rendered with the settings from <c>sandbox.toml</c> and this machine's facts (user ids, time zone, mounts,
    /// UI port).
    /// </summary>
    /// <remarks>
    /// The UI port, the project mounts and the proxy settings are written by the template itself, so
    /// <c>sandbox.toml</c> has no way to change them.
    /// </remarks>
    public void Generate()
    {
        var text = ComposeTemplate.Render(TemplatePath, Model());
        File.WriteAllText(GenPath, text, new UTF8Encoding(false));
        File.Delete(Path.Combine(StateDir, "compose.generated.yml")); // left over from the multi-file setup
    }

    /// <summary>
    /// The template's variables: host, tools, desktop, env, mounts.
    /// </summary>
    private ScriptObject Model()
    {
        var s = Settings;
        var (uid, gid) = NativeUser.UserIds();

        var auth = s.Auth is { } a ? Obj(("user", a.User), ("password", a.Password)) : null;
        return Obj(
            (
                "host",
                Obj(
                    ("id", Id),
                    ("workdir", WorkDir),
                    ("uid", uid.ToString(CultureInfo.InvariantCulture)),
                    ("gid", gid.ToString(CultureInfo.InvariantCulture)),
                    ("ui_port", s.UiPort?.ToString(CultureInfo.InvariantCulture) ?? "")
                )
            ),
            ("tools", Arr(s.Install.Select(t => Obj(("arg", t.Arg), ("value", t.Value))))),
            (
                "desktop",
                Obj(
                    ("title", s.Title),
                    ("sudo", s.Sudo ? "true" : "false"),
                    ("auth", auth),
                    ("timezone", s.TimeZone ?? HostTimeZone()),
                    ("memory", s.Memory),
                    ("cpus", s.Cpus),
                    ("devices", Arr(s.Devices)),
                    ("security_opt", Arr(s.SecurityOpt))
                )
            ),
            ("env", Arr(s.Env.Select(e => Obj(("key", e.Key), ("value", e.Value))))),
            (
                "mounts",
                Arr(
                    Mounts()
                        .Select(m =>
                            Obj(("source", m.Source), ("target", m.Target), ("read_only", m.ReadOnly))
                        )
                )
            )
        );
    }

    /// <summary>
    /// Bind mounts of the desktop: the project, the read-only .sandbox, the protected .git paths, the shares.
    /// </summary>
    private ImmutableArray<Mount> Mounts()
    {
        var mounts = ImmutableArray.CreateBuilder<Mount>();
        mounts.Add(new Mount(ProjectDir, WorkDir, Settings.ReadOnly));
        // The sandbox config is visible but read-only: the agent can't widen its allowlist.
        mounts.Add(new Mount(Dir, $"{WorkDir}/{Path.GetFileName(Dir)}", ReadOnly: true));
        if (Settings.ProtectGit && !Settings.ReadOnly)
            foreach (var p in (ReadOnlySpan<string>)["config", "hooks"])
            {
                var src = Path.Combine(ProjectDir, ".git", p);
                if (File.Exists(src) || Directory.Exists(src))
                    mounts.Add(new Mount(src, $"{WorkDir}/.git/{p}", ReadOnly: true));
            }
        foreach (var sh in Settings.Shares)
            mounts.Add(new Mount(sh.Host, sh.Path, sh.ReadOnly));
        return mounts.DrainToImmutable();
    }

    private static ScriptObject Obj(params ReadOnlySpan<(string Key, object? Value)> members)
    {
        var o = new ScriptObject();
        foreach (var (k, v) in members)
            o[k] = v;
        return o;
    }

    private static ScriptArray Arr(IEnumerable<object> items) => new(items);

    /// <summary>
    /// <c>$TZ</c>, else this machine's IANA time zone, else null (the template's default).
    /// </summary>
    private static string? HostTimeZone()
    {
        var tz = Environment.GetEnvironmentVariable("TZ");
        if (!string.IsNullOrWhiteSpace(tz))
            return tz;
        var local = TimeZoneInfo.Local;
        if (local.HasIanaId)
            return local.Id is "Local" or "" ? null : local.Id;
        return TimeZoneInfo.TryConvertWindowsIdToIanaId(local.Id, out var iana) ? iana : null;
    }
}
