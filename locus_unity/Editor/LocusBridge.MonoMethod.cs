using System;
using System.Collections.Generic;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Locus
{
    // Release-first hot reload: detect methods Mono has inlined. In Release the
    // editor's Mono runtime inlines small methods at their call sites; a detour
    // on the original entry point does not change those already-inlined copies,
    // so the patch silently no-ops there until a recompile. We read the
    // `inline_info`/`inline_failure` bits of the internal `_MonoMethod` struct
    // (the same technique the reference Hot Reload plugin uses) to find them:
    // inline_info without inline_failure means inlined; inline_failure means
    // Mono evaluated it and refused, so the detour holds. When neither bit is
    // set yet (no compiled caller has reached the method) the runtime cannot
    // answer, so we fall back — in Release only — to a static prediction from
    // the method's own metadata, so a method a future caller will inline is not
    // missed. Matches converge via recompile instead of refusing the patch. The
    // Unity Editor always runs on Mono, so this is valid regardless of the
    // project's player scripting backend; any read failure is treated as "not
    // inlined" (safe — the detour stays, no false recompile).
    public static partial class LocusBridge
    {
        // Adjacent bits in the _MonoMethod bitfield (LSB-first on the
        // little-endian targets the Editor runs on); see _MonoMethod in Mono's
        // class-internals.h for the full layout. Mono's inliner sets exactly one
        // of them when it first evaluates a method as an inline candidate.
        [Flags]
        private enum LocusMonoMethodFlags : ushort
        {
            inline_info = 1 << 0,    // evaluated AND inlined
            inline_failure = 1 << 1, // evaluated AND refused (too big / NoInlining)
        }

        // `monoMethodFlags` sits at a fixed offset in the _MonoMethod struct
        // (after flags/iflags/token/klass/signature/name). Explicit layout with
        // Size lets us declare only the field we read; the rest is padding.
        [StructLayout(LayoutKind.Explicit, Size = 8 + sizeof(long) * 3 + 4)]
        private struct LocusMonoMethod64
        {
            [FieldOffset(8 + sizeof(long) * 3)]
            public LocusMonoMethodFlags monoMethodFlags;
        }

        [StructLayout(LayoutKind.Explicit, Size = 8 + sizeof(int) * 3 + 4)]
        private struct LocusMonoMethod32
        {
            [FieldOffset(8 + sizeof(int) * 3)]
            public LocusMonoMethodFlags monoMethodFlags;
        }

        // How a detour relates to Mono's inline decision for a method. A six-state
        // model collapsed to five: Gate A (real Unity Mono) established that
        // force-JITing a synthetic caller stub reliably SETS inline_info for a
        // method Mono will inline, but NEVER sets inline_failure for one it refuses
        // — so there is no reliable "stub rejected" signal and that state is
        // dropped. The three "risk" states (RuntimeInlined/StubInlined/Predicted)
        // route the method to caller refresh + a convergence recompile, so a
        // misprediction costs only a refresh, never correctness; RuntimeRejected
        // and NotRisk leave the live detour as the sole, sufficient mechanism.
        //
        // The two bits live in the _MonoMethod bitfield (Mono's
        // metadata/class-internals.h: `inline_info` / `inline_failure`, adjacent,
        // LSB-first). They are written by Mono's inliner in mini/method-to-ir.c:
        // `mono_method_check_inlining` evaluates a callee (IL size / impl flags / EH
        // / cctor / caller cfg) and `inline_method` records the result. Because that
        // check also reads the *caller's* cfg, a stub-set bit means "Mono would
        // inline this callee in the stub's context" — a strong signal, NOT a proof
        // the real caller did; hence StubInlined is reported as high-confidence, not
        // confirmed. force-evaluation is a precision/latency optimization only:
        // disabling it (config unity_inline_force_evaluate_enabled) reverts to the
        // pure RuntimeInlined/RuntimeRejected/Predicted/NotRisk behavior with no
        // change to convergence correctness.
        internal enum InlineRiskSource
        {
            NotRisk,         // detour holds — not (and not predicted to be) inlined
            RuntimeInlined,  // inline_info set, inline_failure clear — Mono inlined it
            RuntimeRejected, // inline_failure set — Mono evaluated it and refused
            StubInlined,     // force-JIT made Mono set inline_info — high-confidence
            Predicted,       // static heuristic predicts Mono would inline it
        }

        /// <summary>
        /// Whether a detour on <paramref name="method"/> is bypassed by an inlined
        /// copy at some call site — the bool view of
        /// <see cref="ClassifyInlineRisk"/>, preserved for the original contract.
        /// </summary>
        private static bool IsMethodInlined(MethodBase method, bool releaseMode, bool forceEvaluate)
        {
            return IsInlineRiskSource(ClassifyInlineRisk(method, releaseMode, forceEvaluate));
        }

        /// <summary>True for the states whose detour is bypassed at an inlined (or
        /// predicted-inlined) call site, so the method must be reported to the
        /// desktop for convergence.</summary>
        private static bool IsInlineRiskSource(InlineRiskSource source)
        {
            return source == InlineRiskSource.RuntimeInlined
                || source == InlineRiskSource.StubInlined
                || source == InlineRiskSource.Predicted;
        }

        /// <summary>
        /// Classify how a detour on <paramref name="method"/> relates to Mono's
        /// inline decision. Reads the cached _MonoMethod bits first; when Mono has
        /// not evaluated the method yet (both bits clear — no compiled caller
        /// reached it) the runtime cannot answer, so in Release we either force the
        /// evaluation NOW by JIT-ing a synthetic caller stub
        /// (<paramref name="forceEvaluate"/>, Phase B, experimental) or fall back to
        /// a static prediction from the method's own metadata. Any read failure is
        /// NotRisk (safe — the detour stays, no false recompile). With
        /// <paramref name="forceEvaluate"/> false this is byte-identical to the
        /// original three-line decision.
        /// </summary>
        private static InlineRiskSource ClassifyInlineRisk(
            MethodBase method, bool releaseMode, bool forceEvaluate)
        {
            bool infoSet, failureSet;
            if (!TryReadInlineFlags(method, out infoSet, out failureSet))
                return InlineRiskSource.NotRisk;
            if (infoSet)
                return failureSet ? InlineRiskSource.NotRisk : InlineRiskSource.RuntimeInlined;
            if (failureSet)
                return InlineRiskSource.RuntimeRejected;
            // Neither bit set: Mono has not evaluated the method as an inline
            // candidate. Only Release inlines at all.
            if (!releaseMode)
                return InlineRiskSource.NotRisk;
            if (forceEvaluate)
            {
                bool stubInfo, stubFailure;
                if (TryForceEvaluateCalleeInlineRisk(method, out stubInfo, out stubFailure)
                    && stubInfo && !stubFailure)
                {
                    return InlineRiskSource.StubInlined;
                }
                // Stub ran but Mono did not mark inline (no reliable negative —
                // Gate A) or a guard skipped it: fall through to prediction.
            }
            return PredictInlinable(method) ? InlineRiskSource.Predicted : InlineRiskSource.NotRisk;
        }

        // Domain-lifetime memo of method handles whose inline risk we have already
        // force-evaluated, so a method Mono refuses to inline (its bits stay clear
        // even after the stub JITs) is not re-stubbed on a later apply. The inline
        // bit is sticky for the domain, so a cache hit just re-reads it. Touched
        // only on the main thread (hot-patch apply runs there); a domain reload
        // drops the static along with the bits it tracked.
        private static readonly HashSet<IntPtr> _inlineForceEvaluated = new HashSet<IntPtr>();

        /// <summary>
        /// Force Mono to evaluate <paramref name="method"/> as an inline candidate
        /// NOW by JIT-compiling a synthetic caller stub that merely calls it (never
        /// invoking it), then re-read its inline bits. Returns true when the stub
        /// was built and JITed — <paramref name="infoSet"/>/<paramref name="failureSet"/>
        /// then carry Mono's verdict; false when a guard skipped it or the JIT threw
        /// (the caller then falls back to <see cref="PredictInlinable"/>). Gate A
        /// established the stub reliably SETS inline_info for a method Mono will
        /// inline but never sets inline_failure for a refusal, so only a positive is
        /// actionable.
        ///
        /// Guards (any hit → skip, caller predicts): constructors, generic methods,
        /// value-type receivers, by-ref/pointer/typedref parameters,
        /// virtual/abstract methods (a null receiver cannot represent
        /// devirtualization), and — critically — any declaring type with a static
        /// initializer, because force-JITing the stub RUNS that cctor (Gate A probe
        /// 3: a user-visible side effect we must not trigger early). Every guarded
        /// method falls back to prediction, so correctness is unaffected.
        /// </summary>
        private static bool TryForceEvaluateCalleeInlineRisk(
            MethodBase method, out bool infoSet, out bool failureSet)
        {
            infoSet = false;
            failureSet = false;
            try
            {
                var target = method as MethodInfo;
                if (target == null)
                    return false; // constructors are never force-evaluated

                IntPtr handleKey = target.MethodHandle.Value;
                if (handleKey != IntPtr.Zero && _inlineForceEvaluated.Contains(handleKey))
                    return TryReadInlineFlags(target, out infoSet, out failureSet);

                if (target.IsGenericMethodDefinition || target.ContainsGenericParameters)
                    return false;
                if (target.IsVirtual || target.IsAbstract)
                    return false;

                Type declaringType = target.DeclaringType;
                bool hasSelf = !target.IsStatic;
                if (hasSelf && declaringType != null && declaringType.IsValueType)
                    return false;
                // cctor side-effect guard (Gate A): a static initializer runs when
                // the stub JITs. TypeInitializer is non-null for both an explicit
                // static constructor and compiler-generated static-field init.
                if (declaringType != null && declaringType.TypeInitializer != null)
                    return false;

                ParameterInfo[] parameters = target.GetParameters();
                foreach (ParameterInfo parameter in parameters)
                {
                    Type parameterType = parameter.ParameterType;
                    if (parameterType.IsByRef || parameterType.IsPointer ||
                        parameterType == typeof(TypedReference))
                    {
                        return false;
                    }
                }

                var dm = new DynamicMethod(
                    "__LocusInlineEval_" + target.Name,
                    typeof(void),
                    Type.EmptyTypes,
                    typeof(LocusBridge).Module,
                    skipVisibility: true);
                ILGenerator il = dm.GetILGenerator();
                if (hasSelf)
                    il.Emit(OpCodes.Ldnull); // never invoked → a null receiver only has to JIT
                foreach (ParameterInfo parameter in parameters)
                    EmitDefaultStubArgument(il, parameter.ParameterType);
                il.Emit(OpCodes.Call, target); // non-virtual by the guard above
                if (target.ReturnType != typeof(void))
                    il.Emit(OpCodes.Pop);
                il.Emit(OpCodes.Ret);

                // CreateDelegate finalizes the IL; PrepareMethod force-JITs it
                // (Gate A: stable on Unity's Mono, no reflection needed).
                dm.CreateDelegate(typeof(Action));
                RuntimeHelpers.PrepareMethod(dm.MethodHandle);

                if (handleKey != IntPtr.Zero)
                    _inlineForceEvaluated.Add(handleKey);
                return TryReadInlineFlags(target, out infoSet, out failureSet);
            }
            catch
            {
                // Any failure → not force-evaluated; caller falls back to prediction.
                return false;
            }
        }

        // Push a zeroed value of `type` for the synthetic call stub: ldnull for
        // reference types, a local + initobj for any value type. The stub is JITed
        // but never executed, so the actual value is irrelevant — only the IL must
        // be type-correct. (Mirrors the diagnostic probe's EmitDefaultValue; kept
        // here so the production force-evaluation path is self-contained.)
        private static void EmitDefaultStubArgument(ILGenerator il, Type type)
        {
            if (!type.IsValueType)
            {
                il.Emit(OpCodes.Ldnull);
                return;
            }
            LocalBuilder local = il.DeclareLocal(type);
            il.Emit(OpCodes.Ldloca, local);
            il.Emit(OpCodes.Initobj, type);
            il.Emit(OpCodes.Ldloc, local);
        }

        private static unsafe bool TryReadInlineFlags(
            MethodBase method, out bool infoSet, out bool failureSet)
        {
            infoSet = false;
            failureSet = false;
            try
            {
                IntPtr handle = method.MethodHandle.Value;
                if (handle == IntPtr.Zero)
                    return false;
                LocusMonoMethodFlags flags;
                if (IntPtr.Size == sizeof(long))
                    flags = ((LocusMonoMethod64*)handle.ToPointer())->monoMethodFlags;
                else
                    flags = ((LocusMonoMethod32*)handle.ToPointer())->monoMethodFlags;
                infoSet = (flags & LocusMonoMethodFlags.inline_info) != 0;
                failureSet = (flags & LocusMonoMethodFlags.inline_failure) != 0;
                return true;
            }
            catch
            {
                return false;
            }
        }

        // Mono's default IL-size gate for inlining (mono/mini INLINE_LENGTH_LIMIT).
        // A method at or below this with no exception-handling clauses is inlined
        // unless marked NoInlining; AggressiveInlining bypasses the size gate.
        private const int InlineIlSizeLimit = 20;

        /// <summary>
        /// Predict whether Mono's inliner WOULD inline this method, mirroring its
        /// gate (impl flags + IL size + EH clauses), for when the runtime bit has
        /// not been set yet. Errs toward "inlinable" only within Mono's own size
        /// limit; any reflection failure returns false.
        /// </summary>
        private static bool PredictInlinable(MethodBase method)
        {
            try
            {
                MethodImplAttributes impl = method.MethodImplementationFlags;
                if ((impl & MethodImplAttributes.NoInlining) != 0)
                    return false;
                if ((impl & MethodImplAttributes.AggressiveInlining) != 0)
                    return true;
                MethodBody body = method.GetMethodBody();
                if (body == null)
                    return false; // abstract/extern/runtime: no IL to inline.
                if (body.ExceptionHandlingClauses.Count > 0)
                    return false; // Mono does not inline methods with EH clauses.
                byte[] il = body.GetILAsByteArray();
                return il != null && il.Length <= InlineIlSizeLimit;
            }
            catch
            {
                return false;
            }
        }
    }
}
