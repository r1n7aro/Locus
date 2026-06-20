using System.Text;

namespace Locus.CompileServer.Tests;

/// <summary>
/// VERBATIM copies of the Unity-side source generators, used as the golden
/// reference the server ports are compared against character-by-character.
///
///   SplitLeadingUsings      ← locus_unity/Editor/LocusBridge.ExecuteCode.cs
///   BuildAsyncSnippetSource ← locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs
///   BuildRunStatesSource    ← locus_unity/Editor/LocusBridge.RunStates.cs (with helpers)
///
/// When the Unity-side builders change, update these copies in the same
/// commit — the parity tests exist to catch the two compile paths drifting.
/// </summary>
internal static class UnityReferenceImpl
{
    // ── LocusBridge.ExecuteCode.cs ──────────────────────────────────

    public static void SplitLeadingUsings(string code, out string leadingUsings, out string bodyCode)
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

    // ── ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs ────────────

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

    // ── LocusBridge.RunStates.cs ────────────────────────────────────

    public sealed class RunStatesRequestRef
    {
        public string? request_editor_status;
        public string? initial_state;
        public RunStatesStateRequestRef[]? states;
        public string[]? auto_usings;
    }

    public sealed class RunStatesStateRequestRef
    {
        public string? name;
        public string? variables;
        public string? start;
        public string? update;
        public string? end;
    }

    public static string BuildRunStatesSource(RunStatesRequestRef request)
    {
        var sb = new StringBuilder(8192);
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
        sb.AppendLine("using Unity.Profiling;");
        sb.AppendLine("using UnityEditor;");
        sb.AppendLine("using UnityEditor.Profiling;");
        sb.AppendLine("using UnityEditorInternal;");
        sb.AppendLine("using UnityEditor.SceneManagement;");
        sb.AppendLine("using UnityEditor.Animations;");
        AppendRunStatesAutoUsings(sb, request.auto_usings);
        sb.AppendLine("using static UnityEngine.Object;");
        sb.AppendLine("using Object = UnityEngine.Object;");
        sb.AppendLine();
        sb.AppendLine("namespace Locus.RuntimeStateMachines");
        sb.AppendLine("{");
        sb.AppendLine("    public static class __LocusRunStatesHost");
        sb.AppendLine("    {");
        sb.AppendLine("        public static global::Locus.LocusBridge.RuntimeStateMachineDefinition Build()");
        sb.AppendLine("        {");
        sb.AppendLine("            var machine = new global::Locus.LocusBridge.RuntimeStateMachineDefinition();");

        for (int i = 0; i < request.states!.Length; i++)
        {
            RunStatesStateRequestRef state = request.states[i];
            string name = (state.name ?? "").Trim();
            sb.AppendLine("            {");
            AppendRunStatesVariables(sb, name, state.variables, "                ");
            sb.Append("                machine.AddState(").Append(ToCSharpStringLiteral(name)).AppendLine(",");
            AppendRunStatesHandler(sb, name, "start", state.start, "                    ");
            sb.AppendLine(",");
            AppendRunStatesHandler(sb, name, "update", state.update, "                    ");
            sb.AppendLine(",");
            AppendRunStatesHandler(sb, name, "end", state.end, "                    ");
            sb.AppendLine("                );");
            sb.AppendLine("            }");
        }

        sb.AppendLine("            return machine;");
        sb.AppendLine("        }");
        sb.AppendLine("    }");
        sb.AppendLine("}");
        return sb.ToString();
    }

    private static void AppendRunStatesAutoUsings(StringBuilder sb, string[]? namespaces)
    {
        if (namespaces == null || namespaces.Length == 0)
            return;

        var seen = new HashSet<string>(StringComparer.Ordinal);
        for (int i = 0; i < namespaces.Length; i++)
        {
            string ns = (namespaces[i] ?? "").Trim();
            if (string.IsNullOrEmpty(ns) || !seen.Add(ns) || !IsValidUsingNamespace(ns))
                continue;

            sb.Append("using ").Append(ns).AppendLine(";");
        }
    }

    private static bool IsValidUsingNamespace(string ns)
    {
        if (string.IsNullOrEmpty(ns))
            return false;

        for (int i = 0; i < ns.Length; i++)
        {
            char ch = ns[i];
            bool ok = ch == '_' || ch == '.' || char.IsLetterOrDigit(ch);
            if (!ok)
                return false;
        }

        return true;
    }

    private static void AppendRunStatesVariables(StringBuilder sb, string stateName, string? code, string indent)
    {
        if (string.IsNullOrWhiteSpace(code))
            return;

        sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":variables"));
        sb.AppendLine(code);
        sb.Append(indent).AppendLine("    #line default");
    }

    private static void AppendRunStatesHandler(StringBuilder sb, string stateName, string phase, string? code, string indent)
    {
        if (string.IsNullOrWhiteSpace(code))
        {
            sb.Append(indent).Append("null");
            return;
        }

        sb.Append(indent).AppendLine("new global::System.Action<global::Locus.LocusBridge.RuntimeCtx>(ctx =>");
        sb.Append(indent).AppendLine("{");
        sb.Append(indent).AppendLine("    var print = new global::System.Action<object>(ctx.Print);");
        sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":" + phase));
        sb.AppendLine(code);
        sb.Append(indent).AppendLine("    #line default");
        sb.Append(indent).Append("})");
    }

    private static string ToCSharpStringLiteral(string? value)
    {
        if (value == null)
            return "null";

        var sb = new StringBuilder(value.Length + 2);
        sb.Append('"');
        for (int i = 0; i < value.Length; i++)
        {
            char ch = value[i];
            switch (ch)
            {
                case '\\': sb.Append("\\\\"); break;
                case '"': sb.Append("\\\""); break;
                case '\r': sb.Append("\\r"); break;
                case '\n': sb.Append("\\n"); break;
                case '\t': sb.Append("\\t"); break;
                default: sb.Append(ch); break;
            }
        }
        sb.Append('"');
        return sb.ToString();
    }
}
