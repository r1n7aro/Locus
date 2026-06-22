// 2026-06-22 - Unity 6.5 (6000.5) replaced the 32-bit InstanceID with the 64-bit
// EntityId and turned GetInstanceID()/IsLoadingAssetPreview(int) into obsolete
// compile errors (CS0619). See issue #95.

using UnityEditor;
using UnityEngine;

namespace Locus
{
    /// <summary>
    /// Compatibility shim for Unity's object-identity APIs.
    ///
    /// Unity 6.5 (6000.5) introduced the 64-bit <see cref="EntityId"/> and marked
    /// <c>UnityEngine.Object.GetInstanceID()</c> and
    /// <c>AssetPreview.IsLoadingAssetPreview(int)</c> as obsolete <em>errors</em>
    /// (CS0619). Those APIs are still valid on Unity 6.4 and earlier, so the new
    /// EntityId calls are compiled in only under <c>UNITY_6000_5_OR_NEWER</c> and
    /// every older editor keeps its original, untouched code path.
    ///
    /// The rest of the package only needs a 32-bit identity for equality checks,
    /// dictionary grouping and the editor-update wire protocol, so on 6.5+ the
    /// 64-bit EntityId is folded down to its low 32 bits (which preserves the
    /// legacy InstanceID value for an object). This keeps that protocol — and the
    /// behaviour on every supported Unity version — unchanged.
    /// </summary>
    internal static class LocusObjectIdentity
    {
        /// <summary>
        /// Stable 32-bit identity for <paramref name="obj"/> within the current
        /// editor session, or 0 when <paramref name="obj"/> is null.
        /// </summary>
        internal static int InstanceId(UnityEngine.Object obj)
        {
            if (obj == null)
                return 0;
#if UNITY_6000_5_OR_NEWER
            return unchecked((int)EntityId.ToULong(obj.GetEntityId()));
#else
            return obj.GetInstanceID();
#endif
        }

        /// <summary>
        /// Whether the asset preview for <paramref name="asset"/> is still being
        /// generated. Wraps the InstanceID -> EntityId overload split from Unity 6.5.
        /// </summary>
        internal static bool IsLoadingAssetPreview(UnityEngine.Object asset)
        {
#if UNITY_6000_5_OR_NEWER
            return AssetPreview.IsLoadingAssetPreview(asset.GetEntityId());
#else
            return AssetPreview.IsLoadingAssetPreview(asset.GetInstanceID());
#endif
        }
    }
}
