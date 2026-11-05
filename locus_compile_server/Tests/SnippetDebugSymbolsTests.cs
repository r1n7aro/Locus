using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Text.Json.Nodes;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// unity_execute snippets carry an embedded portable PDB so Unity's Mono
/// reports runtime exceptions with snippet line numbers
/// ("in LocusRuntimeAsyncSnippet.cs:5") instead of "&lt;mvid&gt;:0".
/// </summary>
public class SnippetDebugSymbolsTests
{
    private const string UnityStubs = @"
namespace UnityEngine
{
    public class Object { }
    namespace SceneManagement { public sealed class Scene { } }
}

namespace UnityEditor
{
    namespace SceneManagement { public sealed class EditorSceneManager { } }
    namespace Animations { public sealed class AnimatorController { } }
}

namespace Locus
{
    public static class LocusBridge
    {
        public sealed class ScriptGlobals
        {
            public void print(object value) { }
            public void printJson(object value) { }
            public void clear() { }
        }

        public sealed class ExecuteCodeContext
        {
            public void ThrowIfCancellationRequested() { }
        }
    }
}";

    /// <summary>
    /// The loop case also covers LocusSnippetLoopGuard: its rewritten tree
    /// must keep the source encoding (or emit fails with CS8055) and the
    /// snippet line numbers.
    /// </summary>
    [Theory]
    [InlineData(
        "var keys = new[] { \"a\", \"missing\" };\nforeach (var k in keys)\n{\n    string f = k == \"a\" ? \"x\" : null;\n    print(f.Length);\n}",
        5)]
    [InlineData(
        "await System.Threading.Tasks.Task.Yield();\nstring s = null;\nprint(s.Length);",
        3)]
    public void Snippet_embeds_a_pdb_that_maps_to_snippet_lines(string code, int throwingLine)
    {
        Dictionary<string, List<int>> lines = VisibleLinesByMethod(CompileSnippet(code));

        Assert.Contains(throwingLine, lines.Values.SelectMany(value => value));
    }

    /// <summary>
    /// Wrapper code is under `#line hidden`: the ExecuteAsync forwarder has
    /// no visible sequence point (its frame used to report a generated line
    /// such as 52), and no visible point lies beyond the snippet itself
    /// (its last line + 1 for the wrapper's trailing `return null;`).
    /// </summary>
    [Theory]
    [InlineData("print(1);\nprint(2);")]
    [InlineData("using System.Text;\n\nvar sb = new StringBuilder();\nstring s = null;\nprint(sb.Length + s.Length);")]
    public void Wrapper_code_has_no_visible_sequence_points(string code)
    {
        Dictionary<string, List<int>> lines = VisibleLinesByMethod(CompileSnippet(code));
        int snippetLines = code.Split('\n').Length;

        Assert.Empty(lines["ExecuteAsync"]);
        Assert.All(lines.Values.SelectMany(value => value), line => Assert.InRange(line, 1, snippetLines + 1));
        Assert.Contains(snippetLines, lines["ExecuteSync"]);
    }

    /// <summary>
    /// Hoisted leading usings used to shift every body line up by the number
    /// of hoisted lines, and errors in the usings themselves reported
    /// generated line numbers.
    /// </summary>
    [Theory]
    [InlineData("int x = \"s\";", "CS0029 at 1:9")]
    [InlineData("using System.Text;\n\nint x = \"s\";", "CS0029 at 3:9")]
    [InlineData("\nusing System.Text;\nusing Nope.Missing;\nprint(1);", "CS0246 at 3:7")]
    public void Compile_errors_report_snippet_lines(string code, string expected)
    {
        JsonNode result = CompileSnippetResult(code);

        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Contains(expected, result["error"]!.GetValue<string>());
    }

    private static Dictionary<string, List<int>> VisibleLinesByMethod(byte[] image)
    {
        using var pe = new PEReader(new MemoryStream(image));
        DebugDirectoryEntry[] embedded = pe.ReadDebugDirectory()
            .Where(entry => entry.Type == DebugDirectoryEntryType.EmbeddedPortablePdb)
            .ToArray();
        Assert.Single(embedded);

        using MetadataReaderProvider provider = pe.ReadEmbeddedPortablePdbDebugDirectoryData(embedded[0]);
        MetadataReader pdb = provider.GetMetadataReader();
        MetadataReader metadata = pe.GetMetadataReader();

        var lines = new Dictionary<string, List<int>>();
        foreach (MethodDefinitionHandle method in metadata.MethodDefinitions)
        {
            string name = metadata.GetString(metadata.GetMethodDefinition(method).Name);
            if (!lines.TryGetValue(name, out List<int>? methodLines))
                lines[name] = methodLines = new List<int>();
            foreach (SequencePoint point in pdb.GetMethodDebugInformation(method.ToDebugInformationHandle()).GetSequencePoints())
            {
                if (point.IsHidden)
                    continue;
                Assert.Equal(
                    UnitySnippetSource.SourcePath,
                    pdb.GetString(pdb.GetDocument(point.Document).Name));
                methodLines.Add(point.StartLine);
            }
        }

        return lines;
    }

    private static byte[] CompileSnippet(string code)
    {
        JsonNode result = CompileSnippetResult(code);
        Assert.True(result["success"]!.GetValue<bool>(), result["error"]?.GetValue<string>());

        return Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
    }

    private static JsonNode CompileSnippetResult(string code)
    {
        var service = new CompileService();
        JsonNode reference = service.HandleCompileRaw(new JsonObject
        {
            ["sources"] = new JsonArray(new JsonObject { ["path"] = "UnityStubs.cs", ["text"] = UnityStubs }),
            ["useHostBcl"] = true,
            ["assemblyName"] = "SnippetDebugSymbolsUnityStubs",
            ["returnAssemblyPath"] = true,
        });
        Assert.True(reference["success"]!.GetValue<bool>(), reference["error"]?.GetValue<string>());

        IEnumerable<string> referencePaths = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Append(reference["assemblyPath"]!.GetValue<string>());
        JsonNode result = service.HandleCompileSnippet(new JsonObject
        {
            ["code"] = code,
            ["params"] = new JsonObject
            {
                ["fingerprint"] = "snippet-debug-symbols",
                ["domainGeneration"] = "snippet-debug-symbols",
                ["langVersion"] = "9",
                ["referencePaths"] = new JsonArray(referencePaths.Select(path => (JsonNode)path).ToArray()),
                ["defines"] = new JsonArray(),
            },
        });
        return result;
    }
}
