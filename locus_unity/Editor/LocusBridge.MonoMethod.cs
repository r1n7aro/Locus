using System;
using System.Reflection;
using System.Runtime.InteropServices;

namespace Locus
{
    // Release-first hot reload: detect methods Mono has inlined. In Release the
    // editor's Mono runtime inlines small methods at their call sites; a detour
    // on the original entry point does not change those already-inlined copies,
    // so the patch silently no-ops there until a recompile. We read the
    // `inline_info` bit of the internal `_MonoMethod` struct (the same technique
    // the reference Hot Reload plugin uses) to find them, then converge them via
    // recompile instead of refusing the patch. The Unity Editor always runs on
    // Mono, so this is valid regardless of the project's player scripting
    // backend; any read failure is treated as "not inlined" (safe — the detour
    // stays, no false recompile).
    public static partial class LocusBridge
    {
        // Only the `inline_info` bit is read here; see _MonoMethod in Mono's
        // class-internals.h for the full bitfield.
        [Flags]
        private enum LocusMonoMethodFlags : ushort
        {
            inline_info = 1 << 0,
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
        /// True when Mono has inlined this method somewhere. Best-effort: the
        /// bit is only set after Mono has evaluated the method for inlining
        /// (post-JIT), and any handle/read failure returns false.
        /// </summary>
        private static unsafe bool IsMethodInlined(MethodBase method)
        {
            try
            {
                IntPtr handle = method.MethodHandle.Value;
                if (handle == IntPtr.Zero)
                    return false;
                if (IntPtr.Size == sizeof(long))
                {
                    var ptr = (LocusMonoMethod64*)handle.ToPointer();
                    return (ptr->monoMethodFlags & LocusMonoMethodFlags.inline_info)
                        == LocusMonoMethodFlags.inline_info;
                }
                else
                {
                    var ptr = (LocusMonoMethod32*)handle.ToPointer();
                    return (ptr->monoMethodFlags & LocusMonoMethodFlags.inline_info)
                        == LocusMonoMethodFlags.inline_info;
                }
            }
            catch
            {
                return false;
            }
        }
    }
}
