using UnityEngine;
using UnityEditor.Compilation;

using System;
using System.Collections.Generic;
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
            public string MethodKey;
            public HotPatchDetourEntry NewEntry;
            public HotPatchDetourEntry PreviousEntry;
        }

        // Active detour per ORIGINAL method key. Re-patching the same method
        // has one live redirect at a time; failed patch batches restore any
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

            // When non-empty, the "original" method lives in this exact
            // assembly — an earlier patch's shim being re-edited. Resolution
            // then bypasses the usual skip of __LocusHotPatch_ assemblies.
            public string original_assembly;
        }

        [Serializable]
        private sealed class HotPatchLoadedRequest
        {
            public string patch_id;
            public string assembly_b64;
            public string assembly_path;
            public string domain_generation;
            public HotPatchMethodDto[] methods;

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
                    tcs.SetResult(ApplyHotPatchOnMainThread(requestId, patchId, assemblyBytes, request.methods, request.inline_force_evaluate));
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
                    HotPatchDetourEntry previous;
                    if (_hotMethodDetours.TryGetValue(methodKey, out previous))
                    {
                        try { previous.Detour.Dispose(); } catch { }
                        _hotMethodDetours.Remove(methodKey);
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
                        if (previous != null && !RestorePreviousDetour(methodKey, previous, out restoreError))
                            Debug.LogError("[Locus] Failed to restore superseded hot patch for " + methodKey + ": " + restoreError);
                        RollbackHotPatch(applied);
                        return ErrorResponse(requestId, "detour failed for " + DescribeMethod(dto) + ": " + ex.Message);
                    }

                    _hotMethodDetours[methodKey] = entry;
                    applied.Add(new HotPatchApplyChange
                    {
                        MethodKey = methodKey,
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
            };
            Debug.Log("[Locus] Hot patch applied: " + applied.Count + " method(s), patch " + patchId
                + (inlinedKeys.Count > 0 ? " (" + inlinedKeys.Count + " inlined in Release)" : ""));
            return OkResponse(requestId, JsonUtility.ToJson(response));
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
                if (_hotMethodDetours.TryGetValue(change.MethodKey, out current) && ReferenceEquals(current, change.NewEntry))
                    _hotMethodDetours.Remove(change.MethodKey);

                if (change.PreviousEntry != null)
                {
                    string restoreError;
                    if (!RestorePreviousDetour(change.MethodKey, change.PreviousEntry, out restoreError))
                        Debug.LogError("[Locus] Failed to restore superseded hot patch for " + change.MethodKey + ": " + restoreError);
                }
            }
        }

        private static bool RestorePreviousDetour(string methodKey, HotPatchDetourEntry previous, out string error)
        {
            error = null;
            try
            {
                string engine;
                IDisposable detour = CreateMethodDetour(previous.Original, previous.Patch, out engine);
                previous.Detour = detour;
                previous.Engine = engine;
                _hotMethodDetours[methodKey] = previous;
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
