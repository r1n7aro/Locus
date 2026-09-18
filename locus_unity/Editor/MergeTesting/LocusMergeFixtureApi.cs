using System;
using System.IO;
using UnityEditor;
using UnityEngine;
using UnityEngine.Serialization;

namespace Locus.MergeTesting
{
    public static class LocusMergeFixtureApi
    {
        public const long SharedId = 9007199254740993L;

        [Serializable]
        private sealed class Report
        {
            public string unityVersion;
            public string folder;
            public string snapshots;
            public int localValue;
            public int incomingValue;
            public int health;
            public float speed;
            public string alternateLabel;
            public string alternateType;
            public string childOrder;
            public bool sharedIdentity;
            public bool cycleIdentity;
            public bool nullPreserved;
            public bool missingTypes;
            public string sharedId;
            public float x;
            public float y;
            public bool prefabReferences;
        }

        public static string Generate(string folder)
        {
            ValidateFolder(folder);
            if (Directory.Exists(folder)) throw new InvalidOperationException("Fixture folder already exists: " + folder);
            Directory.CreateDirectory(folder);
            AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
            var snapshots = Path.GetFullPath("Library/LocusMergeDriver/" + Path.GetFileName(folder));
            Directory.CreateDirectory(snapshots);
            var assetPath = folder + "/Graph.asset";
            var asset = ScriptableObject.CreateInstance<LocusMergeFixtureAsset>();
            var group = new MergeGroup { label = "group" };
            var shared = new MergeAction { label = "shared", health = 50, speed = 1.5f };
            var alternate = new MergeAction { label = "alternate", health = 30, speed = 3f };
            group.children.Add(shared);
            group.children.Add(alternate);
            group.children.Add(shared);
            group.next = shared;
            shared.next = group;
            asset.root = group;
            asset.alias = shared;
            asset.rows.Add(new MergeRow { key = "alpha", amount = 1, position = new Vector3(1, 2, 3) });
            asset.rows.Add(new MergeRow { key = "beta", amount = 2, position = new Vector3(4, 5, 6) });
            SetId(asset, group, 101);
            SetId(asset, shared, SharedId);
            SetId(asset, alternate, 103);
            AssetDatabase.CreateAsset(asset, assetPath);
            Save(asset);
            Snapshot(assetPath, snapshots, "graph-base.yaml");
            asset.localValue = 99;
            shared.health = 75;
            Save(asset);
            Snapshot(assetPath, snapshots, "graph-target.yaml");
            alternate.speed = 4f;
            Save(asset);
            Snapshot(assetPath, snapshots, "graph-type-target.yaml");
            alternate.speed = 3f;
            asset.localValue = 10;
            shared.health = 50;
            asset.incomingValue = 88;
            shared.speed = 2.5f;
            alternate.label = "source alternate";
            Save(asset);
            Snapshot(assetPath, snapshots, "graph-source.yaml");
            shared.health = 100;
            Save(asset);
            Snapshot(assetPath, snapshots, "graph-conflict.yaml");
            // Unity 6.5 refuses to reassign a registry id after serialization.
            // A fresh host produces a real alternative history with the same
            // host-local ids and a different concrete type at rid 103.
            var typed = ScriptableObject.CreateInstance<LocusMergeFixtureAsset>();
            typed.localValue = asset.localValue;
            typed.incomingValue = asset.incomingValue;
            typed.note = asset.note;
            typed.rows = new System.Collections.Generic.List<MergeRow>(asset.rows);
            var typedGroup = new MergeGroup { label = group.label };
            var typedShared = new MergeAction { label = shared.label, health = shared.health, speed = shared.speed };
            var weighted = new MergeWeightedAction { label = "weighted", health = 40, weight = 0.75f };
            typedGroup.next = typedShared; typedShared.next = typedGroup;
            typedGroup.children.Add(typedShared); typedGroup.children.Add(weighted); typedGroup.children.Add(typedShared);
            typed.root = typedGroup; typed.alias = typedShared;
            SetId(typed, typedGroup, 101); SetId(typed, typedShared, SharedId); SetId(typed, weighted, 103);
            AssetDatabase.CreateFolder(folder, "TypeVariant");
            var typedPath = folder + "/TypeVariant/Graph.asset";
            AssetDatabase.CreateAsset(typed, typedPath);
            typed.name = asset.name;
            Save(typed);
            Snapshot(typedPath, snapshots, "graph-type-change.yaml");
            typedGroup.children.Clear(); typedGroup.children.Add(typedShared); typedGroup.children.Add(typedShared); typedGroup.children.Add(weighted);
            Save(typed);
            Snapshot(typedPath, snapshots, "graph-reorder-target.yaml");
            typedGroup.children.Clear(); typedGroup.children.Add(weighted); typedGroup.children.Add(typedShared); typedGroup.children.Add(typedShared);
            Save(typed);
            Snapshot(typedPath, snapshots, "graph-reordered.yaml");
            Resources.UnloadAsset(typed);
            Resources.UnloadAsset(asset);
            File.Copy(Path.Combine(snapshots, "graph-base.yaml"), assetPath, true);
            AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.ForceSynchronousImport | ImportAssetOptions.ForceUpdate);
            CreatePrefabFixtures(folder, snapshots, assetPath);
            return JsonUtility.ToJson(new Report { unityVersion = Application.unityVersion, folder = folder, snapshots = snapshots });
        }

        public static string Inspect(string folder)
        {
            ValidateFolder(folder);
            var path = folder + "/Graph.asset";
            var loaded = AssetDatabase.LoadAssetAtPath<LocusMergeFixtureAsset>(path);
            if (loaded != null) Resources.UnloadAsset(loaded);
            AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
            var asset = AssetDatabase.LoadAssetAtPath<LocusMergeFixtureAsset>(path);
            if (asset == null) throw new InvalidOperationException("Merged asset cannot be loaded");
            var group = asset.root as MergeGroup;
            var shared = asset.alias as MergeAction;
            if (group == null || shared == null || group.children.Count < 3) throw new InvalidOperationException("Merged graph type/shape changed");
            var prefabPath = folder + "/Parent.prefab";
            AssetDatabase.ImportAsset(prefabPath, ImportAssetOptions.ForceSynchronousImport | ImportAssetOptions.ForceUpdate);
            var prefab = PrefabUtility.LoadPrefabContents(prefabPath);
            try
            {
                var child = prefab.transform.Find("Child");
                var component = prefab.GetComponent<LocusMergeFixtureComponent>();
                return JsonUtility.ToJson(new Report
                {
                    unityVersion = Application.unityVersion,
                    localValue = asset.localValue,
                    incomingValue = asset.incomingValue,
                    health = shared.health,
                    speed = shared.speed,
                    alternateLabel = group.children.Find(node => !ReferenceEquals(node, asset.alias)).label,
                    alternateType = group.children.Find(node => !ReferenceEquals(node, asset.alias)).GetType().Name,
                    childOrder = String.Join("|", group.children.ConvertAll(node => node.label).ToArray()),
                    sharedIdentity = group.children.FindAll(node => ReferenceEquals(node, asset.alias)).Count == 2,
                    cycleIdentity = ReferenceEquals(shared.next, group) && ReferenceEquals(group.next, shared),
                    nullPreserved = asset.optional == null,
                    missingTypes = UnityEditor.SerializationUtility.HasManagedReferencesWithMissingTypes(asset),
                    sharedId = ManagedReferenceUtility.GetManagedReferenceIdForObject(asset, shared).ToString(System.Globalization.CultureInfo.InvariantCulture),
                    x = child.localPosition.x,
                    y = child.localPosition.y,
                    prefabReferences = component.asset == asset && component.sibling == child.gameObject && PrefabUtility.IsPartOfPrefabInstance(prefab.transform.Find("Nested").gameObject),
                });
            }
            finally { PrefabUtility.UnloadPrefabContents(prefab); }
        }

        private static void CreatePrefabFixtures(string folder, string snapshots, string assetPath)
        {
            var fixtureScene = UnityEditor.SceneManagement.EditorSceneManager.NewPreviewScene();
            try
            {
            var nested = CreateFixtureObject("Nested", fixtureScene);
            var nestedAsset = PrefabUtility.SaveAsPrefabAsset(nested, folder + "/Nested.prefab");
            UnityEngine.Object.DestroyImmediate(nested);
            var parent = CreateFixtureObject("Parent", fixtureScene);
            try
            {
                var child = CreateFixtureObject("Child", fixtureScene);
                child.transform.SetParent(parent.transform, false);
                var instance = (GameObject)PrefabUtility.InstantiatePrefab(nestedAsset, fixtureScene);
                instance.transform.SetParent(parent.transform, false);
                var component = parent.AddComponent<LocusMergeFixtureComponent>();
                component.asset = AssetDatabase.LoadAssetAtPath<LocusMergeFixtureAsset>(assetPath);
                component.sibling = child;
                component.behavior = new MergeAction { label = "prefab action", health = 5, speed = 1 };
                var path = folder + "/Parent.prefab";
                PrefabUtility.SaveAsPrefabAsset(parent, path);
                Snapshot(path, snapshots, "prefab-base.yaml");
                child.transform.localPosition = new Vector3(3, 0, 0);
                PrefabUtility.SaveAsPrefabAsset(parent, path);
                Snapshot(path, snapshots, "prefab-target.yaml");
                child.transform.localPosition = new Vector3(0, 4, 0);
                PrefabUtility.SaveAsPrefabAsset(parent, path);
                Snapshot(path, snapshots, "prefab-source.yaml");
                File.Copy(Path.Combine(snapshots, "prefab-base.yaml"), path, true);
                AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
            }
            finally { UnityEngine.Object.DestroyImmediate(parent); }
            }
            finally
            {
                UnityEditor.SceneManagement.EditorSceneManager.ClosePreviewScene(fixtureScene);
            }
        }

        private static GameObject CreateFixtureObject(string name, UnityEngine.SceneManagement.Scene scene)
        {
            var value = EditorUtility.CreateGameObjectWithHideFlags(name, HideFlags.HideAndDontSave);
            UnityEngine.SceneManagement.SceneManager.MoveGameObjectToScene(value, scene);
            value.hideFlags = HideFlags.None;
            foreach (var component in value.GetComponents<Component>()) component.hideFlags = HideFlags.None;
            return value;
        }

        private static void SetId(UnityEngine.Object host, object value, long id)
        {
            if (!ManagedReferenceUtility.SetManagedReferenceIdForObject(host, value, id))
                throw new InvalidOperationException("Unable to assign stable managed reference id " + id);
        }

        private static void Save(UnityEngine.Object value)
        {
            EditorUtility.SetDirty(value);
            AssetDatabase.SaveAssets();
        }

        private static void Snapshot(string path, string snapshots, string name) { File.Copy(path, Path.Combine(snapshots, name), true); }

        private static void ValidateFolder(string folder)
        {
            if (String.IsNullOrEmpty(folder) || !folder.StartsWith("Assets/LocusMergeDriver-", StringComparison.Ordinal) || folder.Contains("..") || folder.Contains("\\") || folder.Substring(7).Contains("/"))
                throw new ArgumentException("Expected a dedicated Assets/LocusMergeDriver-* fixture folder");
        }
    }
}
