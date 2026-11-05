using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis.CSharp;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Golden parity: the server's snippet source generators must produce
/// byte-identical output to the Unity-side implementation
/// (UnityReferenceImpl) for any input, in both wrapper modes.
/// </summary>
public class SnippetSourceParityTests
{
    private static readonly CSharpParseOptions ParseOptions = new(
        languageVersion: LanguageVersion.CSharp9);

    public static TheoryData<string> SnippetInputs => new()
    {
        "",
        "1 + 1",
        "var x = 1;\nreturn x;",
        "using UnityEngine.Rendering;\n\nvar c = new Color(1, 0, 0);\nprint(c);",
        "using A.B;\nusing C;\n\n// comment, not a using\nusing (var d = new MemoryStream()) { }\nreturn 2;",
        "\r\nusing X.Y;\r\nvar v = GameObject.Find(\"Player\");\r\nprint(v);\r\n",
        "   \n\nusing Z;\nreturn 0;",
        "print(\"unicode 中文 🚀\");",
        "using NotTerminated\nvar tail = 1;",
        "// leading comment\nusing After.Comment;\nreturn 1;",
        "var values = new[] { 41 };\nref int value = ref values[0];\nvalue++;\nprint(value);",
        "await ctx.WaitFrame();\nprint(\"done\");",
        "using var replay = Open(@\"F:/a.dereplay\");\nif (!replay.Success) return null;\nprintJson(replay.Summary);",
        "using static UnityEngine.Mathf;\nusing L = System.Collections.Generic.List<int>;\n\nusing FileStream fs = File.OpenRead(p);\nprint(fs.Length);",
        "using (var s = Open()) Use(s);\nreturn 0;",
        "using A; using B;\nreturn 0;",
    };

    public static TheoryData<string> UsingStatementLines => new()
    {
        "using var replay = DustEcho.Replay.ReplayAgentApi.Open(@\"F:/a.dereplay\");",
        "using var x = a == b;",
        "using FileStream fs = File.OpenRead(p);",
        "using System.Collections.Generic.List<int> xs = new();",
        "using (var s = Open()) Use(s);",
    };

    public static TheoryData<string> UsingDirectiveLines => new()
    {
        "using A.B;",
        "using global::A.B;",
        "using static UnityEngine.Mathf;",
        "using L = System.Collections.Generic.List<int>;",
        "using @var = Foo;",
        "using A; using B;",
        // Malformed directives keep being hoisted so their errors stay as before.
        "using Foo.;",
    };

    public static TheoryData<string, bool> AwaitInputs => new()
    {
        { "print(\"done\");", false },
        { "var text = \"await ctx.wait\"; // await ctx.wait", false },
        { "ref int value = ref values[0];", false },
        { "async Task Local() { await Task.Yield(); }\nprint(\"done\");", false },
        { "Func<Task> run = async () => await Task.Yield();", false },
        { "#if ENABLE_AWAIT\nawait ctx.wait;\n#endif\nprint(\"done\");", false },
        { "await ctx.wait;", true },
        { "await foreach (var item in items) { print(item); }", true },
        { "await using var item = resource;", true },
    };

    [Theory]
    [MemberData(nameof(SnippetInputs))]
    public void SplitLeadingUsings_matches_unity(string code)
    {
        UnityReferenceImpl.SplitLeadingUsings(code, out string expectedUsings, out string expectedBody);
        UnitySnippetSource.SplitLeadingUsings(code, out string actualUsings, out string actualBody);

        Assert.Equal(expectedUsings, actualUsings);
        Assert.Equal(expectedBody, actualBody);
    }

    [Theory]
    [MemberData(nameof(UsingStatementLines))]
    public void Leading_using_statements_stay_in_the_snippet_body(string line)
    {
        string code = line + "\nreturn null;";
        UnitySnippetSource.SplitLeadingUsings(code, out string leadingUsings, out string bodyCode);

        Assert.Equal("", leadingUsings);
        Assert.Equal(code, NormalizeNewlines(bodyCode));
    }

    [Theory]
    [MemberData(nameof(UsingDirectiveLines))]
    public void Leading_using_directives_are_hoisted(string line)
    {
        UnitySnippetSource.SplitLeadingUsings(line + "\nreturn null;", out string leadingUsings, out string bodyCode);

        Assert.Equal(line, leadingUsings);
        Assert.Equal("\nreturn null;", NormalizeNewlines(bodyCode));
    }

    [Fact]
    public void Directives_before_a_using_declaration_are_hoisted_alone()
    {
        UnitySnippetSource.SplitLeadingUsings(
            "using static UnityEngine.Mathf;\nusing var d = Open();\nprint(d);",
            out string leadingUsings,
            out string bodyCode);

        Assert.Equal("using static UnityEngine.Mathf;", leadingUsings);
        Assert.Equal("\nusing var d = Open();\nprint(d);", NormalizeNewlines(bodyCode));
    }

    /// <summary>
    /// Both halves start at `#line 1` in the generated source, so each must
    /// keep every snippet line at its original index.
    /// </summary>
    [Fact]
    public void Split_keeps_original_line_numbers_on_both_sides()
    {
        string[] snippet =
        {
            "",
            "using A.B;",
            "",
            "using C;",
            "var x = 1;",
            "print(x);",
        };
        UnitySnippetSource.SplitLeadingUsings(string.Join("\n", snippet), out string leadingUsings, out string bodyCode);

        string[] usingLines = NormalizeNewlines(leadingUsings).Split('\n');
        string[] bodyLines = NormalizeNewlines(bodyCode).Split('\n');
        Assert.Equal(new[] { "", "using A.B;", "", "using C;" }, usingLines);
        Assert.Equal(snippet.Length, bodyLines.Length);
        Assert.All(bodyLines.Take(4), line => Assert.Equal("", line));
        Assert.Equal(snippet.Skip(4), bodyLines.Skip(4));
    }

    /// <summary>
    /// Hoisted above the snippet host, a leading using declaration became a
    /// top-level statement (CS8805) whose local the body could not see
    /// (CS8801). It must compile inside the snippet method instead.
    /// </summary>
    [Fact]
    public void Leading_using_declaration_compiles_inside_the_snippet_method()
    {
        var service = new CompileService();
        var request = new JsonObject
        {
            ["code"] = "using var stream = new MemoryStream();\nif (stream.Length != 0) return null;\nprint(stream.Length);",
            ["params"] = new JsonObject
            {
                ["fingerprint"] = "test",
                ["domainGeneration"] = "gen",
                ["langVersion"] = "9",
                // BCL-only references: the wrapper's Unity types still fail,
                // but no diagnostic may come from the hoisting itself.
                ["referencePaths"] = new JsonArray(typeof(object).Assembly.Location),
                ["defines"] = new JsonArray(),
            },
        };

        string error = service.HandleCompileSnippet(request)["error"]!.GetValue<string>();

        Assert.DoesNotContain("CS8805", error);
        Assert.DoesNotContain("CS8801", error);
    }

    private static string NormalizeNewlines(string text) => text.Replace("\r\n", "\n");

    [Theory]
    [MemberData(nameof(SnippetInputs))]
    public void BuildAsyncSnippetSource_matches_unity_in_both_modes(string code)
    {
        UnityReferenceImpl.SplitLeadingUsings(code, out string leadingUsings, out string bodyCode);
        bool expectedUsesAsync = UnityReferenceImpl.RequiresAsyncWrapper(bodyCode, ParseOptions);
        bool actualUsesAsync = UnitySnippetSource.RequiresAsyncWrapper(bodyCode, ParseOptions);
        Assert.Equal(expectedUsesAsync, actualUsesAsync);

        foreach (bool expressionMode in new[] { false, true })
        {
            string expected = UnityReferenceImpl.BuildAsyncSnippetSource(
                "__LocusAsyncSnippetHost",
                leadingUsings,
                bodyCode,
                expressionMode,
                expectedUsesAsync);
            string actual = UnitySnippetSource.BuildAsyncSnippetSource(
                "__LocusAsyncSnippetHost",
                leadingUsings,
                bodyCode,
                expressionMode,
                actualUsesAsync);

            Assert.Equal(expected, actual);
        }
    }

    [Theory]
    [MemberData(nameof(AwaitInputs))]
    public void RequiresAsyncWrapper_only_for_outer_await(string code, bool expected)
    {
        Assert.Equal(expected, UnitySnippetSource.RequiresAsyncWrapper(code, ParseOptions));
    }

    [Fact]
    public void No_await_uses_sync_body_behind_the_task_entry_contract()
    {
        string source = UnitySnippetSource.BuildAsyncSnippetSource(
            "__LocusAsyncSnippetHost",
            "",
            "ref int value = ref values[0];",
            expressionMode: false,
            useAsyncWrapper: false);

        Assert.Contains("public static global::System.Threading.Tasks.Task<object> ExecuteAsync", source);
        Assert.Contains("private static object ExecuteSync", source);
        Assert.DoesNotContain("public static async global::System.Threading.Tasks.Task<object>", source);
    }

    [Fact]
    public void Outer_await_keeps_the_async_body()
    {
        string source = UnitySnippetSource.BuildAsyncSnippetSource(
            "__LocusAsyncSnippetHost",
            "",
            "await ctx.wait;",
            expressionMode: false,
            useAsyncWrapper: true);

        Assert.Contains("public static async global::System.Threading.Tasks.Task<object> ExecuteAsync", source);
        Assert.DoesNotContain("private static object ExecuteSync", source);
    }

    [Fact]
    public void Common_io_types_use_aliases_without_importing_system_io()
    {
        string source = UnitySnippetSource.BuildAsyncSnippetSource(
            "__LocusAsyncSnippetHost",
            "",
            "print(Path.GetTempPath());",
            expressionMode: false,
            useAsyncWrapper: false);

        Assert.DoesNotContain("using System.IO;", source);
        foreach (string typeName in new[]
        {
            "BinaryReader", "BinaryWriter", "BufferedStream", "Directory", "DirectoryInfo",
            "DirectoryNotFoundException", "EndOfStreamException", "File", "FileAccess",
            "FileAttributes", "FileInfo", "FileMode", "FileNotFoundException", "FileOptions",
            "FileShare", "FileStream", "FileSystemInfo", "IOException", "MemoryStream", "Path",
            "SearchOption", "SeekOrigin", "Stream", "StreamReader", "StreamWriter", "StringReader",
            "StringWriter", "TextReader", "TextWriter",
        })
        {
            Assert.Contains($"using {typeName} = global::System.IO.{typeName};", source);
        }
    }

    [Fact]
    public void Host_type_names_match_the_unity_contract()
    {
        Assert.Equal("__LocusAsyncSnippetHost", UnitySnippetSource.HostTypeName);
        Assert.Equal(
            "Locus.RuntimeSnippets.__LocusAsyncSnippetHost",
            UnitySnippetSource.FullHostTypeName);
        Assert.Equal("LocusRuntimeAsyncSnippet.cs", UnitySnippetSource.SourcePath);
    }
}
