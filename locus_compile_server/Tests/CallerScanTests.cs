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
        try
        {
            Directory.Delete(_tempDir, recursive: true);
        }
        catch
        {
        }
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
}
