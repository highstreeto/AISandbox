namespace AiSandbox.Project;

/// <summary>
/// A bind mount of the desktop container.
/// </summary>
/// <param name="Source">On this machine.</param>
/// <param name="Target">In the container.</param>
/// <param name="ReadOnly">Mounted read-only.</param>
internal sealed record Mount(string Source, string Target, bool ReadOnly);
