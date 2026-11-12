namespace AiSandbox.Project;

/// <summary>
/// One container as <c>docker compose ps</c> reports it.
/// </summary>
internal sealed record ServiceState(
    string Service,
    string State,
    string Health,
    string Status,
    string Ports
);
