using System.Collections.Concurrent;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Locus.CompileServer;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;

var probe = new Probe(args[0]);
await probe.Run();

sealed class Probe(string root)
{
    private readonly List<object> observations = new();
    private readonly List<string> failures = new();
    private readonly string[] bclPaths = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator);
    private MetadataReference[] bcl = null!;
    private byte[] v1 = null!, v2 = null!;
    private int sequence;

    public async Task Run()
    {
        bcl = bclPaths.Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)).ToArray();
        v1 = Build("OldType", "OldCaller", "M");
        v2 = Build("NewType", "NewCaller", "N");
        Require(v1.Length > 64 * 1024, "fixture must exceed small-file mapping thresholds");
        foreach (bool atomic in new[] { false, true })
        foreach (bool warm in new[] { false, true })
        {
            await Lifetime("direct", p => MetadataReference.CreateFromFile(p), null, atomic, warm);
            var cache = new ReferenceCache();
            await Lifetime("cache", p => cache.GetOrCreate(p)!, null, atomic, warm);
        }
        var pruneCache = new ReferenceCache();
        await Lifetime("prune", p => pruneCache.GetOrCreate(p)!, () => pruneCache.PruneExcept(Array.Empty<string>()), true, false);
        Freshness();
        await Scopes();
        foreach (bool atomic in new[] { false, true })
        {
            await Scan(atomic);
        }
        var retired = EvictedReference();
        var weak = retired.Reference;
        for (int i = 0; i < 3 && weak.IsAlive; i++) { GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect(); }
        observations.Add(new { name = "evicted-reference", collected = !weak.IsAlive });
        Require(!weak.IsAlive, "evicted reference remains rooted after its last consumer is gone");
        GC.KeepAlive(retired.Cache);
        var report = new { runtime = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription, roslyn = typeof(MetadataReference).Assembly.GetName().Version!.ToString(), fixtureBytes = v1.Length, observations, failures };
        await File.WriteAllTextAsync(Path.Combine(root, "report.json"), JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true }));
        if (failures.Count > 0) throw new Exception(string.Join("\n", failures));
    }

    private void Require(bool value, string reason) { if (!value) failures.Add(reason); }
    private string FreshPath(string name)
    {
        // CallerScan persists only inside this owned mini-project's Library.
        string directory = Path.Combine(root, "mini-project", "Library", "ScriptAssemblies");
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, $"{Interlocked.Increment(ref sequence)}-{name}.dll");
        File.WriteAllBytes(path, v1);
        File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddMinutes(-1));
        return path;
    }
    private byte[] Build(string type, string caller, string member)
    {
        string source = $"public class {type} {{ }} public class Target {{ public void M() {{ }} public void N() {{ }} }} public class {caller} {{ public void Call(Target t) {{ t.{member}(); }} }}";
        source += string.Concat(Enumerable.Range(0, 1500).Select(i => $"public class Padding{i} {{ public int Field{i}; }}"));
        var compilation = CSharpCompilation.Create("ReplaceableFixture", new[] { CSharpSyntaxTree.ParseText(source, path: $"Assets/{caller}.cs", encoding: Encoding.UTF8) }, bcl, new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        using var bytes = new MemoryStream();
        var result = compilation.Emit(bytes, options: new EmitOptions(debugInformationFormat: DebugInformationFormat.Embedded));
        if (!result.Success) throw new Exception(string.Join("\n", result.Diagnostics));
        return bytes.ToArray();
    }
    private CSharpCompilation Consumer(MetadataReference reference, string type) => CSharpCompilation.Create("Consumer" + Guid.NewGuid().ToString("N"), new[] { CSharpSyntaxTree.ParseText($"public class Consumer {{ public {type} Value; }}") }, bcl.Append(reference), new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    private static (bool Ok, string Error) Emit(CSharpCompilation compilation)
    {
        try { using var output = new MemoryStream(); var result = compilation.Emit(output); return (result.Success, string.Join(" | ", result.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Error))); }
        catch (Exception ex) { return (false, ex.GetType().Name + ": " + ex.Message); }
    }
    private string? Replace(string path, bool atomic)
    {
        try
        {
            if (atomic) { string next = path + ".next"; File.WriteAllBytes(next, v2); File.Replace(next, path, null); }
            else File.WriteAllBytes(path, v2);
            File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddMinutes(1));
            return null;
        }
        catch (IOException ex) { return ex.GetType().Name + ": " + ex.Message; }
    }
    private async Task Lifetime(string name, Func<string, PortableExecutableReference> get, Action? prune, bool atomic, bool warm)
    {
        string path = FreshPath(name);
        var oldReference = get(path);
        var old = Consumer(oldReference, "OldType");
        if (warm) Require(Emit(old).Ok, name + " initial binding failed");
        var resume = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var oldTask = Task.Run(async () => { await resume.Task; return Emit(old); });
        string? replacement;
        (bool Ok, string Error) current;
        try
        {
            (replacement, current) = await Task.Run(() =>
            {
                string? error = Replace(path, atomic);
                prune?.Invoke();
                return (error, Emit(Consumer(get(path), "NewType")));
            });
        }
        finally { resume.SetResult(); }
        var previous = await oldTask;
        GC.KeepAlive(oldReference);
        observations.Add(new { name, atomic, warm, replaced = replacement == null, oldCompleted = previous.Ok, newCompleted = current.Ok, oldError = previous.Error, newError = current.Error, replacementError = replacement });
        Require(replacement == null && current.Ok, name + " replacement/new version failed");
        Require(previous.Ok, name + " old snapshot failed: " + previous.Error);
    }

    private JsonNode Request(string path, string type, string fingerprint) => new JsonObject
    {
        ["sources"] = new JsonArray(new JsonObject { ["path"] = "Consumer.cs", ["text"] = $"public class Consumer {{ public {type} Value; }}" }),
        ["params"] = new JsonObject { ["referencePaths"] = new JsonArray(bclPaths.Append(path).Select(p => (JsonNode?)JsonValue.Create(p)).ToArray()), ["fingerprint"] = fingerprint, ["langVersion"] = "9", ["domainGeneration"] = "fixture" },
    };
    private void Freshness()
    {
        string path = FreshPath("service");
        var service = new CompileService();
        bool initial = service.HandleCompileRaw(Request(path, "OldType", "v1"))["success"]!.GetValue<bool>();
        Require(Replace(path, true) == null, "service DLL replacement failed");
        bool unchanged = service.HandleCompileRaw(Request(path, "NewType", "v1"))["success"]!.GetValue<bool>();
        bool changed = service.HandleCompileRaw(Request(path, "NewType", "v2"))["success"]!.GetValue<bool>();
        Require(initial && changed, "service version refresh failed");
        observations.Add(new { name = "service-fingerprint", initial, sameFingerprintReadsNew = unchanged, changedFingerprintReadsNew = changed });

        path = FreshPath("same-stat");
        var cache = new ReferenceCache(); var previous = cache.GetOrCreate(path)!;
        DateTime time = File.GetLastWriteTimeUtc(path);
        // Padding outside the PE image makes both versions exactly the same file length.
        int length = Math.Max(v1.Length, v2.Length);
        var paddedOld = new byte[length]; v1.CopyTo(paddedOld, 0);
        var paddedNew = new byte[length]; v2.CopyTo(paddedNew, 0);
        File.WriteAllBytes(path, paddedOld); File.SetLastWriteTimeUtc(path, time);
        previous = cache.GetOrCreate(path)!;
        File.WriteAllBytes(path, paddedNew); File.SetLastWriteTimeUtc(path, time);
        var next = cache.GetOrCreate(path)!;
        observations.Add(new { name = "same-stat", sameReference = ReferenceEquals(previous, next), newCompleted = Emit(Consumer(next, "NewType")).Ok });
    }
    private async Task Scopes()
    {
        var registry = new ScopedCompileServiceRegistry();
        var results = new ConcurrentBag<bool>();
        string path = FreshPath("shared-scopes");
        int pending = 4;
        var published = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        await Task.WhenAll(Enumerable.Range(1, 4).Select(async i =>
        {
            var scopeParams = new JsonObject { ["scopeId"] = new JsonObject { ["checkoutId"] = "fixture-" + i, ["workspaceGeneration"] = 1L, ["serviceGeneration"] = 1L, ["unityEditorSessionId"] = "fixture" } };
            var scope = registry.GetOrCreate(scopeParams);
            await scope.RequestGate.WaitAsync();
            try
            {
                bool old = await Task.Run(() => scope.Service.HandleCompileRaw(Request(path, "OldType", "v1"))["success"]!.GetValue<bool>());
                if (Interlocked.Decrement(ref pending) == 0) published.TrySetResult(Replace(path, true) == null);
                bool replaced = await published.Task.WaitAsync(TimeSpan.FromSeconds(20));
                bool next = await Task.Run(() => scope.Service.HandleCompileRaw(Request(path, "NewType", "v2"))["success"]!.GetValue<bool>());
                results.Add(old && replaced && next);
            }
            finally { scope.RequestGate.Release(); }
        }));
        observations.Add(new { name = "parallel-scopes", scopes = results.Count, sharedDll = true, completed = results.All(v => v) });
        Require(results.Count == 4 && results.All(v => v), "production scope concurrency failed");
    }
    private async Task Scan(bool atomic)
    {
        string path = FreshPath("scanner");
        using var loaded = new ManualResetEventSlim();
        using var resume = new ManualResetEventSlim();
        Action checkpoint = () => { loaded.Set(); if (!resume.Wait(TimeSpan.FromSeconds(20))) throw new TimeoutException("scanner checkpoint"); };
        InstrumentedScanner.Checkpoint.AfterRead = checkpoint;
        Task<(string? Error, bool Old, bool New)> task = Task.Run(() => ScanResult(path));
        string? replacement = "checkpoint not reached";
        bool ready = false;
        try { ready = loaded.Wait(TimeSpan.FromSeconds(20)); if (ready) replacement = Replace(path, atomic); }
        finally { resume.Set(); }
        var old = await task;
        InstrumentedScanner.Checkpoint.AfterRead = null;
        string? after = replacement == null ? null : Replace(path, atomic);
        var next = ScanResult(path);
        observations.Add(new { name = "scan", atomic, checkpointReached = ready, replacedWhileScanning = replacement == null, oldCompleted = old.Error == null && old.Old && !old.New, replacedAfterScan = after == null, newCompleted = next.Error == null && next.New && !next.Old, replacementError = replacement });
        Require(ready && old.Error == null && old.Old && !old.New && after == null && next.Error == null && next.New && !next.Old, "scanner result or checkpoint failed");
        Require(replacement == null, "scanner still locks DLL during IL analysis");
    }
    private static (string? Error, bool Old, bool New) ScanResult(string path)
    {
        var result = InstrumentedScanner.CallerScan.Scan(new[] { path }, new[] { new InstrumentedScanner.CallerScanTarget { DeclaringType = "Target", MemberName = "M" }, new InstrumentedScanner.CallerScanTarget { DeclaringType = "Target", MemberName = "N" } });
        return (result.Error, result.CallerFiles["Target|M"].Contains("Assets/OldCaller.cs"), result.CallerFiles["Target|N"].Contains("Assets/NewCaller.cs"));
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    private (ReferenceCache Cache, WeakReference Reference) EvictedReference()
    {
        var cache = new ReferenceCache();
        var reference = cache.GetOrCreate(FreshPath("collect"))!;
        var weak = new WeakReference(reference);
        cache.PruneExcept(Array.Empty<string>());
        GC.KeepAlive(reference);
        return (cache, weak);
    }
}
