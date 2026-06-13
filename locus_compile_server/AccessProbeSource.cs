using System.Text;

namespace Locus.CompileServer;

/// <summary>One probe cell: a generated static method that exercises a
/// single IL operation against one visibility level of the target type's
/// surface. <see cref="Method"/> is the metadata name the Unity side
/// force-JITs via RuntimeHelpers.PrepareMethod.</summary>
public sealed record AccessProbeCell(string Method, string Op, string Visibility);

/// <summary>
/// C0 runtime capability probe (unity-hotreload-compat-plan.md §C0): a fixed
/// synthetic source compiled by `compile/accessProbe` against the project's
/// real reference set (plus the usual IgnoresAccessChecksTo tree). Each cell
/// method genuinely touches one non-public member of the Unity plugin's
/// <c>Locus.LocusAccessProbeTarget</c> (LocusBridge.AccessProbe.cs — the
/// member names/values are a cross-side contract). The Unity side loads the
/// emitted assembly and JITs every cell to measure how the running editor's
/// Mono enforces accessibility per operation × visibility; the result gates
/// the C2′ access-thunk lowering decisions. Nothing here is Mono-version
/// specific by construction — the matrix is measured, never assumed.
/// </summary>
public static class AccessProbeSource
{
    /// <summary>Top-level (no namespace) metadata name of the probe class.</summary>
    public const string ProbeTypeName = "__LocusAccessProbe";

    public const string SourcePath = "LocusAccessProbe.cs";

    /// <summary>The Unity plugin's probe target type (internal, in the
    /// Locus.Editor script assembly), referenced fully qualified so the
    /// generated source needs no usings.</summary>
    public const string TargetTypeName = "global::Locus.LocusAccessProbeTarget";

    // op × visibility × method body ("TARGET" → TargetTypeName). Every body
    // really reaches the member and returns an int so nothing folds away.
    // Instance cells obtain the receiver through the public New() factory:
    // the internal type token is unavoidably part of every cell anyway (the
    // member's parent type must resolve at JIT), so the extra call adds no
    // new check class. castclass/ldtoken × private use the private nested
    // type (type-level checks, distinct from member-level ones); the
    // castclass receiver is a null-initialized static field so the cell also
    // EXECUTES cleanly if anything ever invokes it (null passes castclass).
    private static readonly (string Op, string Visibility, string Body)[] CellBodies = new[]
    {
        ("ldfld", "private", "var t = TARGET.New(); return t._privInst;"),
        ("ldfld", "internal", "var t = TARGET.New(); return t._intInst;"),
        ("stfld", "private", "var t = TARGET.New(); t._privInst = 42; return 1;"),
        ("stfld", "internal", "var t = TARGET.New(); t._intInst = 42; return 1;"),
        ("ldsfld", "private", "return TARGET._privStatic;"),
        ("ldsfld", "internal", "return TARGET._intStatic;"),
        ("stsfld", "private", "TARGET._privStatic = 42; return 1;"),
        ("stsfld", "internal", "TARGET._intStatic = 42; return 1;"),
        ("call", "private", "return TARGET.PrivStatic(3);"),
        ("call", "internal", "return TARGET.IntStatic(3);"),
        ("callvirt", "private", "var t = TARGET.New(); return t.PrivMethod(3);"),
        ("callvirt", "internal", "var t = TARGET.New(); return t.IntMethod(3);"),
        ("newobj", "private", "var t = new TARGET(9); return t == null ? 0 : 1;"),
        ("newobj", "internal", "var t = new TARGET(); return t == null ? 0 : 1;"),
        ("castclass", "private", "var t = (TARGET.PrivNested)_nullObject; return t == null ? 1 : 2;"),
        ("castclass", "internal", "object boxed = TARGET.New(); var t = (TARGET)boxed; return t == null ? 0 : 1;"),
        ("ldtoken", "private", "return typeof(TARGET.PrivNested).Name.Length;"),
        ("ldtoken", "internal", "return typeof(TARGET).Name.Length;"),
    };

    /// <summary>Cell manifest shipped to the Unity side alongside the
    /// compiled assembly (`cells: [{method, op, visibility}]`).</summary>
    public static IReadOnlyList<AccessProbeCell> Cells { get; } = CellBodies
        .Select(c => new AccessProbeCell("Cell_" + c.Op + "_" + c.Visibility, c.Op, c.Visibility))
        .ToArray();

    public static string BuildSource()
    {
        var sb = new StringBuilder(4 * 1024);
        sb.Append("// C0 access probe: one method per (operation x visibility) cell; the\n");
        sb.Append("// Unity side force-JITs each one to measure Mono's access checks.\n");
        sb.Append("public static class ").Append(ProbeTypeName).Append('\n');
        sb.Append("{\n");
        sb.Append("    private static object _nullObject = null;\n");
        foreach (var (op, visibility, body) in CellBodies)
        {
            sb.Append("    public static int Cell_").Append(op).Append('_').Append(visibility)
              .Append("() { ")
              .Append(body.Replace("TARGET", TargetTypeName))
              .Append(" }\n");
        }
        sb.Append("}\n");
        return sb.ToString();
    }
}
