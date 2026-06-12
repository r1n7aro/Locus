using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// TI-B parity rules: top-level public types only, generic arity stripped,
/// first-assembly-wins dedup over name-sorted assemblies, Unity skip list,
/// and the exact (simpleName, ns, assembly) ordering of the Unity export.
/// </summary>
public class TypeIndexSourceTests : IDisposable
{
    private readonly string _tempDir;

    public TypeIndexSourceTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-tiB-tests-" + Guid.NewGuid().ToString("N"));
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

    private string CompileToDisk(string assemblyName, string source)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            new[] { CSharpSyntaxTree.ParseText(source) },
            ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
                .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
                .Where(File.Exists)
                .Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        string path = Path.Combine(_tempDir, assemblyName + ".dll");
        var emit = compilation.Emit(path);
        Assert.True(emit.Success, string.Join("\n", emit.Diagnostics));
        return path;
    }

    private static List<TypeIndexSource.Entry> BuildFromPaths(params string[] paths)
    {
        var cache = new ReferenceCache();
        var references = paths
            .Select(p => cache.GetOrCreate(p))
            .Where(r => r != null)
            .Select(r => r!);
        return TypeIndexSource.Build(references);
    }

    [Fact]
    public void Indexes_top_level_public_types_with_arity_stripped()
    {
        string path = CompileToDisk("TiBSampleA", @"
namespace Game
{
    public class Player { public class NestedPublic { } }
    public struct Vec { }
    public interface IThing { }
    public enum Kind { A }
    public delegate void Handler();
    public class Box<T> { }
    internal class Hidden { }
}
public class GlobalType { }
");

        List<TypeIndexSource.Entry> entries = BuildFromPaths(path);

        var names = entries.Select(e => e.FullName).ToList();
        Assert.Contains("Game.Player", names);
        Assert.Contains("Game.Vec", names);
        Assert.Contains("Game.IThing", names);
        Assert.Contains("Game.Kind", names);
        Assert.Contains("Game.Handler", names);
        Assert.Contains("Game.Box", names); // arity stripped
        Assert.Contains("GlobalType", names);
        Assert.DoesNotContain("Game.Hidden", names);
        Assert.DoesNotContain(names, n => n.Contains("NestedPublic"));
        Assert.All(entries, e => Assert.Equal("TiBSampleA", e.Assembly));

        TypeIndexSource.Entry box = entries.Single(e => e.FullName == "Game.Box");
        Assert.Equal("Box", box.SimpleName);
        Assert.Equal("Game", box.Ns);
    }

    [Fact]
    public void Duplicate_full_names_keep_the_first_assembly_by_name()
    {
        string second = CompileToDisk("TiB_B", "namespace Dup { public class Shared { } }");
        string first = CompileToDisk("TiB_A", "namespace Dup { public class Shared { } }");

        // Input order reversed on purpose: the sort by assembly name decides.
        List<TypeIndexSource.Entry> entries = BuildFromPaths(second, first);

        TypeIndexSource.Entry shared = entries.Single(e => e.FullName == "Dup.Shared");
        Assert.Equal("TiB_A", shared.Assembly);
    }

    [Fact]
    public void Unity_skip_list_applies_to_assembly_names()
    {
        string skipped = CompileToDisk("Locus.Editor", "namespace L { public class BridgeThing { } }");
        string kept = CompileToDisk("TiBKept", "namespace L { public class Kept { } }");

        List<TypeIndexSource.Entry> entries = BuildFromPaths(skipped, kept);

        Assert.DoesNotContain(entries, e => e.FullName == "L.BridgeThing");
        Assert.Contains(entries, e => e.FullName == "L.Kept");

        Assert.True(TypeIndexSource.ShouldSkipAssembly("__LocusHotPatch_00000000_00000001"));
        Assert.True(TypeIndexSource.ShouldSkipAssembly("Microsoft.CodeAnalysis.CSharp"));
        Assert.False(TypeIndexSource.ShouldSkipAssembly("Assembly-CSharp"));
    }

    [Fact]
    public void Entries_sort_by_simple_name_then_namespace_then_assembly()
    {
        string path = CompileToDisk("TiBSort", @"
namespace B { public class Alpha { } }
namespace A { public class Alpha { } }
namespace A { public class Beta { } }
");

        List<TypeIndexSource.Entry> entries = BuildFromPaths(path);
        var ordered = entries.Where(e => e.Ns == "A" || e.Ns == "B").ToList();

        Assert.Equal(new[] { "A.Alpha", "B.Alpha", "A.Beta" }, ordered.Select(e => e.FullName).ToArray());
    }

    [Fact]
    public void Handler_returns_types_for_reference_paths()
    {
        string path = CompileToDisk("TiBHandler", "namespace H { public class Visible { } }");
        var service = new CompileService();

        var request = new JsonObject
        {
            ["params"] = new JsonObject
            {
                ["fingerprint"] = "tiB-test",
                ["domainGeneration"] = Guid.NewGuid().ToString("N"),
                ["langVersion"] = "9",
                ["referencePaths"] = new JsonArray(path),
                ["defines"] = new JsonArray(),
            },
        };

        JsonNode result = service.HandleIndexTypes(request);

        Assert.True(result["count"]!.GetValue<int>() >= 1);
        Assert.Contains(
            result["types"]!.AsArray(),
            t => t!["fullName"]!.GetValue<string>() == "H.Visible" &&
                 t["assembly"]!.GetValue<string>() == "TiBHandler" &&
                 t["simpleName"]!.GetValue<string>() == "Visible" &&
                 t["ns"]!.GetValue<string>() == "H");
    }
}
