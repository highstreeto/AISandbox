using System.Runtime.InteropServices;

namespace AiSandbox.Processes;

/// <summary>
/// The host user's ids, for PUID/PGID in the desktop container.
/// </summary>
internal static partial class NativeUser
{
    /// <summary>
    /// The user and group id (1000 on Windows, and never root).
    /// </summary>
    public static (uint Uid, uint Gid) UserIds()
    {
        if (OperatingSystem.IsWindows())
            return (1000, 1000);
        try
        {
            var (u, g) = (GetUid(), GetGid());
            return u == 0 ? (1000, 1000) : (u, g);
        }
        catch (Exception e) when (e is DllNotFoundException or EntryPointNotFoundException)
        {
            return (1000, 1000);
        }
    }

    [LibraryImport("libc", EntryPoint = "getuid")]
    private static partial uint GetUid();

    [LibraryImport("libc", EntryPoint = "getgid")]
    private static partial uint GetGid();
}
