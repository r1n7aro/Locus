using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json.Nodes;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// B6: partial types through compile/hotPatch. Sibling part files fold into
/// the batch as unchanged baselines (request `baselineSiblings`), the patch
/// re-declares the COMPLETE multi-part type, the cross-part instance-field
/// merge is ordered to match the original assembly (and verified by the
/// layout guard), and any member the original carries with no disk source —
/// a source-generator part or an undiscovered sibling — fails closed by
/// name.
/// </summary>
public class PartialTypeTests : IDisposable
{
    private readonly string _tempDir;

    public PartialTypeTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-partial-tests-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(_tempDir);
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_tempDir, recursive: true);
        }
        catch
        {
        }
    }

    private static string[] HostBclPaths()
    {
        return ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Where(File.Exists)
            .ToArray();
    }

    /// <summary>Compile multiple sources (the part files, in compile order —
    /// the order decides the original's cross-part field layout) into the
    /// "original assembly" and persist it as a file reference.</summary>
    private string CompileOriginal(CompileService service, string assemblyName, params (string Path, string Text)[] sources)
    {
        var request = new JsonObject
        {
            ["assemblyName"] = assemblyName,
            ["sources"] = new JsonArray(sources
                .Select(s => (JsonNode)new JsonObject { ["path"] = s.Path, ["text"] = s.Text })
                .ToArray()),
            ["useHostBcl"] = true,
        };
        JsonNode result = service.HandleCompileRaw(request);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        string path = Path.Combine(_tempDir, assemblyName + ".dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    private static JsonObject ParamsFor(params string[] extraReferences)
    {
        var paths = HostBclPaths().Concat(extraReferences);
        return new JsonObject
        {
            ["fingerprint"] = "partial-test-" + Guid.NewGuid().ToString("N"),
            ["domainGeneration"] = Guid.NewGuid().ToString("N"),
            ["langVersion"] = "9",
            ["referencePaths"] = new JsonArray(paths.Select(p => (JsonNode)p).ToArray()),
            ["defines"] = new JsonArray(),
        };
    }

    private static JsonNode HotPatch(
        CompileService service,
        JsonObject @params,
        (string Path, string Old, string New)[] files,
        params (string Path, string Text)[] siblings)
    {
        var request = new JsonObject
        {
            ["files"] = new JsonArray(files
                .Select(f => (JsonNode)new JsonObject
                {
                    ["path"] = f.Path,
                    ["oldText"] = f.Old,
                    ["newText"] = f.New,
                })
                .ToArray()),
            ["params"] = @params,
        };
        if (siblings.Length > 0)
        {
            request["baselineSiblings"] = new JsonArray(siblings
                .Select(s => (JsonNode)new JsonObject { ["path"] = s.Path, ["text"] = s.Text })
                .ToArray());
        }
        return service.HandleCompileHotPatch(request);
    }

    private static string ColdReasons(JsonNode result)
    {
        Assert.False(result["hot"]!.GetValue<bool>());
        return string.Join("\n", result["files"]!.AsArray()
            .Select(f => string.Join("; ", f!["reasons"]!.AsArray().Select(r => r!.GetValue<string>()))));
    }

    // ── corpus: one type, two part files ─────────────────────────────

    private const string PartA = @"
namespace PartialE2E
{
    public partial class Split
    {
        private int _alpha = 30;
        public int Combine() { return _alpha + Basis() + _beta; }
    }
}";

    private const string PartB = @"
namespace PartialE2E
{
    public partial class Split
    {
        private int _beta = 400;
        private int Basis() { return 5; }
    }
}";

    [Fact]
    public void Partial_body_edit_with_sibling_goes_hot_and_binds_sibling_members()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialSiblingOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace(
            "public int Combine() { return _alpha + Basis() + _beta; }",
            "public int Combine() { return _alpha + Basis() + _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        // Only the edited member detours; the sibling part contributes ZERO
        // detours and no new surface.
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("PartialE2E.Split", method["declaringType"]!.GetValue<string>());
        Assert.Equal("PartialE2E.Split__LocusPatch", method["patchDeclaringType"]!.GetValue<string>());
        Assert.Equal("Combine", method["name"]!.GetValue<string>());
        Assert.Empty(result["newTypes"]!.AsArray());

        // CoreCLR E2E: the multi-part patch type merges into ONE complete
        // copy whose edited body reaches the OTHER part's private field and
        // method.
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());

        var context = new AssemblyLoadContext("partial-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) =>
                name.Name == "PartialSiblingOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchType = patch.GetType("PartialE2E.Split__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchType)!;
            object? value = patchType.GetMethod("Combine")!.Invoke(instance, null);
            Assert.Equal(30 + 5 + 400 + 7000, value);

            // Layout proof: the merged patch copy carries BOTH parts' fields
            // in the original assembly's order.
            string[] patchFields = patchType
                .GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public)
                .Select(f => f.Name)
                .ToArray();
            string[] originalFields = original.GetType("PartialE2E.Split", throwOnError: true)!
                .GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public)
                .Select(f => f.Name)
                .ToArray();
            Assert.Equal(originalFields, patchFields);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Partial_edit_without_sibling_is_cold_naming_the_missing_member()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialMissingSiblingOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) });

        string reasons = ColdReasons(result);
        Assert.Contains("no source on disk", reasons);
        Assert.Contains("PartialE2E.Split", reasons);
    }

    [Fact]
    public void Generator_part_member_is_cold_pointed()
    {
        // The "generator part" is a source the compile saw but the disk
        // does not have: its members are visible in the original assembly
        // and in no batch part.
        const string generatorPart = @"
namespace PartialE2E
{
    public partial class Split
    {
        public int GeneratedHook() { return 99; }
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialGeneratorOriginal",
            ("PartA.cs", PartA), ("PartB.cs", PartB), ("Split.generated.cs", generatorPart));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB));

        string reasons = ColdReasons(result);
        Assert.Contains("no source on disk", reasons);
        Assert.Contains("GeneratedHook", reasons);
        Assert.Contains("source generator", reasons);
    }

    [Fact]
    public void Generator_part_static_field_is_cold_pointed()
    {
        const string generatorPart = @"
namespace PartialE2E
{
    public partial class Split
    {
        public static int GeneratedIndex;
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialGeneratorStaticOriginal",
            ("PartA.cs", PartA), ("PartB.cs", PartB), ("Split.generated.cs", generatorPart));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB));

        string reasons = ColdReasons(result);
        Assert.Contains("no source on disk", reasons);
        Assert.Contains("GeneratedIndex", reasons);
    }

    [Fact]
    public void Generator_part_instance_field_falls_cold_through_the_layout_guard()
    {
        // An instance field from a generator part is the memory-corruption
        // shape: the merged disk layout misses a slot. The constructive
        // layout guard (not the member gate) must catch it.
        const string generatorPart = @"
namespace PartialE2E
{
    public partial class Split
    {
        private int _generated;
        public int GenRead() { return _generated; }
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialGeneratorFieldOriginal",
            ("PartA.cs", PartA), ("PartB.cs", PartB), ("Split.generated.cs", generatorPart));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB));

        // The member gate fires first (GenRead has no source) — and even if
        // a generator emitted ONLY the field, the layout guard would fail
        // closed. Either way: cold, never a wrong patch.
        string reasons = ColdReasons(result);
        Assert.True(
            reasons.Contains("no source on disk") ||
            reasons.Contains("field layout differs"),
            reasons);
    }

    [Fact]
    public void Sibling_tree_order_is_constructed_from_the_original_layout()
    {
        // Original compiled PartB FIRST: cross-part field order (_beta,
        // _alpha). The natural batch order (changed file first) would merge
        // (_alpha, _beta) — the layout-ordering pass must place the sibling
        // tree first, or this goes (falsely) cold.
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialOrderOriginal", ("PartB.cs", PartB), ("PartA.cs", PartA));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        var context = new AssemblyLoadContext("partial-order-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) =>
                name.Name == "PartialOrderOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));
            Type patchType = patch.GetType("PartialE2E.Split__LocusPatch", throwOnError: true)!;

            string[] patchFields = patchType
                .GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public)
                .Select(f => f.Name)
                .ToArray();
            Assert.Equal(new[] { "_beta", "_alpha" }, patchFields);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Unsatisfiable_part_interleave_fails_the_layout_guard_closed()
    {
        // Original field order a, b, c with the disk parts split (a, c) +
        // (b): NO concatenation of the two part files can reproduce it.
        const string originalA = "namespace P { public partial class T { public int a; public int M() { return 1; } } }";
        const string originalB = "namespace P { public partial class T { public int b; } }";
        const string originalC = "namespace P { public partial class T { public int c; } }";

        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialInterleaveOriginal",
            ("A.cs", originalA), ("B.cs", originalB), ("C.cs", originalC));
        JsonObject compileParams = ParamsFor(originalPath);

        // Disk truth (per the request): one file now claims (a, c), the
        // sibling has (b) — a stale-baseline / moved-field configuration.
        const string diskAC = "namespace P { public partial class T { public int a; public int c; public int M() { return 1; } } }";
        string editedAC = diskAC.Replace("return 1;", "return 2;");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("A.cs", diskAC, editedAC) },
            ("B.cs", originalB));

        string reasons = ColdReasons(result);
        Assert.Contains("field layout differs", reasons);
    }

    [Fact]
    public void Single_part_partial_type_goes_hot_without_siblings()
    {
        const string solo = @"
namespace P
{
    public partial class Solo
    {
        private int _x = 3;
        public int M() { return _x; }
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "PartialSoloOriginal", ("Solo.cs", solo));
        JsonObject compileParams = ParamsFor(originalPath);

        string edited = solo.Replace("return _x;", "return _x + 100;");

        JsonNode result = HotPatch(service, compileParams, new[] { ("Solo.cs", solo, edited) });

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("M", method["name"]!.GetValue<string>());
    }

    [Fact]
    public void Unmatched_sibling_candidates_are_ignored()
    {
        // The coordinator's discovery is grep-grade: a candidate declaring
        // an UNRELATED partial type must not join the batch (its body would
        // not even compile).
        const string unrelated = "namespace Q { public partial class W { public int X() { return Missing.Y; } } }";

        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialUnmatchedOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", PartB), ("W.cs", unrelated));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
    }

    [Fact]
    public void Matching_sibling_with_parse_error_fails_closed()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialBrokenSiblingOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");
        const string brokenB = "namespace PartialE2E { public partial class Split { private int _beta = ; } }";

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartB.cs", brokenB));

        string reasons = ColdReasons(result);
        Assert.Contains("partial sibling part does not parse", reasons);
    }

    [Fact]
    public void Sibling_closure_includes_parts_of_types_a_sibling_declares()
    {
        // Editing T pulls in sibling S1 (T's other part) — which also
        // declares partial type U, whose own remaining part S2 must come
        // along or S1's body cannot compile.
        const string fileT1 = "namespace P { public partial class T { public int M() { return 1; } } }";
        const string fileS1 = @"
namespace P
{
    public partial class T { public int Other() { return 2; } }
    public partial class U { public int UVal() { return UBase() + 1; } }
}";
        const string fileS2 = "namespace P { public partial class U { private int UBase() { return 2; } } }";

        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialClosureOriginal",
            ("T1.cs", fileT1), ("S1.cs", fileS1), ("S2.cs", fileS2));
        JsonObject compileParams = ParamsFor(originalPath);

        string edited = fileT1.Replace("return 1;", "return 4242;");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("T1.cs", fileT1, edited) },
            ("S1.cs", fileS1), ("S2.cs", fileS2));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("P.T", method["declaringType"]!.GetValue<string>());
    }

    [Fact]
    public void Added_member_on_partial_type_shims_with_sibling_present()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialAddedOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        // Add a member to part A that reads part B's private field, and
        // re-point the kept Combine at it.
        string newA = PartA.Replace(
            "public int Combine() { return _alpha + Basis() + _beta; }",
            "public int Combine() { return Boost(); }\n        public int Boost() { return _alpha + _beta + 9000; }");

        // Same all-green caps shape as HotPatchTests.GreenRuntimeCaps: the
        // added member reaches the ORIGINAL type's private fields (C2′a).
        var cells = new JsonObject();
        foreach (AccessProbeCell cell in AccessProbeSource.Cells)
            cells[cell.Op + "_" + cell.Visibility] = true;
        var caps = new JsonObject
        {
            ["createDelegateNonPublic"] = true,
            ["dynamicMethodSkipVisibility"] = true,
            ["dynamicMethodByrefReturn"] = false,
            ["cells"] = cells,
        };

        var request = new JsonObject
        {
            ["files"] = new JsonArray(new JsonObject
            {
                ["path"] = "PartA.cs",
                ["oldText"] = PartA,
                ["newText"] = newA,
            }),
            ["params"] = compileParams,
            ["runtimeCaps"] = caps,
            ["baselineSiblings"] = new JsonArray(
                new JsonObject { ["path"] = "PartB.cs", ["text"] = PartB }),
        };
        JsonNode result = service.HandleCompileHotPatch(request);

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        // Combine detours; Boost becomes a shim (no detour entry).
        var methods = result["methods"]!.AsArray();
        var combine = Assert.Single(methods)!;
        Assert.Equal("Combine", combine["name"]!.GetValue<string>());

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("partial-added-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) =>
                name.Name == "PartialAddedOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            // The shim takes the ORIGINAL type as self.
            Type shimType = patch.GetType("PartialE2E.Split__LocusShims", throwOnError: true)!;
            object originalInstance = Activator.CreateInstance(
                original.GetType("PartialE2E.Split", throwOnError: true)!)!;
            object? value = shimType.GetMethod("Boost")!.Invoke(null, new[] { originalInstance });
            Assert.Equal(30 + 400 + 9000, value);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Sibling_path_equal_to_a_changed_file_is_not_folded_twice()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialDupPathOriginal", ("PartA.cs", PartA), ("PartB.cs", PartB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = PartA.Replace("+ _beta; }", "+ _beta + 7000; }");

        // The coordinator never sends a changed file as its own sibling,
        // but the sidecar must stay correct if one slips through.
        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", PartA, newA) },
            ("PartA.cs", PartA), ("PartB.cs", PartB));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.Single(result["methods"]!.AsArray());
    }

    [Fact]
    public void Non_partial_batches_ignore_baseline_siblings()
    {
        const string plain = "namespace P { public class Plain { public int M() { return 1; } } }";
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "PartialIgnoredOriginal", ("Plain.cs", plain));
        JsonObject compileParams = ParamsFor(originalPath);

        string edited = plain.Replace("return 1;", "return 2;");
        const string strayPartial = "namespace P { public partial class W { public int X() { return Missing.Y; } } }";

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("Plain.cs", plain, edited) },
            ("W.cs", strayPartial));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
    }

    [Fact]
    public void Member_removed_from_a_partial_part_passes_the_completeness_gate()
    {
        // Deleting a member from part A: the original still carries it, but
        // the batch REMOVES it deliberately (M5) — the completeness gate
        // must not mistake it for a generator part. The M3 caller scan needs
        // project-shaped assembly paths, which this fixture does not have —
        // asserting the gate is NOT the cold reason is the point here.
        const string partA = @"
namespace P
{
    public partial class T
    {
        public int M() { return 1; }
        private int Doomed() { return 2; }
    }
}";
        const string partB = "namespace P { public partial class T { public int Other() { return 3; } } }";

        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialRemovalOriginal", ("PartA.cs", partA), ("PartB.cs", partB));
        JsonObject compileParams = ParamsFor(originalPath);

        string newA = partA.Replace("        private int Doomed() { return 2; }\n", "");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("PartA.cs", partA, newA) },
            ("PartB.cs", partB));

        if (!result["hot"]!.GetValue<bool>())
        {
            string reasons = ColdReasons(result);
            Assert.DoesNotContain("no source on disk", reasons);
            Assert.Contains("call sites", reasons); // the caller scan, not the B6 gate
        }
    }

    [Fact]
    public void Three_part_partial_type_merges_all_siblings_and_executes()
    {
        // Editing ONE part of a THREE-part type folds BOTH other parts in as
        // baselines; the merged patch copy carries every part's field in the
        // original layout order and the edited body reaches across all parts.
        const string p1 = @"
namespace TriE2E
{
    public partial class Tri
    {
        private int _a = 1;
        public int Sum() { return _a + _b + _c; }
    }
}";
        const string p2 = @"
namespace TriE2E
{
    public partial class Tri
    {
        private int _b = 20;
    }
}";
        const string p3 = @"
namespace TriE2E
{
    public partial class Tri
    {
        private int _c = 300;
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(
            service, "PartialTriOriginal", ("P1.cs", p1), ("P2.cs", p2), ("P3.cs", p3));
        JsonObject compileParams = ParamsFor(originalPath);

        string newP1 = p1.Replace("return _a + _b + _c;", "return _a + _b + _c + 5000;");

        JsonNode result = HotPatch(
            service, compileParams,
            new[] { ("P1.cs", p1, newP1) },
            ("P2.cs", p2), ("P3.cs", p3));

        Assert.True(result["hot"]!.GetValue<bool>(), result.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("Sum", method["name"]!.GetValue<string>());
        Assert.Empty(result["newTypes"]!.AsArray());

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("partial-tri-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "PartialTriOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchType = patch.GetType("TriE2E.Tri__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchType)!;
            Assert.Equal(1 + 20 + 300 + 5000, patchType.GetMethod("Sum")!.Invoke(instance, null));

            string[] fields = patchType
                .GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public)
                .Select(f => f.Name)
                .ToArray();
            Assert.Equal(new[] { "_a", "_b", "_c" }, fields);
        }
        finally
        {
            context.Unload();
        }
    }
}
