using System;
using System.Reflection;
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

        /// <summary>
        /// Whether a detour on <paramref name="method"/> is bypassed by an
        /// inlined copy at some call site. Reads Mono's cached inline decision
        /// from the _MonoMethod bitfield; when Mono has not evaluated the method
        /// yet (both bits clear — no compiled caller reached it) the runtime
        /// cannot answer, so in Release we fall back to a static prediction using
        /// the same inputs Mono's inliner uses. Any read failure is treated as
        /// "not inlined" (safe — the detour stays, no false recompile).
        /// </summary>
        private static bool IsMethodInlined(MethodBase method, bool releaseMode)
        {
            bool infoSet, failureSet;
            if (!TryReadInlineFlags(method, out infoSet, out failureSet))
                return false;
            if (infoSet)
                return !failureSet; // Mono inlined it (unless it also flagged failure).
            if (failureSet)
                return false;       // Mono evaluated it and refused → the detour holds.
            // Not yet JIT-evaluated as an inline candidate. Only Release inlines
            // at all; predict from the method's own metadata so a method a future
            // caller will inline is not silently reported as live.
            return releaseMode && PredictInlinable(method);
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
