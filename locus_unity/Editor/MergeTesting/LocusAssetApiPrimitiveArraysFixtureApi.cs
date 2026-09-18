using System;
using UnityEditor;
using UnityEngine;
using Locus.AssetTesting;

namespace Locus
{
    public static class LocusAssetApiPrimitiveArraysFixtureApi
    {
        [Serializable] public sealed class Pair
        {
            public string yaml;
            public string yaml_live;
            public string live;
            public string kind = "numeric_arrays";
        }
        [Serializable] private sealed class Report { public Pair[] pairs; }

        public static string Create(string folder)
        {
            return JsonUtility.ToJson(new Report { pairs = new[] { CreatePair(folder) } });
        }

        public static Pair CreatePair(string folder)
        {
            if (string.IsNullOrEmpty(folder) || !folder.StartsWith("Assets/LocusAssetApiTests/run-", StringComparison.Ordinal)
                || folder.Contains("..") || folder.Contains("\\") || !AssetDatabase.IsValidFolder(folder))
                throw new Exception("Primitive array fixture requires an existing owned run folder");
            string yaml = folder + "/NumericYaml.asset";
            string online = folder + "/NumericOnline.asset";
            string live = folder + "/NumericLive.asset";
            foreach (string path in new[] { yaml, online, live })
                if (AssetDatabase.LoadMainAssetAtPath(path) != null) throw new Exception("Primitive array fixture already exists: " + path);
            var asset = ScriptableObject.CreateInstance<LocusAssetApiPrimitiveArraysFixture>();
            AssetDatabase.CreateAsset(asset, yaml);
            EditorUtility.SetDirty(asset);
            AssetDatabase.SaveAssetIfDirty(asset);
            if (!AssetDatabase.CopyAsset(yaml, online) || !AssetDatabase.CopyAsset(yaml, live))
                throw new Exception("Primitive array fixture copy failed");
            return new Pair { yaml = yaml, yaml_live = online, live = live };
        }
    }
}
