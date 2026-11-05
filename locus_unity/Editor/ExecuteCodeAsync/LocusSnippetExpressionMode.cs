#nullable disable
// Shared source: compiled into the Unity plugin (in-Unity snippet compile
// path) and linked into the Locus compile server (LocusCompileServer.csproj),
// so both compile paths decide the expression-mode retry the same way. Keep it
// valid for Unity's C# 9 compiler and Roslyn 3.8 (Locus.Roslyn.dll) as well as
// the compile server's current Roslyn.

using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Locus
{
    /// <summary>
    /// A snippet compiles as statements first; a body that is one bare
    /// expression (<c>Selection.activeObject.name</c>) is retried as
    /// <c>return (object)(...)</c> so its value prints. Only such a body can
    /// succeed that way, so statement bodies skip the retry and report their
    /// own compile errors without unrelated expression-mode diagnostics.
    /// </summary>
    public static class LocusSnippetExpressionMode
    {
        public static bool Applies(string bodyCode, CSharpParseOptions parseOptions)
        {
            if (string.IsNullOrWhiteSpace(bodyCode))
                return false;

            ExpressionSyntax expression = SyntaxFactory.ParseExpression(
                bodyCode,
                0,
                parseOptions,
                true);
            return !expression.ContainsDiagnostics;
        }
    }
}
