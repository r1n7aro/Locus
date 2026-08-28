using System.Collections.Concurrent;
using System.Text.Json.Nodes;

namespace Locus.CompileServer;

/// <summary>
/// Owns every stateful CompileService instance. A scope is one immutable
/// checkout runtime and compile-service generation plus one Unity Editor
/// process session; domain generations remain internal to that scope.
/// </summary>
public sealed class ScopedCompileServiceRegistry
{
    public sealed class ScopeState
    {
        public CompileService Service { get; } = new();
        public SemaphoreSlim RequestGate { get; } = new(1, 1);
    }

    private readonly ConcurrentDictionary<string, ScopeState> _scopes =
        new(StringComparer.Ordinal);

    public int Count => _scopes.Count;

    public ScopeState GetOrCreate(JsonNode? requestParams)
    {
        string key = ScopeKey(requestParams);
        return _scopes.GetOrAdd(key, static _ => new ScopeState());
    }

    public async Task<bool> ReleaseAsync(
        JsonNode? requestParams,
        CancellationToken cancellationToken = default)
    {
        string key = ScopeKey(requestParams);
        if (!_scopes.TryGetValue(key, out ScopeState? state))
            return false;

        await state.RequestGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            return _scopes.TryRemove(key, out _);
        }
        finally
        {
            state.RequestGate.Release();
        }
    }

    public static string ScopeKey(JsonNode? requestParams)
    {
        if (requestParams?["scopeId"] is not JsonObject scope)
        {
            throw new RpcInvalidParamsException(
                "scopeId checkout, workspace/service generations, and Unity editor session are required");
        }
        string checkoutId = RequiredScopeString(scope, "checkoutId");
        long workspaceGeneration = RequiredScopeGeneration(scope, "workspaceGeneration");
        long serviceGeneration = RequiredScopeGeneration(scope, "serviceGeneration");
        string editorSessionId = RequiredScopeString(scope, "unityEditorSessionId");
        return checkoutId + "\0" + workspaceGeneration + "\0" + serviceGeneration + "\0" + editorSessionId;
    }

    private static string RequiredScopeString(JsonObject scope, string name)
    {
        if (!scope.TryGetPropertyValue(name, out JsonNode? node) ||
            node is not JsonValue value ||
            !value.TryGetValue<string>(out string? text) ||
            string.IsNullOrWhiteSpace(text))
        {
            throw new RpcInvalidParamsException(
                "scopeId checkout, workspace/service generations, and Unity editor session are required");
        }
        return text.Trim();
    }

    private static long RequiredScopeGeneration(JsonObject scope, string name)
    {
        if (!scope.TryGetPropertyValue(name, out JsonNode? node) ||
            node is not JsonValue value ||
            !value.TryGetValue<long>(out long generation) ||
            generation <= 0)
        {
            throw new RpcInvalidParamsException(
                "scopeId checkout, workspace/service generations, and Unity editor session are required");
        }
        return generation;
    }
}
