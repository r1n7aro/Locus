using System.Text.Json.Nodes;
using Xunit;

namespace Locus.CompileServer.Tests;

public class ScopedCompileServiceRegistryTests
{
    private static JsonObject Scope(
        string checkout,
        string editorSession,
        long workspaceGeneration = 1,
        long serviceGeneration = 1) => new()
    {
        ["scopeId"] = new JsonObject
        {
            ["checkoutId"] = checkout,
            ["workspaceGeneration"] = workspaceGeneration,
            ["serviceGeneration"] = serviceGeneration,
            ["unityEditorSessionId"] = editorSession,
        },
    };

    private static JsonNode CompileRaw(
        CompileService service,
        string path,
        string text,
        string generation,
        bool registerImage,
        bool referenceImages)
    {
        return service.HandleCompileRaw(new JsonObject
        {
            ["sources"] = new JsonArray(new JsonObject { ["path"] = path, ["text"] = text }),
            ["useHostBcl"] = true,
            ["registerImage"] = registerImage,
            ["referenceSessionImages"] = referenceImages,
            ["params"] = new JsonObject { ["domainGeneration"] = generation },
        });
    }

    [Fact]
    public void Different_checkout_editor_scopes_keep_image_registries_isolated()
    {
        var registry = new ScopedCompileServiceRegistry();
        var left = registry.GetOrCreate(Scope("checkout-left", "editor-left"));
        var right = registry.GetOrCreate(Scope("checkout-right", "editor-right"));
        const string generation = "same-domain-generation-for-isolation";

        JsonNode defined = CompileRaw(
            left.Service,
            "Defined.cs",
            "public class ScopedDefined { public int Value = 7; }",
            generation,
            registerImage: true,
            referenceImages: false);
        Assert.True(defined["success"]!.GetValue<bool>());

        const string consumer =
            "public class ScopedConsumer { public int Read() => new ScopedDefined().Value; }";
        JsonNode leftConsumer = CompileRaw(
            left.Service, "Left.cs", consumer, generation, false, true);
        JsonNode rightConsumer = CompileRaw(
            right.Service, "Right.cs", consumer, generation, false, true);

        Assert.True(leftConsumer["success"]!.GetValue<bool>());
        Assert.False(rightConsumer["success"]!.GetValue<bool>());
        Assert.Contains("CS0246", rightConsumer["error"]!.GetValue<string>());
    }

    [Fact]
    public async Task Release_removes_only_the_requested_scope()
    {
        var registry = new ScopedCompileServiceRegistry();
        JsonObject leftId = Scope("checkout-left", "editor-left");
        JsonObject rightId = Scope("checkout-right", "editor-right");
        registry.GetOrCreate(leftId);
        var right = registry.GetOrCreate(rightId);

        Assert.True(await registry.ReleaseAsync(leftId));
        Assert.Equal(1, registry.Count);
        Assert.Same(right, registry.GetOrCreate(rightId));
    }

    [Fact]
    public void Recreated_workspace_or_compile_service_gets_a_fresh_scope()
    {
        var registry = new ScopedCompileServiceRegistry();
        var first = registry.GetOrCreate(Scope("checkout", "editor", 7, 11));
        var recreatedWorkspace = registry.GetOrCreate(Scope("checkout", "editor", 8, 12));
        var restartedService = registry.GetOrCreate(Scope("checkout", "editor", 7, 13));

        Assert.NotSame(first, recreatedWorkspace);
        Assert.NotSame(first, restartedService);
        Assert.Equal(3, registry.Count);
    }

    [Theory]
    [InlineData("null")]
    [InlineData("{\"scopeId\":{}}")]
    [InlineData("{\"scopeId\":{\"checkoutId\":7,\"unityEditorSessionId\":\"editor\"}}")]
    [InlineData("{\"scopeId\":{\"checkoutId\":\"checkout\",\"workspaceGeneration\":1,\"serviceGeneration\":0,\"unityEditorSessionId\":\"editor\"}}")]
    public void Malformed_scope_is_reported_as_invalid_params(string json)
    {
        JsonNode? request = JsonNode.Parse(json);
        Assert.Throws<RpcInvalidParamsException>(() =>
            ScopedCompileServiceRegistry.ScopeKey(request));
    }
}
