using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using Locus.AssetTesting;

namespace Locus
{
    /// <summary>Bounded prefab inheritance evidence for the asset API adapter.</summary>
    public static class LocusAssetApiPrefabFixture
    {
        public static object Create(string folder)
        {
            if (string.IsNullOrEmpty(folder) || !folder.StartsWith("Assets/LocusAssetApiTests/run-", StringComparison.Ordinal)
                || folder.Contains("..") || folder.Contains("\\") || !AssetDatabase.IsValidFolder(folder))
                throw new Exception("Prefab fixture requires an existing owned run folder");
            string basePath = folder + "/InheritanceBase.prefab";
            string yamlPath = folder + "/InheritanceYaml.prefab";
            string livePath = folder + "/InheritanceLive.prefab";
            if (AssetDatabase.LoadMainAssetAtPath(basePath) != null || AssetDatabase.LoadMainAssetAtPath(yamlPath) != null || AssetDatabase.LoadMainAssetAtPath(livePath) != null)
                throw new Exception("Prefab inheritance fixture already exists");
            var previous = UnityEngine.SceneManagement.SceneManager.GetActiveScene();
            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Additive);
            GameObject source = null;
            GameObject instance = null;
            try
            {
                source = new GameObject("Inheritance base");
                UnityEngine.SceneManagement.SceneManager.MoveGameObjectToScene(source, scene);
                var component = source.AddComponent<LocusAssetApiComponent>();
                component.amount = 17;
                var baseAsset = PrefabUtility.SaveAsPrefabAsset(source, basePath);
                UnityEngine.Object.DestroyImmediate(source); source = null;
                instance = (GameObject)PrefabUtility.InstantiatePrefab(baseAsset, scene);
                component = instance.GetComponent<LocusAssetApiComponent>();
                var serialized = new SerializedObject(component);
                serialized.FindProperty("amount").intValue = 23;
                serialized.ApplyModifiedPropertiesWithoutUndo();
                serialized.Dispose();
                PrefabUtility.RecordPrefabInstancePropertyModifications(component);
                PrefabUtility.SaveAsPrefabAsset(instance, yamlPath);
                UnityEngine.Object.DestroyImmediate(instance); instance = null;
                if (!AssetDatabase.CopyAsset(yamlPath, livePath)) throw new Exception("Variant copy failed");
                return new { basePath, yamlPath, livePath, baseObjects = Inspect(basePath), yamlObjects = Inspect(yamlPath), liveObjects = Inspect(livePath) };
            }
            finally
            {
                if (instance != null) UnityEngine.Object.DestroyImmediate(instance);
                if (source != null) UnityEngine.Object.DestroyImmediate(source);
                EditorSceneManager.CloseScene(scene, true);
                if (previous.IsValid() && previous.isLoaded) UnityEngine.SceneManagement.SceneManager.SetActiveScene(previous);
            }
        }

        public static object Inspect(string path)
        {
            if (string.IsNullOrEmpty(path) || !path.StartsWith("Assets/LocusAssetApiTests/run-", StringComparison.Ordinal)
                || path.Contains("..") || path.Contains("\\")) throw new Exception("Owned fixture path required");
            var root = AssetDatabase.LoadAssetAtPath<GameObject>(path);
            if (root == null) throw new Exception("Prefab fixture not found");
            var records = new List<object>();
            foreach (Transform transform in root.GetComponentsInChildren<Transform>(true))
            {
                records.Add(Identity(transform.gameObject));
                foreach (Component component in transform.GetComponents<Component>())
                    if (component != null) records.Add(Identity(component));
            }
            return records;
        }

        private static object Identity(UnityEngine.Object target)
        {
            string guid; long local;
            AssetDatabase.TryGetGUIDAndLocalFileIdentifier(target, out guid, out local);
            var global = GlobalObjectId.GetGlobalObjectIdSlow(target);
            var source = PrefabUtility.GetCorrespondingObjectFromSource(target);
            string sourceGuid = ""; long sourceId = 0;
            if (source != null) AssetDatabase.TryGetGUIDAndLocalFileIdentifier(source, out sourceGuid, out sourceId);
            var component = target as LocusAssetApiComponent;
            return new { type = target.GetType().FullName, local_id = local.ToString(), guid,
                global_object_id = global.targetObjectId.ToString(), global_prefab_id = global.targetPrefabId.ToString(),
                source_guid = sourceGuid, source_id = sourceId.ToString(), amount = component == null ? (int?)null : component.amount };
        }
    }
}
