using System;
using System.Collections.Generic;
using System.Linq;
using UnityEditor;

namespace Locus
{
    /// <summary>
    /// SaveScene/OpenScene/ImportAsset can invoke a global SaveAssets internally.
    /// While an asset API call is active, only its explicit save destination may
    /// pass that callback. Vetoed user assets retain their live dirty state.
    /// </summary>
    internal sealed class LocusAssetApiSaveGuard : AssetModificationProcessor
    {
        private static readonly object Gate = new object();
        // null means ordinary Editor operation; an empty set protects all assets.
        private static HashSet<string> Allowed;

        private sealed class Scope : IDisposable
        {
            private readonly HashSet<string> previous;
            private bool disposed;
            internal Scope(HashSet<string> current)
            {
                lock (Gate) { previous = Allowed; Allowed = current; }
            }
            public void Dispose()
            {
                lock (Gate)
                {
                    if (disposed) return;
                    Allowed = previous;
                    disposed = true;
                }
            }
        }

        internal static IDisposable Protect()
        {
            return new Scope(new HashSet<string>(StringComparer.OrdinalIgnoreCase));
        }

        internal static IDisposable AllowOnly(string path)
        {
            return new Scope(new HashSet<string>(new[] { Normalize(path) }, StringComparer.OrdinalIgnoreCase));
        }

        private static string Normalize(string path)
        {
            string normalized = (path ?? "").Replace('\\', '/');
            return normalized.EndsWith(".meta", StringComparison.OrdinalIgnoreCase)
                ? normalized.Substring(0, normalized.Length - 5) : normalized;
        }

        private static string[] OnWillSaveAssets(string[] paths)
        {
            lock (Gate)
            {
                if (Allowed == null) return paths;
                return (paths ?? new string[0]).Where(path => Allowed.Contains(Normalize(path))).ToArray();
            }
        }
    }
}
