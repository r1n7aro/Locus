using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// compile/hotPatch behavior through the handler layer, including an
/// end-to-end proof of the rewriter's core property: patched bodies bind to
/// the ORIGINAL assembly's types and statics (object identity and static
/// state never split), with accessibility checks bypassed at compile time
/// (IgnoreAccessibility) and at runtime (IgnoresAccessChecksTo, honored by
/// CoreCLR here and by Unity's Mono in the editor).
/// </summary>
public class HotPatchTests : IDisposable
{
    private readonly string _tempDir;

    public HotPatchTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-hotpatch-tests-" + Guid.NewGuid().ToString("N"));
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

    /// <summary>Compile `text` with the host BCL and persist it so it can be
    /// used as an ordinary file reference (the "original assembly").</summary>
    private string CompileOriginal(CompileService service, string assemblyName, string text)
    {
        var request = new JsonObject
        {
            ["assemblyName"] = assemblyName,
            ["sources"] = new JsonArray(new JsonObject { ["path"] = assemblyName + ".cs", ["text"] = text }),
            ["useHostBcl"] = true,
        };
        JsonNode result = service.HandleCompileRaw(request);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        string path = Path.Combine(_tempDir, assemblyName + ".dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    private JsonObject ParamsFor(params string[] extraReferences)
    {
        var paths = HostBclPaths().Concat(extraReferences);
        return new JsonObject
        {
            ["fingerprint"] = "hotpatch-test-" + Guid.NewGuid().ToString("N"),
            ["domainGeneration"] = Guid.NewGuid().ToString("N"),
            ["langVersion"] = "9",
            ["referencePaths"] = new JsonArray(paths.Select(p => (JsonNode)p).ToArray()),
            ["defines"] = new JsonArray(),
        };
    }

    private static JsonNode HotPatch(CompileService service, JsonObject @params, params (string Path, string Old, string New)[] files)
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
        return service.HandleCompileHotPatch(request);
    }

    private const string OriginalSource = @"
namespace HotPatchE2E
{
    public class Calc
    {
        private static int Mode = 1;
        private int _seed = 10;
        public int Value() { return _seed; }
        public object Make() { return new Helper(); }
    }
    public class Helper
    {
        public int Tag = 7;
    }
}";

    [Fact]
    public void Hot_patch_binds_original_types_and_private_statics()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "HotPatchE2EOriginal", OriginalSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newSource = OriginalSource
            .Replace("public int Value() { return _seed; }",
                     "public int Value() { return _seed + 100 + Mode; }")
            .Replace("public object Make() { return new Helper(); }",
                     "public object Make() { var h = new Helper(); h.Tag = 8; return h; }");

        JsonNode result = HotPatch(service, compileParams, ("Calc.cs", OriginalSource, newSource));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.StartsWith("__LocusHotPatch_", result["assemblyName"]!.GetValue<string>());

        var methods = result["methods"]!.AsArray();
        Assert.Equal(2, methods.Count);
        Assert.All(methods, m => Assert.Equal("HotPatchE2E.Calc", m!["declaringType"]!.GetValue<string>()));
        Assert.All(methods, m => Assert.Equal("HotPatchE2E.Calc__LocusPatch", m!["patchDeclaringType"]!.GetValue<string>()));
        Assert.Contains(methods, m => m!["name"]!.GetValue<string>() == "Value");
        Assert.Contains(methods, m => m!["name"]!.GetValue<string>() == "Make");

        // Load original + patch into an isolated context and execute the
        // patched bodies: they must read the ORIGINAL private static and
        // construct the ORIGINAL Helper type.
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());

        var context = new AssemblyLoadContext("hotpatch-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) =>
                name.Name == "HotPatchE2EOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchCalc = patch.GetType("HotPatchE2E.Calc__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchCalc)!;

            object? value = patchCalc.GetMethod("Value")!.Invoke(instance, null);
            Assert.Equal(10 + 100 + 1, value); // _seed(10) + 100 + original private static Mode(1)

            object made = patchCalc.GetMethod("Make")!.Invoke(instance, null)!;
            Assert.Same(original, made.GetType().Assembly); // identity: original Helper, not a patch copy
            Assert.Equal("HotPatchE2E.Helper", made.GetType().FullName);
            Assert.Equal(8, made.GetType().GetField("Tag")!.GetValue(made));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void New_source_file_compiles_as_new_types_without_detours()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        JsonNode result = HotPatch(
            service,
            compileParams,
            ("Spawner.cs", "", "namespace HotPatchE2E { public class Spawner { public int Count() { return 3; } } }"));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.Empty(result["methods"]!.AsArray());

        var newType = Assert.Single(result["newTypes"]!.AsArray())!;
        Assert.Equal("HotPatchE2E.Spawner", newType["metadataName"]!.GetValue<string>());
        Assert.Equal("HotPatchE2E", newType["ns"]!.GetValue<string>());
        Assert.Equal("Spawner", newType["simpleName"]!.GetValue<string>());
        Assert.True(newType["isPublic"]!.GetValue<bool>());
        Assert.True(newType["isTopLevel"]!.GetValue<bool>());
    }

    // Public state only: Mono enforces accessibility when the shim jits, so
    // added members may not touch non-public surface (cold otherwise — see
    // Added_member_touching_private_state_is_cold).
    private const string ShimCalcSource = @"
namespace ShimE2E
{
    public class Calc
    {
        public int Seed = 10;
        public static int Bias = 5;
        public int Value() { return Seed; }
    }
}";

    private const string ShimCallerSource = @"
namespace ShimE2E
{
    public class Caller
    {
        public static int Run() { return 1; }
    }
}";

    [Fact]
    public void Added_method_in_one_file_is_callable_from_another_via_shim()
    {
        var service = new CompileService();
        string calcPath = CompileOriginal(service, "ShimE2ECalc", ShimCalcSource);
        string callerPath = CompileOriginal(service, "ShimE2ECaller", ShimCallerSource);
        JsonObject compileParams = ParamsFor(calcPath, callerPath);

        string newCalc = ShimCalcSource.Replace(
            "public int Value() { return Seed; }",
            "public int Value() { return Seed; }\n        public int Boost(int extra) { return Seed + Bias + extra; }");
        string newCaller = ShimCallerSource.Replace(
            "public static int Run() { return 1; }",
            "public static int Run() { var c = new Calc(); return c.Boost(7); }");

        JsonNode result = HotPatch(
            service, compileParams,
            ("Calc.cs", ShimCalcSource, newCalc),
            ("Caller.cs", ShimCallerSource, newCaller));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        // Only Caller.Run detours; the added Boost is shim-only (no detour
        // on its first appearance).
        var methods = result["methods"]!.AsArray();
        var run = Assert.Single(methods)!;
        Assert.Equal("ShimE2E.Caller", run["declaringType"]!.GetValue<string>());
        Assert.Equal("Run", run["name"]!.GetValue<string>());

        // Execute the patched Run: it must construct the ORIGINAL Calc and
        // reach the shim, which reads the original private field + static.
        byte[] calcBytes = File.ReadAllBytes(calcPath);
        byte[] callerBytes = File.ReadAllBytes(callerPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());

        var context = new AssemblyLoadContext("shim-e2e", isCollectible: true);
        try
        {
            Assembly calcAssembly = context.LoadFromStream(new MemoryStream(calcBytes));
            Assembly callerAssembly = context.LoadFromStream(new MemoryStream(callerBytes));
            context.Resolving += (_, name) => name.Name switch
            {
                "ShimE2ECalc" => calcAssembly,
                "ShimE2ECaller" => callerAssembly,
                _ => null,
            };
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchCaller = patch.GetType("ShimE2E.Caller__LocusPatch", throwOnError: true)!;
            object? value = patchCaller.GetMethod("Run")!.Invoke(null, null);
            Assert.Equal(10 + 5 + 7, value);

            // The shim itself works against an original instance.
            Type shims = patch.GetType("ShimE2E.Calc__LocusShims", throwOnError: true)!;
            object original = Activator.CreateInstance(calcAssembly.GetType("ShimE2E.Calc")!)!;
            object? boosted = shims.GetMethod("Boost")!.Invoke(null, new[] { original, (object)1 });
            Assert.Equal(10 + 5 + 1, boosted);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Reedited_added_member_detours_the_previous_shim()
    {
        var service = new CompileService();
        string calcPath = CompileOriginal(service, "ShimReeditCalc", ShimCalcSource);
        JsonObject compileParams = ParamsFor(calcPath);
        compileParams["domainGeneration"] = "reedit-gen";

        string addedV1 = ShimCalcSource.Replace(
            "public int Value() { return Seed; }",
            "public int Value() { return Seed; }\n        public int Boost() { return 1; }");
        var requestV1 = new JsonObject
        {
            ["files"] = new JsonArray(new JsonObject
            {
                ["path"] = "Calc.cs",
                ["oldText"] = ShimCalcSource,
                ["newText"] = addedV1,
            }),
            ["params"] = compileParams.DeepClone(),
            ["registerImage"] = true, // inline accept: commits shim registry
        };
        JsonNode resultV1 = service.HandleCompileHotPatch(requestV1);
        Assert.True(resultV1["success"]!.GetValue<bool>(), resultV1["error"]?.GetValue<string>());
        Assert.Empty(resultV1["methods"]!.AsArray());
        string assemblyV1 = resultV1["assemblyName"]!.GetValue<string>();

        string addedV2 = ShimCalcSource.Replace(
            "public int Value() { return Seed; }",
            "public int Value() { return Seed; }\n        public int Boost() { return 2; }");
        var requestV2 = new JsonObject
        {
            ["files"] = new JsonArray(new JsonObject
            {
                ["path"] = "Calc.cs",
                ["oldText"] = ShimCalcSource,
                ["newText"] = addedV2,
            }),
            ["params"] = compileParams.DeepClone(),
            ["registerImage"] = true,
        };
        JsonNode resultV2 = service.HandleCompileHotPatch(requestV2);
        Assert.True(resultV2["success"]!.GetValue<bool>(), resultV2["error"]?.GetValue<string>());

        // Re-edit continuity: the old shim method detours to the new one so
        // in-flight delegates pick up the new behavior.
        var detour = Assert.Single(resultV2["methods"]!.AsArray())!;
        Assert.Equal("ShimE2E.Calc__LocusShims", detour["declaringType"]!.GetValue<string>());
        Assert.Equal("ShimE2E.Calc__LocusShims", detour["patchDeclaringType"]!.GetValue<string>());
        Assert.Equal("Boost", detour["name"]!.GetValue<string>());
        Assert.True(detour["isStatic"]!.GetValue<bool>());
        Assert.Equal(assemblyV1, detour["originalAssembly"]!.GetValue<string>());
        Assert.Equal(new[] { "Calc" }, detour["paramTypeNames"]!.AsArray().Select(p => p!.GetValue<string>()));
    }

    // Operators force CS0563/CS0556 in renamed patch copies: unchanged
    // declarations strip, changed ones rename their self-typed parameters.
    private const string OperatorStructSource = @"
public struct Vec
{
    public int Value;

    public int Get() { return 1; }

    public static Vec operator +(Vec a, Vec b)
    {
        var r = new Vec();
        r.Value = a.Value + b.Value;
        return r;
    }

    public static implicit operator int(Vec v) { return v.Value; }
}";

    [Fact]
    public void Unchanged_operators_are_stripped_so_other_edits_stay_hot()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "OpStrip", OperatorStructSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newText = OperatorStructSource.Replace(
            "public int Get() { return 1; }",
            "public int Get() { return 6446; }");

        JsonNode result = HotPatch(service, compileParams, ("Vec.cs", OperatorStructSource, newText));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("Get", method["name"]!.GetValue<string>());

        // The copies are gone from the patch type: CS0563/CS0556 never trip
        // and the original's operators keep serving every call site.
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("op-strip", isCollectible: true);
        try
        {
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));
            Type patchType = patch.GetType("Vec__LocusPatch", throwOnError: true)!;
            Assert.DoesNotContain(
                patchType.GetMethods(BindingFlags.Public | BindingFlags.Static | BindingFlags.DeclaredOnly),
                m => m.Name.StartsWith("op_", StringComparison.Ordinal));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Changed_operator_parameters_rename_to_the_patch_type()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "OpRename", OperatorStructSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newText = OperatorStructSource.Replace(
            "r.Value = a.Value + b.Value;",
            "r.Value = a.Value + b.Value + 7337;");

        JsonNode result = HotPatch(service, compileParams, ("Vec.cs", OperatorStructSource, newText));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("op_Addition", method["name"]!.GetValue<string>());
        Assert.Equal("Vec", method["declaringType"]!.GetValue<string>());
        Assert.Equal(
            new[] { "Vec", "Vec" },
            method["paramTypeNames"]!.AsArray().Select(p => p!.GetValue<string>()));

        // The patch declaration satisfies CS0563 by naming ITS containing
        // type; the Unity side maps it back by stripping the suffix.
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("op-rename", isCollectible: true);
        try
        {
            Assembly originalAssembly = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "OpRename" ? originalAssembly : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));
            Type patchType = patch.GetType("Vec__LocusPatch", throwOnError: true)!;
            MethodInfo op = patchType
                .GetMethods(BindingFlags.Public | BindingFlags.Static | BindingFlags.DeclaredOnly)
                .Single(m => m.Name == "op_Addition");
            Assert.All(op.GetParameters(), p => Assert.Equal("Vec__LocusPatch", p.ParameterType.Name));
        }
        finally
        {
            context.Unload();
        }
    }

    /// <summary>Compile an "original" into Library/ScriptAssemblies so the
    /// M3 caller scan treats it as a project assembly (embedded PDB carries
    /// the source document paths).</summary>
    private string CompileProjectAssembly(CompileService service, string assemblyName, params (string Path, string Text)[] sources)
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

        string dir = Path.Combine(_tempDir, "Library", "ScriptAssemblies");
        Directory.CreateDirectory(dir);
        string path = Path.Combine(dir, assemblyName + ".dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    private const string ScanLibSource = @"
namespace ScanE2E
{
    public class Lib
    {
        public int M(int a) { return a; }
    }
}";

    private const string ScanUseSource = @"
namespace ScanE2E
{
    public class Use
    {
        public static int Go() { var l = new Lib(); return l.M(5); }
    }
}";

    [Fact]
    public void Removal_with_uncovered_caller_is_cold_with_exact_file_list()
    {
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "ScanE2EUncovered",
            ("Assets/Lib.cs", ScanLibSource),
            ("Assets/Use.cs", ScanUseSource));
        JsonObject compileParams = ParamsFor(asmPath);

        string removed = ScanLibSource.Replace("public int M(int a) { return a; }", "");
        JsonNode result = HotPatch(service, compileParams, ("Assets/Lib.cs", ScanLibSource, removed));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        Assert.Equal("Assets/Lib.cs", file["path"]!.GetValue<string>());
        string reason = file["reasons"]!.AsArray().Single()!.GetValue<string>();
        Assert.Contains("Assets/Use.cs", reason);
        Assert.Contains("unity_recompile", reason);
    }

    [Fact]
    public void Rename_with_caller_in_batch_goes_hot_and_executes_via_shim()
    {
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "ScanE2ERename",
            ("Assets/Lib.cs", ScanLibSource),
            ("Assets/Use.cs", ScanUseSource));
        JsonObject compileParams = ParamsFor(asmPath);

        string renamedLib = ScanLibSource.Replace(
            "public int M(int a) { return a; }",
            "public int MM(int a) { return a + 100; }");
        string updatedUse = ScanUseSource.Replace("return l.M(5);", "return l.MM(5);");

        JsonNode result = HotPatch(
            service, compileParams,
            ("Assets/Lib.cs", ScanLibSource, renamedLib),
            ("Assets/Use.cs", ScanUseSource, updatedUse));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.Contains("verified", result["callerScan"]!.GetValue<string>());

        // Only Use.Go detours; the renamed MM is an added shim.
        var detour = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("ScanE2E.Use", detour["declaringType"]!.GetValue<string>());
        Assert.Equal("Go", detour["name"]!.GetValue<string>());

        byte[] originalBytes = File.ReadAllBytes(asmPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("rename-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "ScanE2ERename" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchUse = patch.GetType("ScanE2E.Use__LocusPatch", throwOnError: true)!;
            object? value = patchUse.GetMethod("Go")!.Invoke(null, null);
            Assert.Equal(105, value);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Pure_removal_with_covered_callers_is_a_noop_with_tombstones()
    {
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "ScanE2EPureRemoval",
            ("Assets/Lib.cs", ScanLibSource),
            ("Assets/Use.cs", ScanUseSource));
        JsonObject compileParams = ParamsFor(asmPath);
        compileParams["domainGeneration"] = "removal-gen";

        string removedLib = ScanLibSource.Replace("public int M(int a) { return a; }", "");
        string updatedUse = ScanUseSource.Replace("return l.M(5);", "return 5;");

        // Use.cs changes too (drops the call) so the scan passes; Lib.cs is
        // a pure deletion. Both files hot → Use detours, M tombstones.
        JsonNode result = HotPatch(
            service, compileParams,
            ("Assets/Lib.cs", ScanLibSource, removedLib),
            ("Assets/Use.cs", ScanUseSource, updatedUse));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        // Not a noop overall (Use.Go detours), but the batch carries the
        // tombstone via the pending-shim flow.
        Assert.Single(result["methods"]!.AsArray());
    }

    // ── B1: generic method bodies via remove+add shims ───────────────

    private const string GenericLibSource = @"
namespace GenE2E
{
    public class Lib
    {
        public T Echo<T>(T value) where T : struct { return value; }
    }
}";

    private const string GenericUseSource = @"
namespace GenE2E
{
    public class Use
    {
        public static int Go() { return new Lib().Echo(5); }
        public static int Other() { return 1; }
    }
}";

    [Fact]
    public void Generic_body_change_with_uncovered_caller_is_cold_naming_file()
    {
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "GenE2EUncovered",
            ("Assets/Lib.cs", GenericLibSource),
            ("Assets/Use.cs", GenericUseSource));
        JsonObject compileParams = ParamsFor(asmPath);

        string newLib = GenericLibSource.Replace("{ return value; }", "{ return default(T); }");
        JsonNode result = HotPatch(service, compileParams, ("Assets/Lib.cs", GenericLibSource, newLib));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        string reason = file["reasons"]!.AsArray().Single()!.GetValue<string>();
        Assert.Contains("generic method body changed", reason);
        Assert.Contains("Assets/Use.cs", reason);
    }

    [Fact]
    public void Generic_body_change_executes_via_shim_and_redetours_kept_caller()
    {
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "GenE2ECovered",
            ("Assets/Lib.cs", GenericLibSource),
            ("Assets/Use.cs", GenericUseSource));
        JsonObject compileParams = ParamsFor(asmPath);

        // Echo's body changes; Use.cs joins the batch through an UNRELATED
        // edit — the calling method Go itself is untouched and must be
        // dragged into the detour set for the rewrite to take effect.
        string newLib = GenericLibSource.Replace(
            "{ return value; }",
            "{ var boosted = ((int)(object)value) + 100; return (T)(object)boosted; }");
        string newUse = GenericUseSource.Replace("public static int Other() { return 1; }", "public static int Other() { return 2; }");

        JsonNode result = HotPatch(
            service, compileParams,
            ("Assets/Lib.cs", GenericLibSource, newLib),
            ("Assets/Use.cs", GenericUseSource, newUse));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.Contains("verified", result["callerScan"]!.GetValue<string>());

        // Detours: the co-edited Other AND the kept caller Go; the generic
        // Echo itself never detours (shim-only).
        var methodNames = result["methods"]!.AsArray()
            .Select(m => m!["declaringType"]!.GetValue<string>() + "." + m["name"]!.GetValue<string>())
            .OrderBy(n => n)
            .ToArray();
        Assert.Equal(new[] { "GenE2E.Use.Go", "GenE2E.Use.Other" }, methodNames);

        byte[] originalBytes = File.ReadAllBytes(asmPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("generic-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "GenE2ECovered" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            // The kept caller's patch copy reaches the NEW generic body.
            Type patchUse = patch.GetType("GenE2E.Use__LocusPatch", throwOnError: true)!;
            Assert.Equal(105, patchUse.GetMethod("Go")!.Invoke(null, null));

            // The shim is a plain generic static method (direct-callable,
            // no detour) and carries the struct constraint.
            Type shims = patch.GetType("GenE2E.Lib__LocusShims", throwOnError: true)!;
            MethodInfo echo = shims.GetMethod("Echo")!;
            Assert.True(echo.IsGenericMethodDefinition);
            Assert.True(echo.GetGenericArguments()[0].GenericParameterAttributes
                .HasFlag(GenericParameterAttributes.NotNullableValueTypeConstraint));
            object lib = Activator.CreateInstance(original.GetType("GenE2E.Lib")!)!;
            Assert.Equal(107, echo.MakeGenericMethod(typeof(int)).Invoke(null, new[] { lib, (object)7 }));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Generic_type_method_body_change_executes_via_chain_shim()
    {
        const string source = @"
namespace GenE2E
{
    public class Box<T>
    {
        public int Mul(int k) { return k; }
    }
    public class BoxUser
    {
        public static int Drive() { return new Box<int>().Mul(3) + 0; }
    }
}";
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(service, "GenE2EBox", ("Assets/Box.cs", source));
        JsonObject compileParams = ParamsFor(asmPath);

        // Non-generic method in a generic TYPE: same remove+add path, the
        // shim's type parameter comes from the declaring chain and call
        // sites rely on inference from `self`. Drive is co-edited (a plain
        // changed method) so both redirect styles appear in one batch.
        string newText = source
            .Replace("public int Mul(int k) { return k; }", "public int Mul(int k) { return k * 50; }")
            .Replace("Mul(3) + 0", "Mul(3) + 1");

        JsonNode result = HotPatch(service, compileParams, ("Assets/Box.cs", source, newText));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        var detour = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("GenE2E.BoxUser", detour["declaringType"]!.GetValue<string>());
        Assert.Equal("Drive", detour["name"]!.GetValue<string>());

        byte[] originalBytes = File.ReadAllBytes(asmPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("generic-box-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "GenE2EBox" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchUser = patch.GetType("GenE2E.BoxUser__LocusPatch", throwOnError: true)!;
            Assert.Equal(151, patchUser.GetMethod("Drive")!.Invoke(null, null));

            Type shims = patch.GetType("GenE2E.Box__LocusShims", throwOnError: true)!;
            MethodInfo mul = shims.GetMethod("Mul")!;
            object box = Activator.CreateInstance(
                original.GetType("GenE2E.Box`1")!.MakeGenericType(typeof(int)))!;
            Assert.Equal(350, mul.MakeGenericMethod(typeof(int)).Invoke(null, new[] { box, (object)7 }));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Generic_kept_caller_of_readded_member_fails_closed()
    {
        const string useSource = @"
namespace GenE2E
{
    public class Use
    {
        public static int Relay<T>() { return new Lib().Echo(9); }
        public static int Other() { return 1; }
    }
}";
        var service = new CompileService();
        string asmPath = CompileProjectAssembly(
            service, "GenE2EGenericCaller",
            ("Assets/Lib.cs", GenericLibSource),
            ("Assets/Use.cs", useSource));
        JsonObject compileParams = ParamsFor(asmPath);

        // The only compiled call site of Echo sits inside a KEPT generic
        // method: its patch copy cannot be re-detoured, so the rewrite
        // fails the file closed naming the exact member.
        string newLib = GenericLibSource.Replace("{ return value; }", "{ return default(T); }");
        string newUse = useSource.Replace("public static int Other() { return 1; }", "public static int Other() { return 2; }");

        JsonNode result = HotPatch(
            service, compileParams,
            ("Assets/Lib.cs", GenericLibSource, newLib),
            ("Assets/Use.cs", useSource, newUse));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        Assert.Equal("Assets/Use.cs", file["path"]!.GetValue<string>());
        string reason = file["reasons"]!.AsArray().Single()!.GetValue<string>();
        Assert.Contains("GenE2E.Use.Relay", reason);
        Assert.Contains("cannot be re-detoured", reason);
        Assert.Contains("GenE2E.Lib.Echo", reason);
    }

    // ── added extension methods across cumulative batches ────────────

    private const string ExtHelperSource = @"
namespace ExtE2E
{
    public static class Helper
    {
        public static int Pick() { return 1; }
    }
}";

    private const string ExtSubjectSource = @"
namespace ExtE2E
{
    public class Subject
    {
        public int Probe() { return 0; }
    }
}";

    [Fact]
    public void Added_extension_method_survives_rebatch_with_session_image()
    {
        var service = new CompileService();
        string helperPath = CompileOriginal(service, "ExtRebatchHelper", ExtHelperSource);
        string subjectPath = CompileOriginal(service, "ExtRebatchSubject", ExtSubjectSource);
        JsonObject compileParams = ParamsFor(helperPath, subjectPath);
        compileParams["domainGeneration"] = "ext-rebatch-gen";

        string helperV1 = ExtHelperSource.Replace(
            "public static int Pick() { return 1; }",
            "public static int Pick() { return 1; }\n        public static int Tripled(this int v) { return v * 3; }");
        string subjectV1 = ExtSubjectSource.Replace(
            "public int Probe() { return 0; }",
            "public int Probe() { return 1500.Tripled() + 12; }");

        JsonNode Batch(string helperText, string subjectText)
        {
            var request = new JsonObject
            {
                ["files"] = new JsonArray(
                    new JsonObject { ["path"] = "Helper.cs", ["oldText"] = ExtHelperSource, ["newText"] = helperText },
                    new JsonObject { ["path"] = "Subject.cs", ["oldText"] = ExtSubjectSource, ["newText"] = subjectText }),
                ["params"] = compileParams.DeepClone(),
                ["registerImage"] = true, // inline accept: image + shim registry commit
            };
            return service.HandleCompileHotPatch(request);
        }

        JsonNode resultV1 = Batch(helperV1, subjectV1);
        Assert.True(resultV1["success"]!.GetValue<bool>(), resultV1["error"]?.GetValue<string>());
        string assemblyV1 = resultV1["assemblyName"]!.GetValue<string>();

        // Batch 2 re-sends the SAME files (cumulative coordinator batches)
        // plus a body tweak: extension lookup now sees the batch SOURCE shim
        // and batch 1's image shim — the call site must rewrite to a direct
        // call instead of failing CS0121-ambiguous.
        string helperV2 = helperV1.Replace("return v * 3;", "return v * 3 + 1;");
        JsonNode resultV2 = Batch(helperV2, subjectV1);
        Assert.True(resultV2["hot"]!.GetValue<bool>(), resultV2["files"]?.ToJsonString());
        Assert.True(resultV2["success"]!.GetValue<bool>(), resultV2["error"]?.GetValue<string>());

        // Probe re-detours; the re-edited shim detours old → new.
        var methodNames = resultV2["methods"]!.AsArray()
            .Select(m => m!["declaringType"]!.GetValue<string>() + "." + m["name"]!.GetValue<string>())
            .OrderBy(n => n)
            .ToArray();
        Assert.Equal(new[] { "ExtE2E.Helper__LocusShims.Tripled", "ExtE2E.Subject.Probe" }, methodNames);
        var shimDetour = resultV2["methods"]!.AsArray()
            .Single(m => m!["name"]!.GetValue<string>() == "Tripled")!;
        Assert.Equal(assemblyV1, shimDetour["originalAssembly"]!.GetValue<string>());

        byte[] helperBytes = File.ReadAllBytes(helperPath);
        byte[] subjectBytes = File.ReadAllBytes(subjectPath);
        byte[] patchBytes = Convert.FromBase64String(resultV2["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("ext-rebatch", isCollectible: true);
        try
        {
            Assembly helperAssembly = context.LoadFromStream(new MemoryStream(helperBytes));
            Assembly subjectAssembly = context.LoadFromStream(new MemoryStream(subjectBytes));
            context.Resolving += (_, name) => name.Name switch
            {
                "ExtRebatchHelper" => helperAssembly,
                "ExtRebatchSubject" => subjectAssembly,
                _ => null,
            };
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchSubject = patch.GetType("ExtE2E.Subject__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchSubject)!;
            Assert.Equal(1500 * 3 + 1 + 12, patchSubject.GetMethod("Probe")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Reduced_extension_on_class_receiver_folds_into_first_argument()
    {
        const string source = @"
namespace ExtE2E
{
    public class Subject
    {
        public int Tag = 40;
        public int Probe() { return 0; }
    }
    public static class Helper
    {
        public static int Pick() { return 1; }
    }
}";
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "ExtClassRecv", source);
        JsonObject compileParams = ParamsFor(originalPath);

        string newText = source
            .Replace(
                "public static int Pick() { return 1; }",
                "public static int Pick() { return 1; }\n        public static int Boost(this Subject s) { return s.Tag + 5000; }")
            .Replace(
                "public int Probe() { return 0; }",
                "public int Probe() { var s = new Subject(); return s.Boost(); }");

        JsonNode result = HotPatch(service, compileParams, ("Ext.cs", source, newText));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("ext-class-recv", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "ExtClassRecv" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            // The receiver folded into the shim's first argument and the
            // constructed instance is the ORIGINAL type.
            Type patchSubject = patch.GetType("ExtE2E.Subject__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchSubject)!;
            Assert.Equal(5040, patchSubject.GetMethod("Probe")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    // ── new NESTED types inside pre-existing (renamed) containers ────

    private const string NestedHostSource = @"
namespace NestE2E
{
    public class Host
    {
        public int Probe() { return 0; }
    }
}";

    [Fact]
    public void New_nested_type_reference_requalifies_to_patch_name()
    {
        var service = new CompileService();
        string originalPath = CompileOriginal(service, "NestedNew", NestedHostSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newText = NestedHostSource.Replace(
            "public int Probe() { return 0; }",
            "public int Probe() { return Inner2.Forty(); }\n        public class Inner2 { public static int Forty() { return 4554; } }");

        JsonNode result = HotPatch(service, compileParams, ("Host.cs", NestedHostSource, newText));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        // The new nested type's RUNTIME home is the renamed patch copy.
        var newType = Assert.Single(result["newTypes"]!.AsArray())!;
        Assert.Equal("NestE2E.Host__LocusPatch+Inner2", newType["metadataName"]!.GetValue<string>());
        Assert.False(newType["isTopLevel"]!.GetValue<bool>());

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("nested-new", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "NestedNew" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchHost = patch.GetType("NestE2E.Host__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchHost)!;
            Assert.Equal(4554, patchHost.GetMethod("Probe")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void New_nested_type_cross_file_reference_requalifies()
    {
        const string userSource = @"
namespace NestE2E
{
    public class User
    {
        public static int Go() { return 1; }
    }
}";
        var service = new CompileService();
        string hostPath = CompileOriginal(service, "NestedNewHost", NestedHostSource);
        string userPath = CompileOriginal(service, "NestedNewUser", userSource);
        JsonObject compileParams = ParamsFor(hostPath, userPath);

        string newHost = NestedHostSource.Replace(
            "public int Probe() { return 0; }",
            "public int Probe() { return 1; }\n        public class Inner2 { public static int Forty() { return 4554; } }");
        string newUser = userSource.Replace(
            "public static int Go() { return 1; }",
            "public static int Go() { return Host.Inner2.Forty(); }");

        JsonNode result = HotPatch(
            service, compileParams,
            ("Host.cs", NestedHostSource, newHost),
            ("User.cs", userSource, newUser));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] hostBytes = File.ReadAllBytes(hostPath);
        byte[] userBytes = File.ReadAllBytes(userPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("nested-new-cross", isCollectible: true);
        try
        {
            Assembly hostAssembly = context.LoadFromStream(new MemoryStream(hostBytes));
            Assembly userAssembly = context.LoadFromStream(new MemoryStream(userBytes));
            context.Resolving += (_, name) => name.Name switch
            {
                "NestedNewHost" => hostAssembly,
                "NestedNewUser" => userAssembly,
                _ => null,
            };
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchUser = patch.GetType("NestE2E.User__LocusPatch", throwOnError: true)!;
            Assert.Equal(4554, patchUser.GetMethod("Go")!.Invoke(null, null));
        }
        finally
        {
            context.Unload();
        }
    }

    // ── B4: unsafe bodies follow the project's allow-unsafe setting ──

    private const string UnsafeSource = @"
public class Cursor
{
    public unsafe int Read()
    {
        int x = 3;
        int* p = &x;
        return *p;
    }
}";

    /// <summary>The service's raw compile pins allowUnsafe:false (snippet
    /// parity), so unsafe originals compile directly through Roslyn.</summary>
    private string CompileUnsafeOriginal(string assemblyName, string text)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            new[] { CSharpSyntaxTree.ParseText(text, new CSharpParseOptions(LanguageVersion.CSharp9)) },
            HostBclPaths().Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, allowUnsafe: true));
        string path = Path.Combine(_tempDir, assemblyName + ".dll");
        Microsoft.CodeAnalysis.Emit.EmitResult emit = compilation.Emit(path);
        Assert.True(emit.Success, string.Join("\n", emit.Diagnostics));
        return path;
    }

    [Fact]
    public void Unsafe_body_edit_is_hot_when_params_allow_unsafe()
    {
        var service = new CompileService();
        string originalPath = CompileUnsafeOriginal("UnsafeHot", UnsafeSource);
        JsonObject compileParams = ParamsFor(originalPath);
        compileParams["allowUnsafe"] = true;

        string newText = UnsafeSource.Replace("return *p;", "return *p + 40;");
        JsonNode result = HotPatch(service, compileParams, ("Cursor.cs", UnsafeSource, newText));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        var method = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("Read", method["name"]!.GetValue<string>());

        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("unsafe-e2e", isCollectible: true);
        try
        {
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));
            Type patchType = patch.GetType("Cursor__LocusPatch", throwOnError: true)!;
            object cursor = Activator.CreateInstance(patchType)!;
            Assert.Equal(43, patchType.GetMethod("Read")!.Invoke(cursor, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Unsafe_body_edit_is_a_deterministic_diagnostic_without_allow_unsafe()
    {
        var service = new CompileService();
        string originalPath = CompileUnsafeOriginal("UnsafeCold", UnsafeSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newText = UnsafeSource.Replace("return *p;", "return *p + 40;");
        JsonNode result = HotPatch(service, compileParams, ("Cursor.cs", UnsafeSource, newText));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Contains("CS0227", result["error"]!.GetValue<string>());
    }

    /// <summary>Compile the REAL field-store runtime source (parity with the
    /// shipped Locus.HotReload.Runtime.dll) into a referenceable DLL.</summary>
    private string CompileFieldStoreRuntime(CompileService service)
    {
        string? dir = AppContext.BaseDirectory;
        string? sourcePath = null;
        for (int i = 0; i < 8 && dir != null; i++)
        {
            string candidate = Path.Combine(dir, "locus_hotreload_runtime", "LocusFieldStore.cs");
            if (File.Exists(candidate))
            {
                sourcePath = candidate;
                break;
            }
            dir = Path.GetDirectoryName(dir);
        }
        Assert.NotNull(sourcePath);

        var request = new JsonObject
        {
            ["assemblyName"] = "Locus.HotReload.Runtime",
            ["sources"] = new JsonArray(new JsonObject
            {
                ["path"] = "LocusFieldStore.cs",
                ["text"] = File.ReadAllText(sourcePath!),
            }),
            ["useHostBcl"] = true,
        };
        JsonNode result = service.HandleCompileRaw(request);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        string path = Path.Combine(_tempDir, "Locus.HotReload.Runtime.dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    private const string CounterSource = @"
namespace FieldE2E
{
    public class Counter
    {
        private int _seed = 3;
        public int Tick() { return _seed; }
    }
}";

    /// <summary>Store holders carry a batch-unique suffix
    /// ("__LocusFields_Counter_0000000A"): locate by prefix.</summary>
    private static Type? FindStoreType(Assembly assembly, string fullNamePrefix)
    {
        return assembly.GetTypes()
            .SingleOrDefault(t => t.FullName != null &&
                t.FullName.StartsWith(fullNamePrefix, StringComparison.Ordinal));
    }

    private static JsonNode HotPatchWithRuntime(
        CompileService service,
        JsonObject @params,
        string runtimePath,
        bool registerImage,
        params (string Path, string Old, string New)[] files)
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
            ["params"] = @params.DeepClone(),
            ["registerImage"] = registerImage,
            ["extraReferencePaths"] = new JsonArray(runtimePath),
        };
        return service.HandleCompileHotPatch(request);
    }

    [Fact]
    public void Added_field_virtualizes_through_the_store()
    {
        var service = new CompileService();
        string runtimePath = CompileFieldStoreRuntime(service);
        string originalPath = CompileOriginal(service, "FieldE2EOriginal", CounterSource);
        JsonObject compileParams = ParamsFor(originalPath);

        string newSource = CounterSource
            .Replace("private int _seed = 3;", "private int _seed = 3;\n        private int _count = 10;")
            .Replace("public int Tick() { return _seed; }",
                     "public int Tick() { _count += 1; return _seed + _count; }");

        JsonNode result = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: false,
            ("Counter.cs", CounterSource, newSource));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        // Tick redirects; the implicit ctor redirects (initializer).
        var methodNames = result["methods"]!.AsArray()
            .Select(m => m!["name"]!.GetValue<string>())
            .OrderBy(n => n, StringComparer.Ordinal)
            .ToArray();
        Assert.Equal(new[] { ".ctor", "Tick" }, methodNames);

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] runtimeBytes = File.ReadAllBytes(runtimePath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());

        var context = new AssemblyLoadContext("field-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            Assembly runtime = context.LoadFromStream(new MemoryStream(runtimeBytes));
            context.Resolving += (_, name) => name.Name switch
            {
                "FieldE2EOriginal" => original,
                "Locus.HotReload.Runtime" => runtime,
                _ => null,
            };
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            // The patch type's REAL layout matches the original exactly:
            // the added field is store-virtualized, not declared.
            Type patchCounter = patch.GetType("FieldE2E.Counter__LocusPatch", throwOnError: true)!;
            Assert.Null(patchCounter.GetField("_count", BindingFlags.NonPublic | BindingFlags.Instance));
            Assert.NotNull(patchCounter.GetField("_seed", BindingFlags.NonPublic | BindingFlags.Instance));

            // New instance: ctor writes the initializer through the store.
            object instance = Activator.CreateInstance(patchCounter)!;
            Assert.Equal(3 + 11, patchCounter.GetMethod("Tick")!.Invoke(instance, null));
            Assert.Equal(3 + 12, patchCounter.GetMethod("Tick")!.Invoke(instance, null));

            // Pre-existing instances the store never saw read default(T).
            Type storeHolder = FindStoreType(patch, "FieldE2E.__LocusFields_Counter")!;
            Assert.NotNull(storeHolder);
            object store = storeHolder.GetField("_count")!.GetValue(null)!;
            object preExisting = Activator.CreateInstance(original.GetType("FieldE2E.Counter")!)!;
            object? value = store.GetType().GetMethod("Ref")!.Invoke(store, new[] { preExisting });
            Assert.Equal(0, value);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Removed_field_keeps_a_layout_placeholder()
    {
        var service = new CompileService();
        string runtimePath = CompileFieldStoreRuntime(service);
        const string source = @"
namespace FieldE2E
{
    public class Holder
    {
        private int _a = 1;
        private int _b = 2;
        public int Sum() { return _a + _b; }
    }
}";
        string originalPath = CompileOriginal(service, "FieldE2ERemoval", source);
        JsonObject compileParams = ParamsFor(originalPath);

        string newSource = source
            .Replace("private int _b = 2;\n", "")
            .Replace("public int Sum() { return _a + _b; }", "public int Sum() { return _a; }");

        JsonNode result = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: false,
            ("Holder.cs", source, newSource));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        var context = new AssemblyLoadContext("field-removal-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "FieldE2ERemoval" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            // The removed field stays as a placeholder: identical layout.
            Type patchHolder = patch.GetType("FieldE2E.Holder__LocusPatch", throwOnError: true)!;
            var fields = patchHolder.GetFields(BindingFlags.NonPublic | BindingFlags.Instance)
                .Select(f => f.Name)
                .ToArray();
            Assert.Equal(new[] { "_a", "_b" }, fields);

            object instance = Activator.CreateInstance(patchHolder)!;
            Assert.Equal(1, patchHolder.GetMethod("Sum")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Reedited_field_binds_to_the_first_batch_store()
    {
        var service = new CompileService();
        string runtimePath = CompileFieldStoreRuntime(service);
        string originalPath = CompileOriginal(service, "FieldE2EReuse", CounterSource);
        JsonObject compileParams = ParamsFor(originalPath);
        compileParams["domainGeneration"] = "field-reuse-gen";

        string v1 = CounterSource
            .Replace("private int _seed = 3;", "private int _seed = 3;\n        private int _count = 10;")
            .Replace("return _seed;", "return _seed + _count;");
        JsonNode result1 = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: true,
            ("Counter.cs", CounterSource, v1));
        Assert.True(result1["success"]!.GetValue<bool>(), result1["error"]?.GetValue<string>());

        string v2 = CounterSource
            .Replace("private int _seed = 3;", "private int _seed = 3;\n        private int _count = 10;")
            .Replace("return _seed;", "return _seed + _count * 2;");
        JsonNode result2 = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: true,
            ("Counter.cs", CounterSource, v2));
        Assert.True(result2["success"]!.GetValue<bool>(), result2["error"]?.GetValue<string>());

        // The second patch binds to the FIRST batch's store instead of
        // declaring its own (values must not split).
        byte[] patch2Bytes = Convert.FromBase64String(result2["assemblyB64"]!.GetValue<string>());
        byte[] patch1Bytes = Convert.FromBase64String(result1["assemblyB64"]!.GetValue<string>());
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] runtimeBytes = File.ReadAllBytes(runtimePath);
        var context = new AssemblyLoadContext("field-reuse-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            Assembly runtime = context.LoadFromStream(new MemoryStream(runtimeBytes));
            Assembly patch1 = context.LoadFromStream(new MemoryStream(patch1Bytes));
            context.Resolving += (_, name) =>
            {
                if (name.Name == "FieldE2EReuse")
                    return original;
                if (name.Name == "Locus.HotReload.Runtime")
                    return runtime;
                if (name.Name == patch1.GetName().Name)
                    return patch1;
                return null;
            };
            Assembly patch2 = context.LoadFromStream(new MemoryStream(patch2Bytes));

            Assert.NotNull(FindStoreType(patch1, "FieldE2E.__LocusFields_Counter"));
            Assert.Null(FindStoreType(patch2, "FieldE2E.__LocusFields_Counter"));

            // Write through patch1's path, read through patch2's body.
            Type patch1Counter = patch1.GetType("FieldE2E.Counter__LocusPatch", throwOnError: true)!;
            Type patch2Counter = patch2.GetType("FieldE2E.Counter__LocusPatch", throwOnError: true)!;
            object store = FindStoreType(patch1, "FieldE2E.__LocusFields_Counter")!
                .GetField("_count")!.GetValue(null)!;

            object instance2 = Activator.CreateInstance(patch2Counter)!;
            // patch2 Tick: _seed(3) + _count(10 via shared store) * 2 = 23.
            Assert.Equal(23, patch2Counter.GetMethod("Tick")!.Invoke(instance2, null));
            _ = patch1Counter;
            _ = store;
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Second_added_field_on_the_same_type_declares_a_distinct_store()
    {
        // Regression: batch 2 adding ANOTHER field to the same type used to
        // declare a holder with the SAME name as batch 1's — the source
        // declaration shadowed (CS0436) the earlier holder that the re-sent
        // first field still binds to, failing with CS0117 on that field.
        var service = new CompileService();
        string runtimePath = CompileFieldStoreRuntime(service);
        string originalPath = CompileOriginal(service, "FieldE2ESecond", CounterSource);
        JsonObject compileParams = ParamsFor(originalPath);
        compileParams["domainGeneration"] = "field-second-gen";

        string v1 = CounterSource
            .Replace("private int _seed = 3;", "private int _seed = 3;\n        private int _count = 10;")
            .Replace("return _seed;", "return _seed + _count;");
        JsonNode result1 = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: true,
            ("Counter.cs", CounterSource, v1));
        Assert.True(result1["success"]!.GetValue<bool>(), result1["error"]?.GetValue<string>());

        string v2 = v1
            .Replace("private int _count = 10;", "private int _count = 10;\n        private static int s_total = 6600;")
            .Replace("return _seed + _count;", "s_total += 1; return _seed + _count + s_total;");
        JsonNode result2 = HotPatchWithRuntime(
            service, compileParams, runtimePath, registerImage: true,
            ("Counter.cs", CounterSource, v2));
        Assert.True(result2["success"]!.GetValue<bool>(), result2["error"]?.GetValue<string>());

        byte[] patch1Bytes = Convert.FromBase64String(result1["assemblyB64"]!.GetValue<string>());
        byte[] patch2Bytes = Convert.FromBase64String(result2["assemblyB64"]!.GetValue<string>());
        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] runtimeBytes = File.ReadAllBytes(runtimePath);
        var context = new AssemblyLoadContext("field-second-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            Assembly runtime = context.LoadFromStream(new MemoryStream(runtimeBytes));
            Assembly patch1 = context.LoadFromStream(new MemoryStream(patch1Bytes));
            context.Resolving += (_, name) =>
            {
                if (name.Name == "FieldE2ESecond")
                    return original;
                if (name.Name == "Locus.HotReload.Runtime")
                    return runtime;
                if (name.Name == patch1.GetName().Name)
                    return patch1;
                return null;
            };
            Assembly patch2 = context.LoadFromStream(new MemoryStream(patch2Bytes));

            // Distinct holder names: batch 2's own holder must not shadow
            // batch 1's, and `_count` lives ONLY in batch 1's.
            Type store1 = FindStoreType(patch1, "FieldE2E.__LocusFields_Counter")!;
            Type store2 = FindStoreType(patch2, "FieldE2E.__LocusFields_Counter")!;
            Assert.NotNull(store1);
            Assert.NotNull(store2);
            Assert.NotEqual(store1.FullName, store2.FullName);
            Assert.NotNull(store1.GetField("_count"));
            Assert.Null(store2.GetField("_count"));
            Assert.NotNull(store2.GetField("s_total"));

            // Execution: new instance through patch 2 — _seed(3) +
            // _count(10, batch 1's store) + s_total(6601 after the bump).
            Type patch2Counter = patch2.GetType("FieldE2E.Counter__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patch2Counter)!;
            Assert.Equal(3 + 10 + 6601, patch2Counter.GetMethod("Tick")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Removed_unity_message_method_detours_to_an_empty_stub()
    {
        var service = new CompileService();
        const string playerSource = @"
namespace StubE2E
{
    public class Player
    {
        private int _ticks;
        public void Update() { _ticks += 1; }
        public int Ticks() { return _ticks; }
    }
}";
        string asmPath = CompileProjectAssembly(service, "StubE2EPlayer", ("Assets/Player.cs", playerSource));
        JsonObject compileParams = ParamsFor(asmPath);

        string removed = playerSource.Replace("public void Update() { _ticks += 1; }", "");
        JsonNode result = HotPatch(service, compileParams, ("Assets/Player.cs", playerSource, removed));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        var stub = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("StubE2E.Player", stub["declaringType"]!.GetValue<string>());
        Assert.Equal("StubE2E.Player__LocusPatch", stub["patchDeclaringType"]!.GetValue<string>());
        Assert.Equal("Update", stub["name"]!.GetValue<string>());
        Assert.True(stub["isStub"]!.GetValue<bool>());

        // The stub is genuinely empty: invoking it must not touch state.
        byte[] originalBytes = File.ReadAllBytes(asmPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("stub-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "StubE2EPlayer" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchPlayer = patch.GetType("StubE2E.Player__LocusPatch", throwOnError: true)!;
            object instance = Activator.CreateInstance(patchPlayer)!;
            patchPlayer.GetMethod("Update")!.Invoke(instance, null);
            Assert.Equal(0, patchPlayer.GetMethod("Ticks")!.Invoke(instance, null));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Cold_change_reports_hot_false_with_reasons()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        // Field REORDER stays cold (only add/remove/retype virtualizes).
        const string oldText = "class A { int _a; int _b; void M() { } }";
        const string newText = "class A { int _b; int _a; void M() { } }";

        JsonNode result = HotPatch(service, compileParams, ("A.cs", oldText, newText));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        Assert.Equal("A.cs", file["path"]!.GetValue<string>());
        Assert.Contains(
            file["reasons"]!.AsArray(),
            r => r!.GetValue<string>().Contains("field layout changed"));
    }

    [Fact]
    public void Stale_baseline_field_layout_falls_cold()
    {
        var service = new CompileService();
        // The "original" assembly has an extra field the baseline text lacks
        // (the file changed outside this session).
        string originalPath = CompileOriginal(
            service,
            "HotPatchStale",
            "namespace Stale { public class S { private int _a; private int _extra; public int M() { return _a; } } }");
        JsonObject compileParams = ParamsFor(originalPath);

        const string oldText = "namespace Stale { public class S { private int _a; public int M() { return _a; } } }";
        const string newText = "namespace Stale { public class S { private int _a; public int M() { return _a + 1; } } }";

        JsonNode result = HotPatch(service, compileParams, ("S.cs", oldText, newText));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        Assert.Contains(
            file["reasons"]!.AsArray(),
            r => r!.GetValue<string>().Contains("field layout differs"));
    }

    [Fact]
    public void Comment_only_edit_is_a_noop()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        const string oldText = "class A { void M() { } }";
        const string newText = "class A { void M() { /* tick */ } }";

        JsonNode result = HotPatch(service, compileParams, ("A.cs", oldText, newText));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>());
        Assert.True(result["noop"]!.GetValue<bool>());
    }

    [Fact]
    public void Appended_enum_member_materializes_as_cast_literal()
    {
        var service = new CompileService();
        const string source = @"
namespace EnumE2E
{
    public enum Mode { Idle = 0, Run = 1 }
    public class Driver
    {
        public int Decide(Mode mode)
        {
            switch (mode)
            {
                case Mode.Run: return 10;
                default: return 0;
            }
        }
    }
}";
        string originalPath = CompileOriginal(service, "EnumE2EOriginal", source);
        JsonObject compileParams = ParamsFor(originalPath);

        string newSource = source
            .Replace("public enum Mode { Idle = 0, Run = 1 }", "public enum Mode { Idle = 0, Run = 1, Fly = 7 }")
            .Replace("case Mode.Run: return 10;", "case Mode.Run: return 10;\n                case Mode.Fly: return 77;");

        JsonNode result = HotPatch(service, compileParams, ("Mode.cs", source, newSource));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] originalBytes = File.ReadAllBytes(originalPath);
        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("enum-e2e", isCollectible: true);
        try
        {
            Assembly original = context.LoadFromStream(new MemoryStream(originalBytes));
            context.Resolving += (_, name) => name.Name == "EnumE2EOriginal" ? original : null;
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));

            Type patchDriver = patch.GetType("EnumE2E.Driver__LocusPatch", throwOnError: true)!;
            object driver = Activator.CreateInstance(patchDriver)!;
            Type originalMode = original.GetType("EnumE2E.Mode")!;

            // The new member's VALUE routes through the patched switch even
            // though the ORIGINAL enum type has no such member.
            object fly = Enum.ToObject(originalMode, 7);
            Assert.Equal(77, patchDriver.GetMethod("Decide")!.Invoke(driver, new[] { fly }));
            object run = Enum.ToObject(originalMode, 1);
            Assert.Equal(10, patchDriver.GetMethod("Decide")!.Invoke(driver, new[] { run }));
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Deleted_file_produces_stub_class_for_magic_methods()
    {
        var service = new CompileService();
        const string source = @"
namespace DeleteE2E
{
    public class Spinner
    {
        private int _angle;
        public void Update() { _angle += 1; }
        public int Angle() { return _angle; }
    }
}";
        string asmPath = CompileProjectAssembly(service, "DeleteE2ESpinner", ("Assets/Spinner.cs", source));
        JsonObject compileParams = ParamsFor(asmPath);

        // Whole-file deletion: newText is empty.
        JsonNode result = HotPatch(service, compileParams, ("Assets/Spinner.cs", source, ""));

        Assert.True(result["hot"]!.GetValue<bool>(), result["files"]?.ToJsonString());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        var stub = Assert.Single(result["methods"]!.AsArray())!;
        Assert.Equal("DeleteE2E.Spinner", stub["declaringType"]!.GetValue<string>());
        Assert.Equal("DeleteE2E.Spinner__LocusStub", stub["patchDeclaringType"]!.GetValue<string>());
        Assert.Equal("Update", stub["name"]!.GetValue<string>());
        Assert.True(stub["isStub"]!.GetValue<bool>());

        byte[] patchBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        var context = new AssemblyLoadContext("delete-e2e", isCollectible: true);
        try
        {
            Assembly patch = context.LoadFromStream(new MemoryStream(patchBytes));
            Type stubType = patch.GetType("DeleteE2E.Spinner__LocusStub", throwOnError: true)!;
            object instance = Activator.CreateInstance(stubType)!;
            stubType.GetMethod("Update", BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance)!
                .Invoke(instance, null);
        }
        finally
        {
            context.Unload();
        }
    }

    [Fact]
    public void Deleted_type_with_uncovered_references_is_cold()
    {
        var service = new CompileService();
        const string libSource = @"
namespace DeleteScanE2E
{
    public class Tool
    {
        public int Use() { return 1; }
    }
}";
        const string userSource = @"
namespace DeleteScanE2E
{
    public class Workshop
    {
        public int Work() { return new Tool().Use(); }
    }
}";
        string asmPath = CompileProjectAssembly(
            service, "DeleteScanE2E",
            ("Assets/Tool.cs", libSource),
            ("Assets/Workshop.cs", userSource));
        JsonObject compileParams = ParamsFor(asmPath);

        JsonNode result = HotPatch(service, compileParams, ("Assets/Tool.cs", libSource, ""));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        string reason = file["reasons"]!.AsArray().Single()!.GetValue<string>();
        Assert.Contains("Assets/Workshop.cs", reason);
    }

    [Fact]
    public void Syntax_error_in_new_text_is_a_deterministic_compile_failure()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        JsonNode result = HotPatch(
            service, compileParams,
            ("A.cs", "class A { void M() { } }", "class A { void M() { int x = ; } }"));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.False(result["success"]!.GetValue<bool>());
        Assert.StartsWith("compilation failed:", result["error"]!.GetValue<string>());
    }

    [Fact]
    public void Semantic_error_in_new_text_is_a_deterministic_compile_failure()
    {
        var service = new CompileService();
        const string oldText = "class A { void M() { } }";
        string originalPath = CompileOriginal(service, "HotPatchSemErr", oldText);
        JsonObject compileParams = ParamsFor(originalPath);

        JsonNode result = HotPatch(
            service, compileParams,
            ("A.cs", oldText, "class A { void M() { UndefinedSymbol(); } }"));

        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Contains("CS0103", result["error"]!.GetValue<string>());
    }

    [Fact]
    public void Missing_original_assembly_falls_cold()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        JsonNode result = HotPatch(
            service, compileParams,
            ("A.cs", "class A { void M() { } }", "class A { void M() { int x = 1; } }"));

        Assert.False(result["hot"]!.GetValue<bool>());
        var file = Assert.Single(result["files"]!.AsArray())!;
        Assert.Contains(
            file["reasons"]!.AsArray(),
            r => r!.GetValue<string>().Contains("original type not found"));
    }
}

/// <summary>
/// Golden tests for the patch source rewriter: rename, reference
/// requalification, static access rewrite and static initializer/cctor
/// suppression are all verbatim-pinned.
/// </summary>
public class PatchRewriterGoldenTests : IDisposable
{
    private readonly string _tempDir;

    public PatchRewriterGoldenTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-rewriter-golden-" + Guid.NewGuid().ToString("N"));
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

    private static readonly CSharpParseOptions ParseOptions = new(languageVersion: LanguageVersion.CSharp9);

    private (PatchRewriteResult Result, string Text) RewriteWithOriginal(string assemblyName, string oldText, string newText)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            new[] { CSharpSyntaxTree.ParseText(oldText, ParseOptions) },
            ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
                .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
                .Where(File.Exists)
                .Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        string originalPath = Path.Combine(_tempDir, assemblyName + ".dll");
        var emit = compilation.Emit(originalPath);
        Assert.True(emit.Success, string.Join("\n", emit.Diagnostics));

        var references = compilation.References
            .Append(MetadataReference.CreateFromFile(originalPath))
            .ToArray();

        HotDiffFileResult diff = HotDiff.Analyze(oldText, newText, ParseOptions);
        Assert.True(diff.Hot, string.Join("; ", diff.Reasons));

        PatchRewriteResult result = PatchRewriter.Rewrite(
            "Golden.cs", newText, diff,
            ParseOptions,
            System.Collections.Immutable.ImmutableArray.CreateRange(references));
        Assert.Null(result.ColdReason);
        return (result, result.Tree!.ToString());
    }

    [Fact]
    public void Rewrite_is_verbatim_stable()
    {
        const string oldText = @"namespace Game
{
    public class Player
    {
        public static int Score = 5;
        static Player() { Score = 9; }
        private int _hp = 3;
        public int Tick() { return _hp; }
        public Player Clone() { return new Player(); }
        public class Buff { public int Power; }
    }
}";
        string newText = oldText.Replace(
            "public int Tick() { return _hp; }",
            "public int Tick() { Score += 1; return _hp + Player.Score + new Buff().Power; }");

        var (result, text) = RewriteWithOriginal("GoldenPlayer", oldText, newText);

        const string expected = @"namespace Game
{
    public class Player__LocusPatch
    {
        public static int Score;
        static Player__LocusPatch() {}
        private int _hp = 3;
        public int Tick() { global::Game.Player.Score += 1; return _hp + global::Game.Player.Score + new global::Game.Player.Buff().Power; }
        public global::Game.Player Clone() { return new global::Game.Player(); }
        public class Buff { public int Power; }
    }
}";
        Assert.Equal(expected.ReplaceLineEndings("\n"), text.ReplaceLineEndings("\n"));

        var method = Assert.Single(result.Methods);
        Assert.Equal("Game.Player", method.DeclaringType);
        Assert.Equal("Game.Player__LocusPatch", method.PatchDeclaringType);
        Assert.Equal("Tick", method.Name);
        Assert.Equal(new[] { "GoldenPlayer" }, result.OriginalAssemblies);
    }

    [Fact]
    public void Added_member_shim_rewrite_is_verbatim_stable()
    {
        const string oldText = @"namespace Game
{
    public class Player
    {
        public int Mp = 3;
        public int Tick() { return 1; }
    }
}";
        string newText = oldText
            .Replace(
                "public int Tick() { return 1; }",
                "public int Tick() { return Mana(); }\n        public int Mana() { return Mp; }");

        var (result, text) = RewriteWithOriginal("GoldenShim", oldText, newText);

        const string expected = @"namespace Game
{
    public class Player__LocusPatch
    {
        public int Mp = 3;
        public int Tick() { return global::Game.Player__LocusShims.Mana(((global::Game.Player)(object)this)); }
    }

public static class Player__LocusShims
{
    public static int Mana(this global::Game.Player self)
    {
        return self.Mp;
    }
}}";
        Assert.Equal(expected.ReplaceLineEndings("\n"), text.ReplaceLineEndings("\n"));

        var method = Assert.Single(result.Methods);
        Assert.Equal("Tick", method.Name);
        var registration = Assert.Single(result.ShimRegistrations);
        Assert.Equal("Game.Player__LocusShims", registration.Entry.ShimTypeMetadataName);
        Assert.Equal("Mana", registration.Entry.ShimMethod);
        Assert.True(registration.Entry.HasSelf);
    }

    [Fact]
    public void Generic_body_change_rewrite_is_verbatim_stable()
    {
        // B1: the generic body re-materializes as a generic static shim;
        // the KEPT caller's call site becomes a direct call with explicit
        // type arguments and the caller joins the detour set.
        const string oldText = @"namespace Game
{
    public class Player
    {
        public int Tick() { return Echo(7); }
        public T Echo<T>(T value) { return value; }
    }
}";
        string newText = oldText.Replace(
            "public T Echo<T>(T value) { return value; }",
            "public T Echo<T>(T value) { return default(T); }");

        var (result, text) = RewriteWithOriginal("GoldenGenericShim", oldText, newText);

        const string expected = @"namespace Game
{
    public class Player__LocusPatch
    {
        public int Tick() { return global::Game.Player__LocusShims.Echo<int>(((global::Game.Player)(object)this),7); }
    }

public static class Player__LocusShims
{
    public static T Echo<T>(this global::Game.Player self, T value)
    {
        return default(T);
    }
}}";
        Assert.Equal(expected.ReplaceLineEndings("\n"), text.ReplaceLineEndings("\n"));

        // Tick is a KEPT member dragged into the detour set (its call site
        // rewrote to the shim); the generic Echo itself never detours.
        var method = Assert.Single(result.Methods);
        Assert.Equal("Tick", method.Name);
        Assert.Equal("Game.Player", method.DeclaringType);
        Assert.Equal("Game.Player__LocusPatch", method.PatchDeclaringType);

        // The re-add registers the live shim; the same-key tombstone is
        // suppressed (it would overwrite the entry in the registry).
        var registration = Assert.Single(result.ShimRegistrations);
        Assert.Equal("added", registration.Entry.Kind);
        Assert.True(registration.Entry.GenericShim);
        Assert.Equal("Echo", registration.Entry.ShimMethod);
    }

    [Fact]
    public void Reduced_extension_this_receiver_rewrite_is_verbatim_stable()
    {
        // `this.Boost()` in a KEPT member: the reduced receiver folds into
        // the static shim's first argument, cast to the extension's
        // this-parameter type (the runtime object is an original instance;
        // only the static type in the patch copy differs).
        const string oldText = @"namespace Game
{
    public class Player
    {
        public int Hp = 7;
        public int Tick() { return this.Hp; }
    }
    public static class PlayerOps
    {
        public static int Noop() { return 0; }
    }
}";
        string newText = oldText
            .Replace(
                "public int Tick() { return this.Hp; }",
                "public int Tick() { return this.Boost(); }")
            .Replace(
                "public static int Noop() { return 0; }",
                "public static int Noop() { return 0; }\n        public static int Boost(this Player p) { return p.Hp + 9000; }");

        var (result, text) = RewriteWithOriginal("GoldenReducedThis", oldText, newText);

        Assert.Contains(
            "public int Tick() { return global::Game.PlayerOps__LocusShims.Boost(((global::Game.Player)(object)this)); }",
            text.ReplaceLineEndings("\n"));
        var method = Assert.Single(result.Methods);
        Assert.Equal("Tick", method.Name);
    }

    private string RewriteExpectingCold(string assemblyName, string oldText, string newText)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            new[] { CSharpSyntaxTree.ParseText(oldText, ParseOptions) },
            ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
                .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
                .Where(File.Exists)
                .Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        string originalPath = Path.Combine(_tempDir, assemblyName + ".dll");
        var emit = compilation.Emit(originalPath);
        Assert.True(emit.Success, string.Join("\n", emit.Diagnostics));

        var references = compilation.References
            .Append(MetadataReference.CreateFromFile(originalPath))
            .ToArray();

        HotDiffFileResult diff = HotDiff.Analyze(oldText, newText, ParseOptions);
        Assert.True(diff.Hot, string.Join("; ", diff.Reasons));

        PatchRewriteResult result = PatchRewriter.Rewrite(
            "Cold.cs", newText, diff,
            ParseOptions,
            System.Collections.Immutable.ImmutableArray.CreateRange(references));
        Assert.NotNull(result.ColdReason);
        return result.ColdReason!;
    }

    [Fact]
    public void Added_member_touching_private_state_is_cold()
    {
        // Mono enforces accessibility at JIT time and ignores
        // IgnoresAccessChecksTo for project assemblies: the shim would throw
        // FieldAccessException at its first call.
        const string oldText = @"namespace Game
{
    public class Player
    {
        private int _mp = 3;
        public int Tick() { return _mp; }
    }
}";
        string newText = oldText.Replace(
            "public int Tick() { return _mp; }",
            "public int Tick() { return _mp; }\n        public int Mana() { return _mp; }");

        string reason = RewriteExpectingCold("ColdShimPrivate", oldText, newText);
        Assert.Contains("added member references non-public surface", reason);
        Assert.Contains("_mp", reason);
    }

    [Fact]
    public void Added_member_calling_private_method_is_cold()
    {
        const string oldText = @"namespace Game
{
    public class Player
    {
        private int Hidden() { return 3; }
        public int Tick() { return Hidden(); }
    }
}";
        string newText = oldText.Replace(
            "public int Tick() { return Hidden(); }",
            "public int Tick() { return Hidden(); }\n        public int Mana() { return Hidden(); }");

        string reason = RewriteExpectingCold("ColdShimPrivateMethod", oldText, newText);
        Assert.Contains("added member references non-public surface", reason);
        Assert.Contains("Hidden", reason);
    }

    [Fact]
    public void Added_instance_member_on_internal_type_is_cold()
    {
        // The (public) shim could not even NAME the internal declaring type
        // across assemblies on the Unity runtime.
        const string oldText = @"namespace Game
{
    class Helper
    {
        public int Tick() { return 1; }
    }
}";
        string newText = oldText.Replace(
            "public int Tick() { return 1; }",
            "public int Tick() { return 1; }\n        public int Mana() { return 2; }");

        string reason = RewriteExpectingCold("ColdShimInternalType", oldText, newText);
        Assert.Contains("non-public type", reason);
    }

    [Fact]
    public void Conversion_to_the_declaring_type_change_is_cold()
    {
        const string oldText = @"
public struct Wrap
{
    public int Value;
    public static implicit operator Wrap(int v) { var r = new Wrap(); r.Value = v; return r; }
}";
        string newText = oldText.Replace("r.Value = v;", "r.Value = v + 1;");

        HotDiffFileResult diff = HotDiff.Analyze(oldText, newText, ParseOptions);
        Assert.False(diff.Hot);
        Assert.Contains(diff.Reasons, r => r.Contains("conversion to the declaring type changed"));
    }

    [Fact]
    public void Added_member_chain_to_other_added_member_stays_hot()
    {
        // Added→added calls route shim-to-shim inside the patch assembly:
        // no original-surface access, so accessibility stays irrelevant.
        const string oldText = @"namespace Game
{
    public class Player
    {
        public int Tick() { return 1; }
    }
}";
        string newText = oldText.Replace(
            "public int Tick() { return 1; }",
            "public int Tick() { return 1; }\n        public int Boost() { return BoostCore() + 7000; }\n        public int BoostCore() { return 707; }");

        var (result, _) = RewriteWithOriginal("HotShimChain", oldText, newText);
        Assert.Equal(2, result.ShimRegistrations.Count);
    }

    [Fact]
    public void Nested_type_member_maps_through_renamed_outer()
    {
        const string oldText = @"namespace Game
{
    public class Outer
    {
        public class Inner
        {
            public int M() { return 1; }
        }
    }
}";
        string newText = oldText.Replace("return 1;", "return 2;");

        var (result, _) = RewriteWithOriginal("GoldenNested", oldText, newText);

        var method = Assert.Single(result.Methods);
        Assert.Equal("Game.Outer+Inner", method.DeclaringType);
        Assert.Equal("Game.Outer__LocusPatch+Inner", method.PatchDeclaringType);
    }
}
