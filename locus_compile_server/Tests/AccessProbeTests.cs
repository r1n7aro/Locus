using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Runtime.Loader;
using System.Text.Json.Nodes;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// C0 access probe (compile/accessProbe): locks the probe's OWN correctness
/// on CoreCLR. .NET honors IgnoresAccessChecksTo and restrictedSkipVisibility,
/// so every cell must JIT and all three emit primitives must succeed here —
/// any red on this runtime is a probe bug, not a capability signal. The Mono
/// truth only ever comes from the in-editor run (hot_reload_access_probe).
/// </summary>
public class AccessProbeTests : IDisposable
{
    private readonly string _tempDir;

    public AccessProbeTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-accessprobe-tests-" + Guid.NewGuid().ToString("N"));
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
            ["fingerprint"] = "accessprobe-test-" + Guid.NewGuid().ToString("N"),
            ["domainGeneration"] = Guid.NewGuid().ToString("N"),
            ["langVersion"] = "9",
            ["referencePaths"] = new JsonArray(paths.Select(p => (JsonNode)p).ToArray()),
            ["defines"] = new JsonArray(),
        };
    }

    /// <summary>CoreCLR mirror of the Unity plugin's probe target
    /// (LocusBridge.AccessProbe.cs). The probe source binds these members BY
    /// NAME — keep names, shapes and seed values in sync with the plugin.</summary>
    private const string TargetSource = @"
namespace Locus
{
#pragma warning disable 0414
    internal sealed class LocusAccessProbeTarget
    {
        private int _privInst = 7;
        internal int _intInst = 11;
        private static int _privStatic = 13;
        internal static int _intStatic = 17;

        private LocusAccessProbeTarget(int seed) { _privInst = seed; }
        internal LocusAccessProbeTarget() { }

        private int PrivMethod(int x) { return x * 2 + 1; }
        internal int IntMethod(int x) { return x * 3 + 1; }
        private static int PrivStatic(int x) { return x * 5 + 1; }
        internal static int IntStatic(int x) { return x * 7 + 1; }

        public static LocusAccessProbeTarget New() { return new LocusAccessProbeTarget(); }

        public int ReadPrivInst() { return _privInst; }

        private sealed class PrivNested { }
    }
#pragma warning restore 0414
}";

    private const string TargetAssemblyName = "LocusAccessProbeTargetRef";

    private JsonNode CompileProbe(CompileService service, out string targetPath)
    {
        targetPath = CompileOriginal(service, TargetAssemblyName, TargetSource);
        var request = new JsonObject { ["params"] = ParamsFor(targetPath) };
        return service.HandleCompileAccessProbe(request);
    }

    [Fact]
    public void Probe_source_compiles_with_full_cell_manifest()
    {
        var service = new CompileService();
        JsonNode result = CompileProbe(service, out _);

        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.StartsWith("__LocusAccessProbe_", result["assemblyName"]!.GetValue<string>());

        var cells = result["cells"]!.AsArray();
        Assert.Equal(AccessProbeSource.Cells.Count, cells.Count);
        Assert.Equal(18, cells.Count);

        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (JsonNode? cell in cells)
        {
            string op = cell!["op"]!.GetValue<string>();
            string visibility = cell["visibility"]!.GetValue<string>();
            Assert.Equal("Cell_" + op + "_" + visibility, cell["method"]!.GetValue<string>());
            Assert.True(seen.Add(op + "|" + visibility), "duplicate cell " + op + "|" + visibility);
        }
        foreach (string op in new[] { "ldfld", "stfld", "ldsfld", "stsfld", "call", "callvirt", "newobj", "castclass", "ldtoken" })
        {
            Assert.Contains(op + "|private", seen);
            Assert.Contains(op + "|internal", seen);
        }
    }

    [Fact]
    public void Every_cell_jits_on_coreclr()
    {
        var service = new CompileService();
        JsonNode result = CompileProbe(service, out string targetPath);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        byte[] targetBytes = File.ReadAllBytes(targetPath);
        byte[] probeBytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());

        var context = new AssemblyLoadContext("access-probe-cells", isCollectible: true);
        try
        {
            Assembly target = context.LoadFromStream(new MemoryStream(targetBytes));
            context.Resolving += (_, name) =>
                name.Name == TargetAssemblyName ? target : null;
            Assembly probe = context.LoadFromStream(new MemoryStream(probeBytes));

            Type probeType = probe.GetType(AccessProbeSource.ProbeTypeName, throwOnError: true)!;
            foreach (JsonNode? cell in result["cells"]!.AsArray())
            {
                string methodName = cell!["method"]!.GetValue<string>();
                MethodInfo? method = probeType.GetMethod(methodName, BindingFlags.Public | BindingFlags.Static);
                Assert.True(method != null, "probe method missing: " + methodName);

                // CoreCLR honors IgnoresAccessChecksTo: every cell must JIT.
                Exception? failure = Record.Exception(() => RuntimeHelpers.PrepareMethod(method!.MethodHandle));
                Assert.True(failure == null, methodName + " failed to JIT: " + failure);
            }
        }
        finally
        {
            context.Unload();
        }
    }

    // ── the three C2′ primitives, mirrored from LocusBridge.AccessProbe.cs ──

    private sealed class PrimitiveTarget
    {
        private int _privInst = 7;
        private int PrivMethod(int x) { return x * 2 + 1; }
        public int ReadPrivInst() { return _privInst; }
    }

    private delegate ref int PrimitiveRefGetter(PrimitiveTarget target);

    [Fact]
    public void Emit_primitives_all_pass_on_coreclr()
    {
        // create_delegate_non_public: open-instance delegate over a private
        // method, created AND invoked.
        MethodInfo priv = typeof(PrimitiveTarget).GetMethod(
            "PrivMethod", BindingFlags.NonPublic | BindingFlags.Instance)!;
        var call = (Func<PrimitiveTarget, int, int>)Delegate.CreateDelegate(
            typeof(Func<PrimitiveTarget, int, int>), priv);
        Assert.Equal(11, call(new PrimitiveTarget(), 5));

        // dynamic_method_skip_visibility: ldfld of a private field.
        FieldInfo field = typeof(PrimitiveTarget).GetField(
            "_privInst", BindingFlags.NonPublic | BindingFlags.Instance)!;
        var read = new DynamicMethod(
            "__LocusProbeReadPriv", typeof(int), new[] { typeof(PrimitiveTarget) }, restrictedSkipVisibility: true);
        ILGenerator il = read.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldfld, field);
        il.Emit(OpCodes.Ret);
        var reader = (Func<PrimitiveTarget, int>)read.CreateDelegate(typeof(Func<PrimitiveTarget, int>));
        Assert.Equal(7, reader(new PrimitiveTarget()));

        // dynamic_method_byref_return: ldflda + ref-return, read/write
        // round-trip through the reference (M4 LocusFieldStore.Ref shape).
        var byref = new DynamicMethod(
            "__LocusProbeRefPriv", typeof(int).MakeByRefType(), new[] { typeof(PrimitiveTarget) }, restrictedSkipVisibility: true);
        il = byref.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldflda, field);
        il.Emit(OpCodes.Ret);
        var getter = (PrimitiveRefGetter)byref.CreateDelegate(typeof(PrimitiveRefGetter));
        var target = new PrimitiveTarget();
        ref int slot = ref getter(target);
        Assert.Equal(7, slot);
        slot = 21;
        Assert.Equal(21, target.ReadPrivInst());
        Assert.Equal(21, getter(target));
    }

    [Fact]
    public void Runtime_caps_round_trip_through_hot_patch()
    {
        var service = new CompileService();
        const string oldText = @"
namespace CapsE2E
{
    public class Calc
    {
        public int Value() { return 1; }
    }
}";
        string newText = oldText.Replace("return 1;", "return 2;");
        string originalPath = CompileOriginal(service, "AccessCapsEcho", oldText);

        var caps = new JsonObject
        {
            ["createDelegateNonPublic"] = true,
            ["dynamicMethodSkipVisibility"] = false,
            ["dynamicMethodByrefReturn"] = true,
            ["cells"] = new JsonObject
            {
                ["ldfld_private"] = true,
                ["castclass_internal"] = false,
            },
        };

        var request = new JsonObject
        {
            ["files"] = new JsonArray(new JsonObject
            {
                ["path"] = "Calc.cs",
                ["oldText"] = oldText,
                ["newText"] = newText,
            }),
            ["params"] = ParamsFor(originalPath),
            ["runtimeCaps"] = caps.DeepClone(),
        };

        JsonNode result = service.HandleCompileHotPatch(request);
        Assert.True(result["hot"]!.GetValue<bool>());
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        Assert.True(
            JsonNode.DeepEquals(caps, result["runtimeCaps"]),
            "runtimeCaps echo mismatch: " + result["runtimeCaps"]?.ToJsonString());
    }
}
