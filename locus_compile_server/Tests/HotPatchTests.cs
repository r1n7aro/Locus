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

    [Fact]
    public void Cold_change_reports_hot_false_with_reasons()
    {
        var service = new CompileService();
        JsonObject compileParams = ParamsFor();

        const string oldText = "class A { void M() { } }";
        const string newText = "class A { int _x; void M() { } }";

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
