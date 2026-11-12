using System.Globalization;
using System.Text;
using Scriban;
using Scriban.Runtime;
using Scriban.Syntax;

namespace AiSandbox.Templating;

/// <summary>
/// Renders <c>compose.yml.sbn</c> (the embedded Scriban template) into the one compose file docker compose runs with.
/// </summary>
/// <remarks>
/// The template holds every service, network and hardening setting literally; only the values that come from
/// <c>sandbox.toml</c> or this machine are expressions. Text values go through <c>q</c>, which writes a quoted YAML
/// string, so a path like <c>C:\Users\x</c> or a value containing <c>": #"</c> can't break or inject YAML.
/// </remarks>
internal static class ComposeTemplate
{
    /// <param name="text">The template text.</param>
    /// <param name="name">The template's name, for error messages.</param>
    /// <param name="model">The template's variables.</param>
    public static string Render(string text, string name, ScriptObject model)
    {
        var template = Template.Parse(text, name);
        if (template.HasErrors)
            throw new SandboxException(
                $"{name} is not a valid template: {string.Join("; ", template.Messages)}"
            );

        var globals = new ScriptObject();
        globals.Import("q", new Func<object?, string>(Quote));
        var context = new TemplateContext
        {
            StrictVariables = true,
            EnableRelaxedMemberAccess = false,
        };
        context.PushGlobal(globals);
        context.PushGlobal(model);
        try
        {
            return template.Render(context).ReplaceLineEndings("\n");
        }
        catch (ScriptRuntimeException e)
        {
            throw new SandboxException($"{name}: {e.Message}");
        }
    }

    /// <summary>
    /// A double-quoted YAML scalar: backslash, quote and control characters escaped.
    /// </summary>
    private static string Quote(object? value)
    {
        var s = value switch
        {
            null => "",
            bool b => b ? "true" : "false",
            IFormattable f => f.ToString(null, CultureInfo.InvariantCulture),
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
                    sb.Append(CultureInfo.InvariantCulture, $"\\u{(int)c:x4}");
                    break;
                default:
                    sb.Append(c);
                    break;
            }
        return sb.Append('"').ToString();
    }
}
