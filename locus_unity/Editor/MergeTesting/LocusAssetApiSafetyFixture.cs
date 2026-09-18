using System;
using System.Collections.Generic;
using UnityEditor;
using Locus.AssetTesting;

namespace Locus
{
    /// <summary>Owned acceptance fixtures only; never saves the dirty test state.</summary>
    public static class LocusAssetApiSafetyFixture
    {
        private sealed class Backup
        {
            public string json;
            public bool dirty;
        }

        private static readonly Dictionary<string, Backup> Backups = new Dictionary<string, Backup>(StringComparer.Ordinal);

        private static LocusAssetApiFixture Resolve(string path)
        {
            if (string.IsNullOrEmpty(path) || !path.StartsWith("Assets/LocusAssetApiTests/run-", StringComparison.Ordinal)
                || path.Contains("..") || path.Contains("\\") || !path.EndsWith(".asset", StringComparison.Ordinal))
                throw new Exception("Safety fixture only accepts an owned asset API test asset");
            var asset = AssetDatabase.LoadAssetAtPath<LocusAssetApiFixture>(path);
            if (asset == null) throw new Exception("Safety fixture asset not found: " + path);
            return asset;
        }

        public static object MarkDirty(string path, int amount)
        {
            var asset = Resolve(path);
            if (!Backups.ContainsKey(path)) Backups[path] = new Backup { json = EditorJsonUtility.ToJson(asset), dirty = EditorUtility.IsDirty(asset) };
            asset.amount = amount;
            EditorUtility.SetDirty(asset);
            return Inspect(path);
        }

        public static object Inspect(string path)
        {
            var asset = Resolve(path);
            return new { path, amount = asset.amount, dirty = EditorUtility.IsDirty(asset) };
        }

        public static object Restore(string path)
        {
            var asset = Resolve(path);
            Backup backup;
            if (Backups.TryGetValue(path, out backup))
            {
                EditorJsonUtility.FromJsonOverwrite(backup.json, asset);
                if (backup.dirty) EditorUtility.SetDirty(asset); else EditorUtility.ClearDirty(asset);
                Backups.Remove(path);
            }
            return Inspect(path);
        }

        public static object RestoreAll()
        {
            var states = new List<object>();
            foreach (string path in new List<string>(Backups.Keys)) states.Add(Restore(path));
            return states;
        }
    }
}
