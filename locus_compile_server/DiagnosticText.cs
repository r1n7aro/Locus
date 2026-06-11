using System.Text;
using Microsoft.CodeAnalysis;

namespace Locus.CompileServer;

public static class DiagnosticText
{
    /// <summary>
    /// Verbatim port of LocusBridge.ExecuteCode.cs BuildDiagnosticErrorText.
    /// The "compilation failed:" framing is a contract: agent prompts and the
    /// Unity fallback path both produce exactly this shape, so the two compile
    /// paths must stay byte-identical (locked by golden tests).
    /// </summary>
    public static string? BuildDiagnosticErrorText(IEnumerable<Diagnostic>? diagnostics)
    {
        if (diagnostics == null)
            return null;

        var sb = new StringBuilder();
        bool hasError = false;

        foreach (Diagnostic? diagnostic in diagnostics)
        {
            if (diagnostic == null)
                continue;

            if (diagnostic.Severity != DiagnosticSeverity.Error)
                continue;

            if (!hasError)
            {
                hasError = true;
                sb.Append("compilation failed:\n");
            }

            int line = 0;
            int column = 0;

            try
            {
                FileLinePositionSpan span = diagnostic.Location.GetMappedLineSpan();
                line = span.StartLinePosition.Line + 1;
                column = span.StartLinePosition.Character + 1;
            }
            catch
            {
            }

            sb.Append("  ");
            sb.Append(diagnostic.Id);
            sb.Append(" at ");
            sb.Append(line);
            sb.Append(":");
            sb.Append(column);
            sb.Append(": ");
            sb.Append(diagnostic.GetMessage());
            sb.Append("\n");
        }

        return hasError ? sb.ToString() : null;
    }
}
