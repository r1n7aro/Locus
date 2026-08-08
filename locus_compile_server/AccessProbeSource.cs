using System.Text;

namespace Locus.CompileServer;

/// <summary>One probe cell: a generated static method that exercises a
/// single IL operation against one visibility level of the target type's
/// surface. Unity force-JITs, invokes, and verifies <see cref="Expected"/> so
/// runtimes that defer access checks until first execution cannot false-green.</summary>
public sealed record AccessProbeCell(string Method, string Op, string Visibility, int Expected);

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
    private static readonly (string Op, string Visibility, string Body, int Expected)[] CellBodies = new[]
    {
        ("ldfld", "private", "var t = TARGET.New(); return t._privInst;", 7),
        ("ldfld", "internal", "var t = TARGET.New(); return t._intInst;", 11),
        ("stfld", "private", "var t = TARGET.New(); t._privInst = 42; return t.ReadPrivInst();", 42),
        ("stfld", "internal", "var t = TARGET.New(); t._intInst = 43; return t.ReadIntInst();", 43),
        ("ldsfld", "private", "TARGET.ResetStatics(); return TARGET._privStatic;", 13),
        ("ldsfld", "internal", "TARGET.ResetStatics(); return TARGET._intStatic;", 17),
        ("stsfld", "private", "TARGET.ResetStatics(); TARGET._privStatic = 47; return TARGET.ReadPrivStatic();", 47),
        ("stsfld", "internal", "TARGET.ResetStatics(); TARGET._intStatic = 53; return TARGET.ReadIntStatic();", 53),
        ("ldflda", "private", "var t = TARGET.New(); ref int slot = ref t._privInst; slot = 59; return t.ReadPrivInst();", 59),
        ("ldflda", "internal", "var t = TARGET.New(); ref int slot = ref t._intInst; slot = 61; return t.ReadIntInst();", 61),
        ("ldsflda", "private", "TARGET.ResetStatics(); ref int slot = ref TARGET._privStatic; slot = 67; return TARGET.ReadPrivStatic();", 67),
        ("ldsflda", "internal", "TARGET.ResetStatics(); ref int slot = ref TARGET._intStatic; slot = 71; return TARGET.ReadIntStatic();", 71),
        ("call", "private", "return TARGET.PrivStatic(3);", 16),
        ("call", "internal", "return TARGET.IntStatic(3);", 22),
        ("callvirt", "private", "var t = TARGET.New(); return t.PrivMethod(3);", 7),
        ("callvirt", "internal", "var t = TARGET.New(); return t.IntMethod(3);", 10),
        ("newobj", "private", "var t = new TARGET(9); return t.ReadPrivInst();", 9),
        ("newobj", "internal", "var t = new TARGET(); return t.ReadPrivInst();", 7),
        ("ldftn", "private", "var t = TARGET.New(); global::System.Func<int, int> f = t.PrivMethod; return f(5);", 11),
        ("ldftn", "internal", "var t = TARGET.New(); global::System.Func<int, int> f = t.IntMethod; return f(5);", 16),
        ("property_get", "private", "var t = TARGET.New(); return t.PrivProperty;", 23),
        ("property_get", "internal", "var t = TARGET.New(); return t.IntProperty;", 29),
        ("property_set", "private", "var t = TARGET.New(); t.PrivProperty = 31; return t.ReadPrivProperty();", 31),
        ("property_set", "internal", "var t = TARGET.New(); t.IntProperty = 37; return t.ReadIntProperty();", 37),
        ("event_add", "private", "var t = TARGET.New(); global::System.Action h = delegate { }; t.PrivEvent += h; return t.ReadPrivEventSubscribers();", 1),
        ("event_add", "internal", "var t = TARGET.New(); global::System.Action h = delegate { }; t.IntEvent += h; return t.ReadIntEventSubscribers();", 1),
        ("generic_call", "private", "var t = TARGET.New(); return t.PrivGeneric<int>(41);", 41),
        ("generic_call", "internal", "var t = TARGET.New(); return t.IntGeneric<int>(43);", 43),
        ("ref_call", "private", "var t = TARGET.New(); int value = 7; t.PrivRef(ref value); return value;", 12),
        ("ref_call", "internal", "var t = TARGET.New(); int value = 7; t.IntRef(ref value); return value;", 14),
        ("castclass", "private", "var t = (TARGET.PrivNested)_nullObject; return t == null ? 1 : 2;", 1),
        ("castclass", "internal", "object boxed = TARGET.New(); var t = (TARGET)boxed; return t == null ? 0 : 1;", 1),
        ("ldtoken", "private", "return typeof(TARGET.PrivNested).Name.Length > 0 ? 1 : 0;", 1),
        ("ldtoken", "internal", "return typeof(TARGET).Name.Length > 0 ? 1 : 0;", 1),
    };

    /// <summary>Cell manifest shipped to the Unity side alongside the
    /// compiled assembly (`cells: [{method, op, visibility}]`).</summary>
    public static IReadOnlyList<AccessProbeCell> Cells { get; } = CellBodies
        .Select(c => new AccessProbeCell(
            "Cell_" + c.Op + "_" + c.Visibility,
            c.Op,
            c.Visibility,
            c.Expected))
        .ToArray();

    public static string BuildSource()
    {
        var sb = new StringBuilder(4 * 1024);
        sb.Append("// C0 access probe: one method per (operation x visibility) cell; the\n");
        sb.Append("// Unity side force-JITs each one to measure Mono's access checks.\n");
        sb.Append("public static class ").Append(ProbeTypeName).Append('\n');
        sb.Append("{\n");
        sb.Append("    private static object _nullObject = null;\n");
        foreach (var (op, visibility, body, _) in CellBodies)
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
