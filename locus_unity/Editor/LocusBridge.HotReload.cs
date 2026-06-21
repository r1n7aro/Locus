using UnityEngine;
using UnityEditor.Compilation;

using System;
using System.Collections.Generic;
using System.Linq.Expressions;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;

using MonoMod.RuntimeDetour;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    // Hot reload support: the compile-server sidecar builds patch assemblies
    // from method-body level edits; this side loads them and redirects the
    // original methods with MonoMod detours, so changes take effect without
    // a script recompile or domain reload. See unity-hotreload-plan.md.
    public static partial class LocusBridge
    {
        // ───────────────── patch registry ─────────────────

        private sealed class HotPatchDetourEntry
        {
            public IDisposable Detour;
            public string PatchId;
            public string Engine;
            public MethodBase Original;
            public MethodBase Patch;
        }

        private sealed class HotPatchApplyChange
        {
            // Report identity (declaring type + signature), byte-identical with
            // the Rust unity_method_key for the inlined-key round-trip.
            public string MethodKey;
            // Detour-cache identity: MethodKey plus the resolved original
            // assembly, so same-named methods in different assemblies do not
            // share a cache slot (see DetourKey).
            public string CacheKey;
            public HotPatchDetourEntry NewEntry;
            public HotPatchDetourEntry PreviousEntry;
        }

        // Active detour per ORIGINAL (method, assembly) key. Re-patching the same
        // method has one live redirect at a time; failed patch batches restore any
        // detours they temporarily superseded.
        private static readonly object _hotPatchLock = new object();
        private static readonly Dictionary<string, HotPatchDetourEntry> _hotMethodDetours =
            new Dictionary<string, HotPatchDetourEntry>(StringComparer.Ordinal);

        // ───────────────── hot_reload_probe ─────────────────

        [Serializable]
        private sealed class HotReloadProbePayload
        {
            public bool detour_ok;
            public string code_optimization;
            public bool domain_reload_on_play;
            public string detour_engine;
            public string error;
        }

        private static async Task<PipeEnvelope> HandleHotReloadProbe(string requestId)
        {
            try
            {
                return await LocusAsync.RunOnMainThreadAsync<PipeEnvelope>(delegate
                {
                    var payload = new HotReloadProbePayload();
                    payload.code_optimization =
                        CompilationPipeline.codeOptimization == CodeOptimization.Debug
                            ? "debug"
                            : "release";
                    payload.domain_reload_on_play = ReadDomainReloadOnPlay();

                    string engine;
                    string error;
                    payload.detour_ok = RunDetourSelfTest(out engine, out error);
                    payload.detour_engine = engine ?? "";
                    payload.error = error ?? "";

                    return OkResponse(requestId, JsonUtility.ToJson(payload));
                }, ExecuteTimeoutMs);
            }
            catch (TimeoutException)
            {
                return ErrorResponse(requestId, "hot_reload_probe timed out");
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_reload_probe failed: " + ex.Message);
            }
        }

        // ───────────────── hot_reload_set_debug ─────────────────

        [Serializable]
        private sealed class CodeOptimizationDto
        {
            public string code_optimization;
        }

        private static bool TryParseCodeOptimization(
            string requestJson,
            out CodeOptimization optimization,
            out string error)
        {
            optimization = CodeOptimization.Debug;
            error = null;

            string desired = (requestJson ?? "").Trim();
            if (desired.StartsWith("{", StringComparison.Ordinal))
            {
                try
                {
                    CodeOptimizationDto request = JsonUtility.FromJson<CodeOptimizationDto>(desired);
                    desired = request != null ? request.code_optimization : "";
                }
                catch (Exception ex)
                {
                    error = "Code Optimization request parse failed: " + ex.Message;
                    return false;
                }
            }

            if (string.Equals(desired, "debug", StringComparison.OrdinalIgnoreCase))
            {
                optimization = CodeOptimization.Debug;
                return true;
            }
            if (string.Equals(desired, "release", StringComparison.OrdinalIgnoreCase))
            {
                optimization = CodeOptimization.Release;
                return true;
            }

            error = "Code Optimization must be 'debug' or 'release'";
            return false;
        }

        /// <summary>
        /// Switch the editor's Code Optimization. Same effect as clicking the
        /// bug icon in the status bar — Unity schedules a script recompile.
        /// The assignment and read-back are synchronous, so the response
        /// carries the resulting value before the recompile is processed.
        /// </summary>
        private static async Task<PipeEnvelope> HandleHotReloadSetCodeOptimization(
            string requestId,
            string requestJson)
        {
            CodeOptimization desired;
            string parseError;
            if (!TryParseCodeOptimization(requestJson, out desired, out parseError))
                return ErrorResponse(requestId, parseError);

            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    if (CompilationPipeline.codeOptimization != desired)
                        CompilationPipeline.codeOptimization = desired;

                    var payload = new CodeOptimizationDto();
                    payload.code_optimization =
                        CompilationPipeline.codeOptimization == CodeOptimization.Debug
                            ? "debug"
                            : "release";
                    tcs.SetResult(OkResponse(requestId, JsonUtility.ToJson(payload)));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, "hot_reload_set_debug failed: " + ex.Message));
                }
            });
            return await tcs.Task;
        }

        private static Task<PipeEnvelope> HandleHotReloadSetDebug(string requestId)
        {
            return HandleHotReloadSetCodeOptimization(requestId, "debug");
        }

        // ───────────────── hot_reload_set_play_mode_reload ─────────────────

        [Serializable]
        private sealed class PlayModeReloadDto
        {
            public bool domain_reload_on_play;
        }

        /// <summary>
        /// Whether entering Play Mode reloads the managed domain. Unity reloads
        /// UNLESS Enter Play Mode Options are enabled AND DisableDomainReload is
        /// set; we report the EFFECTIVE behavior so the popover toggle matches
        /// what actually happens on Play.
        /// </summary>
        private static bool ReadDomainReloadOnPlay()
        {
            if (!UnityEditor.EditorSettings.enterPlayModeOptionsEnabled)
                return true;
            return (UnityEditor.EditorSettings.enterPlayModeOptions
                    & UnityEditor.EnterPlayModeOptions.DisableDomainReload) == 0;
        }

        /// <summary>
        /// Flip EditorSettings so entering Play Mode does (or skips) a domain
        /// reload, touching ONLY the DisableDomainReload bit — the user's
        /// scene-reload choice is preserved. Disabling the reload requires the
        /// options to be enabled for the flag to take effect.
        /// </summary>
        private static void ApplyDomainReloadOnPlay(bool domainReload)
        {
            UnityEditor.EnterPlayModeOptions options =
                UnityEditor.EditorSettings.enterPlayModeOptions;
            if (domainReload)
            {
                options &= ~UnityEditor.EnterPlayModeOptions.DisableDomainReload;
                UnityEditor.EditorSettings.enterPlayModeOptions = options;
            }
            else
            {
                UnityEditor.EditorSettings.enterPlayModeOptionsEnabled = true;
                options |= UnityEditor.EnterPlayModeOptions.DisableDomainReload;
                UnityEditor.EditorSettings.enterPlayModeOptions = options;
            }
        }

        private static bool TryParsePlayModeReload(
            string requestJson,
            out bool domainReload,
            out string error)
        {
            domainReload = true;
            error = null;

            string desired = (requestJson ?? "").Trim();
            if (desired.StartsWith("{", StringComparison.Ordinal))
            {
                try
                {
                    PlayModeReloadDto request = JsonUtility.FromJson<PlayModeReloadDto>(desired);
                    domainReload = request != null && request.domain_reload_on_play;
                    return true;
                }
                catch (Exception ex)
                {
                    error = "Play Mode reload request parse failed: " + ex.Message;
                    return false;
                }
            }

            if (string.Equals(desired, "on", StringComparison.OrdinalIgnoreCase)
                || string.Equals(desired, "true", StringComparison.OrdinalIgnoreCase))
            {
                domainReload = true;
                return true;
            }
            if (string.Equals(desired, "off", StringComparison.OrdinalIgnoreCase)
                || string.Equals(desired, "false", StringComparison.OrdinalIgnoreCase))
            {
                domainReload = false;
                return true;
            }

            error = "Play Mode reload must be 'on' or 'off'";
            return false;
        }

        /// <summary>
        /// Set whether entering Play Mode reloads the domain. Unlike a Code
        /// Optimization switch this does NOT schedule a recompile; the assignment
        /// and read-back are synchronous, so the response carries the resulting
        /// effective value.
        /// </summary>
        private static async Task<PipeEnvelope> HandleHotReloadSetPlayModeReload(
            string requestId,
            string requestJson)
        {
            bool desired;
            string parseError;
            if (!TryParsePlayModeReload(requestJson, out desired, out parseError))
                return ErrorResponse(requestId, parseError);

            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    ApplyDomainReloadOnPlay(desired);

                    var payload = new PlayModeReloadDto();
                    payload.domain_reload_on_play = ReadDomainReloadOnPlay();
                    tcs.SetResult(OkResponse(requestId, JsonUtility.ToJson(payload)));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId,
                        "hot_reload_set_play_mode_reload failed: " + ex.Message));
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

        // ───────────────── hot_patch_loaded ─────────────────

        [Serializable]
        private sealed class HotPatchMethodDto
        {
            public string declaring_type;
            public string patch_declaring_type;
            public string name;
            public string[] param_type_names;

            // Enriched per-parameter identity (namespace + closed generic
            // arguments) parallel to param_type_names. Present from newer
            // sidecars; used only to break a same-simple-name overload tie.
            public string[] param_type_sigs;
            public bool is_static;
            public bool is_ctor;

            // The edited file this detour came from — used to clear that file's
            // stale message-pump registrations before re-adding (replace-by-source).
            public string source_path;

            // When non-empty, the "original" method lives in this exact
            // assembly — an earlier patch's shim being re-edited. Resolution
            // then bypasses the usual skip of __LocusHotPatch_ assemblies.
            public string original_assembly;
        }

        // A newly added Unity message the engine never dispatches after load. The
        // runtime wires a driver by `kind`: "player_loop" (Update/LateUpdate/
        // FixedUpdate, driven each frame by a pump) or "component_proxy" (physics/
        // trigger events forwarded by a proxy MonoBehaviour on the target object).
        [Serializable]
        private sealed class MessageDriverDto
        {
            public string kind;             // "player_loop" | "component_proxy"
            public string declaring_type;   // original type whose live instances are driven
            public string shim_type;        // static shim class in the patch assembly
            public string shim_method;      // shim method (leading parameter is the instance)
            public string message;          // e.g. "Update", "OnTriggerEnter"
            public string param_type;       // engine arg type for component_proxy ("Collider"); empty for player_loop
            public string source_path;      // edited file (for replace-by-source teardown)
        }

        [Serializable]
        private sealed class HotPatchLoadedRequest
        {
            public string patch_id;
            public string assembly_b64;
            public string assembly_path;
            public string domain_generation;
            public HotPatchMethodDto[] methods;

            // Newly added Unity messages the engine never discovers after load,
            // each tagged with a driver kind. Absent on older sidecars.
            public MessageDriverDto[] message_drivers;

            // Experimental (Phase B, default off): when true, inline-risk
            // classification may JIT a synthetic caller stub to force Mono to
            // evaluate a not-yet-evaluated callee instead of relying on the static
            // heuristic. Delivered from the desktop config
            // unity_inline_force_evaluate_enabled; absent (→ false) on older desktops.
            public bool inline_force_evaluate;
        }

        [Serializable]
        private sealed class HotPatchLoadedResponse
        {
            public string patch_id;
            public int method_count;
            public string detour_engine;

            // MethodKey identities Unity inlined in Release: their detours are
            // live but bypassed at inlined call sites, so the desktop queues a
            // convergence recompile for them. Empty in Debug / when none.
            public string[] inlined_method_keys;

            // Parallel to inlined_method_keys (same order and length): the
            // InlineRiskSource that flagged each entry — "RuntimeInlined" (Mono's
            // cached bit), "StubInlined" (force-evaluated) or "Predicted" (static
            // heuristic). Lets the desktop word convergence by confidence; older
            // desktops ignore the unknown field.
            public string[] inlined_sources;

            // Message-driver capability echo. The desktop fails closed (recompile)
            // when it sent message_drivers but pump_supported is absent/false — so a
            // plugin that can't drive added messages never reports them live.
            // pump_skipped_count = added messages the runtime did NOT drive (not a
            // MonoBehaviour, parameter not the real engine type, edit-time-only, or a
            // catch-up with no live instance). pump_skipped_messages carries each
            // one's "message — reason" so the desktop reports them by name instead of
            // a single vague count; older desktops ignore the unknown field.
            public bool pump_supported;
            public int pumped_count;
            public int pump_skipped_count;
            public string[] pump_skipped_messages;
            // Hard wiring failures (type/shim missing, unbindable shim). When > 0
            // the desktop fails closed to a recompile rather than reporting success.
            public int pump_failed_count;
        }

        /// <summary>
        /// Load a sidecar-compiled hot-patch assembly and redirect each
        /// original method to its patch counterpart. All-or-nothing per
        /// patch: any resolution/detour failure rolls back this patch's
        /// detours and reports an error (the Rust side queues a real
        /// recompile, which always converges).
        /// </summary>
        private static async Task<PipeEnvelope> HandleHotPatchLoaded(string requestId, string requestJson)
        {
            if (string.IsNullOrEmpty(requestJson))
                return ErrorResponse(requestId, "empty hot_patch_loaded request");

            HotPatchLoadedRequest request;
            try
            {
                request = JsonUtility.FromJson<HotPatchLoadedRequest>(requestJson);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_patch_loaded request parse failed: " + ex.Message);
            }

            if (request == null ||
                (string.IsNullOrEmpty(request.assembly_b64) &&
                 string.IsNullOrEmpty(request.assembly_path)))
                return ErrorResponse(requestId, "hot_patch_loaded request missing assembly bytes");
            if (request.methods == null)
                request.methods = new HotPatchMethodDto[0];

            if (!string.IsNullOrEmpty(request.domain_generation) &&
                !string.Equals(request.domain_generation, _compileDomainGeneration, StringComparison.Ordinal))
            {
                return ErrorResponse(
                    requestId,
                    "hot patch was compiled for a previous domain generation; re-run after the reload settles");
            }

            byte[] assemblyBytes;
            try
            {
                assemblyBytes = ReadAssemblyPayload(request.assembly_b64, request.assembly_path);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_patch_loaded assembly load failed: " + ex.Message);
            }

            string patchId = string.IsNullOrEmpty(request.patch_id) ? Guid.NewGuid().ToString("N") : request.patch_id;

            // Apply on the main thread, between frames: the whole patch
            // lands atomically with respect to Update loops.
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    tcs.SetResult(ApplyHotPatchOnMainThread(requestId, patchId, assemblyBytes, request.methods, request.message_drivers, request.inline_force_evaluate));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, "hot patch apply failed: " + ex));
                }
            });
            return await tcs.Task;
        }

        private static PipeEnvelope ApplyHotPatchOnMainThread(
            string requestId,
            string patchId,
            byte[] assemblyBytes,
            HotPatchMethodDto[] methods,
            MessageDriverDto[] messageDrivers,
            bool forceEvaluateInline)
        {
            // Release-first: apply detours regardless of Code Optimization. In
            // Release, Mono inlines some small methods, whose inlined call sites
            // bypass the detour; we detect those after applying and converge
            // them with a recompile (see below) rather than refusing the patch.
            Assembly patchAssembly;
            try
            {
                patchAssembly = Assembly.Load(assemblyBytes);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot patch assembly load failed: " + ex.Message);
            }

            var applied = new List<HotPatchApplyChange>(methods.Length);
            string engineSummary = null;
            int pumpedCount = 0;
            var pumpSkippedDetails = new List<string>();
            int pumpFailedCount = 0;

            lock (_hotPatchLock)
            {
                foreach (HotPatchMethodDto dto in methods)
                {
                    string error;
                    MethodBase original = ResolveOriginalMethod(dto, out error);
                    if (original == null)
                    {
                        RollbackHotPatch(applied);
                        return ErrorResponse(requestId, "hot patch could not resolve " + DescribeMethod(dto) + ": " + error);
                    }

                    MethodBase patch = ResolvePatchMethod(patchAssembly, dto, out error);
                    if (patch == null)
                    {
                        RollbackHotPatch(applied);
                        return ErrorResponse(requestId, "hot patch missing patched " + DescribeMethod(dto) + ": " + error);
                    }

                    if (!ValidateDetourSignature(original, patch, out error))
                    {
                        RollbackHotPatch(applied);
                        return ErrorResponse(requestId, "hot patch signature mismatch for " + DescribeMethod(dto) + ": " + error);
                    }

                    string methodKey = MethodKey(dto);
                    string cacheKey = DetourKey(dto);
                    HotPatchDetourEntry previous;
                    if (_hotMethodDetours.TryGetValue(cacheKey, out previous))
                    {
                        try { previous.Detour.Dispose(); } catch { }
                        _hotMethodDetours.Remove(cacheKey);
                    }

                    HotPatchDetourEntry entry;
                    try
                    {
                        string engine;
                        IDisposable detour = CreateMethodDetour(original, patch, out engine);
                        entry = new HotPatchDetourEntry
                        {
                            Detour = detour,
                            PatchId = patchId,
                            Engine = engine,
                            Original = original,
                            Patch = patch,
                        };
                    }
                    catch (Exception ex)
                    {
                        string restoreError;
                        if (previous != null && !RestorePreviousDetour(cacheKey, previous, out restoreError))
                            Debug.LogError("[Locus] Failed to restore superseded hot patch for " + cacheKey + ": " + restoreError);
                        RollbackHotPatch(applied);
                        return ErrorResponse(requestId, "detour failed for " + DescribeMethod(dto) + ": " + ex.Message);
                    }

                    _hotMethodDetours[cacheKey] = entry;
                    applied.Add(new HotPatchApplyChange
                    {
                        MethodKey = methodKey,
                        CacheKey = cacheKey,
                        NewEntry = entry,
                        PreviousEntry = previous,
                    });
                    engineSummary = engineSummary == null || engineSummary == entry.Engine
                        ? entry.Engine
                        : "mixed";
                }

                // Fail closed on shim JIT-ability: shims are direct-called
                // (no detour pre-JITs them), so an access-check violation
                // the compiler waved through would otherwise surface as a
                // runtime exception at the first call — long after this
                // patch reported success. Force-JIT every shim now and roll
                // the whole batch back on failure.
                string shimError = PrepareHotPatchShims(patchAssembly);
                if (shimError != null)
                {
                    RollbackHotPatch(applied);
                    return ErrorResponse(requestId, "shim verification failed: " + shimError);
                }

                // Newly added Unity messages the engine never discovers after
                // load: wire each to its driver (PlayerLoop pump or component
                // proxy) by kind, replacing any prior registration per source file.
                RegisterMessageDrivers(methods, messageDrivers, patchAssembly, out pumpedCount, pumpSkippedDetails, out pumpFailedCount);
            }

            // Release-first: a method Unity inlined keeps a live detour, but its
            // inlined call sites bypass it, so the patch won't take effect there
            // until a recompile. Report those originals (with the source that
            // flagged each) so the desktop can queue a convergence recompile and
            // word it by confidence. Skip ctors and compiler-generated members
            // (state machines / lambdas), mirroring the reference plugin. Debug
            // never inlines, so the Release flag gates ClassifyInlineRisk's static
            // fallback (a not-yet-JIT-evaluated method only matters in Release).
            bool releaseMode = CompilationPipeline.codeOptimization == CodeOptimization.Release;
            var inlinedKeys = new List<string>();
            var inlinedSources = new List<string>();
            foreach (HotPatchApplyChange change in applied)
            {
                MethodBase original = change.NewEntry.Original;
                if (original == null || original is ConstructorInfo)
                    continue;
                bool synthesized =
                    (original.Name != null && original.Name.IndexOf('<') >= 0)
                    || (original.DeclaringType != null && original.DeclaringType.Name.IndexOf('<') >= 0);
                if (synthesized)
                    continue;
                InlineRiskSource source = ClassifyInlineRisk(original, releaseMode, forceEvaluateInline);
                if (IsInlineRiskSource(source))
                {
                    inlinedKeys.Add(change.MethodKey);
                    inlinedSources.Add(source.ToString());
                }
            }

            var response = new HotPatchLoadedResponse
            {
                patch_id = patchId,
                method_count = applied.Count,
                detour_engine = engineSummary ?? "load_only",
                inlined_method_keys = inlinedKeys.ToArray(),
                inlined_sources = inlinedSources.ToArray(),
                // Tells the desktop this plugin can drive added Unity messages, so
                // an add-a-message patch is not falsely reported live. driven /
                // skipped (benign) / failed (hard) let the desktop fail closed on a
                // real wiring failure instead of reporting a false "driven".
                pump_supported = true,
                pumped_count = pumpedCount,
                pump_skipped_count = pumpSkippedDetails.Count,
                pump_skipped_messages = pumpSkippedDetails.ToArray(),
                pump_failed_count = pumpFailedCount,
            };
            Debug.Log("[Locus] Hot patch applied: " + applied.Count + " method(s), patch " + patchId
                + (inlinedKeys.Count > 0 ? " (" + inlinedKeys.Count + " inlined in Release)" : ""));
            return OkResponse(requestId, JsonUtility.ToJson(response));
        }

        /// <summary>Wire newly added Unity messages to their runtime driver, by
        /// <c>kind</c>: a PlayerLoop pump (player_loop), a proxy MonoBehaviour on
        /// the target object (component_proxy), a one-shot run on existing instances
        /// (catch_up), or compiled-but-dormant (inert). First clear every
        /// registration from the source files this patch touched (replace-by-source),
        /// then register what remains, each bound through a compiled delegate.
        ///
        /// The sidecar classifies by syntactic name (no semantic model), so the
        /// RUNTIME is the authority and reports three outcomes:
        ///  • driven  — actually wired (pump/proxy) or run (catch_up);
        ///  • skipped — not driven (the declaring type is not a MonoBehaviour, the
        ///    parameter is not the engine's event type — a same-named custom/aliased
        ///    type, an edit-time-only callback, or a catch-up with no live instance),
        ///    so it stays a harmless plain method. Each skip's "message — reason" is
        ///    collected into <paramref name="skipped"/> so the desktop reports it by
        ///    name rather than as a single vague count;
        ///  • failed  — a coordinate that should resolve does not (type/shim missing
        ///    or an unbindable shim), so the desktop fails closed to a recompile
        ///    instead of reporting a false "driven".</summary>
        private static void RegisterMessageDrivers(
            HotPatchMethodDto[] methods, MessageDriverDto[] drivers, Assembly patchAssembly,
            out int driven, List<string> skipped, out int failed)
        {
            driven = 0;
            failed = 0;

            // Replace-by-source: clear stale registrations (pump AND proxy) for
            // every file this patch touched (through a detour OR a driver entry)
            // before re-adding what is still present.
            var touched = new HashSet<string>(StringComparer.Ordinal);
            if (methods != null)
            {
                foreach (HotPatchMethodDto m in methods)
                {
                    if (m != null && !string.IsNullOrEmpty(m.source_path))
                        touched.Add(m.source_path);
                }
            }
            if (drivers != null)
            {
                foreach (MessageDriverDto d in drivers)
                {
                    if (d != null && !string.IsNullOrEmpty(d.source_path))
                        touched.Add(d.source_path);
                }
            }
            foreach (string path in touched)
            {
                LocusMessagePump.ClearSource(path);
                LocusMessageProxyHub.ClearSource(path);
            }

            if (drivers == null)
                return;

            foreach (MessageDriverDto driver in drivers)
            {
                if (driver == null || string.IsNullOrEmpty(driver.shim_type) || string.IsNullOrEmpty(driver.shim_method))
                    continue;

                Type declaringType = ResolveHotPatchOriginalType(driver.declaring_type);
                if (declaringType == null)
                {
                    failed++;
                    Debug.LogWarning("[Locus] message driver: type " + driver.declaring_type
                        + " not found in domain; '" + driver.message + "' could not be wired.");
                    continue;
                }

                Type shimType = patchAssembly.GetType(driver.shim_type, false);
                // Shims are emitted public static; NonPublic is defensive against a
                // future visibility change so the lookup never silently misses.
                MethodInfo shim = shimType?.GetMethod(driver.shim_method,
                    BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static);
                if (shim == null)
                {
                    failed++;
                    Debug.LogWarning("[Locus] message driver: shim " + driver.shim_type + "." + driver.shim_method
                        + " not found; '" + driver.message + "' could not be wired.");
                    continue;
                }

                // Outcome: +1 driven, 0 skipped (benign — not a Unity message, or
                // nothing live to drive yet), -1 failed (hard). `reason` explains a
                // skip so the desktop can report it by name, not a vague count.
                int outcome;
                string reason = "";
                switch (driver.kind)
                {
                    case "component_proxy":
                        outcome = WireComponentProxy(driver, declaringType, shim, out reason);
                        break;
                    case "catch_up":
                    {
                        // Lifecycle catch-up: run `static void M(T self)` once on each
                        // existing instance now (native timing already passed), gated to
                        // match the message — Awake/Start are play-mode only (Start also
                        // enabled-only); OnValidate is an editor-time callback.
                        Action<object> invoke = BuildShimInvoker(shim);
                        if (invoke == null)
                        {
                            outcome = -1;   // unbindable shim — hard failure
                            break;
                        }
                        CatchUpGate gate;
                        switch (driver.message)
                        {
                            case "OnValidate": gate = CatchUpGate.Always; break;
                            case "Start": gate = CatchUpGate.PlayingEnabled; break;
                            default: gate = CatchUpGate.PlayingActive; break;   // Awake (+ any future lifecycle)
                        }
                        int ran = LocusMessageCatchUp.RunOnce(declaringType, invoke, gate);
                        if (ran < 0)
                        {
                            outcome = 0;
                            reason = "declaring type is not a MonoBehaviour";
                        }
                        else if (ran == 0)
                        {
                            // Real message, but nothing eligible ran — don't report it as
                            // "driven". Distinguish edit-mode (the lifecycle message is
                            // deferred) from simply having no live instance.
                            outcome = 0;
                            reason = (gate != CatchUpGate.Always && !Application.isPlaying)
                                ? "edit mode — " + driver.message + " is play-mode lifecycle; enter play mode and re-apply, or recompile"
                                : "no live instance to catch up (new instances need a recompile)";
                        }
                        else
                        {
                            outcome = 1;
                        }
                        break;
                    }
                    case "inert":
                        // Compiled but not driven at runtime (e.g. Reset). Benign; the
                        // agent was told why via the result note.
                        outcome = 0;
                        reason = "edit-time only (no runtime trigger)";
                        break;
                    default:
                    {
                        // "player_loop": shim is `static void M(T self)`.
                        Action<object> invoke = BuildShimInvoker(shim);
                        if (invoke == null)
                        {
                            outcome = -1;   // unbindable shim — hard failure
                        }
                        else if (LocusMessagePump.Register(driver.message, declaringType, driver.source_path, invoke))
                        {
                            outcome = 1;
                        }
                        else
                        {
                            outcome = 0;
                            reason = "declaring type is not a MonoBehaviour";
                        }
                        break;
                    }
                }

                if (outcome > 0)
                    driven++;
                else if (outcome == 0)
                    skipped.Add(driver.message + " — " + (reason.Length > 0 ? reason : "left as a plain method"));
                else
                    failed++;
            }
        }

        // Engine-delivered argument type for each parameterized component-proxy
        // message. The sidecar only matched the simple name, so the runtime checks
        // the shim's parameter is REALLY this type before forwarding. Parameterless
        // messages (OnGUI, mouse, OnDestroy, …) are absent — the shim is M(self).
        private static readonly Dictionary<string, Type> ProxyArgTypes = new Dictionary<string, Type>(StringComparer.Ordinal)
        {
            { "OnTriggerEnter", typeof(Collider) },
            { "OnTriggerStay", typeof(Collider) },
            { "OnTriggerExit", typeof(Collider) },
            { "OnCollisionEnter", typeof(Collision) },
            { "OnCollisionStay", typeof(Collision) },
            { "OnCollisionExit", typeof(Collision) },
            { "OnTriggerEnter2D", typeof(Collider2D) },
            { "OnTriggerStay2D", typeof(Collider2D) },
            { "OnTriggerExit2D", typeof(Collider2D) },
            { "OnCollisionEnter2D", typeof(Collision2D) },
            { "OnCollisionStay2D", typeof(Collision2D) },
            { "OnCollisionExit2D", typeof(Collision2D) },
            { "OnAnimatorIK", typeof(int) },
            { "OnParticleCollision", typeof(GameObject) },
            { "OnControllerColliderHit", typeof(ControllerColliderHit) },
        };

        /// <summary>Validate and register a component-proxy driver. Returns +1 driven,
        /// 0 skipped (not a Unity message — non-MonoBehaviour, or the parameter is not
        /// the engine event type; <paramref name="reason"/> says which), -1 failed (the
        /// shim shape cannot be bound).</summary>
        private static int WireComponentProxy(MessageDriverDto driver, Type declaringType, MethodInfo shim, out string reason)
        {
            reason = "";
            // The sidecar matched only the parameter's simple name, so a same-named
            // custom/aliased type would slip through and throw on cast at the first
            // event. Verify the shim's argument is the REAL engine type; if not, this
            // is not the Unity message — leave it as a harmless plain method.
            if (ProxyArgTypes.TryGetValue(driver.message, out Type expectedArg))
            {
                ParameterInfo[] ps = shim.GetParameters();
                if (ps.Length != 2 || ps[1].ParameterType != expectedArg)
                {
                    reason = "parameter is not " + expectedArg.FullName + " (a same-named custom type) — left as a plain method";
                    Debug.LogWarning("[Locus] message driver: '" + driver.message + "' parameter is not "
                        + expectedArg.FullName + "; not a Unity message — left as a plain method.");
                    return 0;
                }
            }
            // Proxy shim is `static void M(T self [, EngineArg arg])`.
            Action<object, object> invoke = BuildProxyInvoker(shim);
            if (invoke == null)
                return -1;
            if (LocusMessageProxyHub.Register(driver.message, declaringType, driver.source_path, invoke))
                return 1;
            reason = "declaring type is not a MonoBehaviour";
            return 0;
        }

        /// <summary>Compile <c>(object o) =&gt; Shim((T)o)</c> for a static shim
        /// whose single parameter is the instance type — a direct delegate call
        /// each frame instead of a reflection Invoke. Null if the shim is not the
        /// expected single-parameter shape or cannot be compiled.</summary>
        private static Action<object> BuildShimInvoker(MethodInfo shim)
        {
            ParameterInfo[] parameters = shim.GetParameters();
            if (parameters.Length != 1)
                return null;
            try
            {
                ParameterExpression instance = Expression.Parameter(typeof(object), "instance");
                MethodCallExpression call = Expression.Call(
                    shim, Expression.Convert(instance, parameters[0].ParameterType));
                return Expression.Lambda<Action<object>>(call, instance).Compile();
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] message driver: could not bind shim " + shim.Name + ": " + ex.Message);
                return null;
            }
        }

        /// <summary>Compile an <c>Action&lt;object, object&gt;</c> for a component-proxy
        /// shim: a one-parameter shim <c>M(T self)</c> ignores the engine arg (the
        /// parameterless messages — OnGUI, mouse, …), a two-parameter shim
        /// <c>M(T self, A arg)</c> uses it (OnTriggerEnter(Collider), …). Null if
        /// the shim is neither shape or cannot be compiled.</summary>
        private static Action<object, object> BuildProxyInvoker(MethodInfo shim)
        {
            ParameterInfo[] parameters = shim.GetParameters();
            try
            {
                ParameterExpression self = Expression.Parameter(typeof(object), "self");
                ParameterExpression arg = Expression.Parameter(typeof(object), "arg");
                MethodCallExpression call;
                if (parameters.Length == 1)
                {
                    call = Expression.Call(shim, Expression.Convert(self, parameters[0].ParameterType));
                }
                else if (parameters.Length == 2)
                {
                    call = Expression.Call(
                        shim,
                        Expression.Convert(self, parameters[0].ParameterType),
                        Expression.Convert(arg, parameters[1].ParameterType));
                }
                else
                {
                    return null;
                }
                return Expression.Lambda<Action<object, object>>(call, self, arg).Compile();
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] message driver: could not bind shim " + shim.Name + ": " + ex.Message);
                return null;
            }
        }

        /// <summary>Force-JIT every method of the patch's shim/store classes
        /// so Mono's accessibility checks run NOW (returns the first failure,
        /// or null). Matching is on FullName so the compiler-generated NESTED
        /// types of shim bodies — async/iterator state machines and lambda
        /// display classes, whose own Name is "&lt;M&gt;d__0" — are covered too:
        /// their MoveNext/lambda methods carry the same violating IL but are
        /// instance methods that no detour pre-JITs (C2′b). Store holders
        /// additionally get their CCTOR prepared (compiled, NOT run): an
        /// added static field's initializer reads original surface there on
        /// first touch, long after apply. Generic shim methods (and nested
        /// types of generic shims) are skipped: they JIT per instantiation
        /// and direct call sites surface errors deterministically.</summary>
        private static string PrepareHotPatchShims(Assembly patchAssembly)
        {
            Type[] types;
            try
            {
                types = patchAssembly.GetTypes();
            }
            catch (ReflectionTypeLoadException ex)
            {
                types = ex.Types ?? new Type[0];
            }

            foreach (Type type in types)
            {
                if (type == null)
                    continue;
                string name = type.FullName ?? type.Name;
                if (!name.Contains("__LocusShims") && !name.Contains("__LocusFields_"))
                    continue;

                MethodInfo[] methods;
                try
                {
                    methods = type.GetMethods(
                        BindingFlags.Public | BindingFlags.NonPublic |
                        BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly);
                }
                catch (Exception ex)
                {
                    return type.FullName + ": " + ex.Message;
                }

                foreach (MethodInfo method in methods)
                {
                    if (method.IsGenericMethodDefinition || method.ContainsGenericParameters)
                        continue;
                    try
                    {
                        RuntimeHelpers.PrepareMethod(method.MethodHandle);
                    }
                    catch (Exception ex)
                    {
                        Exception detail = ex.InnerException ?? ex;
                        return type.Name + "." + method.Name + ": " + detail.Message;
                    }
                }

                if (!type.ContainsGenericParameters)
                {
                    ConstructorInfo cctor = type.TypeInitializer;
                    if (cctor != null)
                    {
                        try
                        {
                            RuntimeHelpers.PrepareMethod(cctor.MethodHandle);
                        }
                        catch (Exception ex)
                        {
                            Exception detail = ex.InnerException ?? ex;
                            return type.Name + "..cctor: " + detail.Message;
                        }
                    }
                }
            }
            return null;
        }

        private static void RollbackHotPatch(List<HotPatchApplyChange> applied)
        {
            for (int i = applied.Count - 1; i >= 0; i--)
            {
                HotPatchApplyChange change = applied[i];
                try { change.NewEntry.Detour.Dispose(); } catch { }
                HotPatchDetourEntry current;
                if (_hotMethodDetours.TryGetValue(change.CacheKey, out current) && ReferenceEquals(current, change.NewEntry))
                    _hotMethodDetours.Remove(change.CacheKey);

                if (change.PreviousEntry != null)
                {
                    string restoreError;
                    if (!RestorePreviousDetour(change.CacheKey, change.PreviousEntry, out restoreError))
                        Debug.LogError("[Locus] Failed to restore superseded hot patch for " + change.CacheKey + ": " + restoreError);
                }
            }
        }

        private static bool RestorePreviousDetour(string cacheKey, HotPatchDetourEntry previous, out string error)
        {
            error = null;
            try
            {
                string engine;
                IDisposable detour = CreateMethodDetour(previous.Original, previous.Patch, out engine);
                previous.Detour = detour;
                previous.Engine = engine;
                _hotMethodDetours[cacheKey] = previous;
                return true;
            }
            catch (Exception ex)
            {
                error = ex.Message;
                return false;
            }
        }

        private static bool ValidateDetourSignature(MethodBase original, MethodBase patch, out string error)
        {
            error = null;
            ParameterInfo[] originalParams = original.GetParameters();
            ParameterInfo[] patchParams = patch.GetParameters();
            if (originalParams.Length != patchParams.Length)
            {
                error = "parameter count differs";
                return false;
            }
            for (int i = 0; i < originalParams.Length; i++)
            {
                if (!SameDetourType(originalParams[i].ParameterType, patchParams[i].ParameterType))
                {
                    error = "parameter " + i + " differs: " +
                        DisplayType(originalParams[i].ParameterType) + " vs " +
                        DisplayType(patchParams[i].ParameterType);
                    return false;
                }
            }

            MethodInfo originalMethod = original as MethodInfo;
            MethodInfo patchMethod = patch as MethodInfo;
            if ((originalMethod == null) != (patchMethod == null))
            {
                error = "method kind differs";
                return false;
            }
            if (originalMethod != null &&
                !SameDetourType(originalMethod.ReturnType, patchMethod.ReturnType))
            {
                error = "return type differs: " +
                    DisplayType(originalMethod.ReturnType) + " vs " +
                    DisplayType(patchMethod.ReturnType);
                return false;
            }

            return true;
        }

        private static bool SameDetourType(Type left, Type right)
        {
            if (left == right)
                return true;
            if (left == null || right == null)
                return false;
            if (left.IsByRef || right.IsByRef)
            {
                return left.IsByRef == right.IsByRef &&
                    SameDetourType(left.GetElementType(), right.GetElementType());
            }
            if (left.IsArray || right.IsArray)
            {
                return left.IsArray == right.IsArray &&
                    left.GetArrayRank() == right.GetArrayRank() &&
                    SameDetourType(left.GetElementType(), right.GetElementType());
            }
            if (left.IsGenericParameter || right.IsGenericParameter)
            {
                return left.IsGenericParameter == right.IsGenericParameter &&
                    left.GenericParameterPosition == right.GenericParameterPosition;
            }
            if (left.IsGenericType || right.IsGenericType)
            {
                if (left.IsGenericType != right.IsGenericType)
                    return false;
                if (!SameDetourType(left.GetGenericTypeDefinition(), right.GetGenericTypeDefinition()))
                    return false;
                Type[] leftArgs = left.GetGenericArguments();
                Type[] rightArgs = right.GetGenericArguments();
                if (leftArgs.Length != rightArgs.Length)
                    return false;
                for (int i = 0; i < leftArgs.Length; i++)
                {
                    if (!SameDetourType(leftArgs[i], rightArgs[i]))
                        return false;
                }
                return true;
            }
            if (string.Equals(left.FullName, right.FullName, StringComparison.Ordinal) &&
                string.Equals(SafeAssemblyName(left.Assembly), SafeAssemblyName(right.Assembly), StringComparison.Ordinal))
            {
                return true;
            }

            // Original type vs its layout-identical patch copy: self-typed
            // operator/conversion parameters carry the rename. The copy is
            // ABI-compatible by construction (same field sequence), so the
            // detour is safe.
            return string.Equals(
                StripPatchTypeSuffix(left.FullName), StripPatchTypeSuffix(right.FullName), StringComparison.Ordinal);
        }

        private static string DisplayType(Type type)
        {
            if (type == null)
                return "<null>";
            return type.FullName + ", " + SafeAssemblyName(type.Assembly);
        }

        private static string MethodKey(HotPatchMethodDto dto)
        {
            return dto.declaring_type + "|" + dto.name + "|" +
                string.Join(",", dto.param_type_names ?? new string[0]) +
                (dto.is_static ? "|s" : "|i");
        }

        /// <summary>The detour-cache key: <see cref="MethodKey"/> plus the
        /// resolved ORIGINAL assembly. MethodKey alone (declaring type +
        /// signature) is NOT unique across assemblies — a play-mode-born type and
        /// a same-named compiled type (e.g. a new file redefining an existing
        /// class) both resolve their own original, so caching detours by MethodKey
        /// would let one method's patch evict/replace the other's. The assembly
        /// suffix gives each (method, original assembly) its own cache slot, while
        /// re-patches of the SAME original (a stable original_assembly) still
        /// replace as before. MethodKey itself is left unchanged for the
        /// inlined-key report, which must stay byte-identical with the Rust
        /// unity_method_key round-trip.</summary>
        private static string DetourKey(HotPatchMethodDto dto)
        {
            return MethodKey(dto) + "|" + (dto.original_assembly ?? "");
        }

        private static string DescribeMethod(HotPatchMethodDto dto)
        {
            return dto.declaring_type + "." + dto.name + "(" +
                string.Join(", ", dto.param_type_names ?? new string[0]) + ")";
        }

        private static MethodBase ResolveOriginalMethod(HotPatchMethodDto dto, out string error)
        {
            Type type;
            if (!string.IsNullOrEmpty(dto.original_assembly))
            {
                // Targeted resolution (M2 re-edit): the "original" is an
                // earlier patch's shim — search exactly that assembly,
                // bypassing the usual __LocusHotPatch_ skip.
                type = ResolveTypeInAssembly(dto.original_assembly, dto.declaring_type);
                if (type == null)
                {
                    error = "type " + dto.declaring_type + " not found in assembly " + dto.original_assembly +
                        " (earlier patch unloaded?)";
                    return null;
                }
            }
            else
            {
                type = ResolveHotPatchOriginalType(dto.declaring_type);
                if (type == null)
                {
                    error = "type not found in loaded assemblies";
                    return null;
                }
            }
            return ResolveMethodOnType(type, dto, out error);
        }

        private static Type ResolveTypeInAssembly(string assemblyName, string metadataName)
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly asm = assemblies[i];
                if (asm == null || asm.IsDynamic)
                    continue;
                if (!string.Equals(SafeAssemblyName(asm), assemblyName, StringComparison.Ordinal))
                    continue;
                Type type = asm.GetType(metadataName, false);
                if (type != null)
                    return type;
            }
            return null;
        }

        private static MethodBase ResolvePatchMethod(Assembly patchAssembly, HotPatchMethodDto dto, out string error)
        {
            Type type = patchAssembly.GetType(dto.patch_declaring_type, false);
            if (type == null)
            {
                error = "patch type " + dto.patch_declaring_type + " not found in patch assembly";
                return null;
            }
            return ResolveMethodOnType(type, dto, out error);
        }

        /// <summary>Resolve the original declaring type across the domain,
        /// skipping other patch assemblies and inactive skill packages.</summary>
        private static Type ResolveHotPatchOriginalType(string metadataName)
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly asm = assemblies[i];
                if (asm == null || asm.IsDynamic)
                    continue;

                string assemblyName = SafeAssemblyName(asm);
                if (assemblyName.StartsWith("__LocusHotPatch_", StringComparison.Ordinal))
                    continue;
                if (IsInactiveSkillPackageAssemblyName(assemblyName))
                    continue;

                Type type = asm.GetType(metadataName, false);
                if (type != null)
                    return type;
            }
            return null;
        }

        private static MethodBase ResolveMethodOnType(Type type, HotPatchMethodDto dto, out string error)
        {
            error = null;
            string[] wanted = dto.param_type_names ?? new string[0];
            string[] sigs = dto.param_type_sigs ?? new string[0];

            MethodBase[] candidates;
            if (dto.is_ctor)
            {
                candidates = type.GetConstructors(
                    BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.DeclaredOnly);
            }
            else
            {
                candidates = type.GetMethods(
                    BindingFlags.Public | BindingFlags.NonPublic |
                    BindingFlags.Instance | BindingFlags.Static | BindingFlags.DeclaredOnly);
            }

            // Phase 1 — coarse match on the simple parameter names the desktop
            // always sends. This is the historical identity and its behaviour is
            // unchanged; the only difference is that ALL matches are collected
            // instead of failing on the second, so a same-simple-name overload
            // can be disambiguated below rather than rejected outright.
            var coarse = new List<MethodBase>();
            for (int i = 0; i < candidates.Length; i++)
            {
                MethodBase candidate = candidates[i];
                if (!dto.is_ctor && !string.Equals(candidate.Name, dto.name, StringComparison.Ordinal))
                    continue;
                if (candidate.IsStatic != dto.is_static)
                    continue;
                if (!dto.is_ctor && candidate.IsGenericMethodDefinition)
                    continue;

                ParameterInfo[] parameters = candidate.GetParameters();
                if (parameters.Length != wanted.Length)
                    continue;
                if (CoarseParamsMatch(parameters, wanted))
                    coarse.Add(candidate);
            }

            if (coarse.Count == 0)
            {
                error = "no matching overload";
                return null;
            }
            if (coarse.Count == 1)
                return coarse[0];

            // Phase 2 — the simple names collide (overloads distinct only by
            // parameter namespace or generic argument). Break the tie with the
            // enriched per-parameter signatures. A token the desktop left
            // un-qualified still matches every coarse candidate (suffix
            // tolerance), so this only ever narrows the set — never past where
            // the simple names already pointed.
            if (sigs.Length == wanted.Length && wanted.Length > 0)
            {
                try
                {
                    MethodBase refined = null;
                    bool refinedAmbiguous = false;
                    foreach (MethodBase candidate in coarse)
                    {
                        if (!SigParamsMatch(candidate.GetParameters(), sigs))
                            continue;
                        if (refined != null)
                        {
                            refinedAmbiguous = true;
                            break;
                        }
                        refined = candidate;
                    }
                    if (refined != null && !refinedAmbiguous)
                        return refined;
                }
                catch
                {
                    // Any reflection oddity falls through to the fail-closed
                    // ambiguous verdict below — never worse than before.
                }
            }

            error = "ambiguous overload";
            return null;
        }

        private static bool CoarseParamsMatch(ParameterInfo[] parameters, string[] wanted)
        {
            for (int p = 0; p < parameters.Length; p++)
            {
                // Patch copies rename self-typed operator/conversion parameters
                // ("Foo__LocusPatch"): match against the original-name identity
                // the desktop sent.
                string parameterName = StripPatchTypeSuffix(parameters[p].ParameterType.Name);
                if (!string.Equals(parameterName, wanted[p], StringComparison.Ordinal) &&
                    !string.Equals(parameters[p].ParameterType.Name, wanted[p], StringComparison.Ordinal))
                {
                    return false;
                }
            }
            return true;
        }

        private static bool SigParamsMatch(ParameterInfo[] parameters, string[] sigs)
        {
            for (int p = 0; p < parameters.Length; p++)
            {
                if (!TypeTokenMatch(sigs[p], BuildSigToken(parameters[p].ParameterType)))
                    return false;
            }
            return true;
        }

        /// <summary>Reflected parameter type rendered in the desktop's signature
        /// grammar (HotDiff.QualifiedTypeName): namespace-qualified name,
        /// "Name`N&lt;arg,...&gt;" for closed generics, "[]" for arrays, "&amp;"
        /// for byref.</summary>
        private static string BuildSigToken(Type t)
        {
            if (t == null)
                return "";
            if (t.IsByRef)
                return BuildSigToken(t.GetElementType()) + "&";
            if (t.IsArray)
                return BuildSigToken(t.GetElementType()) + "[" + new string(',', t.GetArrayRank() - 1) + "]";
            if (t.IsGenericType && !t.IsGenericTypeDefinition)
            {
                Type def = t.GetGenericTypeDefinition();
                string genericHead = NormalizeTypeName(def.FullName ?? def.Name);
                Type[] args = t.GetGenericArguments();
                var rendered = new string[args.Length];
                for (int i = 0; i < args.Length; i++)
                    rendered[i] = BuildSigToken(args[i]);
                return genericHead + "<" + string.Join(",", rendered) + ">";
            }
            return NormalizeTypeName(t.FullName ?? t.Name);
        }

        /// <summary>Strip the patch-copy marker and unify the nested-type '+'
        /// with the '.' the desktop writes, so suffix comparison is uniform.</summary>
        private static string NormalizeTypeName(string name)
        {
            return StripPatchTypeSuffix(name ?? "").Replace('+', '.');
        }

        /// <summary>True when the desktop signature token <paramref name="want"/>
        /// identifies the reflected token <paramref name="refl"/>. Heads match on
        /// a namespace-suffix boundary (the desktop may send a less-qualified
        /// name); generic arguments are compared only when the desktop supplied
        /// them, so an un-enriched token stays as permissive as its simple
        /// name.</summary>
        private static bool TypeTokenMatch(string want, string refl)
        {
            if (want == null || refl == null)
                return false;
            want = want.Trim();
            refl = refl.Trim();

            bool wantByRef = want.EndsWith("&", StringComparison.Ordinal);
            bool reflByRef = refl.EndsWith("&", StringComparison.Ordinal);
            if (wantByRef != reflByRef)
                return false;
            if (wantByRef)
                return TypeTokenMatch(want.Substring(0, want.Length - 1), refl.Substring(0, refl.Length - 1));

            string wantArray = TrailingArraySuffix(want);
            string reflArray = TrailingArraySuffix(refl);
            if (wantArray.Length > 0 || reflArray.Length > 0)
            {
                if (!string.Equals(wantArray, reflArray, StringComparison.Ordinal))
                    return false;
                return TypeTokenMatch(
                    want.Substring(0, want.Length - wantArray.Length),
                    refl.Substring(0, refl.Length - reflArray.Length));
            }

            string wantHead, reflHead;
            string[] wantArgs, reflArgs;
            SplitGeneric(want, out wantHead, out wantArgs);
            SplitGeneric(refl, out reflHead, out reflArgs);

            if (!HeadMatch(wantHead, reflHead))
                return false;

            if (wantArgs == null)
                return true; // desktop did not qualify the generic arguments
            if (reflArgs == null || wantArgs.Length != reflArgs.Length)
                return false;
            for (int i = 0; i < wantArgs.Length; i++)
            {
                if (!TypeTokenMatch(wantArgs[i], reflArgs[i]))
                    return false;
            }
            return true;
        }

        private static bool HeadMatch(string want, string refl)
        {
            want = NormalizeTypeName(want);
            refl = NormalizeTypeName(refl);
            return string.Equals(refl, want, StringComparison.Ordinal) ||
                refl.EndsWith("." + want, StringComparison.Ordinal);
        }

        /// <summary>The trailing array rank group ("[]", "[,]", …) of a token, or
        /// "" when it is not an array. Generic argument lists use "&lt;&gt;", so a
        /// trailing "[...]" of only commas is unambiguously an array.</summary>
        private static string TrailingArraySuffix(string token)
        {
            if (token.Length == 0 || token[token.Length - 1] != ']')
                return "";
            int open = token.LastIndexOf('[');
            if (open < 0)
                return "";
            for (int i = open + 1; i < token.Length - 1; i++)
            {
                if (token[i] != ',')
                    return "";
            }
            return token.Substring(open);
        }

        /// <summary>Split "Head&lt;arg,arg&gt;" into the head and its top-level
        /// argument tokens; args is null when there is no generic list.</summary>
        private static void SplitGeneric(string token, out string head, out string[] args)
        {
            int open = token.IndexOf('<');
            if (open < 0)
            {
                head = token;
                args = null;
                return;
            }
            head = token.Substring(0, open);
            var list = new List<string>();
            int depth = 0;
            int start = open + 1;
            for (int i = open; i < token.Length; i++)
            {
                char c = token[i];
                if (c == '<' || c == '[')
                {
                    depth++;
                }
                else if (c == ']')
                {
                    depth--;
                }
                else if (c == '>')
                {
                    depth--;
                    if (depth == 0)
                    {
                        list.Add(token.Substring(start, i - start));
                        break;
                    }
                }
                else if (c == ',' && depth == 1)
                {
                    list.Add(token.Substring(start, i - start));
                    start = i + 1;
                }
            }
            args = list.ToArray();
        }

        /// <summary>"Foo__LocusPatch" → "Foo", "Outer__LocusPatch+Inner" →
        /// "Outer+Inner" (patch copies rename the top-level type; the marker
        /// never appears in legitimate user type names).</summary>
        private static string StripPatchTypeSuffix(string typeName)
        {
            if (string.IsNullOrEmpty(typeName))
                return typeName;
            return typeName.Replace("__LocusPatch", "");
        }

        // ───────────────── hot_patch_dispose ─────────────────

        /// <summary>Release detours by patch id, or every detour when the
        /// payload is "all"/empty (used before a converging recompile).</summary>
        private static async Task<PipeEnvelope> HandleHotPatchDispose(string requestId, string payload)
        {
            string target = (payload ?? "").Trim();
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    int removed = 0;
                    lock (_hotPatchLock)
                    {
                        var keys = new List<string>(_hotMethodDetours.Keys);
                        foreach (string key in keys)
                        {
                            HotPatchDetourEntry entry = _hotMethodDetours[key];
                            if (target.Length != 0 &&
                                !string.Equals(target, "all", StringComparison.OrdinalIgnoreCase) &&
                                !string.Equals(entry.PatchId, target, StringComparison.Ordinal))
                            {
                                continue;
                            }
                            try { entry.Detour.Dispose(); } catch { }
                            _hotMethodDetours.Remove(key);
                            removed++;
                        }
                    }
                    tcs.SetResult(OkResponse(requestId, "disposed:" + removed));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.ToString()));
                }
            });
            return await tcs.Task;
        }
    }
}
