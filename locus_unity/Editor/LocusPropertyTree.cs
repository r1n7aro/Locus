using UnityEngine;

namespace Locus
{
    /// <summary>
    /// Public, read-only Property Tree formatter available to unity_execute
    /// snippets and other Editor tooling. The snapshot source is shared with
    /// Locus's inspector and unity_yaml_read live path.
    /// </summary>
    public static class LocusPropertyTree
    {
        /// <summary>
        /// Formats a Unity object as the compact, progressively expanded Locus
        /// Property Tree. Compound scalar values such as vectors and colors are
        /// always rendered inline and do not consume an extra depth level.
        /// </summary>
        public static string Format(
            Object target,
            int depth = 2,
            int maxArrayItems = 4)
        {
            return LocusBridge.FormatPropertyTreeForExecute(target, depth, maxArrayItems);
        }
    }
}
