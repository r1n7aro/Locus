using System.Text.Json.Nodes;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Behavior tests through the JSON-RPC handler layer, using the host BCL as
/// the reference set (no Unity assemblies available in CI).
/// </summary>
public class CompileServiceTests
{
    private static JsonNode CompileRaw(CompileService service, string path, string text, JsonObject? extra = null)
    {
        var request = new JsonObject
        {
            ["sources"] = new JsonArray(new JsonObject { ["path"] = path, ["text"] = text }),
            ["useHostBcl"] = true,
        };
        if (extra != null)
        {
            foreach (var pair in extra)
                request[pair.Key] = pair.Value?.DeepClone();
        }
        return service.HandleCompileRaw(request);
    }

    [Fact]
    public void Compile_raw_emits_a_pe_image()
    {
        var service = new CompileService();
        JsonNode result = CompileRaw(service, "A.cs", "class A { }", new JsonObject
        {
            ["assemblyName"] = "GoldenA",
        });

        Assert.True(result["success"]!.GetValue<bool>());
        Assert.Equal("GoldenA", result["assemblyName"]!.GetValue<string>());
        byte[] bytes = Convert.FromBase64String(result["assemblyB64"]!.GetValue<string>());
        Assert.True(bytes.Length > 512);
        Assert.Equal((byte)'M', bytes[0]);
        Assert.Equal((byte)'Z', bytes[1]);
    }

    /// <summary>
    /// The exact diagnostic framing is a contract with the agent prompts and
    /// must match the Unity-side BuildDiagnosticErrorText byte for byte.
    /// </summary>
    [Fact]
    public void Diagnostics_keep_the_legacy_compilation_failed_framing()
    {
        var service = new CompileService();
        JsonNode result = CompileRaw(service, "B.cs", "class B { void M() { int x = \"oops\"; } }");

        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Equal("compile", result["errorStage"]!.GetValue<string>());
        Assert.Equal(
            "compilation failed:\n  CS0029 at 1:30: Cannot implicitly convert type 'string' to 'int'\n",
            result["error"]!.GetValue<string>());
    }

    /// <summary>
    /// Diagnostics must honor #line mapping (the snippet wrapper resets the
    /// user body to line 1) — GetMappedLineSpan semantics.
    /// </summary>
    [Fact]
    public void Diagnostics_use_mapped_line_spans()
    {
        var service = new CompileService();
        string source = string.Join('\n', new[]
        {
            "class H",
            "{",
            "    static void M()",
            "    {",
            "#line 1",
            "int bad = \"x\";",
            "#line default",
            "    }",
            "}",
        });
        JsonNode result = CompileRaw(service, "H.cs", source);

        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Contains("CS0029 at 1:11:", result["error"]!.GetValue<string>());
    }

    /// <summary>
    /// Snippet compiles cannot succeed without the Unity reference set, but
    /// the two-attempt semantics and the combined error format (primary +
    /// "expression fallback:" section) must match the Unity-side
    /// CompileAsyncSnippet wording.
    /// </summary>
    [Fact]
    public void Snippet_failure_combines_both_modes_like_unity()
    {
        var service = new CompileService();
        var request = new JsonObject
        {
            ["code"] = "1 + 1",
            ["params"] = new JsonObject
            {
                ["fingerprint"] = "test",
                ["domainGeneration"] = "gen",
                ["langVersion"] = "9",
                // BCL-only references: the wrapper's Unity types cannot
                // resolve, so both modes fail deterministically.
                ["referencePaths"] = new JsonArray(typeof(object).Assembly.Location),
                ["defines"] = new JsonArray(),
            },
        };

        JsonNode result = service.HandleCompileSnippet(request);

        Assert.False(result["success"]!.GetValue<bool>());
        string error = result["error"]!.GetValue<string>();
        Assert.StartsWith("compilation failed:\n", error);
        Assert.Contains("\n\nexpression fallback:\ncompilation failed:\n", error);
    }

    [Fact]
    public void Lang_version_pins_to_csharp9()
    {
        var service = new CompileService();
        // File-scoped namespaces are C#10; under the pinned C#9 they must
        // produce an error (Unity parity: LanguageVersion.CSharp9).
        JsonNode result = CompileRaw(service, "N.cs", "namespace N;\nclass A { }");

        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Contains("CS8773", result["error"]!.GetValue<string>());
    }

    [Fact]
    public void Run_states_validation_fails_before_compiling()
    {
        var service = new CompileService();
        var request = new JsonObject
        {
            ["request"] = new JsonObject
            {
                ["request_editor_status"] = "playing",
                ["initial_state"] = "missing",
                ["states"] = new JsonArray(new JsonObject
                {
                    ["name"] = "main",
                    ["update"] = "ctx.Done(\"x\");",
                }),
            },
        };

        JsonNode result = service.HandleCompileRunStates(request);

        Assert.False(result["success"]!.GetValue<bool>());
        Assert.Equal("validation", result["errorStage"]!.GetValue<string>());
        Assert.Equal("initial_state not found in states: missing", result["error"]!.GetValue<string>());
    }

    // ── session image registry ───────────────────────────────────────

    [Fact]
    public void Session_images_resolve_within_one_generation_only()
    {
        var service = new CompileService();
        const string genOne = "11112222333344445555666677778888";
        const string genTwo = "99990000999900009999000099990000";

        JsonNode defined = CompileRaw(
            service,
            "A.cs",
            "public class SessionTypeA { public int Value = 7; }",
            new JsonObject
            {
                ["registerImage"] = true,
                ["params"] = new JsonObject { ["domainGeneration"] = genOne },
            });
        Assert.True(defined["success"]!.GetValue<bool>());

        const string consumer =
            "public class SessionTypeB { public int Read() { return new SessionTypeA().Value; } }";

        JsonNode sameGeneration = CompileRaw(service, "B.cs", consumer, new JsonObject
        {
            ["referenceSessionImages"] = true,
            ["params"] = new JsonObject { ["domainGeneration"] = genOne },
        });
        Assert.True(sameGeneration["success"]!.GetValue<bool>());

        JsonNode otherGeneration = CompileRaw(service, "B2.cs", consumer, new JsonObject
        {
            ["referenceSessionImages"] = true,
            ["params"] = new JsonObject { ["domainGeneration"] = genTwo },
        });
        Assert.False(otherGeneration["success"]!.GetValue<bool>());
        Assert.Contains("CS0246", otherGeneration["error"]!.GetValue<string>());

        // Registering under a new generation discards the old images.
        JsonNode newGenerationDefine = CompileRaw(service, "C.cs", "public class SessionTypeC { }", new JsonObject
        {
            ["registerImage"] = true,
            ["params"] = new JsonObject { ["domainGeneration"] = genTwo },
        });
        Assert.True(newGenerationDefine["success"]!.GetValue<bool>());

        JsonNode staleAfterReload = CompileRaw(service, "B3.cs", consumer, new JsonObject
        {
            ["referenceSessionImages"] = true,
            ["params"] = new JsonObject { ["domainGeneration"] = genOne },
        });
        Assert.False(staleAfterReload["success"]!.GetValue<bool>());
    }

    [Fact]
    public void Image_registry_isolates_generations()
    {
        var registry = new ImageRegistry();
        registry.Register("gen-1", "AsmA", new byte[] { 1, 2, 3 });

        Assert.Equal(1, registry.CountFor("gen-1"));
        Assert.Equal(0, registry.CountFor("gen-2"));
        Assert.Equal(0, registry.CountFor(null));

        registry.Register("gen-2", "AsmB", new byte[] { 4, 5, 6 });
        Assert.Equal(0, registry.CountFor("gen-1"));
        Assert.Equal(1, registry.CountFor("gen-2"));
    }

    [Fact]
    public void Assembly_names_embed_kind_generation_and_counter()
    {
        var service = new CompileService();
        JsonNode first = CompileRaw(service, "A.cs", "class A { }", new JsonObject
        {
            ["params"] = new JsonObject { ["domainGeneration"] = "abcdef0123456789" },
        });
        string name = first["assemblyName"]!.GetValue<string>();
        Assert.Matches("^__LocusRaw_abcdef01_[0-9A-F]{8}$", name);
    }
}
