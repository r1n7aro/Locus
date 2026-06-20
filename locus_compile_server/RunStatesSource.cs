using System.Text;
using System.Text.Json.Serialization;

namespace Locus.CompileServer;

/// <summary>
/// JSON model of a unity_run_states request — mirrors the [Serializable]
/// RunStatesRequest in LocusBridge.RunStates.cs (snake_case field names).
/// </summary>
public sealed class RunStatesRequest
{
    [JsonPropertyName("request_editor_status")]
    public string? RequestEditorStatus { get; set; }

    [JsonPropertyName("initial_state")]
    public string? InitialState { get; set; }

    [JsonPropertyName("states")]
    public RunStatesStateRequest[]? States { get; set; }

    [JsonPropertyName("auto_usings")]
    public string[]? AutoUsings { get; set; }
}

public sealed class RunStatesStateRequest
{
    [JsonPropertyName("name")]
    public string? Name { get; set; }

    [JsonPropertyName("variables")]
    public string? Variables { get; set; }

    [JsonPropertyName("start")]
    public string? Start { get; set; }

    [JsonPropertyName("update")]
    public string? Update { get; set; }

    [JsonPropertyName("end")]
    public string? End { get; set; }
}

/// <summary>
/// Verbatim ports of the unity_run_states validation and source generation
/// from LocusBridge.RunStates.cs. Validation messages and generated source
/// must stay identical with the Unity-side implementation while both compile
/// paths coexist — golden tests pin the output.
/// </summary>
public static class RunStatesSource
{
    public const string FullHostTypeName = "Locus.RuntimeStateMachines.__LocusRunStatesHost";
    public const string SourcePath = "LocusRunStates.cs";

    /// <summary>Port of LocusBridge.RunStates.cs IsRunStatesEditorStatus.</summary>
    public static bool IsRunStatesEditorStatus(string? status)
    {
        switch (status)
        {
            case "editing":
            case "playing":
            case "playing_paused":
                return true;
            default:
                return false;
        }
    }

    /// <summary>Port of LocusBridge.RunStates.cs ValidateRunStatesRequest.</summary>
    public static string? ValidateRunStatesRequest(RunStatesRequest? request)
    {
        if (request == null)
            return "run_states request is empty";
        if (string.IsNullOrWhiteSpace(request.RequestEditorStatus))
            return "request_editor_status is required";
        if (!IsRunStatesEditorStatus(request.RequestEditorStatus!.Trim()))
            return "unsupported request_editor_status: " + request.RequestEditorStatus.Trim();
        if (string.IsNullOrWhiteSpace(request.InitialState))
            return "initial_state is required";
        if (request.States == null || request.States.Length == 0)
            return "states must contain at least one state";

        var names = new HashSet<string>(StringComparer.Ordinal);
        for (int i = 0; i < request.States.Length; i++)
        {
            RunStatesStateRequest? state = request.States[i];
            if (state == null)
                return "states[" + i + "] is empty";

            string name = (state.Name ?? "").Trim();
            if (string.IsNullOrEmpty(name))
                return "states[" + i + "].name is required";
            if (!names.Add(name))
                return "duplicate state name: " + name;
            if (string.IsNullOrWhiteSpace(state.Update))
                return "state '" + name + "' requires update code";
        }

        if (!names.Contains(request.InitialState!.Trim()))
            return "initial_state not found in states: " + request.InitialState;

        return null;
    }

    /// <summary>Port of LocusBridge.RunStates.cs BuildRunStatesSource.</summary>
    public static string BuildRunStatesSource(RunStatesRequest request)
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
        AppendRunStatesAutoUsings(sb, request.AutoUsings);
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

        for (int i = 0; i < request.States!.Length; i++)
        {
            RunStatesStateRequest state = request.States[i];
            string name = (state.Name ?? "").Trim();
            sb.AppendLine("            {");
            AppendRunStatesVariables(sb, name, state.Variables, "                ");
            sb.Append("                machine.AddState(").Append(ToCSharpStringLiteral(name)).AppendLine(",");
            AppendRunStatesHandler(sb, name, "start", state.Start, "                    ");
            sb.AppendLine(",");
            AppendRunStatesHandler(sb, name, "update", state.Update, "                    ");
            sb.AppendLine(",");
            AppendRunStatesHandler(sb, name, "end", state.End, "                    ");
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
