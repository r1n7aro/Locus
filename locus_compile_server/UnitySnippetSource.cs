using System.Text;

namespace Locus.CompileServer;

/// <summary>
/// Verbatim ports of the unity_execute snippet source generators from
/// locus_unity/Editor (SplitLeadingUsings, BuildAsyncSnippetSource). The
/// generated source must stay byte-identical with the Unity-side builders
/// while both compile paths coexist — golden tests pin the output.
///
/// The entry-point contract types (LocusBridge.ScriptGlobals,
/// LocusBridge.ExecuteCodeContext) live in the Unity plugin; this template
/// references them through the metadata references Unity supplies.
/// </summary>
public static class UnitySnippetSource
{
    public const string HostTypeName = "__LocusAsyncSnippetHost";
    public const string FullHostTypeName = "Locus.RuntimeSnippets.__LocusAsyncSnippetHost";
    public const string SourcePath = "LocusRuntimeAsyncSnippet.cs";

    /// <summary>Port of LocusBridge.ExecuteCode.cs SplitLeadingUsings.</summary>
    public static void SplitLeadingUsings(string? code, out string leadingUsings, out string bodyCode)
    {
        if (string.IsNullOrEmpty(code))
        {
            leadingUsings = "";
            bodyCode = "";
            return;
        }

        string normalized = code.Replace("\r\n", "\n");
        string[] lines = normalized.Split('\n');

        var usingSb = new StringBuilder();
        var bodySb = new StringBuilder();

        bool stillInUsingBlock = true;

        for (int i = 0; i < lines.Length; i++)
        {
            string line = lines[i];
            string trimmed = line.Trim();

            if (stillInUsingBlock)
            {
                if (string.IsNullOrEmpty(trimmed))
                {
                    if (usingSb.Length > 0)
                        usingSb.AppendLine(line);
                    else
                        bodySb.AppendLine(line);

                    continue;
                }

                if (trimmed.StartsWith("using ", StringComparison.Ordinal) &&
                    trimmed.EndsWith(";", StringComparison.Ordinal))
                {
                    usingSb.AppendLine(line);
                    continue;
                }

                stillInUsingBlock = false;
            }

            bodySb.AppendLine(line);
        }

        leadingUsings = usingSb.ToString().TrimEnd();
        bodyCode = bodySb.ToString().TrimEnd();
    }

    /// <summary>Port of LocusBridge.ExecuteCodeAsync.cs BuildAsyncSnippetSource.</summary>
    public static string BuildAsyncSnippetSource(
        string hostTypeName,
        string leadingUsings,
        string bodyCode,
        bool expressionMode)
    {
        var sb = new StringBuilder(4096);

        sb.AppendLine("using System;");
        sb.AppendLine("using System.IO;");
        sb.AppendLine("using System.Text;");
        sb.AppendLine("using System.Linq;");
        sb.AppendLine("using System.Reflection;");
        sb.AppendLine("using System.Threading;");
        sb.AppendLine("using System.Threading.Tasks;");
        sb.AppendLine("using System.Collections;");
        sb.AppendLine("using System.Collections.Generic;");
        sb.AppendLine("using UnityEngine;");
        sb.AppendLine("using UnityEngine.SceneManagement;");
        sb.AppendLine("using UnityEngine.UI;");
        sb.AppendLine("using UnityEditor;");
        sb.AppendLine("using UnityEditor.SceneManagement;");
        sb.AppendLine("using UnityEditor.Animations;");
        sb.AppendLine("using static UnityEngine.Object;");
        sb.AppendLine("using Object = UnityEngine.Object;");

        if (!string.IsNullOrWhiteSpace(leadingUsings))
            sb.AppendLine(leadingUsings);

        sb.AppendLine("namespace Locus.RuntimeSnippets");
        sb.AppendLine("{");
        sb.Append("    public static class ").Append(hostTypeName).AppendLine();
        sb.AppendLine("    {");
        sb.AppendLine("        public static async global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
        sb.AppendLine("        {");
        sb.AppendLine("            var print = new global::System.Action<object>(globals.print);");
        sb.AppendLine("            var printJson = new global::System.Action<object>(globals.printJson);");
        sb.AppendLine("            var clear = new global::System.Action(globals.clear);");
        sb.AppendLine("            var ct = cancellationToken;");
        sb.AppendLine("            ctx.ThrowIfCancellationRequested();");
        sb.AppendLine("            #line 1");

        if (expressionMode)
        {
            if (string.IsNullOrWhiteSpace(bodyCode))
            {
                sb.AppendLine("            return null;");
            }
            else
            {
                sb.Append("            return (object)(");
                sb.Append(bodyCode);
                sb.AppendLine(");");
            }
        }
        else
        {
            if (!string.IsNullOrWhiteSpace(bodyCode))
                sb.AppendLine(bodyCode);

            sb.AppendLine("            return null;");
        }

        sb.AppendLine("            #line default");
        sb.AppendLine("        }");
        sb.AppendLine("    }");
        sb.AppendLine("}");

        return sb.ToString();
    }
}
