namespace AiSandbox;

/// <summary>
/// An expected failure with a message for the user: printed as an error, exit code 1, no stack trace.
/// </summary>
internal sealed class SandboxException(string message) : Exception(message);
