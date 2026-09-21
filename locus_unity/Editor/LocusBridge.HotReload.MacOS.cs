#if UNITY_EDITOR_OSX
using System.Threading.Tasks;

namespace Locus
{
    // macOS basic bridge deliberately does not load MonoMod or hot-patch
    // runtime assemblies. Keep the command surface explicit for older clients.
    public static partial class LocusBridge
    {
        private static Task<PipeEnvelope> MacHotReloadUnavailable(string requestId)
        {
            return Task.FromResult(ErrorResponse(requestId,
                "Unity hot reload and runtime probes are not supported on macOS. Use unity_recompile."));
        }

        private static Task<PipeEnvelope> HandleHotReloadProbe(string id) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadSetCodeOptimization(string id, string payload) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadSetDebug(string id) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadSetPlayModeReload(string id, string payload) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadAccessProbe(string id, string payload) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadInlineProbe(string id) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotReloadInliningActive(string id) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotPatchLoaded(string id, string payload) { return MacHotReloadUnavailable(id); }
        private static Task<PipeEnvelope> HandleHotPatchDispose(string id, string payload) { return MacHotReloadUnavailable(id); }
    }
}
#endif
