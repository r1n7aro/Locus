using System;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Text;
using System.Threading.Tasks;

using UnityEditor.Compilation;
using UnityEngine;

namespace Locus
{
    // EXPERIMENTAL — Phase A inline-risk probe. This file is pure diagnostics and
    // is NOT wired into IsMethodInlined / the hot-patch decision path: running it
    // has zero effect on a normal hot reload. Its job is to gather the real Mono
    // data we need before deciding whether to "force-evaluate" a callee's inline
    // risk by JIT-compiling a synthetic caller stub.
    //
    // For each sample method it: (1) reads the Mono inline_info/inline_failure
    // bits BEFORE; (2) builds a DynamicMethod stub that merely CALLS the method
    // and force-JITs it (so Mono's inliner evaluates the callee at compile time);
    // (3) reads the bits AFTER. Comparing before/after answers five questions:
    //   1  small static  — does JITing the stub SET inline_info on a fresh method
    //                       (i.e. is the bit even moveable from the outside)?
    //   2a NoInlining / 2b large — does the rejected/oversized method stay clear
    //                       or set inline_failure (is failure a reliable negative)?
    //   3  AggressiveInlining on a class with a static cctor — does force-JITing
    //                       the stub RUN the callee's cctor (a side effect we must
    //                       guard against)? Tracked via a separate witness class.
    //   4  private instance — does DynamicMethod+skipVisibility JIT it without
    //                       throwing?
    // Plus, per target, it compares two JIT-forcing APIs — RuntimeHelpers
    // .PrepareDelegate vs RuntimeHelpers.PrepareMethod(DynamicMethod.MethodHandle)
    // — since the latter is documented to throw on some runtimes.
    //
    // Reuses TryReadInlineFlags / PredictInlinable from LocusBridge.MonoMethod.cs
    // (same partial class). The proven precedent that PrepareMethod works on the
    // Editor's Mono is PrepareHotPatchShims (LocusBridge.HotReload.cs).
    public static partial class LocusBridge
    {
        // ── probe corpus ──
        // These members are never called anywhere except by the probe, so until
        // the probe force-JITs a caller stub Mono has never evaluated them as an
        // inline candidate — giving probe #1 a genuinely "fresh" before-read on
        // the first run after a domain reload.
        private static class InlineProbeCorpus
        {
            // #1 — tiny static, well under Mono's IL-size inline gate.
            public static int SmallStatic()
            {
                return 7;
            }

            // #2a — explicitly opted out: Mono must evaluate-and-refuse.
            [MethodImpl(MethodImplOptions.NoInlining)]
            public static int NoInline()
            {
                return 7;
            }

            // #2b — well over the IL-size gate (no EH, no NoInlining), so Mono
            // should evaluate it and refuse on size.
            public static int Large(int x)
            {
                int a = x;
                a = a * 3 + 1;
                a ^= 0x5a5a;
                a += a << 2;
                a -= 7;
                a = a * 31 + 11;
                a ^= a >> 3;
                a += 99;
                a *= 2;
                a -= x;
                a += 13;
                return a;
            }

            // #3 — the cctor records its side effect into a SEPARATE
            // beforefieldinit class so the probe can observe "did the cctor run"
            // without itself triggering WithCctor's cctor (any managed read of a
            // WithCctor member would run it and contaminate the measurement).
            private static class CctorWitness
            {
                public static int Ran;
            }

            public static class WithCctor
            {
                static WithCctor()
                {
                    CctorWitness.Ran = 1;
                }

                [MethodImpl(MethodImplOptions.AggressiveInlining)]
                public static int Touch()
                {
                    return 7;
                }
            }

            // Reads the witness without touching WithCctor (CctorWitness has no
            // explicit cctor → beforefieldinit → reading Ran is a side-effect-free
            // way to learn whether WithCctor's cctor has run).
            public static bool WithCctorHasRun()
            {
                return CctorWitness.Ran != 0;
            }

            // #4 — private instance method reached only through skipVisibility.
            public sealed class Inst
            {
                private int _x = 7;

                private int Priv()
                {
                    return _x;
                }
            }
        }

        // A normal small method (NO AggressiveInlining): Mono inlines it only
        // when the runtime is Release-EFFECTIVE and refuses under Debug. So
        // force-JITing a caller stub and reading inline_info is a RUNTIME check
        // of whether Mono is actually inlining right now — which can disagree
        // with the CompilationPipeline.codeOptimization SETTING (observed: the
        // setting reads release while play-mode JIT runs Debug-effective, so no
        // method inlines). The self-test gates its "inlined in Release" assertions
        // on this, not on the setting.
        private static class InliningModeCanary
        {
            public static int Probe()
            {
                return 7;
            }
        }

        private enum InlineProbeApi
        {
            PrepareDelegate,
            PrepareMethodHandle,
        }

        // RuntimeHelpers.PrepareMethod is in .NET Standard and known-good on the
        // Editor's Mono (PrepareHotPatchShims calls it directly). PrepareDelegate
        // is NOT in the .NET Standard reference set, so resolve it reflectively:
        // the file then compiles under any API Compatibility Level and the probe
        // reports at runtime whether the API exists at all.
        private static readonly MethodInfo PrepareDelegateMethod =
            typeof(RuntimeHelpers).GetMethod("PrepareDelegate", new[] { typeof(Delegate) });

        // ── bridge entry (dispatched from HandleMessageAsync) ──
        // Runs on the main thread to mirror PrepareHotPatchShims, the proven
        // force-JIT precedent; the work is pure reflection + JIT (no Unity API)
        // and completes quickly.
        private static async Task<PipeEnvelope> HandleHotReloadInlineProbe(string requestId)
        {
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    tcs.SetResult(OkResponse(requestId, RunInlineProbesText()));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.ToString()));
                }
            });
            return await tcs.Task.ConfigureAwait(false);
        }

        [Serializable]
        private sealed class InliningActiveDto
        {
            public bool inlining_active;
            public string code_optimization;
            public string detail;
        }

        // Cheap runtime "is Mono inlining now?" probe the self-test queries to gate
        // its Release-inline assertions on the JIT's EFFECTIVE behavior instead of
        // the (possibly disagreeing) codeOptimization setting.
        private static async Task<PipeEnvelope> HandleHotReloadInliningActive(string requestId)
        {
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    string detail;
                    bool active = ComputeInliningActive(out detail);
                    bool releaseSetting = false;
                    try
                    {
                        releaseSetting = CompilationPipeline.codeOptimization == CodeOptimization.Release;
                    }
                    catch
                    {
                    }
                    var dto = new InliningActiveDto
                    {
                        inlining_active = active,
                        code_optimization = releaseSetting ? "release" : "debug",
                        detail = detail,
                    };
                    tcs.SetResult(OkResponse(requestId, JsonUtility.ToJson(dto)));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.ToString()));
                }
            });
            return await tcs.Task.ConfigureAwait(false);
        }

        // Force-JIT a caller stub for the normal-small canary and report whether
        // Mono inlined it (inline_info set, inline_failure clear). True ⇒ the
        // runtime is Release-effective and inline-bit data is meaningful. Within a
        // single domain the result is stable; a domain reload resets the bit.
        private static bool ComputeInliningActive(out string detail)
        {
            MethodInfo canary = typeof(InliningModeCanary).GetMethod(
                "Probe", BindingFlags.Public | BindingFlags.Static);
            if (canary == null)
            {
                detail = "canary_not_found";
                return false;
            }

            bool beforeInfo, beforeFail;
            TryReadInlineFlags(canary, out beforeInfo, out beforeFail);
            // Force-JIT a caller stub. Prefer the delegate path; if it is
            // unavailable in this profile (or did not JIT), fall back to the
            // method-handle path so the canary still fires regardless of API.
            string jitNote;
            bool jitted = TryForceEvaluateInlineRisk(canary, InlineProbeApi.PrepareDelegate, out jitNote);
            if (!jitted)
            {
                string handleNote;
                TryForceEvaluateInlineRisk(canary, InlineProbeApi.PrepareMethodHandle, out handleNote);
                jitNote = jitNote + " | handle:" + handleNote;
            }
            bool afterInfo, afterFail;
            bool read = TryReadInlineFlags(canary, out afterInfo, out afterFail);

            detail = "before=" + Bits(beforeInfo, beforeFail)
                + " after=" + (read ? Bits(afterInfo, afterFail) : "read_failed")
                + " jit=" + jitNote;
            return read && afterInfo && !afterFail;
        }

        private static string RunInlineProbesText()
        {
            var sb = new StringBuilder(2048);
            sb.Append("Inline-risk probes (Phase A) — Mono inline-bit force-evaluation\n");
            sb.Append("note: a callee's inline bit is sticky for the domain lifetime; the first run after a domain reload is the clean before-read.\n");

            // SETTING (may disagree with effective JIT inlining). Report the raw
            // value and distinguish a genuine Debug from a read that threw.
            string codeOptSetting;
            try
            {
                codeOptSetting = CompilationPipeline.codeOptimization.ToString().ToLowerInvariant();
            }
            catch (Exception ex)
            {
                codeOptSetting = "read_threw:" + OneLine(ex.Message);
            }
            sb.Append("code_optimization_setting=").Append(codeOptSetting).Append('\n');

            // A managed debugger / Mono soft-debugger agent disables JIT inlining
            // (every method keeps a real entry for breakpoints), so Release won't
            // inline while one is attached — a prime suspect for "setting=release
            // but nothing inlines". IsAttached only catches an ACTIVE attach, not a
            // merely-listening agent, but a true here is conclusive.
            bool debuggerAttached = false;
            try
            {
                debuggerAttached = System.Diagnostics.Debugger.IsAttached;
            }
            catch
            {
            }
            sb.Append("debugger_attached=").Append(debuggerAttached ? "yes" : "no");
            if (debuggerAttached)
                sb.Append("  *** a managed debugger is attached → Mono disables inlining → Release cannot inline ***");
            sb.Append('\n');

            // Runtime ground truth: did Mono actually inline the small canary?
            // This is what makes the inline-bit rows below meaningful — the
            // setting above can read release while the JIT runs Debug-effective.
            string canaryDetail;
            bool inliningActive = ComputeInliningActive(out canaryDetail);
            sb.Append("inlining_active=").Append(inliningActive ? "yes" : "no");
            if (!inliningActive)
                sb.Append("  *** WARNING: Mono is NOT inlining (Debug-effective) — every inline-bit row below is INCONCLUSIVE, re-run in confirmed Release ***");
            sb.Append(" [canary ").Append(canaryDetail).Append("]\n");

            const BindingFlags PublicStatic = BindingFlags.Public | BindingFlags.Static;
            AppendProbeLine(
                sb, "1_small_static",
                typeof(InlineProbeCorpus).GetMethod("SmallStatic", PublicStatic), false);
            AppendProbeLine(
                sb, "2a_no_inlining",
                typeof(InlineProbeCorpus).GetMethod("NoInline", PublicStatic), false);
            AppendProbeLine(
                sb, "2b_large",
                typeof(InlineProbeCorpus).GetMethod("Large", PublicStatic), false);
            AppendProbeLine(
                sb, "3_aggressive_cctor",
                typeof(InlineProbeCorpus.WithCctor).GetMethod("Touch", PublicStatic), true);
            AppendProbeLine(
                sb, "4_private_instance",
                typeof(InlineProbeCorpus.Inst).GetMethod(
                    "Priv", BindingFlags.NonPublic | BindingFlags.Instance), false);

            return sb.ToString();
        }

        private static void AppendProbeLine(
            StringBuilder sb, string id, MethodBase target, bool trackCctor)
        {
            sb.Append("probe=").Append(id);
            if (target == null)
            {
                sb.Append(" ERROR=target_method_not_found\n");
                return;
            }

            sb.Append(" method=").Append(DescribeProbeMethod(target));

            bool cctorBefore = trackCctor && InlineProbeCorpus.WithCctorHasRun();

            bool beforeInfo, beforeFail;
            bool readBefore = TryReadInlineFlags(target, out beforeInfo, out beforeFail);

            // Force-eval via the preferred delegate path first, then via the
            // method-handle path on a fresh stub, reading the bits between so we
            // can attribute any transition (only the first JIT can move a sticky
            // bit; the second call's value is its API stability).
            string delNote;
            bool delOk = TryForceEvaluateInlineRisk(target, InlineProbeApi.PrepareDelegate, out delNote);
            bool midInfo, midFail;
            bool readMid = TryReadInlineFlags(target, out midInfo, out midFail);

            string handleNote;
            bool handleOk = TryForceEvaluateInlineRisk(target, InlineProbeApi.PrepareMethodHandle, out handleNote);
            bool afterInfo, afterFail;
            bool readAfter = TryReadInlineFlags(target, out afterInfo, out afterFail);

            bool cctorAfter = trackCctor && InlineProbeCorpus.WithCctorHasRun();

            sb.Append(" before[")
                .Append(readBefore ? Bits(beforeInfo, beforeFail) : "read_failed").Append(']');
            sb.Append(" delegate[ok=").Append(delOk ? '1' : '0')
                .Append(" note=").Append(delNote).Append(']');
            sb.Append(" mid[")
                .Append(readMid ? Bits(midInfo, midFail) : "read_failed").Append(']');
            sb.Append(" handle[ok=").Append(handleOk ? '1' : '0')
                .Append(" note=").Append(handleNote).Append(']');
            sb.Append(" after[")
                .Append(readAfter ? Bits(afterInfo, afterFail) : "read_failed").Append(']');
            sb.Append(" predict_inlinable=").Append(PredictInlinable(target) ? '1' : '0');
            if (trackCctor)
            {
                sb.Append(" cctor[before=").Append(cctorBefore ? '1' : '0')
                    .Append(" after=").Append(cctorAfter ? '1' : '0').Append(']');
            }
            sb.Append('\n');
        }

        private static string Bits(bool info, bool fail)
        {
            return "info=" + (info ? '1' : '0') + ",fail=" + (fail ? '1' : '0');
        }

        /// <summary>
        /// Build a DynamicMethod that merely CALLS <paramref name="callee"/> with
        /// default arguments and force-JIT it (never invoking it), so Mono's
        /// inliner evaluates the callee at compile time and sets its inline bit.
        /// Returns true if the stub was built and JITed; false (with a note) if a
        /// guard skipped it or the JIT/handle threw. Handles only non-generic
        /// methods with no by-ref/pointer parameters and no value-type receiver —
        /// the same envelope the eventual production evaluator would accept.
        /// </summary>
        private static bool TryForceEvaluateInlineRisk(
            MethodBase callee, InlineProbeApi api, out string note)
        {
            note = "";
            try
            {
                var method = callee as MethodInfo;
                if (method == null)
                {
                    note = "skip:not_methodinfo";
                    return false;
                }
                if (method.IsGenericMethodDefinition || method.ContainsGenericParameters)
                {
                    note = "skip:generic";
                    return false;
                }

                Type declaringType = method.DeclaringType;
                bool hasSelf = !method.IsStatic;
                if (hasSelf && declaringType != null && declaringType.IsValueType)
                {
                    note = "skip:value_type_instance";
                    return false;
                }

                var dm = new DynamicMethod(
                    "__LocusInlineProbe_" + method.Name,
                    typeof(void),
                    Type.EmptyTypes,
                    typeof(LocusBridge).Module,
                    skipVisibility: true);
                ILGenerator il = dm.GetILGenerator();

                if (hasSelf)
                    il.Emit(OpCodes.Ldnull); // never invoked → a null receiver only has to JIT

                foreach (ParameterInfo parameter in method.GetParameters())
                {
                    Type parameterType = parameter.ParameterType;
                    if (parameterType.IsByRef || parameterType.IsPointer ||
                        parameterType == typeof(TypedReference))
                    {
                        note = "skip:byref_or_pointer_param";
                        return false;
                    }
                    EmitDefaultValue(il, parameterType);
                }

                bool useCallvirt = method.IsVirtual && !method.IsFinal;
                il.Emit(useCallvirt ? OpCodes.Callvirt : OpCodes.Call, method);
                if (method.ReturnType != typeof(void))
                    il.Emit(OpCodes.Pop);
                il.Emit(OpCodes.Ret);

                // CreateDelegate finalizes the IL — required before either the
                // delegate is prepared or dm.MethodHandle is read.
                Delegate del = dm.CreateDelegate(typeof(Action));
                if (api == InlineProbeApi.PrepareDelegate)
                {
                    if (PrepareDelegateMethod == null)
                    {
                        note = "skip:PrepareDelegate_unavailable_in_profile";
                        return false;
                    }
                    // A TargetInvocationException is unwrapped by the outer catch.
                    PrepareDelegateMethod.Invoke(null, new object[] { del });
                    note = "prepared_delegate";
                }
                else
                {
                    // DynamicMethod.MethodHandle is documented to throw on some
                    // runtimes — the whole point of this comparison.
                    RuntimeHelpers.PrepareMethod(dm.MethodHandle);
                    note = "prepared_methodhandle";
                }
                return true;
            }
            catch (Exception ex)
            {
                Exception detail = ex.InnerException ?? ex;
                note = "exn:" + detail.GetType().Name + ":" + OneLine(detail.Message);
                return false;
            }
        }

        private static void EmitDefaultValue(ILGenerator il, Type type)
        {
            if (!type.IsValueType)
            {
                il.Emit(OpCodes.Ldnull);
                return;
            }
            // Uniform zeroed value for every value type (primitive, enum, struct):
            // a local + initobj. Correctness of the value is irrelevant — the stub
            // is JITed, never executed.
            LocalBuilder local = il.DeclareLocal(type);
            il.Emit(OpCodes.Ldloca, local);
            il.Emit(OpCodes.Initobj, type);
            il.Emit(OpCodes.Ldloc, local);
        }

        private static string DescribeProbeMethod(MethodBase method)
        {
            string declaring = method.DeclaringType != null ? method.DeclaringType.Name : "?";
            int parameterCount = method.GetParameters().Length;
            string kind = method.IsStatic ? "static" : "instance";
            return declaring + "." + method.Name + "/" + parameterCount + "(" + kind + ")";
        }

        private static string OneLine(string text)
        {
            if (string.IsNullOrEmpty(text))
                return "";
            string flat = text.Replace('\n', ' ').Replace('\r', ' ');
            if (flat.Length > 120)
                flat = flat.Substring(0, 120);
            return flat;
        }
    }
}
