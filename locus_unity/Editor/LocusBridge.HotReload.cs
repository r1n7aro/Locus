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

        // ───────────────── hot_patch_loaded ─────────────────

        [Serializable]
        private sealed class HotPatchMethodDto
        {
            public string declaring_type;
            public string patch_declaring_type;
            public string name;
            public string[] param_type_names;
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
            public string domain_generation;
            public HotPatchMethodDto[] methods;
        }

        [Serializable]
        private sealed class HotPatchLoadedResponse
        {
            public string patch_id;
            public int method_count;
            public string detour_engine;
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

            if (request == null || string.IsNullOrEmpty(request.assembly_b64))
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
                assemblyBytes = Convert.FromBase64String(request.assembly_b64);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_patch_loaded assembly decode failed: " + ex.Message);
            }

            string patchId = string.IsNullOrEmpty(request.patch_id) ? Guid.NewGuid().ToString("N") : request.patch_id;

            // Apply on the main thread, between frames: the whole patch
            // lands atomically with respect to Update loops.
            var tcs = new TaskCompletionSource<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    tcs.SetResult(ApplyHotPatchOnMainThread(requestId, patchId, assemblyBytes, request.methods));
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
            HotPatchMethodDto[] methods)
        {
            if (CompilationPipeline.codeOptimization != CodeOptimization.Debug)
            {
                return ErrorResponse(
                    requestId,
                    "hot reload requires Editor Code Optimization = Debug (Release inlines call sites past the redirect)");
            }

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
            }

            var response = new HotPatchLoadedResponse
            {
                patch_id = patchId,
                method_count = applied.Count,
                detour_engine = engineSummary ?? "load_only",
            };
            Debug.Log("[Locus] Hot patch applied: " + applied.Count + " method(s), patch " + patchId);
            return OkResponse(requestId, JsonUtility.ToJson(response));
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
            return string.Equals(left.FullName, right.FullName, StringComparison.Ordinal) &&
                string.Equals(SafeAssemblyName(left.Assembly), SafeAssemblyName(right.Assembly), StringComparison.Ordinal);
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

            MethodBase match = null;
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

                bool paramsMatch = true;
                for (int p = 0; p < parameters.Length; p++)
                {
                    if (!string.Equals(parameters[p].ParameterType.Name, wanted[p], StringComparison.Ordinal))
                    {
                        paramsMatch = false;
                        break;
                    }
                }
                if (!paramsMatch)
                    continue;

                if (match != null)
                {
                    error = "ambiguous overload";
                    return null;
                }
                match = candidate;
            }

            if (match == null)
                error = "no matching overload";
            return match;
        }

        // ───────────────── hot_patch_dispose ─────────────────

        /// <summary>Release detours by patch id, or every detour when the
        /// payload is "all"/empty (used before a converging recompile).</summary>
        private static async Task<PipeEnvelope> HandleHotPatchDispose(string requestId, string payload)
        {
            string target = (payload ?? "").Trim();
            var tcs = new TaskCompletionSource<PipeEnvelope>();
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
