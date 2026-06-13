using System.Text.Json.Nodes;
using Xunit;
using Xunit.Abstractions;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Replay of the live Unity self-test batch accumulation (P00b..P27c) at the
/// sidecar layer: every batch re-sends every dirty file against the ORIGINAL
/// baseline (the live coordinator's hot_reload(None) semantics) and registers
/// the accepted image + shim registrations inline, so ~38 consecutive batches
/// accumulate session images that re-declare the same patch/shim/holder type
/// names. Locks the two shapes that failed in the real editor on a pre-fix
/// sidecar build (and pass since the fixes):
///   P17c — a brand-new NESTED type under a renamed container, referenced by
///          a kept-file call site, must re-qualify to the PATCH name
///          (pre-fix: CS0117 'LocusSelfTestSubject' has no 'Inner2');
///   P27c — an added EXTENSION method re-edited in a later batch binds
///          ambiguously between the batch source and the earlier image's
///          shim, and the call site must still rewrite to a direct shim call
///          (pre-fix: CS0121 twin LocusSelfTestHelper__LocusShims.Tripled).
/// </summary>
public class SelfTestReplayTests : IDisposable
{
    private readonly string _tempDir;
    private readonly ITestOutputHelper _output;

    public SelfTestReplayTests(ITestOutputHelper output)
    {
        _output = output;
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-selftest-replay-" + Guid.NewGuid().ToString("N"));
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

    private string CompileProjectAssembly(
        CompileService service, string assemblyName, JsonObject? compileParams, params (string Path, string Text)[] sources)
    {
        var request = new JsonObject
        {
            ["assemblyName"] = assemblyName,
            ["sources"] = new JsonArray(sources
                .Select(s => (JsonNode)new JsonObject { ["path"] = s.Path, ["text"] = s.Text })
                .ToArray()),
        };
        if (compileParams != null)
            request["params"] = compileParams.DeepClone();
        else
            request["useHostBcl"] = true;
        JsonNode result = service.HandleCompileRaw(request);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        string dir = Path.Combine(_tempDir, "Library", "ScriptAssemblies");
        Directory.CreateDirectory(dir);
        string path = Path.Combine(dir, assemblyName + ".dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    private const string UnityStubSource = @"
namespace UnityEngine
{
    public class Object { }
    public class Component : Object { }
    public class Behaviour : Component { }
    public class MonoBehaviour : Behaviour { }
}";

    private const string NegBaseline = @"public class LocusSelfTestNegative
{
    public int Echo(int v) { return v; }
}

public class LocusSelfTestNegGeneric<T>
{
    public int Val() { return 7; }
}
";

    private const string SubjectBaseline = @"using UnityEngine;
using System.Threading.Tasks;

public class LocusSelfTestSubject : MonoBehaviour
{
    public static LocusSelfTestSubject Instance;
    public static int EvtCount;
    public static int UpdateBeats;
    public static System.Func<int> Captured;
    private int _ticks = 0;
    private int _seed = 40;
    private int _legacy = 3;

    public int Ticks { get { return _ticks; } }
    public int Gauge { get { return 17; } }
    public int Doomed { get { return 1; } }
    public int Arrow => 12;
    private int _stash = 8;
    public int Stash { get { return _stash; } set { _stash = value; } }
    public int this[int i] { get { return i; } }
    public event System.Action Surge { add { EvtCount += 1; } remove { } }

    public class Inner
    {
        public int Nine() { return 1; }
    }

    void Awake() { if (Instance == null) Instance = this; }
    void Update() { _ticks += Step(); }

    public int Step() { return 1; }
    public int Mult() { return 1002; }
    public int Snare() { return 5; }
    public int Cond()
    {
#if UNITY_EDITOR
        return 1;
#else
        return 2;
#endif
    }
    public int Probe() { return 0; }
    public int Spare() { return 1; }
    public int Relay() { return new LocusSelfTestNegative().Echo(20) + 1; }
    public int RelayVal() { return new LocusSelfTestNegGeneric<int>().Val(); }
    public int Seed() { return _seed; }
    public int Legacy() { return _legacy; }
    public int Lambda() { int basis = 1; System.Func<int> f = () => basis + 1; return f(); }
    public int Local() { int InnerFn() { return 1; } return InnerFn(); }
    public int Anon() { var a = new { V = 1 }; return a.V; }
    public int Match(object k) { return k is int ? 1 : 0; }
    public int Flip() { return 3; }
    public int Shrink() { return 2; }
    public int CallRenamed() { return LocusSelfTestHelper.Renamed(); }
    public int CallBump() { int v = 1; LocusSelfTestHelper.Bump(ref v); return v; }
    public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a); }
    public int ModeValue(LocusSelfTestMode mode)
    {
        switch (mode)
        {
            case LocusSelfTestMode.A: return 11;
            case LocusSelfTestMode.B: return 22;
            default: return 0;
        }
    }
    public Task<int> Pulse() { return Task.FromResult(2001); }
    public System.Collections.IEnumerator Counting() { yield return 1; }
}
";

    private const string HelperBaseline = @"public static class LocusSelfTestHelper
{
    public static int Twice(int a) { return a * 2; }
    public static int Pick() { return 1; }
    public static int Renamed() { return 21; }
    public static void Bump(ref int v) { v += 1; }
}
";

    private const string ModeBaseline = "public enum LocusSelfTestMode { A = 0, B = 1 }\n";

    // B6: the live corpus' two-part partial type (LocusSelfTestPartialA/B.cs).
    private const string PartialABaseline = @"public partial class LocusSelfTestPartial
{
    private int _alpha = 30;

    public int Combine() { return _alpha + Basis() + _beta; }
}
";

    private const string PartialBBaseline = @"public partial class LocusSelfTestPartial
{
    private int _beta = 400;

    private int Basis() { return 5; }
}
";

    private static string Swap(string text, string from, string to)
    {
        Assert.Contains(from, text);
        int first = text.IndexOf(from, StringComparison.Ordinal);
        Assert.True(text.IndexOf(from, first + 1, StringComparison.Ordinal) < 0, "swap pattern not unique: " + from);
        return text.Replace(from, to);
    }

    /// <summary>Verbatim constants follow the checkout's line endings
    /// (core.autocrlf); the swap patterns are \n-escaped — normalize.</summary>
    private static string Lf(string text) => text.Replace("\r\n", "\n");

    /// <summary>Compile the REAL field-store runtime source into a DLL.</summary>
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
            ["params"] = new JsonObject
            {
                ["fingerprint"] = "selftest-replay-runtime",
                ["langVersion"] = "9",
                ["referencePaths"] = new JsonArray(HostBclPaths().Select(p => (JsonNode)p).ToArray()),
                ["defines"] = new JsonArray(),
            },
        };
        JsonNode result = service.HandleCompileRaw(request);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());
        string path = Path.Combine(_tempDir, "Locus.HotReload.Runtime.dll");
        File.WriteAllBytes(path, Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>()));
        return path;
    }

    [Fact]
    public void Replay_selftest_accumulated_batches()
    {
        var service = new CompileService();
        string runtimePath = CompileFieldStoreRuntime(service);

        // UnityEngine stub so the subject can extend MonoBehaviour like the
        // live corpus does.
        string unityStubPath;
        {
            var stubRequest = new JsonObject
            {
                ["assemblyName"] = "UnityEngine.CoreModule",
                ["sources"] = new JsonArray(new JsonObject { ["path"] = "UnityEngine.cs", ["text"] = UnityStubSource }),
                ["useHostBcl"] = true,
            };
            JsonNode stubResult = service.HandleCompileRaw(stubRequest);
            Assert.True(stubResult["success"]!.GetValue<bool>(), stubResult["error"]?.GetValue<string>());
            unityStubPath = Path.Combine(_tempDir, "UnityEngine.CoreModule.dll");
            File.WriteAllBytes(unityStubPath, Convert.FromBase64String(stubResult["assemblyB64"]!.GetValue<string>()));
        }

        string subjectBaseline = Lf(SubjectBaseline);
        string helperBaseline = Lf(HelperBaseline);
        string modeBaseline = Lf(ModeBaseline);

        var originalParams = new JsonObject
        {
            ["fingerprint"] = "selftest-replay-original",
            ["langVersion"] = "9",
            ["referencePaths"] = new JsonArray(HostBclPaths().Append(unityStubPath).Select(p => (JsonNode)p).ToArray()),
            ["defines"] = new JsonArray("UNITY_EDITOR"),
        };
        string partialABaseline = Lf(PartialABaseline);
        string partialBBaseline = Lf(PartialBBaseline);

        string asmPath = CompileProjectAssembly(
            service, "Assembly-CSharp", originalParams,
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestSubject.cs", subjectBaseline),
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestHelper.cs", helperBaseline),
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestMode.cs", modeBaseline),
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestNegative.cs", Lf(NegBaseline)),
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestPartialA.cs", partialABaseline),
            ("Assets/LocusHotReloadSelfTest/LocusSelfTestPartialB.cs", partialBBaseline));

        var paths = HostBclPaths().Append(unityStubPath).Append(asmPath);
        var compileParams = new JsonObject
        {
            ["fingerprint"] = "selftest-replay",
            ["domainGeneration"] = "selftest-replay-gen",
            ["langVersion"] = "9",
            ["referencePaths"] = new JsonArray(paths.Select(p => (JsonNode)p).ToArray()),
            ["defines"] = new JsonArray("UNITY_EDITOR"),
        };

        string subject = subjectBaseline;
        string helper = helperBaseline;
        string mode = modeBaseline;
        string partialA = partialABaseline;
        string? fresh = null;
        string probeLine = "    public int Probe() { return 0; }";

        bool helperDirty = false;
        bool modeDirty = false;
        bool partialDirty = false;

        var failures = new List<string>();

        void Batch(string name)
        {
            // Live batch order is a Rust HashMap iteration — arbitrary. Use
            // the unlucky order: subject (the call-site file) LAST.
            var files = new JsonArray();
            if (fresh != null)
            {
                files.Add(new JsonObject
                {
                    ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestFresh.cs",
                    ["oldText"] = "",
                    ["newText"] = fresh,
                });
            }
            if (helperDirty)
            {
                files.Add(new JsonObject
                {
                    ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestHelper.cs",
                    ["oldText"] = helperBaseline,
                    ["newText"] = helper,
                });
            }
            if (modeDirty)
            {
                files.Add(new JsonObject
                {
                    ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestMode.cs",
                    ["oldText"] = modeBaseline,
                    ["newText"] = mode,
                });
            }
            if (partialDirty)
            {
                files.Add(new JsonObject
                {
                    ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialA.cs",
                    ["oldText"] = partialABaseline,
                    ["newText"] = partialA,
                });
            }
            files.Add(new JsonObject
            {
                ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestSubject.cs",
                ["oldText"] = subjectBaseline,
                ["newText"] = subject,
            });

            var request = new JsonObject
            {
                ["files"] = files,
                ["params"] = compileParams.DeepClone(),
                ["registerImage"] = true,
                ["extraReferencePaths"] = new JsonArray(runtimePath),
            };
            if (partialDirty)
            {
                // B6: the live coordinator ships the sibling part candidates
                // with every batch whose pending edits mention partial types.
                request["baselineSiblings"] = new JsonArray(new JsonObject
                {
                    ["path"] = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialB.cs",
                    ["text"] = partialBBaseline,
                });
            }
            JsonNode result = service.HandleCompileHotPatch(request);
            bool hot = result["hot"]?.GetValue<bool>() ?? false;
            bool success = result["success"]?.GetValue<bool>() ?? false;
            if (!hot || !success)
            {
                string detail = result["error"]?.GetValue<string>()
                    ?? result["files"]?.ToJsonString()
                    ?? result.ToJsonString();
                failures.Add(name + ": " + detail);
                _output.WriteLine("FAIL " + name + ": " + detail);
            }
            else
            {
                _output.WriteLine("ok   " + name);
            }
        }

        void SetProbe(string newProbeLine)
        {
            subject = Swap(subject, probeLine, newProbeLine);
            probeLine = newProbeLine;
        }

        // P00b
        subject = Swap(subject, "public int Snare() { return 5; }", "public int Snare() { return 7667; }");
        Batch("P00b");
        // P01
        subject = Swap(subject, "public int Mult() { return 1002; }", "public int Mult() { return 4221; }");
        Batch("P01");
        // P01b
        subject = Swap(subject, "void Update() { _ticks += Step(); }", "void Update() { _ticks += Step(); UpdateBeats += 1; }");
        Batch("P01b");
        // P02
        subject = Swap(
            subject,
            "public Task<int> Pulse() { return Task.FromResult(2001); }",
            "public async Task<int> Pulse() { await Task.Yield(); return 2002; }");
        Batch("P02");
        // P02b
        subject = Swap(subject, "return 2002;", "return 2112;");
        Batch("P02b");
        // P03
        subject = Swap(
            subject,
            "    public int Step() { return 1; }\n",
            "    public int Step() { return 1; }\n    public int Boost() { return BoostCore() + 7000; }\n    public int BoostCore() { return 707; }\n");
        SetProbe("    public int Probe() { return Boost(); }");
        Batch("P03");
        // P04a writes the helper (cold verdict in live); P04b applies both.
        helper = Swap(
            helper,
            "public static int Twice(int a) { return a * 2; }",
            "public static int Twice(int a, int extra) { return a * 2 + extra; }");
        helperDirty = true;
        subject = Swap(
            subject,
            "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a); }",
            "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a, 100); }");
        Batch("P04b");
        // P05
        subject = Swap(
            subject,
            "    private int _legacy = 3;\n",
            "    private int _legacy = 3;\n    private int _bonus = 5050;\n");
        SetProbe("    public int Probe() { return _bonus + 9090; }");
        Batch("P05");
        // P05c
        subject = Swap(
            subject,
            "    private int _bonus = 5050;\n",
            "    private int _bonus = 5050;\n    private System.Collections.Generic.List<int> _list = new System.Collections.Generic.List<int> { 41, 1 };\n");
        SetProbe("    public int Probe() { return (_list == null ? 0 : _list.Count) + 4664; }");
        Batch("P05c");
        // P06
        subject = Swap(
            subject,
            "    public static int EvtCount;\n",
            "    public static int EvtCount;\n    private static int s_total = 6600;\n");
        SetProbe("    public int Probe() { s_total += 1; return s_total; }");
        Batch("P06");
        // P07 (using added -> whole-file rehook)
        subject = Swap(subject, "using UnityEngine;", "using UnityEngine;\nusing System.Text;");
        subject = Swap(
            subject,
            "public int Step() { return 1; }",
            "public int Step() { return 8800 + new StringBuilder(\"ab\").Length; }");
        Batch("P07");
        // P08 (enum append + subject case)
        mode = Swap(
            mode,
            "public enum LocusSelfTestMode { A = 0, B = 1 }",
            "public enum LocusSelfTestMode { A = 0, B = 1, C = 7 }");
        modeDirty = true;
        subject = Swap(
            subject,
            "            case LocusSelfTestMode.B: return 22;\n",
            "            case LocusSelfTestMode.B: return 22;\n            case LocusSelfTestMode.C: return 3377;\n");
        Batch("P08");
        // P09 (new file with new type)
        fresh = "public static class LocusSelfTestFresh { public static int Ping() { return 4242; } }\n";
        SetProbe("    public int Probe() { return LocusSelfTestFresh.Ping(); }");
        Batch("P09");
        // P10
        subject = Swap(subject, "public int Gauge { get { return 17; } }", "public int Gauge { get { return 7117; } }");
        Batch("P10");
        // P10b
        subject = Swap(
            subject,
            "public int Stash { get { return _stash; } set { _stash = value; } }",
            "public int Stash { get { return _stash; } set { _stash = value + 4880; } }");
        Batch("P10b");
        // P10c
        subject = Swap(subject, "public int Arrow => 12;", "public int Arrow => 7447;");
        Batch("P10c");
        // P11
        subject = Swap(
            subject,
            "public int this[int i] { get { return i; } }",
            "public int this[int i] { get { return i + 5005; } }");
        Batch("P11");
        // P12
        subject = Swap(
            subject,
            "public event System.Action Surge { add { EvtCount += 1; } remove { } }",
            "public event System.Action Surge { add { EvtCount += 4400; } remove { } }");
        Batch("P12");
        // P13
        subject = Swap(
            subject,
            "public int Lambda() { int basis = 1; System.Func<int> f = () => basis + 1; return f(); }",
            "public int Lambda() { int basis = 6060; System.Func<int> f = () => basis + 1; return f(); }");
        Batch("P13");
        // P13b
        subject = Swap(
            subject,
            "public int Lambda() { int basis = 6060; System.Func<int> f = () => basis + 1; return f(); }",
            "public int Lambda() { int basis = 6060; int extra = 1029; System.Func<int> f = () => basis + extra; return f(); }");
        Batch("P13b");
        // P14
        subject = Swap(
            subject,
            "public int Local() { int InnerFn() { return 1; } return InnerFn(); }",
            "public int Local() { int InnerFn() { return 9119; } return InnerFn(); }");
        Batch("P14");
        // P15
        subject = Swap(
            subject,
            "public int Anon() { var a = new { V = 1 }; return a.V; }",
            "public int Anon() { var a = new { V = 7997 }; return a.V; }");
        Batch("P15");
        // P16
        subject = Swap(
            subject,
            "public int Match(object k) { return k is int ? 1 : 0; }",
            "public int Match(object k) { return k is int n ? n + 6770 : 0; }");
        Batch("P16");
        // P16b (#if-block body edit; UNITY_EDITOR is defined)
        subject = Swap(subject, "#if UNITY_EDITOR\n        return 1;", "#if UNITY_EDITOR\n        return 8338;");
        Batch("P16b");
        // P17
        subject = Swap(subject, "public int Nine() { return 1; }", "public int Nine() { return 5665; }");
        Batch("P17");
        // P17b
        subject = Swap(
            subject,
            "    public class Inner\n    {\n",
            "    public class Inner\n    {\n        public int W = 9;\n");
        subject = Swap(subject, "public int Nine() { return 5665; }", "public int Nine() { return W + 5660; }");
        Batch("P17b");
        // P17c — failed live on the pre-fix sidecar (CS0117 Inner2). In a
        // fixed run the step lands, so the ledger keeps Inner2 from here on:
        // every later batch re-sends the new nested type with the session
        // images in scope.
        subject = Swap(
            subject,
            "    public class Inner\n    {\n",
            "    public class Inner2 { public static int Forty() { return 4554; } }\n\n    public class Inner\n    {\n");
        SetProbe("    public int Probe() { return Inner2.Forty(); }");
        Batch("P17c");
        // P18
        subject = Swap(
            subject,
            "public System.Collections.IEnumerator Counting() { yield return 1; }",
            "public System.Collections.IEnumerator Counting() { yield return 4334; }");
        Batch("P18");
        // P18b
        subject = Swap(
            subject,
            "public System.Collections.IEnumerator Counting() { yield return 4334; }",
            "public System.Collections.IEnumerator Counting() { return new int[] { 5225 }.GetEnumerator(); }");
        Batch("P18b");
        // P20
        subject = Swap(subject, "private int _seed = 40;", "private int _seed = 7557;");
        Batch("P20");
        // P21
        subject = Swap(subject, "    private int _legacy = 3;\n", "");
        subject = Swap(subject, "public int Legacy() { return _legacy; }", "public int Legacy() { return 8448; }");
        Batch("P21");
        // P22
        helper = Swap(helper, "public static int Renamed() { return 21; }", "public static int Thrice() { return 7227; }");
        subject = Swap(
            subject,
            "public int CallRenamed() { return LocusSelfTestHelper.Renamed(); }",
            "public int CallRenamed() { return LocusSelfTestHelper.Thrice(); }");
        Batch("P22");
        // P23
        helper = Swap(
            helper,
            "public static void Bump(ref int v) { v += 1; }",
            "public static void Bump(out int v) { v = 9229; }");
        subject = Swap(
            subject,
            "public int CallBump() { int v = 1; LocusSelfTestHelper.Bump(ref v); return v; }",
            "public int CallBump() { int v; LocusSelfTestHelper.Bump(out v); return v; }");
        Batch("P23");
        // P24
        subject = Swap(subject, "public int Flip() { return 3; }", "public static int Flip() { return 6336; }");
        SetProbe("    public int Probe() { return Flip(); }");
        Batch("P24");
        // P25
        subject = Swap(subject, "public int Shrink() { return 2; }", "private int Shrink() { return 4884; }");
        SetProbe("    public int Probe() { return Shrink(); }");
        Batch("P25");
        // P26
        helper = Swap(helper, "public static int Pick() { return 1; }", "public static int Pick() { return 3113; }");
        Batch("P26");
        // P27
        helper += "\npublic class LocusSelfTestExtra\n{\n    public static int Nine() { return 9559; }\n}\n";
        SetProbe("    public int Probe() { return LocusSelfTestExtra.Nine(); }");
        Batch("P27");
        // P27b
        helper = Swap(
            helper,
            "    public static int Pick() { return 3113; }\n",
            "    public static int Pick() { return 3113; }\n    public static int Tripled(this int v) { return v * 3; }\n");
        SetProbe("    public int Probe() { return 1500.Tripled() + 12; }");
        Batch("P27b");
        // P27c — failed live on the pre-fix sidecar (CS0121 twin shims): the
        // P27b image now provides Tripled to extension lookup alongside the
        // re-sent batch source, and the call site must still materialize as
        // a direct shim call.
        helper = Swap(
            helper,
            "public static int Tripled(this int v) { return v * 3; }",
            "public static int Tripled(this int v) { return v * 3 + 1; }");
        Batch("P27c");
        // P44 — partial part body edit (B6): the sibling part folds in as a
        // baseline, the two-part patch copy re-declares the COMPLETE type.
        partialA = Swap(
            partialA,
            "public int Combine() { return _alpha + Basis() + _beta; }",
            "public int Combine() { return _alpha + Basis() + _beta + 7000; }");
        partialDirty = true;
        Batch("P44");
        // P44b — the live accumulation shape: every LATER batch re-sends the
        // dirty partial part (and its sibling) alongside the new edit, with
        // the P44 image — which already declares the partial patch/sibling
        // copies — in the session-image scope.
        subject = Swap(subject, "public int Snare() { return 7667; }", "public int Snare() { return 7669; }");
        Batch("P44b");
        // P44c — a re-edit of the partial part itself on top of its image.
        partialA = Swap(partialA, "+ _beta + 7000; }", "+ _beta + 8000; }");
        Batch("P44c");

        Assert.True(failures.Count == 0, string.Join("\n", failures));
    }
}
