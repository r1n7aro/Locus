using UnityEngine;

using System;
using System.Collections.Generic;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;

namespace Locus
{
    // C0 runtime capability probe (unity-hotreload-compat-plan.md §C0).
    // Measured fact (first on-device round): this editor's Mono enforces
    // accessibility at JIT time and ignores IgnoresAccessChecksTo for
    // project assemblies — but the per-operation × per-visibility behavior
    // and the three reflection/emit primitives the C2' access thunks rely on
    // vary with the Mono build, so they are MEASURED here per editor, never
    // assumed. The sidecar compiles a probe assembly (AccessProbeSource.cs)
    // whose methods each touch one non-public member of
    // LocusAccessProbeTarget; this side loads it, force-JITs every cell,
    // runs the primitives, and returns the matrix. The desktop coordinator
    // caches it per domain generation and feeds it back into
    // compile/hotPatch as runtimeCaps.

    /// <summary>
    /// Probe target with a stable non-public surface. The sidecar's probe
    /// source (AccessProbeSource.cs) and its CoreCLR test mirror reference
    /// these members BY NAME — keep names, shapes and seed values in sync.
    /// </summary>
#pragma warning disable 0414
    internal sealed class LocusAccessProbeTarget
    {
        private int _privInst = 7;
        internal int _intInst = 11;
        private static int _privStatic = 13;
        internal static int _intStatic = 17;

        private LocusAccessProbeTarget(int seed) { _privInst = seed; }
        internal LocusAccessProbeTarget() { }

        private int PrivMethod(int x) { return x * 2 + 1; }
        internal int IntMethod(int x) { return x * 3 + 1; }
        private static int PrivStatic(int x) { return x * 5 + 1; }
        internal static int IntStatic(int x) { return x * 7 + 1; }

        public static LocusAccessProbeTarget New() { return new LocusAccessProbeTarget(); }

        /// <summary>Verification accessor: lets the byref primitive assert
        /// that a write through the emitted reference actually landed.</summary>
        public int ReadPrivInst() { return _privInst; }

        /// <summary>The castclass/ldtoken × private cells target this type
        /// (type-level checks, distinct from member-level ones).</summary>
        private sealed class PrivNested { }
    }
#pragma warning restore 0414

    public static partial class LocusBridge
    {
        // ───────────────── hot_reload_access_probe ─────────────────

        [Serializable]
        private sealed class AccessProbeCellRequest
        {
            public string method;
            public string op;
            public string visibility;
        }

        [Serializable]
        private sealed class AccessProbeRequest
        {
            public string assembly_b64;
            public AccessProbeCellRequest[] cells;
        }

        [Serializable]
        private sealed class AccessProbeCellResult
        {
            public string op;
            public string visibility;
            public bool ok;
            public string error;
        }

        [Serializable]
        private sealed class AccessProbePrimitives
        {
            public bool create_delegate_non_public;
            public bool dynamic_method_skip_visibility;
            public bool dynamic_method_byref_return;
        }

        [Serializable]
        private sealed class AccessProbeResponse
        {
            public List<AccessProbeCellResult> cells;
            public AccessProbePrimitives primitives;
            public List<string> errors;
        }

        /// <summary>Last measured matrix (JSON), kept for potential reuse
        /// within this domain; the desktop caches per domain generation.</summary>
        private static string _accessProbeMatrixJson;

        private static async Task<PipeEnvelope> HandleHotReloadAccessProbe(string requestId, string requestJson)
        {
            if (string.IsNullOrEmpty(requestJson))
                return ErrorResponse(requestId, "empty hot_reload_access_probe request");

            AccessProbeRequest request;
            try
            {
                request = JsonUtility.FromJson<AccessProbeRequest>(requestJson);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_reload_access_probe request parse failed: " + ex.Message);
            }

            if (request == null || string.IsNullOrEmpty(request.assembly_b64))
                return ErrorResponse(requestId, "hot_reload_access_probe request missing assembly bytes");

            byte[] assemblyBytes;
            try
            {
                assemblyBytes = Convert.FromBase64String(request.assembly_b64);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_reload_access_probe assembly decode failed: " + ex.Message);
            }

            AccessProbeCellRequest[] cells = request.cells ?? new AccessProbeCellRequest[0];

            // Main thread, like every other JIT-touching handler: keeps the
            // probe's Mono interactions on the same thread the real patches
            // use. The whole run is milliseconds.
            try
            {
                return await LocusAsync.RunOnMainThreadAsync<PipeEnvelope>(delegate
                {
                    AccessProbeResponse response = RunAccessProbe(assemblyBytes, cells);
                    _accessProbeMatrixJson = JsonUtility.ToJson(response);
                    return OkResponse(requestId, _accessProbeMatrixJson);
                }, ExecuteTimeoutMs);
            }
            catch (TimeoutException)
            {
                return ErrorResponse(requestId, "hot_reload_access_probe timed out");
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "hot_reload_access_probe failed: " + ex.Message);
            }
        }

        private static AccessProbeResponse RunAccessProbe(byte[] assemblyBytes, AccessProbeCellRequest[] cells)
        {
            var response = new AccessProbeResponse
            {
                cells = new List<AccessProbeCellResult>(cells.Length),
                primitives = new AccessProbePrimitives(),
                errors = new List<string>(),
            };

            // 1) JIT matrix: force-JIT every sidecar-compiled cell. A failure
            //    means this Mono rejects that operation × visibility at JIT
            //    time even with IgnoresAccessChecksTo in place.
            Assembly probeAssembly = null;
            try
            {
                probeAssembly = Assembly.Load(assemblyBytes);
            }
            catch (Exception ex)
            {
                response.errors.Add("probe assembly load failed: " + ex.Message);
            }

            Type probeType = null;
            if (probeAssembly != null)
            {
                probeType = probeAssembly.GetType("__LocusAccessProbe", false);
                if (probeType == null)
                    response.errors.Add("__LocusAccessProbe type not found in probe assembly");
            }

            foreach (AccessProbeCellRequest cell in cells)
            {
                var result = new AccessProbeCellResult
                {
                    op = cell.op ?? "",
                    visibility = cell.visibility ?? "",
                    ok = false,
                    error = "",
                };
                if (probeType == null)
                {
                    result.error = "probe assembly unavailable";
                }
                else
                {
                    try
                    {
                        MethodInfo method = probeType.GetMethod(
                            cell.method,
                            BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static);
                        if (method == null)
                        {
                            result.error = "probe method " + cell.method + " not found";
                        }
                        else
                        {
                            RuntimeHelpers.PrepareMethod(method.MethodHandle);
                            result.ok = true;
                        }
                    }
                    catch (Exception ex)
                    {
                        Exception detail = ex.InnerException ?? ex;
                        result.error = detail.GetType().Name + ": " + detail.Message;
                    }
                }
                response.cells.Add(result);
            }

            // 2) The three primitives C2' thunks build on. Pure local
            //    reflection/emit — independent of the probe assembly, so
            //    they report even when the assembly failed to load.
            response.primitives.create_delegate_non_public =
                ProbeCreateDelegateNonPublic(response.errors);
            response.primitives.dynamic_method_skip_visibility =
                ProbeDynamicMethodSkipVisibility(response.errors);
            response.primitives.dynamic_method_byref_return =
                ProbeDynamicMethodByrefReturn(response.errors);

            return response;
        }

        /// <summary>Delegate.CreateDelegate over a non-public instance method
        /// (open-instance form — exactly the C2' method-thunk shape): must
        /// both create AND return the right value when invoked.</summary>
        private static bool ProbeCreateDelegateNonPublic(List<string> errors)
        {
            try
            {
                MethodInfo method = typeof(LocusAccessProbeTarget).GetMethod(
                    "PrivMethod", BindingFlags.NonPublic | BindingFlags.Instance);
                if (method == null)
                {
                    errors.Add("create_delegate_non_public: PrivMethod not found");
                    return false;
                }
                var call = (Func<LocusAccessProbeTarget, int, int>)Delegate.CreateDelegate(
                    typeof(Func<LocusAccessProbeTarget, int, int>), method);
                int got = call(LocusAccessProbeTarget.New(), 5);
                if (got != 11)
                {
                    errors.Add("create_delegate_non_public: wrong result " + got);
                    return false;
                }
                return true;
            }
            catch (Exception ex)
            {
                Exception detail = ex.InnerException ?? ex;
                errors.Add("create_delegate_non_public: " + detail.GetType().Name + ": " + detail.Message);
                return false;
            }
        }

        /// <summary>DynamicMethod(restrictedSkipVisibility: true) reading a
        /// private field (ldfld): create, JIT, invoke, verify the value.</summary>
        private static bool ProbeDynamicMethodSkipVisibility(List<string> errors)
        {
            try
            {
                FieldInfo field = typeof(LocusAccessProbeTarget).GetField(
                    "_privInst", BindingFlags.NonPublic | BindingFlags.Instance);
                if (field == null)
                {
                    errors.Add("dynamic_method_skip_visibility: _privInst not found");
                    return false;
                }
                var read = new DynamicMethod(
                    "__LocusProbeReadPriv",
                    typeof(int),
                    new[] { typeof(LocusAccessProbeTarget) },
                    true);
                ILGenerator il = read.GetILGenerator();
                il.Emit(OpCodes.Ldarg_0);
                il.Emit(OpCodes.Ldfld, field);
                il.Emit(OpCodes.Ret);
                var reader = (Func<LocusAccessProbeTarget, int>)read.CreateDelegate(
                    typeof(Func<LocusAccessProbeTarget, int>));
                int got = reader(LocusAccessProbeTarget.New());
                if (got != 7)
                {
                    errors.Add("dynamic_method_skip_visibility: wrong value " + got);
                    return false;
                }
                return true;
            }
            catch (Exception ex)
            {
                Exception detail = ex.InnerException ?? ex;
                errors.Add("dynamic_method_skip_visibility: " + detail.GetType().Name + ": " + detail.Message);
                return false;
            }
        }

        private delegate ref int AccessProbeRefGetter(LocusAccessProbeTarget target);

        /// <summary>Byref-returning DynamicMethod (ldflda + ret, the M4
        /// LocusFieldStore.Ref shape C2' field thunks reuse): read through
        /// the reference, write through it, and verify the write landed.</summary>
        private static bool ProbeDynamicMethodByrefReturn(List<string> errors)
        {
            try
            {
                FieldInfo field = typeof(LocusAccessProbeTarget).GetField(
                    "_privInst", BindingFlags.NonPublic | BindingFlags.Instance);
                if (field == null)
                {
                    errors.Add("dynamic_method_byref_return: _privInst not found");
                    return false;
                }
                var byref = new DynamicMethod(
                    "__LocusProbeRefPriv",
                    typeof(int).MakeByRefType(),
                    new[] { typeof(LocusAccessProbeTarget) },
                    true);
                ILGenerator il = byref.GetILGenerator();
                il.Emit(OpCodes.Ldarg_0);
                il.Emit(OpCodes.Ldflda, field);
                il.Emit(OpCodes.Ret);
                var getter = (AccessProbeRefGetter)byref.CreateDelegate(typeof(AccessProbeRefGetter));
                LocusAccessProbeTarget target = LocusAccessProbeTarget.New();
                ref int slot = ref getter(target);
                if (slot != 7)
                {
                    errors.Add("dynamic_method_byref_return: initial read " + slot);
                    return false;
                }
                slot = 21;
                if (target.ReadPrivInst() != 21)
                {
                    errors.Add("dynamic_method_byref_return: write did not land");
                    return false;
                }
                ref int again = ref getter(target);
                if (again != 21)
                {
                    errors.Add("dynamic_method_byref_return: re-read " + again);
                    return false;
                }
                return true;
            }
            catch (Exception ex)
            {
                Exception detail = ex.InnerException ?? ex;
                errors.Add("dynamic_method_byref_return: " + detail.GetType().Name + ": " + detail.Message);
                return false;
            }
        }
    }
}
