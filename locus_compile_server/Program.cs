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

using System.Globalization;
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
var service = new CompileService();
var shutdownRequested = false;

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

    if (id == null)
        continue; // Unknown notification.

    JsonNode? result = null;
    JsonObject? error = null;
    try
    {
        switch (method)
        {
            case "initialize":
                result = service.HandleInitialize(@params);
                break;
            case "shutdown":
                shutdownRequested = true;
                result = null;
                break;
            case "compile/raw":
                result = service.HandleCompileRaw(@params);
                break;
            case "image/register":
                result = service.HandleRegisterImage(@params);
                break;
            case "compile/snippet":
                result = service.HandleCompileSnippet(@params);
                break;
            case "compile/runStates":
                result = service.HandleCompileRunStates(@params);
                break;
            case "compile/viewScript":
                result = service.HandleCompileViewScript(@params);
                break;
            case "analyze/hotDiff":
                result = service.HandleAnalyzeHotDiff(@params);
                break;
            case "compile/hotPatch":
                result = service.HandleCompileHotPatch(@params);
                break;
            case "compile/accessProbe":
                result = service.HandleCompileAccessProbe(@params);
                break;
            case "index/types":
                result = service.HandleIndexTypes(@params);
                break;
            default:
                error = RpcError(-32601, $"method not found: {method}");
                break;
        }
    }
    catch (RpcInvalidParamsException ex)
    {
        error = RpcError(-32602, ex.Message);
    }
    catch (Exception ex)
    {
        error = RpcError(-32603, $"internal error in {method}: {ex}");
    }

    var response = new JsonObject
    {
        ["jsonrpc"] = "2.0",
        ["id"] = id.DeepClone(),
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
        break;
    }

    if (shutdownRequested && method == "shutdown")
        continue; // Keep draining until `exit` so the response flushes first.
}

return 0;

static JsonObject RpcError(int code, string message) =>
    new() { ["code"] = code, ["message"] = message };
