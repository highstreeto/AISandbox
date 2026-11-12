namespace AiSandbox.Network;

/// <summary>
/// One host in <c>allowlist.txt</c>.
/// </summary>
/// <param name="Host">Host name or wildcard pattern.</param>
/// <param name="Group">The <c># --- group ---</c> header it is under ("" above the first one).</param>
/// <param name="Active">Allowed (false: commented out, ready to enable).</param>
/// <param name="Note">The comment after a commented-out entry.</param>
internal sealed record AllowEntry(string Host, string Group, bool Active, string Note);
