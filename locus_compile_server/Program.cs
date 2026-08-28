// Locus C# compile server.
//
// A stdio JSON-RPC (Content-Length framed, LSP-style) sidecar that compiles
// Unity C# snippets / run_states state machines / arbitrary file sets with a
// modern Roslyn on CoreCLR, so the Unity Editor process only has to
// `Assembly.Load` the resulting bytes. See coreclr-compile-sidecar-plan.md.
//
// Protocol methods:
//   initialize        -> handshake (protocol + wrapper contract versions)
//   shutdown / exit   -> graceful stop
//   compile/raw       -> compile a set of in-memory sources to a DLL
//   compile/snippet   -> wrap + compile a unity_execute snippet
//   compile/runStates -> wrap + compile a unity_run_states state machine
//   analyze/hotDiff   -> classify edited files as hot-patchable or not
//   compile/hotPatch  -> diff + rewrite + compile a hot-patch assembly
//   compile/accessProbe -> compile the C0 runtime access-probe assembly
//   index/types       -> Unity type index built from reference metadata
//   index/schema      -> SerializedProperty schema built from reference metadata

using System.Globalization;
using System.Collections.Concurrent;
using System.Text.Json;
using System.Text.Json.Nodes;
using Locus.CompileServer;

// Keep every string the server produces locale-independent: agents parse
// compiler diagnostics verbatim. The project runs in invariant-globalization
// mode with English-only satellite resources, so the invariant culture is
// the only one available — and exactly what we want.
CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;
CultureInfo.DefaultThreadCurrentUICulture = CultureInfo.InvariantCulture;

var stdin = Console.OpenStandardInput();
var stdout = Console.OpenStandardOutput();
var transport = new StdioTransport(stdin, stdout);
var initializeService = new CompileService();
var scopes = new ScopedCompileServiceRegistry();
var inFlight = new List<Task>();
var inFlightLock = new object();
var requestCancellations = new ConcurrentDictionary<string, CancellationTokenSource>(
    StringComparer.Ordinal);

while (true)
{
    byte[]? body;
    try
    {
        body = transport.ReadMessage();
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine($"[LocusCompileServer] transport read failed: {ex.Message}");
        break;
    }

    if (body == null)
        break; // stdin closed: parent process is gone.

    JsonNode? message;
    try
    {
        message = JsonNode.Parse(body);
    }
    catch (JsonException ex)
    {
        Console.Error.WriteLine($"[LocusCompileServer] invalid JSON frame ignored: {ex.Message}");
        continue;
    }

    var id = message?["id"];
    var method = message?["method"]?.GetValue<string>();
    var @params = message?["params"];

    if (method == null)
        continue; // A response — the server never issues requests.

    if (method == "exit")
        break;

    if (method == "$/cancelRequest")
    {
        JsonNode? cancelledId = @params?["id"];
        if (cancelledId != null &&
            requestCancellations.TryGetValue(cancelledId.ToJsonString(), out var source))
        {
            try
            {
                source.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // The request completed between lookup and cancellation.
            }
        }
        continue;
    }

    if (id == null)
        continue; // Unknown notification.

    JsonNode requestId = id.DeepClone();
    string requestMethod = method;
    JsonNode? requestParams = @params?.DeepClone();
    string requestKey = requestId.ToJsonString();
    var cancellation = new CancellationTokenSource();
    requestCancellations[requestKey] = cancellation;
    Task dispatch = Task.Run(async delegate
    {
        JsonNode? result = null;
        JsonObject? error = null;
        ScopedCompileServiceRegistry.ScopeState? scope = null;
        bool scopeGateHeld = false;
        try
        {
            if (requestMethod == "initialize")
            {
                result = initializeService.HandleInitialize(requestParams);
            }
            else if (requestMethod == "shutdown")
            {
                result = null;
            }
            else if (requestMethod == "scope/release")
            {
                bool released = await scopes
                    .ReleaseAsync(requestParams, cancellation.Token)
                    .ConfigureAwait(false);
                result = new JsonObject { ["released"] = released };
            }
            else
            {
                scope = scopes.GetOrCreate(requestParams);
                await scope.RequestGate.WaitAsync(cancellation.Token).ConfigureAwait(false);
                scopeGateHeld = true;
                cancellation.Token.ThrowIfCancellationRequested();
                result = requestMethod switch
                {
                    "compile/raw" => scope.Service.HandleCompileRaw(requestParams),
                    "image/register" => scope.Service.HandleRegisterImage(requestParams),
                    "compile/snippet" => scope.Service.HandleCompileSnippet(requestParams),
                    "compile/runStates" => scope.Service.HandleCompileRunStates(requestParams),
                    "compile/viewScript" => scope.Service.HandleCompileViewScript(requestParams),
                    "analyze/hotDiff" => scope.Service.HandleAnalyzeHotDiff(requestParams),
                    "compile/hotPatch" => scope.Service.HandleCompileHotPatch(requestParams),
                    "compile/accessProbe" => scope.Service.HandleCompileAccessProbe(requestParams),
                    "caller/query" => scope.Service.HandleCallerQuery(requestParams),
                    "index/types" => scope.Service.HandleIndexTypes(requestParams),
                    "index/schema" => scope.Service.HandleIndexSchema(requestParams),
                    _ => throw new RpcMethodNotFoundException(requestMethod),
                };
                cancellation.Token.ThrowIfCancellationRequested();
            }
        }
        catch (RpcMethodNotFoundException ex)
        {
            error = RpcError(-32601, $"method not found: {ex.Method}");
        }
        catch (RpcInvalidParamsException ex)
        {
            error = RpcError(-32602, ex.Message);
        }
        catch (OperationCanceledException)
        {
            error = RpcError(-32800, $"request cancelled: {requestMethod}");
        }
        catch (Exception ex)
        {
            error = RpcError(-32603, $"internal error in {requestMethod}: {ex}");
        }
        finally
        {
            if (scopeGateHeld)
                scope!.RequestGate.Release();
            requestCancellations.TryRemove(requestKey, out _);
            cancellation.Dispose();
        }

        var response = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = requestId,
        };
        if (error != null)
            response["error"] = error;
        else
            response["result"] = result;

        try
        {
            transport.WriteMessage(JsonSerializer.SerializeToUtf8Bytes(response));
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[LocusCompileServer] transport write failed: {ex.Message}");
        }
    });
    lock (inFlightLock)
    {
        inFlight.RemoveAll(static task => task.IsCompleted);
        inFlight.Add(dispatch);
    }
}

Task[] remaining;
lock (inFlightLock)
    remaining = inFlight.ToArray();
await Task.WhenAll(remaining).ConfigureAwait(false);

return 0;

static JsonObject RpcError(int code, string message) =>
    new() { ["code"] = code, ["message"] = message };

sealed class RpcMethodNotFoundException : Exception
{
    public string Method { get; }
    public RpcMethodNotFoundException(string method) => Method = method;
}
