// Renders engine/compose.yml.sbn (a Scriban template) into the one compose file docker compose runs with.
//
// The template holds every service, network and hardening setting literally; only the values that come
// from sandbox.toml or this machine are expressions. Text values go through `q`, which writes a quoted
// YAML string, so a path like C:\Users\x or a value containing ": #" can't break or inject YAML.
using System.Text;
using Scriban;
using Scriban.Runtime;

static class ComposeTemplate
{
    public static string Render(string templatePath, ScriptObject model)
    {
        var name = Path.GetFileName(templatePath);
        var template = Template.Parse(File.ReadAllText(templatePath), templatePath);
        if (template.HasErrors)
            throw new SandboxException(
                $"engine/{name} is not a valid template: {string.Join("; ", template.Messages)}"
            );

        var globals = new ScriptObject();
        globals.Import("q", new Func<object?, string>(Q));
        var context = new TemplateContext { StrictVariables = true, EnableRelaxedMemberAccess = false };
        context.PushGlobal(globals);
        context.PushGlobal(model);
        try
        {
            return template.Render(context).ReplaceLineEndings("\n");
        }
        catch (Scriban.Syntax.ScriptRuntimeException e)
        {
            throw new SandboxException($"engine/{name}: {e.Message}");
        }
    }

    /// A double-quoted YAML scalar: backslash, quote and control characters escaped.
    public static string Q(object? value)
    {
        var s = value switch
        {
            null => "",
            bool b => b ? "true" : "false",
            IFormattable f => f.ToString(null, System.Globalization.CultureInfo.InvariantCulture),
            _ => value.ToString() ?? "",
        };
        var sb = new StringBuilder(s.Length + 2).Append('"');
        foreach (var c in s)
            switch (c)
            {
                case '\\':
                    sb.Append(@"\\");
                    break;
                case '"':
                    sb.Append("\\\"");
                    break;
                case '\n':
                    sb.Append(@"\n");
                    break;
                case '\r':
                    sb.Append(@"\r");
                    break;
                case '\t':
                    sb.Append(@"\t");
                    break;
                case < ' ' or '\u007f' or '\u0085' or '\u2028' or '\u2029':
                    sb.Append($"\\u{(int)c:x4}");
                    break;
                default:
                    sb.Append(c);
                    break;
            }
        return sb.Append('"').ToString();
    }
}
