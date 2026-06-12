using UnityEngine;
using UnityEditor.Compilation;

using System;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;

using MonoMod.RuntimeDetour;

namespace Locus
{
    // Hot reload support: the compile-server sidecar builds patch assemblies
    // from method-body level edits; this side loads them and redirects the
    // original methods with MonoMod detours, so changes take effect without
    // a script recompile or domain reload. See unity-hotreload-plan.md.
    public static partial class LocusBridge
    {
        // ───────────────── hot_reload_probe ─────────────────

        [Serializable]
        private sealed class HotReloadProbePayload
        {
            public bool detour_ok;
            public string code_optimization;
            public string detour_engine;
            public string error;
        }

        private static async Task<PipeEnvelope> HandleHotReloadProbe(string requestId)
        {
            var tcs = new TaskCompletionSource<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    var payload = new HotReloadProbePayload();
                    payload.code_optimization =
                        CompilationPipeline.codeOptimization == CodeOptimization.Debug
                            ? "debug"
                            : "release";

                    string engine;
                    string error;
                    payload.detour_ok = RunDetourSelfTest(out engine, out error);
                    payload.detour_engine = engine ?? "";
                    payload.error = error ?? "";

                    tcs.SetResult(OkResponse(requestId, JsonUtility.ToJson(payload)));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, "hot_reload_probe failed: " + ex.Message));
                }
            });
            return await tcs.Task;
        }

        // NoInlining so the reflection invocations below always go through
        // the patched native entry, regardless of the editor's own
        // optimization mode.
        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int HotReloadProbeOriginal()
        {
            return 1;
        }

        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int HotReloadProbeReplacement()
        {
            return 2;
        }

        /// <summary>
        /// Detour a dummy method, verify the redirect, dispose, and verify
        /// the restore — proves the bundled MonoMod engine works inside this
        /// editor's Mono runtime before any real patch is attempted.
        /// </summary>
        private static bool RunDetourSelfTest(out string engine, out string error)
        {
            engine = "";
            error = "";

            MethodInfo original = typeof(LocusBridge).GetMethod(
                "HotReloadProbeOriginal", BindingFlags.NonPublic | BindingFlags.Static);
            MethodInfo replacement = typeof(LocusBridge).GetMethod(
                "HotReloadProbeReplacement", BindingFlags.NonPublic | BindingFlags.Static);
            if (original == null || replacement == null)
            {
                error = "probe methods not found";
                return false;
            }

            IDisposable detour;
            try
            {
                detour = CreateMethodDetour(original, replacement, out engine);
            }
            catch (Exception ex)
            {
                error = "detour creation failed: " + ex.Message;
                return false;
            }

            try
            {
                int patched = (int)original.Invoke(null, null);
                if (patched != 2)
                {
                    error = "detour did not redirect (got " + patched + ")";
                    return false;
                }
            }
            catch (Exception ex)
            {
                error = "detoured invoke failed: " + ex.Message;
                return false;
            }
            finally
            {
                try { detour.Dispose(); } catch { }
            }

            try
            {
                int restored = (int)original.Invoke(null, null);
                if (restored != 1)
                {
                    error = "detour did not restore (got " + restored + ")";
                    return false;
                }
            }
            catch (Exception ex)
            {
                error = "restored invoke failed: " + ex.Message;
                return false;
            }

            return true;
        }

        /// <summary>
        /// Create a method redirection, preferring the managed Detour (which
        /// validates signatures and supports chaining) and falling back to
        /// NativeDetour — the raw entry-point jump — when Detour rejects the
        /// pair (e.g. instance methods whose `this` types differ between the
        /// original type and the rewritten patch type).
        /// </summary>
        private static IDisposable CreateMethodDetour(
            MethodBase original,
            MethodBase replacement,
            out string engine)
        {
            try
            {
                var detour = new Detour(original, replacement);
                engine = "detour";
                return detour;
            }
            catch (Exception)
            {
                var native = new NativeDetour(original, replacement);
                engine = "native_detour";
                return native;
            }
        }
    }
}
