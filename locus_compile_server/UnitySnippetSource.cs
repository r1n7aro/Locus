using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

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

    /// <summary>
    /// Keep common IO names convenient without importing the whole System.IO
    /// namespace. Unity's Mono mscorlib contains the internal extension method
    /// System.IO.MonoLinqHelper.ToArray; importing System.IO while snippet
    /// accessibility checks are relaxed makes it ambiguous with LINQ ToArray.
    /// </summary>
    internal static void AppendCommonIoAliases(StringBuilder sb)
    {
        sb.AppendLine("using BinaryReader = global::System.IO.BinaryReader;");
        sb.AppendLine("using BinaryWriter = global::System.IO.BinaryWriter;");
        sb.AppendLine("using BufferedStream = global::System.IO.BufferedStream;");
        sb.AppendLine("using Directory = global::System.IO.Directory;");
        sb.AppendLine("using DirectoryInfo = global::System.IO.DirectoryInfo;");
        sb.AppendLine("using DirectoryNotFoundException = global::System.IO.DirectoryNotFoundException;");
        sb.AppendLine("using EndOfStreamException = global::System.IO.EndOfStreamException;");
        sb.AppendLine("using File = global::System.IO.File;");
        sb.AppendLine("using FileAccess = global::System.IO.FileAccess;");
        sb.AppendLine("using FileAttributes = global::System.IO.FileAttributes;");
        sb.AppendLine("using FileInfo = global::System.IO.FileInfo;");
        sb.AppendLine("using FileMode = global::System.IO.FileMode;");
        sb.AppendLine("using FileNotFoundException = global::System.IO.FileNotFoundException;");
        sb.AppendLine("using FileOptions = global::System.IO.FileOptions;");
        sb.AppendLine("using FileShare = global::System.IO.FileShare;");
        sb.AppendLine("using FileStream = global::System.IO.FileStream;");
        sb.AppendLine("using FileSystemInfo = global::System.IO.FileSystemInfo;");
        sb.AppendLine("using IOException = global::System.IO.IOException;");
        sb.AppendLine("using MemoryStream = global::System.IO.MemoryStream;");
        sb.AppendLine("using Path = global::System.IO.Path;");
        sb.AppendLine("using SearchOption = global::System.IO.SearchOption;");
        sb.AppendLine("using SeekOrigin = global::System.IO.SeekOrigin;");
        sb.AppendLine("using Stream = global::System.IO.Stream;");
        sb.AppendLine("using StreamReader = global::System.IO.StreamReader;");
        sb.AppendLine("using StreamWriter = global::System.IO.StreamWriter;");
        sb.AppendLine("using StringReader = global::System.IO.StringReader;");
        sb.AppendLine("using StringWriter = global::System.IO.StringWriter;");
        sb.AppendLine("using TextReader = global::System.IO.TextReader;");
        sb.AppendLine("using TextWriter = global::System.IO.TextWriter;");
    }

    /// <summary>
    /// Detect await syntax that belongs to the generated snippet method itself.
    /// Await inside a local function or lambda has its own async context and
    /// therefore does not require the outer snippet body to be async.
    /// </summary>
    public static bool RequiresAsyncWrapper(string bodyCode, CSharpParseOptions parseOptions)
    {
        if (string.IsNullOrWhiteSpace(bodyCode))
            return false;

        StatementSyntax body = SyntaxFactory.ParseStatement(
            "{\n" + bodyCode + "\n}",
            options: parseOptions,
            consumeFullText: true);
        return ContainsTopLevelAwait(body);
    }

    private static bool ContainsTopLevelAwait(SyntaxNode node)
    {
        if (node is AnonymousFunctionExpressionSyntax or LocalFunctionStatementSyntax)
            return false;

        foreach (SyntaxToken token in node.ChildTokens())
        {
            if (token.IsKind(SyntaxKind.AwaitKeyword))
                return true;
        }

        foreach (SyntaxNode child in node.ChildNodes())
        {
            if (ContainsTopLevelAwait(child))
                return true;
        }

        return false;
    }

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
        bool expressionMode,
        bool useAsyncWrapper)
    {
        var sb = new StringBuilder(4096);

        sb.AppendLine("using System;");
        AppendCommonIoAliases(sb);
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
        sb.AppendLine("using Locus;");
        sb.AppendLine("using static UnityEngine.Object;");
        sb.AppendLine("using Object = UnityEngine.Object;");

        if (!string.IsNullOrWhiteSpace(leadingUsings))
            sb.AppendLine(leadingUsings);

        sb.AppendLine("namespace Locus.RuntimeSnippets");
        sb.AppendLine("{");
        sb.Append("    public static class ").Append(hostTypeName).AppendLine();
        sb.AppendLine("    {");

        if (useAsyncWrapper)
        {
            sb.AppendLine("        public static async global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
        }
        else
        {
            sb.AppendLine("        public static global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
            sb.AppendLine("        {");
            sb.AppendLine("            return global::System.Threading.Tasks.Task.FromResult<object>(ExecuteSync(globals, ctx, cancellationToken));");
            sb.AppendLine("        }");
            sb.AppendLine();
            sb.AppendLine("        private static object ExecuteSync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
        }

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
