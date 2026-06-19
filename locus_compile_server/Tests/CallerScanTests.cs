using System.Security.Cryptography;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Location precision of the IL caller scan (M3): synthetic assemblies with
/// portable PDBs, callers mapped back to their source documents.
/// </summary>
public class CallerScanTests : IDisposable
{
    private readonly string _tempDir;

    public CallerScanTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-callerscan-tests-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(_tempDir);
    }

    public void Dispose()
    {
        // The scan now mirrors each built index to the shared temp cache dir;
        // remove the files this run produced (keyed by each DLL's path hash) so
        // tests don't litter, without touching a concurrently-running sidecar's
        // cache for unrelated assemblies.
        try
        {
            if (Directory.Exists(_tempDir))
            {
                foreach (string dll in Directory.EnumerateFiles(_tempDir, "*.dll"))
                {
                    try
                    {
                        File.Delete(PersistedCachePath(new FileInfo(dll).FullName));
                    }
                    catch
                    {
                    }
                }
            }
        }
        catch
        {
        }
        try
        {
            Directory.Delete(_tempDir, recursive: true);
        }
        catch
        {
        }
    }

    /// <summary>The on-disk path CallerScan persists an assembly's index to,
    /// for an assembly outside the Unity layout (temp fallback). MUST mirror
    /// <c>CallerScan.PersistDir</c>/<c>PersistFileName</c> — keep in sync.</summary>
    private static string PersistedCachePath(string assemblyFullPath)
    {
        string name = Path.GetFileNameWithoutExtension(assemblyFullPath);
        var sanitized = new StringBuilder(name.Length);
        foreach (char ch in name)
            sanitized.Append(char.IsLetterOrDigit(ch) ? ch : '_');
        byte[] digest = SHA1.HashData(Encoding.UTF8.GetBytes(assemblyFullPath.ToLowerInvariant()));
        string hash = Convert.ToHexString(digest, 0, 6).ToLowerInvariant();
        string dir = Path.Combine(Path.GetTempPath(), "Locus", "CallerIndex");
        return Path.Combine(dir, $"{sanitized}-{hash}-v1.json");
    }

    private static Guid AssemblyMvid(string assemblyFullPath)
    {
        using FileStream stream = File.OpenRead(assemblyFullPath);
        using var peReader = new PEReader(stream);
        MetadataReader reader = peReader.GetMetadataReader();
        return reader.GetGuid(reader.GetModuleDefinition().Mvid);
    }

    private static readonly CSharpParseOptions ParseOptions = new(languageVersion: LanguageVersion.CSharp9);

    private static IEnumerable<MetadataReference> HostBcl()
    {
        return ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Where(File.Exists)
            .Select(p => (MetadataReference)MetadataReference.CreateFromFile(p));
    }

    /// <summary>Compile sources into a DLL with a side-by-side portable PDB
    /// (the Unity ScriptAssemblies shape). Source paths become PDB docs.</summary>
    private string Compile(string assemblyName, (string Path, string Text)[] sources, bool emitPdb = true, params string[] references)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            sources.Select(s => CSharpSyntaxTree.ParseText(
                s.Text, ParseOptions, path: s.Path, encoding: System.Text.Encoding.UTF8)),
            HostBcl().Concat(references.Select(r => (MetadataReference)MetadataReference.CreateFromFile(r))),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));

        string dllPath = Path.Combine(_tempDir, assemblyName + ".dll");
        if (emitPdb)
        {
            string pdbPath = Path.Combine(_tempDir, assemblyName + ".pdb");
            using var dll = File.Create(dllPath);
            using var pdb = File.Create(pdbPath);
            EmitResult result = compilation.Emit(
                dll, pdb,
                options: new EmitOptions(debugInformationFormat: DebugInformationFormat.PortablePdb, pdbFilePath: pdbPath));
            Assert.True(result.Success, string.Join("\n", result.Diagnostics));
        }
        else
        {
            using var dll = File.Create(dllPath);
            EmitResult result = compilation.Emit(dll);
            Assert.True(result.Success, string.Join("\n", result.Diagnostics));
        }
        return dllPath;
    }

    private const string TargetSource = @"
namespace G
{
    public class Target
    {
        public void M() { }
        public void N() { }
    }
}";

    [Fact]
    public void External_caller_is_located_by_source_file()
    {
        string targetDll = Compile("ScanTargetLib", new[] { ("Assets/Target.cs", TargetSource) });
        string userDll = Compile(
            "ScanUserLib",
            new[]
            {
                ("Assets/User.cs", @"
namespace G
{
    public class User
    {
        public void Call(Target t) { t.M(); }
    }
}"),
            },
            emitPdb: true,
            targetDll);

        CallerScanResult result = CallerScan.Scan(
            new[] { targetDll, userDll },
            new[]
            {
                new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" },
                new CallerScanTarget { DeclaringType = "G.Target", MemberName = "N" },
            });

        Assert.Null(result.Error);
        Assert.Equal(new[] { "Assets/User.cs" }, result.CallerFiles["G.Target|M"].OrderBy(f => f));
        Assert.Empty(result.CallerFiles["G.Target|N"]);
    }

    [Fact]
    public void Same_assembly_caller_is_located()
    {
        string dll = Compile("ScanSameAsm", new[]
        {
            ("Assets/Target.cs", TargetSource),
            ("Assets/Neighbor.cs", @"
namespace G
{
    public class Neighbor
    {
        public void Go() { new Target().M(); }
    }
}"),
        });

        CallerScanResult result = CallerScan.Scan(
            new[] { dll },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });

        Assert.Null(result.Error);
        Assert.Contains("Assets/Neighbor.cs", result.CallerFiles["G.Target|M"]);
        Assert.DoesNotContain("Assets/Target.cs", result.CallerFiles["G.Target|M"]);
    }

    [Fact]
    public void Type_deletion_scan_finds_constructions_and_casts()
    {
        string targetDll = Compile("ScanTypeLib", new[] { ("Assets/Target.cs", TargetSource) });
        string userDll = Compile(
            "ScanTypeUser",
            new[]
            {
                ("Assets/Spawner.cs", @"
namespace G
{
    public class Spawner
    {
        public object Make() { return new Target(); }
    }
}"),
            },
            emitPdb: true,
            targetDll);

        CallerScanResult result = CallerScan.Scan(
            new[] { targetDll, userDll },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "" } });

        Assert.Null(result.Error);
        Assert.Contains("Assets/Spawner.cs", result.CallerFiles["G.Target|"]);
    }

    [Fact]
    public void Missing_pdb_fails_closed()
    {
        string targetDll = Compile("ScanNoPdbLib", new[] { ("Assets/Target.cs", TargetSource) });
        string userDll = Compile(
            "ScanNoPdbUser",
            new[]
            {
                ("Assets/User.cs", @"
namespace G
{
    public class User
    {
        public void Call(Target t) { t.M(); }
    }
}"),
            },
            emitPdb: false,
            targetDll);

        CallerScanResult result = CallerScan.Scan(
            new[] { targetDll, userDll },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });

        Assert.NotNull(result.Error);
        Assert.Contains("PDB", result.Error);
    }

    [Fact]
    public void Method_group_and_property_accessors_are_found()
    {
        string dll = Compile("ScanGroups", new[]
        {
            ("Assets/Lib.cs", @"
namespace G
{
    public class Lib
    {
        public int Value { get { return 1; } }
        public void M() { }
    }
}"),
            ("Assets/Consumer.cs", @"
using System;
namespace G
{
    public class Consumer
    {
        public Action Capture(Lib lib) { return lib.M; }
        public int Read(Lib lib) { return lib.Value; }
    }
}"),
        });

        CallerScanResult result = CallerScan.Scan(
            new[] { dll },
            new[]
            {
                new CallerScanTarget { DeclaringType = "G.Lib", MemberName = "M" },
                new CallerScanTarget { DeclaringType = "G.Lib", MemberName = "get_Value" },
            });

        Assert.Null(result.Error);
        Assert.Contains("Assets/Consumer.cs", result.CallerFiles["G.Lib|M"]);
        Assert.Contains("Assets/Consumer.cs", result.CallerFiles["G.Lib|get_Value"]);
    }

    // Generic METHOD calls reference a MethodSpec token (the instantiation),
    // not the MemberRef/MethodDef itself: the scan must resolve the chain or
    // every generic call site is a silent miss (fail-open). B1 gates on this.

    private const string GenericTargetSource = @"
namespace G
{
    public class Target
    {
        public T Echo<T>(T value) { return value; }
        public static T Pick<T>(T value) { return value; }
    }
}";

    [Fact]
    public void Same_assembly_generic_method_call_is_located_via_methodspec()
    {
        string dll = Compile("ScanGenericSame", new[]
        {
            ("Assets/Target.cs", GenericTargetSource),
            ("Assets/Neighbor.cs", @"
namespace G
{
    public class Neighbor
    {
        public int Go() { return new Target().Echo(5); }
        public System.Func<int, int> Grab() { return Target.Pick<int>; }
    }
}"),
        });

        CallerScanResult result = CallerScan.Scan(
            new[] { dll },
            new[]
            {
                new CallerScanTarget { DeclaringType = "G.Target", MemberName = "Echo" },
                new CallerScanTarget { DeclaringType = "G.Target", MemberName = "Pick" },
            });

        Assert.Null(result.Error);
        Assert.Contains("Assets/Neighbor.cs", result.CallerFiles["G.Target|Echo"]);
        Assert.Contains("Assets/Neighbor.cs", result.CallerFiles["G.Target|Pick"]);
        Assert.DoesNotContain("Assets/Target.cs", result.CallerFiles["G.Target|Echo"]);
    }

    [Fact]
    public void Cross_assembly_generic_method_call_is_located_via_methodspec()
    {
        string targetDll = Compile("ScanGenericLib", new[] { ("Assets/Target.cs", GenericTargetSource) });
        string userDll = Compile(
            "ScanGenericUser",
            new[]
            {
                ("Assets/User.cs", @"
namespace G
{
    public class User
    {
        public int Call(Target t) { return t.Echo<int>(7); }
    }
}"),
            },
            emitPdb: true,
            targetDll);

        CallerScanResult result = CallerScan.Scan(
            new[] { targetDll, userDll },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "Echo" } });

        Assert.Null(result.Error);
        Assert.Equal(new[] { "Assets/User.cs" }, result.CallerFiles["G.Target|Echo"].OrderBy(f => f));
    }

    // ── Persistent tier ──────────────────────────────────────────────
    // The reverse caller graph survives a sidecar restart: a successful build
    // is mirrored to one JSON file per assembly, keyed by on-disk identity, and
    // reloaded on the next process instead of re-walking the IL.

    [Fact]
    public void Built_index_is_mirrored_to_disk()
    {
        string suffix = Guid.NewGuid().ToString("N");
        string targetDll = Compile("ScanPersistTarget" + suffix, new[] { ("Assets/Target.cs", TargetSource) });
        string userDll = Compile(
            "ScanPersistUser" + suffix,
            new[]
            {
                ("Assets/User.cs", @"
namespace G
{
    public class User
    {
        public void Call(Target t) { t.M(); }
    }
}"),
            },
            emitPdb: true,
            targetDll);

        CallerScanResult result = CallerScan.Scan(
            new[] { targetDll, userDll },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });
        Assert.Null(result.Error);
        Assert.Contains("Assets/User.cs", result.CallerFiles["G.Target|M"]);

        // The caller of G.Target.M lives in the USER assembly, so its index file
        // is the one that must carry the location.
        string cachePath = PersistedCachePath(new FileInfo(userDll).FullName);
        Assert.True(File.Exists(cachePath), $"expected a persisted index at {cachePath}");

        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(cachePath));
        JsonElement root = doc.RootElement;
        Assert.Equal(1, root.GetProperty("FormatVersion").GetInt32());
        Assert.Equal(new FileInfo(userDll).Length, root.GetProperty("Length").GetInt64());
        JsonElement locations = root.GetProperty("LocationsByTarget").GetProperty("G.Target|M");
        Assert.Contains(
            locations.EnumerateArray(),
            element => element.GetProperty("File").GetString() == "Assets/User.cs");
    }

    [Fact]
    public void Persisted_index_is_loaded_on_cache_miss()
    {
        // A freshly-named assembly never scanned in this process: the in-memory
        // cache misses, so the only possible source of a caller location is the
        // seeded disk file. The real IL has no caller of G.Target.M at all, so a
        // returned sentinel proves the persisted index was loaded, not rebuilt.
        string dll = Compile(
            "ScanPersistLoad" + Guid.NewGuid().ToString("N"),
            new[] { ("Assets/Target.cs", TargetSource) });
        var info = new FileInfo(dll);
        string fullPath = info.FullName;

        const string sentinel = "Assets/__persisted_sentinel__.cs";
        string cachePath = PersistedCachePath(fullPath);
        Directory.CreateDirectory(Path.GetDirectoryName(cachePath)!);
        var persisted = new JsonObject
        {
            ["FormatVersion"] = 1,
            ["LastWriteUtcTicks"] = info.LastWriteTimeUtc.Ticks,
            ["Length"] = info.Length,
            ["Mvid"] = AssemblyMvid(fullPath).ToString(),
            ["TargetKeys"] = new JsonArray { "G.Target|M" },
            ["LocationsByTarget"] = new JsonObject
            {
                ["G.Target|M"] = new JsonArray
                {
                    new JsonObject
                    {
                        ["File"] = sentinel,
                        ["CallerMethodKey"] = "G.Probe|Call|1|i",
                        ["DeclaringType"] = "G.Probe",
                        ["MemberName"] = "Call",
                    },
                },
            },
            ["UnmappedByTarget"] = new JsonObject(),
        };
        File.WriteAllText(cachePath, persisted.ToJsonString());

        CallerScanResult result = CallerScan.Scan(
            new[] { fullPath },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });

        Assert.Null(result.Error);
        Assert.Contains(sentinel, result.CallerFiles["G.Target|M"]);
    }

    [Fact]
    public void Stale_persisted_index_is_ignored_when_length_changes()
    {
        // A cache file whose recorded identity no longer matches the assembly on
        // disk must be discarded (fail-safe), not trusted: the scan rebuilds.
        string dll = Compile(
            "ScanPersistStale" + Guid.NewGuid().ToString("N"),
            new[] { ("Assets/Target.cs", TargetSource) });
        var info = new FileInfo(dll);

        const string sentinel = "Assets/__stale_sentinel__.cs";
        string cachePath = PersistedCachePath(info.FullName);
        Directory.CreateDirectory(Path.GetDirectoryName(cachePath)!);
        var persisted = new JsonObject
        {
            ["FormatVersion"] = 1,
            ["LastWriteUtcTicks"] = info.LastWriteTimeUtc.Ticks,
            ["Length"] = info.Length + 1, // identity mismatch → stale.
            ["Mvid"] = Guid.Empty.ToString(),
            ["TargetKeys"] = new JsonArray { "G.Target|M" },
            ["LocationsByTarget"] = new JsonObject
            {
                ["G.Target|M"] = new JsonArray
                {
                    new JsonObject { ["File"] = sentinel, ["CallerMethodKey"] = "G.Probe|Call|1|i" },
                },
            },
            ["UnmappedByTarget"] = new JsonObject(),
        };
        File.WriteAllText(cachePath, persisted.ToJsonString());

        CallerScanResult result = CallerScan.Scan(
            new[] { info.FullName },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });

        Assert.Null(result.Error);
        Assert.DoesNotContain(sentinel, result.CallerFiles["G.Target|M"]);
    }

    [Fact]
    public void Stale_persisted_index_is_ignored_when_mvid_changes()
    {
        // A same path/mtime/length collision can still identify different IL.
        // MVID is the metadata identity that distinguishes those assemblies.
        string dll = Compile(
            "ScanPersistMvidStale" + Guid.NewGuid().ToString("N"),
            new[] { ("Assets/Target.cs", TargetSource) });
        var info = new FileInfo(dll);
        Guid actualMvid = AssemblyMvid(info.FullName);
        Guid staleMvid;
        do
        {
            staleMvid = Guid.NewGuid();
        } while (staleMvid == actualMvid);

        const string sentinel = "Assets/__stale_mvid_sentinel__.cs";
        string cachePath = PersistedCachePath(info.FullName);
        Directory.CreateDirectory(Path.GetDirectoryName(cachePath)!);
        var persisted = new JsonObject
        {
            ["FormatVersion"] = 1,
            ["LastWriteUtcTicks"] = info.LastWriteTimeUtc.Ticks,
            ["Length"] = info.Length,
            ["Mvid"] = staleMvid.ToString(),
            ["TargetKeys"] = new JsonArray { "G.Target|M" },
            ["LocationsByTarget"] = new JsonObject
            {
                ["G.Target|M"] = new JsonArray
                {
                    new JsonObject { ["File"] = sentinel, ["CallerMethodKey"] = "G.Probe|Call|1|i" },
                },
            },
            ["UnmappedByTarget"] = new JsonObject(),
        };
        File.WriteAllText(cachePath, persisted.ToJsonString());

        CallerScanResult result = CallerScan.Scan(
            new[] { info.FullName },
            new[] { new CallerScanTarget { DeclaringType = "G.Target", MemberName = "M" } });

        Assert.Null(result.Error);
        Assert.DoesNotContain(sentinel, result.CallerFiles["G.Target|M"]);
    }
}
