
using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.Globalization;
using System.Text;
using System.Collections.Generic;
using System.Reflection;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private struct ReadYamlArgs
        {
            public string file_path;
            public string object_path;
        }

        /// <summary>
        /// </summary>
        private static async Task<PipeEnvelope> HandleReadYaml(string requestId, string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                return ErrorResponse(requestId, "empty read_yaml args");

            ReadYamlArgs args;
            try
            {
                args = JsonUtility.FromJson<ReadYamlArgs>(json);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "invalid read_yaml args: " + ex.Message);
            }

            if (string.IsNullOrEmpty(args.file_path))
                return ErrorResponse(requestId, "missing file_path");

            string result = await RunReadYamlOnMainThreadAsync(args);

            if (result.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                return ErrorResponse(requestId, result.Substring("__ERROR__: ".Length));

            return OkResponse(requestId, result);
        }

        private static async Task<string> RunReadYamlOnMainThreadAsync(ReadYamlArgs args)
        {
            var tcs = new TaskCompletionSource<string>();

            PostToMainThread(delegate
            {
                try
                {
                    string output = ExecuteReadYaml(args);
                    tcs.TrySetResult(output);
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] read_yaml exception: " + ex);
                    tcs.TrySetResult("__ERROR__: " + ex);
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return "__ERROR__: read_yaml timed out";

            return tcs.Task.Result ?? "";
        }

        /// <summary>
        /// </summary>
        private static string ExecuteReadYaml(ReadYamlArgs args)
        {
            string filePath = TrimToProjectAssetPath(args.file_path);
            if (string.IsNullOrEmpty(filePath))
                return "__ERROR__: file_path must be under Assets/ or Packages/: " + args.file_path;

            string ext = System.IO.Path.GetExtension(filePath).ToLowerInvariant();
            bool isScene = ext == ".unity";
            bool isPrefab = ext == ".prefab";

            if (!isScene && !isPrefab)
                return "__ERROR__: read_yaml only supports .unity and .prefab files via Unity API";

            string objectPath = string.IsNullOrEmpty(args.object_path) ? null : args.object_path;

            if (isScene)
                return ReadScene(filePath, objectPath);
            else
                return ReadPrefab(filePath, objectPath);
        }

        // ───────────────── Scene reading ─────────────────

        /// <summary>
        /// </summary>
        private static string ReadScene(string scenePath, string objectPath)
        {
            var activeScene = EditorSceneManager.GetActiveScene();
            bool isActiveScene = activeScene.IsValid() && activeScene.path == scenePath;

            if (!isActiveScene)
            {
                for (int i = 0; i < EditorSceneManager.sceneCount; i++)
                {
                    var s = EditorSceneManager.GetSceneAt(i);
                    if (s.IsValid() && s.isLoaded && s.path == scenePath)
                    {
                        return ReadSceneContents(s, scenePath, objectPath);
                    }
                }
                return "__ERROR__: scene not loaded in editor, falling back to YAML parsing";
            }

            return ReadSceneContents(activeScene, scenePath, objectPath);
        }

        private static string ReadSceneContents(UnityEngine.SceneManagement.Scene scene, string scenePath, string objectPath)
        {
            var roots = scene.GetRootGameObjects();

            if (objectPath != null)
                return ReadGameObjectDetail(roots, objectPath, scenePath);

            return BuildHierarchySummary(roots, scenePath);
        }

        // ───────────────── Prefab reading ─────────────────

        /// <summary>
        /// </summary>
        private static string ReadPrefab(string prefabPath, string objectPath)
        {
            var prefabAsset = AssetDatabase.LoadAssetAtPath<GameObject>(prefabPath);
            if (prefabAsset == null)
                return "__ERROR__: failed to load prefab: " + prefabPath;

            if (objectPath != null)
                return ReadGameObjectDetail(new[] { prefabAsset }, objectPath, prefabPath);

            return BuildHierarchySummary(new[] { prefabAsset }, prefabPath);
        }

        // ───────────────── Hierarchy summary ─────────────────

        /// <summary>
        /// </summary>
        private sealed class HierarchySummaryNode
        {
            public string Name;
            public string NormalizedName;
            public string ComponentSuffix;
            public string ComponentSignature;
            public string Annotations;
            public bool IsPrefabRoot;
            public bool BoneFolded;
            public int BoneDescCount;
            public List<HierarchySummaryNode> Children = new List<HierarchySummaryNode>();
            public string StructureSignature;
        }

        private sealed class HierarchySummaryGroup
        {
            public HierarchySummaryNode Representative;
            public List<HierarchySummaryNode> Members = new List<HierarchySummaryNode>();
        }

        private static string BuildHierarchySummary(GameObject[] roots, string filePath)
        {
            var sb = new StringBuilder();
            bool isScene = filePath.EndsWith(".unity", StringComparison.OrdinalIgnoreCase);

            sb.AppendLine(isScene ? "Scene: " + filePath : "Prefab: " + filePath);
            sb.AppendLine("Top-level objects: " + roots.Length);

            int prefabInstanceCount = 0;
            var uniquePrefabSources = new HashSet<string>();
            CountPrefabInstances(roots, ref prefabInstanceCount, uniquePrefabSources);

            if (prefabInstanceCount > 0)
            {
                sb.AppendLine("Unique prefab sources: " + uniquePrefabSources.Count);
                sb.AppendLine("Total prefab instances: " + prefabInstanceCount);
            }

            var boneTransforms = new HashSet<Transform>();
            if (roots.Length > 1)
            {
                foreach (var root in roots)
                {
                    foreach (var smr in root.GetComponentsInChildren<SkinnedMeshRenderer>(true))
                    {
                        if (smr.bones == null) continue;
                        foreach (var bone in smr.bones)
                            if (bone != null) boneTransforms.Add(bone);
                    }
                }
            }

            sb.AppendLine();
            sb.AppendLine("── Hierarchy ──");
            sb.AppendLine();

            var summaryRoots = new List<HierarchySummaryNode>();
            foreach (var go in roots)
                summaryRoots.Add(BuildHierarchySummaryNode(go, boneTransforms));
            WriteGroupedSummaryNodes(sb, summaryRoots, 0);

            // Drill-down hints
            sb.AppendLine();
            sb.AppendLine("Drill down with object_path:");
            sb.AppendLine("- \"ObjectName\" → GameObject components detail");
            sb.AppendLine("- \"Parent/Child\" → nested GameObject components");
            sb.AppendLine("- Use exact names from \"Instances\" lines when a folded group has duplicates");

            return sb.ToString();
        }

        private static HierarchySummaryNode BuildHierarchySummaryNode(GameObject go, HashSet<Transform> boneTransforms)
        {
            var node = new HierarchySummaryNode
            {
                Name = go.name,
                NormalizedName = StripNumericSuffix(go.name),
                ComponentSuffix = BuildComponentSuffix(go),
                ComponentSignature = BuildComponentSignature(go),
                Annotations = BuildGoAnnotations(go),
                IsPrefabRoot = PrefabUtility.IsAnyPrefabInstanceRoot(go),
            };

            int childCount = go.transform.childCount;
            if (childCount > 0 && boneTransforms.Count > 0 && AreAllChildrenBones(go.transform, boneTransforms))
            {
                int descCount = CountDescendants(go.transform);
                if (descCount >= 3)
                {
                    node.BoneFolded = true;
                    node.BoneDescCount = descCount;
                    node.StructureSignature = BuildNodeStructureSignature(node);
                    return node;
                }
            }

            for (int i = 0; i < childCount; i++)
                node.Children.Add(BuildHierarchySummaryNode(go.transform.GetChild(i).gameObject, boneTransforms));

            node.StructureSignature = BuildNodeStructureSignature(node);
            return node;
        }

        private static string BuildNodeStructureSignature(HierarchySummaryNode node)
        {
            var sb = new StringBuilder();
            sb.Append("name:").Append(node.NormalizedName)
              .Append("|components:").Append(node.ComponentSignature)
              .Append("|annotations:").Append(node.Annotations)
              .Append("|prefabRoot:").Append(node.IsPrefabRoot ? "1" : "0");

            if (node.BoneFolded)
            {
                sb.Append("|bones:").Append(node.BoneDescCount);
                return sb.ToString();
            }

            sb.Append("|children:[");
            for (int i = 0; i < node.Children.Count; i++)
            {
                if (i > 0) sb.Append("||");
                sb.Append(node.Children[i].StructureSignature);
            }
            sb.Append("]");
            return sb.ToString();
        }

        private static bool AreAllChildrenBones(Transform parent, HashSet<Transform> boneTransforms)
        {
            int childCount = parent.childCount;
            if (childCount == 0) return false;

            for (int i = 0; i < childCount; i++)
            {
                if (!boneTransforms.Contains(parent.GetChild(i)))
                    return false;
            }

            return true;
        }

        private static void WriteGroupedSummaryNodes(StringBuilder sb, List<HierarchySummaryNode> nodes, int depth)
        {
            var groups = GroupSummaryNodes(nodes);
            string indent = new string(' ', depth * 2);

            foreach (var group in groups)
            {
                var representative = group.Representative;

                if (group.Members.Count > 1)
                {
                    sb.AppendLine(indent + FormatSummaryNodeLabel(representative, true) + " ×" + group.Members.Count);
                    sb.AppendLine(indent + "  Instances: " + FormatInstanceSample(group.Members));
                    if (!representative.BoneFolded && representative.Children.Count > 0)
                    {
                        sb.AppendLine(indent + "  Shared subtree:");
                        WriteGroupedSummaryNodes(sb, representative.Children, depth + 2);
                    }
                    continue;
                }

                sb.AppendLine(indent + FormatSummaryNodeLabel(representative, false));
                if (!representative.BoneFolded && representative.Children.Count > 0)
                    WriteGroupedSummaryNodes(sb, representative.Children, depth + 1);
            }
        }

        private static List<HierarchySummaryGroup> GroupSummaryNodes(List<HierarchySummaryNode> nodes)
        {
            var groups = new List<HierarchySummaryGroup>();
            var groupIndex = new Dictionary<string, int>(StringComparer.Ordinal);

            foreach (var node in nodes)
            {
                int idx;
                if (groupIndex.TryGetValue(node.StructureSignature, out idx))
                {
                    groups[idx].Members.Add(node);
                    continue;
                }

                idx = groups.Count;
                groupIndex[node.StructureSignature] = idx;
                var group = new HierarchySummaryGroup();
                group.Representative = node;
                group.Members.Add(node);
                groups.Add(group);
            }

            return groups;
        }

        private static string FormatSummaryNodeLabel(HierarchySummaryNode node, bool collapsed)
        {
            string prefix = (!collapsed && node.IsPrefabRoot) ? "[P] " : "";
            string name = collapsed ? node.NormalizedName : node.Name;
            string suffix = prefix + name + node.ComponentSuffix + node.Annotations;
            if (node.BoneFolded)
                suffix += " [" + node.BoneDescCount + " bones]";
            return suffix;
        }

        private static string FormatInstanceSample(List<HierarchySummaryNode> members)
        {
            const int sampleLimit = 5;
            var sample = new List<string>();
            int count = members.Count < sampleLimit ? members.Count : sampleLimit;
            for (int i = 0; i < count; i++)
                sample.Add(members[i].Name);

            if (members.Count <= sampleLimit)
                return string.Join(", ", sample.ToArray());

            return string.Join(", ", sample.ToArray()) + ", ... +" + (members.Count - sampleLimit);
        }

        /// <summary>
        /// </summary>
        private static int CountDescendants(Transform t)
        {
            int count = t.childCount;
            for (int i = 0; i < t.childCount; i++)
                count += CountDescendants(t.GetChild(i));
            return count;
        }

        /// <summary>
        /// </summary>
        private static string BuildComponentSuffix(GameObject go)
        {
            var components = go.GetComponents<Component>();
            var names = new List<string>();
            foreach (var comp in components)
            {
                if (comp == null) continue;
                string typeName = comp.GetType().Name;
                if (typeName == "Transform" || typeName == "RectTransform" || typeName == "CanvasRenderer")
                    continue;
                names.Add(typeName);
            }
            if (names.Count == 0) return "";
            return " (" + string.Join(", ", names) + ")";
        }

        // ───────────────── GameObject detail ─────────────────

        /// <summary>
        /// </summary>
        private static string ReadGameObjectDetail(GameObject[] roots, string objectPath, string filePath)
        {
            GameObject target = FindGameObjectByPath(roots, objectPath);
            if (target == null)
            {
                var rootNames = new List<string>();
                foreach (var r in roots)
                    rootNames.Add(r.name);
                return "__ERROR__: GameObject '" + objectPath + "' not found. Available roots: " + string.Join(", ", rootNames);
            }

            var sb = new StringBuilder();
            sb.AppendLine("Components of '" + objectPath + "' (" + filePath + "):");

            sb.AppendLine();
            sb.AppendLine("--- GameObject ---");
            sb.AppendLine("  Name: " + target.name);
            sb.AppendLine("  Active: " + (target.activeSelf ? "true" : "false"));
            sb.AppendLine("  Static: " + (target.isStatic ? "true" : "false"));
            sb.AppendLine("  Layer: " + target.layer + " (" + LayerMask.LayerToName(target.layer) + ")");
            sb.AppendLine("  Tag: " + target.tag);
            if (PrefabUtility.IsPartOfAnyPrefab(target))
            {
                var srcObj = PrefabUtility.GetCorrespondingObjectFromOriginalSource(target);
                if (srcObj != null)
                {
                    string srcPath = AssetDatabase.GetAssetPath(srcObj);
                    if (!string.IsNullOrEmpty(srcPath))
                        sb.AppendLine("  Source Prefab: " + srcPath);
                }
                var nearestRoot = PrefabUtility.GetNearestPrefabInstanceRoot(target);
                if (nearestRoot != null && nearestRoot != target)
                    sb.AppendLine("  Prefab Instance Root: " + nearestRoot.name);
            }

            var components = target.GetComponents<Component>();
            foreach (var comp in components)
            {
                if (comp == null)
                {
                    sb.AppendLine("\n--- Missing Script ---");
                    continue;
                }

                sb.AppendLine("\n--- " + comp.GetType().Name + " ---");
                bool isEnabled;
                if (TryGetComponentEnabledState(comp, out isEnabled))
                    sb.AppendLine("  Enabled: " + (isEnabled ? "true" : "false"));
                AppendWorldTransformFields(sb, comp);

                var so = new SerializedObject(comp);
                var prop = so.GetIterator();
                bool enterChildren = true;

                while (prop.NextVisible(enterChildren))
                {
                    enterChildren = false;
                    if (prop.name == "m_Enabled")
                        continue;
                    FormatSerializedProperty(sb, prop, 1);
                }
            }

            return sb.ToString();
        }

        private static bool TryGetComponentEnabledState(Component comp, out bool enabled)
        {
            enabled = false;
            if (comp == null)
                return false;

            var prop = comp.GetType().GetProperty(
                "enabled",
                BindingFlags.Instance | BindingFlags.Public
            );

            if (prop == null
                || prop.PropertyType != typeof(bool)
                || !prop.CanRead
                || prop.GetIndexParameters().Length != 0)
                return false;

            try
            {
                enabled = (bool)prop.GetValue(comp, null);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static void AppendWorldTransformFields(StringBuilder sb, Component comp)
        {
            var transform = comp as Transform;
            if (transform == null)
                return;

            if (transform.parent != null)
                sb.AppendLine("  parent: {" + FormatHierarchyNodeLabel(transform.parent.gameObject) + "}");
            else
                sb.AppendLine("  parent: {none}");

            if (transform.childCount == 0)
            {
                sb.AppendLine("  children: []");
            }
            else
            {
                sb.AppendLine("  children:");
                for (int i = 0; i < transform.childCount; i++)
                {
                    var child = transform.GetChild(i);
                    sb.AppendLine("    - {" + FormatHierarchyNodeLabel(child.gameObject) + "}");
                }
            }

            sb.AppendLine("  World Position: " + FormatVector3(transform.position));
            sb.AppendLine("  World Rotation: " + FormatVector3(transform.rotation.eulerAngles));
            sb.AppendLine("  World Scale: " + FormatVector3(transform.lossyScale));
        }

        /// <summary>
        /// </summary>
        private static void FormatSerializedProperty(StringBuilder sb, SerializedProperty prop, int indentLevel)
        {
            string indent = new string(' ', indentLevel * 2);
            string name = prop.displayName;

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                    sb.AppendLine(indent + name + ": " + prop.intValue);
                    break;
                case SerializedPropertyType.Boolean:
                    sb.AppendLine(indent + name + ": " + (prop.boolValue ? "true" : "false"));
                    break;
                case SerializedPropertyType.Float:
                    sb.AppendLine(indent + name + ": " + prop.floatValue.ToString("G5"));
                    break;
                case SerializedPropertyType.String:
                    sb.AppendLine(indent + name + ": \"" + prop.stringValue + "\"");
                    break;
                case SerializedPropertyType.Enum:
                    sb.AppendLine(indent + name + ": " + prop.enumDisplayNames[prop.enumValueIndex]);
                    break;
                case SerializedPropertyType.ObjectReference:
                {
                    var obj = prop.objectReferenceValue;
                    if (obj != null)
                    {
                        sb.AppendLine(indent + name + ": " + FormatObjectReference(obj));
                    }
                    else
                    {
                        sb.AppendLine(indent + name + ": None");
                    }
                    break;
                }
                case SerializedPropertyType.Vector2:
                    sb.AppendLine(indent + name + ": " + prop.vector2Value);
                    break;
                case SerializedPropertyType.Vector3:
                    sb.AppendLine(indent + name + ": " + prop.vector3Value);
                    break;
                case SerializedPropertyType.Vector4:
                    sb.AppendLine(indent + name + ": " + prop.vector4Value);
                    break;
                case SerializedPropertyType.Quaternion:
                    sb.AppendLine(indent + name + ": " + prop.quaternionValue.eulerAngles);
                    break;
                case SerializedPropertyType.Color:
                    sb.AppendLine(indent + name + ": " + prop.colorValue);
                    break;
                case SerializedPropertyType.Rect:
                    sb.AppendLine(indent + name + ": " + prop.rectValue);
                    break;
                case SerializedPropertyType.LayerMask:
                    sb.AppendLine(indent + name + ": " + prop.intValue);
                    break;
                case SerializedPropertyType.ArraySize:
                    sb.AppendLine(indent + name + ": " + prop.intValue);
                    break;
                default:
                    sb.AppendLine(indent + name + ": [" + prop.propertyType + "]");
                    break;
            }
        }

        private static string FormatVector3(Vector3 value)
        {
            return "{x: " + FormatFloat(value.x)
                + ", y: " + FormatFloat(value.y)
                + ", z: " + FormatFloat(value.z)
                + "}";
        }

        private static string FormatFloat(float value)
        {
            return value.ToString("G5", CultureInfo.InvariantCulture);
        }

        // ───────────────── Helpers ─────────────────

        private static GameObject FindGameObjectByPath(GameObject[] roots, string path)
        {
            string[] parts = path.Split('/');
            if (parts.Length == 0) return null;

            GameObject current = null;
            foreach (var root in roots)
            {
                if (root.name == parts[0])
                {
                    current = root;
                    break;
                }
            }
            if (current == null) return null;

            for (int i = 1; i < parts.Length; i++)
            {
                Transform child = current.transform.Find(parts[i]);
                if (child == null) return null;
                current = child.gameObject;
            }

            return current;
        }

        private static void CountPrefabInstances(GameObject[] objects, ref int count, HashSet<string> sources)
        {
            foreach (var go in objects)
                CountPrefabInstancesRecursive(go, ref count, sources);
        }

        private static void CountPrefabInstancesRecursive(GameObject go, ref int count, HashSet<string> sources)
        {
            if (PrefabUtility.IsAnyPrefabInstanceRoot(go))
            {
                count++;
                var prefabAsset = PrefabUtility.GetCorrespondingObjectFromOriginalSource(go);
                if (prefabAsset != null)
                {
                    string assetPath = AssetDatabase.GetAssetPath(prefabAsset);
                    if (!string.IsNullOrEmpty(assetPath))
                        sources.Add(assetPath);
                }
                return;
            }

            for (int i = 0; i < go.transform.childCount; i++)
                CountPrefabInstancesRecursive(go.transform.GetChild(i).gameObject, ref count, sources);
        }

        /// <summary>
        /// </summary>
        private static string StripNumericSuffix(string name)
        {
            int parenIdx = name.LastIndexOf(" (", StringComparison.Ordinal);
            if (parenIdx > 0 && name.EndsWith(")"))
            {
                string numPart = name.Substring(parenIdx + 2, name.Length - parenIdx - 3);
                int dummy;
                if (int.TryParse(numPart, out dummy))
                    return name.Substring(0, parenIdx);
            }

            int underIdx = name.LastIndexOf('_');
            if (underIdx > 0 && underIdx < name.Length - 1)
            {
                string numPart = name.Substring(underIdx + 1);
                int dummy;
                if (int.TryParse(numPart, out dummy))
                    return name.Substring(0, underIdx);
            }

            return name;
        }

        /// <summary>
        /// </summary>
        private static string BuildComponentSignature(GameObject go)
        {
            var components = go.GetComponents<Component>();
            var names = new List<string>();
            foreach (var comp in components)
            {
                if (comp == null) continue;
                string typeName = comp.GetType().Name;
                if (typeName == "Transform" || typeName == "RectTransform" || typeName == "CanvasRenderer")
                    continue;
                names.Add(typeName);
            }
            names.Sort(StringComparer.Ordinal);
            return string.Join(",", names);
        }

        /// <summary>
        /// </summary>
        private static string GetHierarchyPath(GameObject go)
        {
            var parts = new List<string>();
            Transform t = go.transform;
            while (t != null)
            {
                parts.Add(t.name);
                t = t.parent;
            }
            parts.Reverse();
            return string.Join("/", parts);
        }

        private static string FormatHierarchyNodeLabel(GameObject go)
        {
            string prefix = PrefabUtility.IsAnyPrefabInstanceRoot(go) ? "Prefab:" : "GO:";
            return prefix + GetHierarchyPath(go);
        }

        /// <summary>
        /// </summary>
        private static string FormatObjectReference(UnityEngine.Object obj)
        {
            if (obj is Component comp)
            {
                string hierarchyPath = GetHierarchyPath(comp.gameObject);
                return hierarchyPath + "." + comp.GetType().Name;
            }

            if (obj is GameObject go)
            {
                string assetPath = AssetDatabase.GetAssetPath(go);
                if (!string.IsNullOrEmpty(assetPath) && !assetPath.EndsWith(".unity"))
                    return go.name + " (" + assetPath + ")";
                return GetHierarchyPath(go) + " [GameObject]";
            }

            string path = AssetDatabase.GetAssetPath(obj);
            if (!string.IsNullOrEmpty(path))
                return obj.name + " (" + path + ")";

            return obj.name + " [" + obj.GetType().Name + "]";
        }

        /// <summary>
        /// </summary>
        private static string BuildGoAnnotations(GameObject go)
        {
            var parts = new List<string>();
            if (go.isStatic)
                parts.Add("Static");
            if (!go.activeSelf)
                parts.Add("Inactive");
            if (go.tag != "Untagged" && !string.IsNullOrEmpty(go.tag))
                parts.Add("Tag:" + go.tag);
            if (go.layer != 0)
            {
                string layerName = LayerMask.LayerToName(go.layer);
                parts.Add(string.IsNullOrEmpty(layerName) ? "Layer:" + go.layer : "Layer:" + layerName);
            }
            if (parts.Count == 0) return "";
            return "  [" + string.Join(", ", parts) + "]";
        }
    }
}
