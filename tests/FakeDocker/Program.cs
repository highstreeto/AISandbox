// Fake docker CLI for the sandbox tests.
//
//   FAKE_DOCKER_LOG    file; every call appends one JSON line {"name": "docker", "args": [...]}
//   FAKE_DOCKER_RULES  file with a JSON array of {"when", "stdout", "stderr", "exit"}; the first rule whose
//                      "when" is a substring of the space-joined arguments decides the output and exit code.
//                      No match: no output, exit 0.
//
// The same binary is also installed as "xdg-open"; "name" tells the two apart.
using System.Text.Json;

var name = Path.GetFileNameWithoutExtension(Environment.ProcessPath ?? "docker");
var joined = string.Join(' ', args);

if (Environment.GetEnvironmentVariable("FAKE_DOCKER_LOG") is { Length: > 0 } log)
{
    var line = JsonSerializer.Serialize(new Call(name, args)) + "\n";
    // Calls can overlap (the app polls `compose ps` while `up` runs): retry on a sharing violation.
    for (var i = 0; ; i++)
        try
        {
            File.AppendAllText(log, line);
            break;
        }
        catch (IOException) when (i < 50)
        {
            Thread.Sleep(20);
        }
}

if (Environment.GetEnvironmentVariable("FAKE_DOCKER_RULES") is { Length: > 0 } rulesFile && File.Exists(rulesFile))
{
    var rules = JsonSerializer.Deserialize<List<Rule>>(File.ReadAllText(rulesFile)) ?? [];
    foreach (var r in rules)
    {
        if (!joined.Contains(r.When ?? "", StringComparison.Ordinal))
            continue;
        Console.Out.Write(r.Stdout ?? "");
        Console.Error.Write(r.Stderr ?? "");
        return r.Exit;
    }
}
return 0;

sealed record Call(
    [property: System.Text.Json.Serialization.JsonPropertyName("name")] string Name,
    [property: System.Text.Json.Serialization.JsonPropertyName("args")] string[] Args
);

sealed record Rule(
    [property: System.Text.Json.Serialization.JsonPropertyName("when")] string? When,
    [property: System.Text.Json.Serialization.JsonPropertyName("stdout")] string? Stdout,
    [property: System.Text.Json.Serialization.JsonPropertyName("stderr")] string? Stderr,
    [property: System.Text.Json.Serialization.JsonPropertyName("exit")] int Exit
);
