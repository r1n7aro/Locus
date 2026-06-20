
using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
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
            public int max_depth;
            public int max_nodes;
            public int limit;
            public int max_field_depth;
            public int max_array_items;
            public string query;
            public string component_filter;
            public string match_fields;
            public string path_prefix;
        }

        private const string UnityYamlModeList = "list";
        private const string UnityYamlModeSearch = "search";
        private const string UnityYamlModeRead = "read";
        private const int DefaultHierarchyMaxNodes = 1000;
        private const int DefaultSerializedFieldMaxDepth = 2;
        private const int DefaultSerializedMaxArrayItems = 20;
        private const int HardSerializedFieldMaxDepth = 6;
        private const int HardSerializedMaxArrayItems = 200;

        /// <summary>
        /// </summary>
        private static async Task<PipeEnvelope> HandleListYaml(string requestId, string json)
        {
            return await HandleYamlTool(requestId, json, UnityYamlModeList, "list_yaml");
        }

        private static async Task<PipeEnvelope> HandleSearchYaml(string requestId, string json)
        {
            return await HandleYamlTool(requestId, json, UnityYamlModeSearch, "search_yaml");
        }

        private static async Task<PipeEnvelope> HandleReadYaml(string requestId, string json)
        {
            return await HandleYamlTool(requestId, json, UnityYamlModeRead, "read_yaml");
        }

        private static async Task<PipeEnvelope> HandleYamlTool(
            string requestId,
            string json,
            string mode,
            string toolName)
        {
            if (string.IsNullOrWhiteSpace(json))
                return ErrorResponse(requestId, "empty " + toolName + " args");

            ReadYamlArgs args;
            try
            {
                args = JsonUtility.FromJson<ReadYamlArgs>(json);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "invalid " + toolName + " args: " + ex.Message);
            }

            if (string.IsNullOrEmpty(args.file_path))
                return ErrorResponse(requestId, "missing file_path");

            string result = await RunReadYamlOnMainThreadAsync(args, mode, toolName);

            if (result.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                return ErrorResponse(requestId, result.Substring("__ERROR__: ".Length));

            return OkResponse(requestId, result);
        }

        private static async Task<string> RunReadYamlOnMainThreadAsync(
            ReadYamlArgs args,
            string mode,
            string toolName)
        {
            var tcs = LocusAsync.CreateTcs<string>();

            PostToMainThread(delegate
            {
                try
                {
                    string output = ExecuteReadYaml(args, mode);
                    tcs.TrySetResult(output);
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] " + toolName + " exception: " + ex);
                    tcs.TrySetResult("__ERROR__: " + ex);
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return "__ERROR__: " + toolName + " timed out";

            return tcs.Task.Result ?? "";
        }

        /// <summary>
        /// </summary>
        private static string ExecuteReadYaml(ReadYamlArgs args, string mode)
        {
            string filePath = TrimToProjectAssetPath(args.file_path);
            if (string.IsNullOrEmpty(filePath))
                return "__ERROR__: file_path must be under Assets/ or Packages/: " + args.file_path;

            string ext = System.IO.Path.GetExtension(filePath).ToLowerInvariant();
            bool isScene = ext == ".unity";
            bool isPrefab = ext == ".prefab";

            if (!isScene && !isPrefab)
                return "__ERROR__: Unity YAML editor tools only support .unity and .prefab files via Unity API";

            string objectPath = string.IsNullOrEmpty(args.object_path) ? null : args.object_path;
            var options = ReadYamlOptions.FromArgs(args);

            // Search-mode field matching must reach nested serialized data —
            // UnityEvent bindings live at m_OnClick.m_PersistentCalls
            // .m_Calls[i].m_MethodName (depth 4), which the shallow display
            // default (2) never reaches. Matching only compares strings, so
            // the hard cap is affordable here.
            if (mode == UnityYamlModeSearch && args.max_field_depth <= 0)
                options.SerializedFieldMaxDepth = HardSerializedFieldMaxDepth;

            if (mode == UnityYamlModeRead && string.IsNullOrEmpty(objectPath))
                return "__ERROR__: unity_yaml_read requires object_path for .unity/.prefab files";
            if (mode == UnityYamlModeSearch && !options.HasSearchFilters)
                return "__ERROR__: unity_yaml_search requires query or component_filter";
            if (mode == UnityYamlModeList)
                objectPath = null;

            if (isScene)
                return ReadScene(filePath, objectPath, options, mode);
            else
                return ReadPrefab(filePath, objectPath, options, mode);
        }

        // ───────────────── Scene reading ─────────────────

        /// <summary>
        /// </summary>
        private static string ReadScene(
            string scenePath,
            string objectPath,
            ReadYamlOptions options,
            string mode)
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
                        return ReadSceneContents(s, scenePath, objectPath, options, mode);
                    }
                }
                return "__ERROR__: scene not loaded in editor, falling back to YAML parsing";
            }

            return ReadSceneContents(activeScene, scenePath, objectPath, options, mode);
        }

        private static string ReadSceneContents(
            UnityEngine.SceneManagement.Scene scene,
            string scenePath,
            string objectPath,
            ReadYamlOptions options,
            string mode)
        {
            var roots = scene.GetRootGameObjects();

            if (mode == UnityYamlModeRead)
                return ReadGameObjectDetail(roots, objectPath, scenePath, options);

            if (mode == UnityYamlModeSearch)
                return BuildHierarchySearchResults(roots, scenePath, options);

            return BuildHierarchySummary(roots, scenePath, options);
        }

        // ───────────────── Prefab reading ─────────────────

        /// <summary>
        /// </summary>
        private static string ReadPrefab(
            string prefabPath,
            string objectPath,
            ReadYamlOptions options,
            string mode)
        {
            var prefabStage = PrefabStageUtility.GetCurrentPrefabStage();
            if (prefabStage != null
                && string.Equals(prefabStage.assetPath, prefabPath, StringComparison.OrdinalIgnoreCase)
                && prefabStage.prefabContentsRoot != null)
            {
                var stageRoot = prefabStage.prefabContentsRoot;
                if (mode == UnityYamlModeRead)
                    return ReadGameObjectDetail(new[] { stageRoot }, objectPath, prefabPath, options);

                if (mode == UnityYamlModeSearch)
                    return BuildHierarchySearchResults(new[] { stageRoot }, prefabPath, options);

                return BuildHierarchySummary(new[] { stageRoot }, prefabPath, options);
            }

            var prefabAsset = AssetDatabase.LoadAssetAtPath<GameObject>(prefabPath);
            if (prefabAsset == null)
                return "__ERROR__: failed to load prefab: " + prefabPath;

            if (mode == UnityYamlModeRead)
                return ReadGameObjectDetail(new[] { prefabAsset }, objectPath, prefabPath, options);

            if (mode == UnityYamlModeSearch)
                return BuildHierarchySearchResults(new[] { prefabAsset }, prefabPath, options);

            return BuildHierarchySummary(new[] { prefabAsset }, prefabPath, options);
        }

        // ───────────────── Hierarchy summary ─────────────────

        /// <summary>
        /// </summary>
        private sealed class ReadYamlOptions
        {
            public int MaxDepth;
            public int MaxNodes;
            public bool HasExplicitMaxNodes;
            public int Limit;
            public string Query;
            public Regex QueryRegex;
            public string PathPrefix;
            public bool HasExplicitMatchFields;
            public bool MatchPath = true;
            public bool MatchName = true;
            public bool MatchComponent = true;
            public bool MatchAnnotation = true;
            public bool MatchTag = true;
            public bool MatchLayer = true;
            public bool MatchPrefabSource = true;
            public bool MatchFieldName;
            public bool MatchFieldValue;
            public int SerializedFieldMaxDepth = DefaultSerializedFieldMaxDepth;
            public int SerializedMaxArrayItems = DefaultSerializedMaxArrayItems;
            public List<string> ComponentFilters = new List<string>();
            public List<string> MatchFieldLabels = new List<string>();

            public bool HasSearchFilters
            {
                get
                {
                    return !string.IsNullOrEmpty(Query) || ComponentFilters.Count > 0;
                }
            }

            public bool HasAnyOption
            {
                get
                {
                    return MaxDepth > 0
                        || HasExplicitMaxNodes
                        || !string.IsNullOrEmpty(Query)
                        || !string.IsNullOrEmpty(PathPrefix)
                        || ComponentFilters.Count > 0;
                }
            }

            public static ReadYamlOptions FromArgs(ReadYamlArgs args)
            {
                var options = new ReadYamlOptions();
                options.MaxDepth = args.max_depth > 0 ? args.max_depth : 0;
                options.HasExplicitMaxNodes = args.max_nodes > 0;
                options.MaxNodes = options.HasExplicitMaxNodes ? args.max_nodes : DefaultHierarchyMaxNodes;
                options.Limit = args.limit > 0 ? args.limit : 0;
                options.SerializedFieldMaxDepth = ClampInt(
                    args.max_field_depth > 0 ? args.max_field_depth : DefaultSerializedFieldMaxDepth,
                    1,
                    HardSerializedFieldMaxDepth);
                options.SerializedMaxArrayItems = ClampInt(
                    args.max_array_items > 0 ? args.max_array_items : DefaultSerializedMaxArrayItems,
                    1,
                    HardSerializedMaxArrayItems);
                options.Query = string.IsNullOrWhiteSpace(args.query) ? null : args.query.Trim();
                if (!string.IsNullOrEmpty(options.Query)
                    && options.Query.StartsWith("re:", StringComparison.Ordinal)
                    && options.Query.Length > 3)
                {
                    try
                    {
                        options.QueryRegex = new Regex(
                            options.Query.Substring(3),
                            RegexOptions.IgnoreCase | RegexOptions.CultureInvariant
                        );
                    }
                    catch
                    {
                        options.QueryRegex = null;
                    }
                }
                options.PathPrefix = string.IsNullOrWhiteSpace(args.path_prefix) ? null : args.path_prefix.Trim();
                options.ApplyMatchFields(args.match_fields);
                if (!string.IsNullOrWhiteSpace(args.component_filter))
                {
                    string[] parts = args.component_filter.Split(',');
                    foreach (var part in parts)
                    {
                        string trimmed = part.Trim();
                        if (!string.IsNullOrEmpty(trimmed))
                            options.ComponentFilters.Add(trimmed);
                    }
                }
                return options;
            }

            private void ApplyMatchFields(string value)
            {
                if (string.IsNullOrWhiteSpace(value))
                    return;

                HasExplicitMatchFields = true;
                MatchPath = false;
                MatchName = false;
                MatchComponent = false;
                MatchAnnotation = false;
                MatchTag = false;
                MatchLayer = false;
                MatchPrefabSource = false;
                MatchFieldName = false;
                MatchFieldValue = false;
                MatchFieldLabels.Clear();

                string[] parts = value.Split(',', '|');
                foreach (var part in parts)
                {
                    string normalized = NormalizeMatchField(part);
                    if (string.IsNullOrEmpty(normalized))
                        continue;

                    if (normalized == "default")
                    {
                        EnableDefaultMatchFields();
                        continue;
                    }
                    if (normalized == "all")
                    {
                        EnableDefaultMatchFields();
                        MatchFieldName = true;
                        MatchFieldValue = true;
                        AddMatchFieldLabel("field_name");
                        AddMatchFieldLabel("field_value");
                        continue;
                    }

                    if (EnableMatchField(normalized))
                        AddMatchFieldLabel(normalized);
                }
            }

            private static string NormalizeMatchField(string value)
            {
                if (string.IsNullOrWhiteSpace(value))
                    return null;
                return value.Trim().ToLowerInvariant().Replace("-", "_");
            }

            private void EnableDefaultMatchFields()
            {
                MatchPath = true;
                MatchName = true;
                MatchComponent = true;
                MatchAnnotation = true;
                MatchTag = true;
                MatchLayer = true;
                MatchPrefabSource = true;
                AddMatchFieldLabel("default");
            }

            private bool EnableMatchField(string normalized)
            {
                switch (normalized)
                {
                    case "path":
                        MatchPath = true;
                        return true;
                    case "name":
                        MatchName = true;
                        return true;
                    case "component":
                    case "components":
                        MatchComponent = true;
                        return true;
                    case "annotation":
                    case "annotations":
                    case "state":
                        MatchAnnotation = true;
                        return true;
                    case "tag":
                        MatchTag = true;
                        return true;
                    case "layer":
                        MatchLayer = true;
                        return true;
                    case "prefab":
                    case "prefab_source":
                    case "source_prefab":
                        MatchPrefabSource = true;
                        return true;
                    case "field":
                    case "fields":
                        MatchFieldName = true;
                        MatchFieldValue = true;
                        return true;
                    case "field_name":
                    case "field_names":
                    case "property_name":
                    case "property_path":
                        MatchFieldName = true;
                        return true;
                    case "field_value":
                    case "field_values":
                    case "property_value":
                        MatchFieldValue = true;
                        return true;
                    default:
                        return false;
                }
            }

            private void AddMatchFieldLabel(string label)
            {
                if (!MatchFieldLabels.Contains(label))
                    MatchFieldLabels.Add(label);
            }

            private static int ClampInt(int value, int min, int max)
            {
                if (value < min) return min;
                if (value > max) return max;
                return value;
            }

            public string FormatSummaryLine()
            {
                if (!HasAnyOption)
                    return null;

                var parts = new List<string>();
                if (!string.IsNullOrEmpty(PathPrefix))
                    parts.Add("path_prefix=\"" + PathPrefix + "\"");
                if (!string.IsNullOrEmpty(Query))
                    parts.Add("query=\"" + Query + "\"");
                if (ComponentFilters.Count > 0)
                    parts.Add("component_filter=\"" + string.Join(",", ComponentFilters.ToArray()) + "\"");
                if (MaxDepth > 0)
                    parts.Add("max_depth=" + MaxDepth);
                if (HasExplicitMaxNodes)
                    parts.Add("max_nodes=" + MaxNodes);

                return "Hierarchy filters: " + string.Join(", ", parts.ToArray());
            }

            public string FormatSearchLine()
            {
                var parts = new List<string>();
                if (!string.IsNullOrEmpty(PathPrefix))
                    parts.Add("path_prefix=\"" + PathPrefix + "\"");
                if (!string.IsNullOrEmpty(Query))
                    parts.Add("query=\"" + Query + "\"");
                if (ComponentFilters.Count > 0)
                    parts.Add("component_filter=\"" + string.Join(",", ComponentFilters.ToArray()) + "\"");
                if (HasExplicitMatchFields && MatchFieldLabels.Count > 0)
                    parts.Add("match_fields=\"" + string.Join(",", MatchFieldLabels.ToArray()) + "\"");
                if (Limit > 0)
                    parts.Add("limit=" + Limit);

                if (parts.Count == 0)
                    return "Search filters: none";

                return "Search filters: " + string.Join(", ", parts.ToArray());
            }
        }

        private sealed class HierarchyWriteState
        {
            public int PrintedNodes;
            public int HiddenByMaxNodes;
        }

        /// <summary>
        /// </summary>
        private sealed class HierarchySummaryNode
        {
            public GameObject GameObject;
            public string Name;
            public string NormalizedName;
            public string ComponentSuffix;
            public string ComponentSignature;
            public string Annotations;
            public string SourcePrefabPath;
            public string Path;
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

        private static string BuildHierarchySummary(GameObject[] roots, string filePath, ReadYamlOptions options)
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
            string optionLine = options.FormatSummaryLine();
            if (!string.IsNullOrEmpty(optionLine))
                sb.AppendLine(optionLine);

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

            var summaryRoots = BuildHierarchySummaryNodes(roots, boneTransforms);

            summaryRoots = ApplyHierarchyOptions(summaryRoots, options);
            if (summaryRoots.Count == 0)
            {
                sb.AppendLine("No hierarchy nodes matched filters.");
            }
            else
            {
                var state = new HierarchyWriteState();
                WriteGroupedSummaryNodes(sb, summaryRoots, "", true, 1, options, state);
                if (state.HiddenByMaxNodes > 0)
                    sb.AppendLine("... (" + state.HiddenByMaxNodes + " hierarchy nodes hidden by max_nodes)");
            }

            // Drill-down hints
            sb.AppendLine();
            sb.AppendLine("Drill down with object_path:");
            sb.AppendLine("- \"ObjectName\" → GameObject components detail");
            sb.AppendLine("- \"Parent/Child\" → nested GameObject components");
            sb.AppendLine("- Names are displayed as ⟦Name⟧; omit ⟦⟧ in object_path");
            sb.AppendLine("- Use paths from the hierarchy or \"Instances\" lines for object_path");

            return sb.ToString();
        }

        private static string BuildHierarchySearchResults(
            GameObject[] roots,
            string filePath,
            ReadYamlOptions options)
        {
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

            var summaryRoots = BuildHierarchySummaryNodes(roots, boneTransforms);
            if (!string.IsNullOrEmpty(options.PathPrefix))
                summaryRoots = SelectPathPrefixRoots(summaryRoots, options.PathPrefix);

            var matches = new List<HierarchySummaryNode>();
            foreach (var root in summaryRoots)
                CollectSearchMatches(root, options, matches);

            var sb = new StringBuilder();
            sb.AppendLine("Search: " + filePath);
            sb.AppendLine(options.FormatSearchLine());
            sb.AppendLine("Total matches: " + matches.Count);

            if (matches.Count == 0)
            {
                sb.AppendLine();
                sb.AppendLine("No hierarchy nodes matched filters.");
                return sb.ToString();
            }

            int limit = options.Limit > 0 ? options.Limit : 50;
            int shown = matches.Count < limit ? matches.Count : limit;
            sb.AppendLine("Showing: 1-" + shown);
            sb.AppendLine();
            for (int i = 0; i < shown; i++)
                FormatSearchResultNode(sb, matches[i]);

            if (matches.Count > shown)
            {
                sb.AppendLine();
                sb.AppendLine("... (" + (matches.Count - shown) + " more matches hidden by limit)");
            }

            return sb.ToString();
        }

        private static void CollectSearchMatches(
            HierarchySummaryNode node,
            ReadYamlOptions options,
            List<HierarchySummaryNode> matches)
        {
            if (NodeMatchesFilters(node, options))
                matches.Add(node);

            foreach (var child in node.Children)
                CollectSearchMatches(child, options, matches);
        }

        private static void FormatSearchResultNode(StringBuilder sb, HierarchySummaryNode node)
        {
            string path = string.IsNullOrEmpty(node.Path) ? node.Name : node.Path;
            sb.AppendLine("- " + path + node.ComponentSuffix + node.Annotations);
        }

        private static List<HierarchySummaryNode> BuildHierarchySummaryNodes(
            GameObject[] roots,
            HashSet<Transform> boneTransforms)
        {
            var nodes = new List<HierarchySummaryNode>();
            var totals = CountSiblingNames(roots);
            var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);

            foreach (var go in roots)
            {
                string path = FormatIndexedPathSegment(go.name, totals, ordinals);
                nodes.Add(BuildHierarchySummaryNode(go, boneTransforms, path));
            }

            return nodes;
        }

        private static HierarchySummaryNode BuildHierarchySummaryNode(
            GameObject go,
            HashSet<Transform> boneTransforms,
            string path)
        {
            var node = new HierarchySummaryNode
            {
                Name = go.name,
                NormalizedName = StripNumericSuffix(go.name),
                ComponentSuffix = BuildComponentSuffix(go),
                ComponentSignature = BuildComponentSignature(go),
                Annotations = BuildGoAnnotations(go),
                SourcePrefabPath = BuildSourcePrefabPath(go),
                GameObject = go,
                Path = path,
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

            var childTotals = CountSiblingNames(go.transform);
            var childOrdinals = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < childCount; i++)
            {
                GameObject child = go.transform.GetChild(i).gameObject;
                string childPath = path + "/" + FormatIndexedPathSegment(child.name, childTotals, childOrdinals);
                node.Children.Add(BuildHierarchySummaryNode(child, boneTransforms, childPath));
            }

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

        private static void WriteGroupedSummaryNodes(
            StringBuilder sb,
            List<HierarchySummaryNode> nodes,
            string prefix,
            bool topLevel,
            int logicalDepth,
            ReadYamlOptions options,
            HierarchyWriteState state)
        {
            var groups = GroupSummaryNodes(nodes);

            for (int groupIndex = 0; groupIndex < groups.Count; groupIndex++)
            {
                var group = groups[groupIndex];
                if (options.MaxNodes > 0 && state.PrintedNodes >= options.MaxNodes)
                {
                    state.HiddenByMaxNodes += CountGroupNodes(group);
                    continue;
                }

                bool isLast = groupIndex + 1 == groups.Count;
                string linePrefix = TreeLinePrefix(prefix, isLast, topLevel);
                string childPrefix = TreeChildPrefix(prefix, isLast, topLevel);
                string metadataPrefix = childPrefix + "  ";
                var representative = group.Representative;
                state.PrintedNodes++;

                if (group.Members.Count > 1)
                {
                    sb.AppendLine(linePrefix + FormatSummaryNodeLabel(representative, true) + " ×" + group.Members.Count);
                    sb.AppendLine(metadataPrefix + "Instances: " + FormatInstanceSample(group.Members));
                    if (!representative.BoneFolded && representative.Children.Count > 0)
                    {
                        if (options.MaxDepth > 0 && logicalDepth >= options.MaxDepth)
                        {
                            int hidden = 0;
                            foreach (var member in group.Members)
                                hidden += CountHierarchyNodes(member.Children);
                            sb.AppendLine(metadataPrefix + "... (" + hidden + " child nodes hidden by max_depth)");
                        }
                        else
                        {
                            sb.AppendLine(metadataPrefix + "Shared subtree:");
                            WriteGroupedSummaryNodes(
                                sb,
                                representative.Children,
                                childPrefix + "  ",
                                false,
                                logicalDepth + 1,
                                options,
                                state);
                        }
                    }
                    continue;
                }

                sb.AppendLine(linePrefix + FormatSummaryNodeLabel(representative, false));
                if (!representative.BoneFolded && representative.Children.Count > 0)
                {
                    if (options.MaxDepth > 0 && logicalDepth >= options.MaxDepth)
                    {
                        sb.AppendLine(metadataPrefix + "... (" + CountHierarchyNodes(representative.Children) + " child nodes hidden by max_depth)");
                    }
                    else
                    {
                        WriteGroupedSummaryNodes(
                            sb,
                            representative.Children,
                            childPrefix,
                            false,
                            logicalDepth + 1,
                            options,
                            state);
                    }
                }
            }
        }

        private static string TreeLinePrefix(string prefix, bool isLast, bool topLevel)
        {
            if (topLevel)
                return "";
            return prefix + (isLast ? "└─ " : "├─ ");
        }

        private static string TreeChildPrefix(string prefix, bool isLast, bool topLevel)
        {
            if (topLevel)
                return "";
            return prefix + (isLast ? "   " : "│  ");
        }

        private static int CountHierarchyNodes(List<HierarchySummaryNode> nodes)
        {
            int count = 0;
            foreach (var node in nodes)
                count += 1 + CountHierarchyNodes(node.Children);
            return count;
        }

        private static int CountGroupNodes(HierarchySummaryGroup group)
        {
            int count = 0;
            foreach (var member in group.Members)
                count += 1 + CountHierarchyNodes(member.Children);
            return count;
        }

        private static Dictionary<string, int> CountSiblingNames(GameObject[] objects)
        {
            var counts = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (var go in objects)
            {
                int count;
                counts.TryGetValue(go.name, out count);
                counts[go.name] = count + 1;
            }
            return counts;
        }

        private static Dictionary<string, int> CountSiblingNames(Transform parent)
        {
            var counts = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < parent.childCount; i++)
            {
                string name = parent.GetChild(i).name;
                int count;
                counts.TryGetValue(name, out count);
                counts[name] = count + 1;
            }
            return counts;
        }

        private static string FormatIndexedPathSegment(
            string name,
            Dictionary<string, int> totals,
            Dictionary<string, int> ordinals)
        {
            int total;
            totals.TryGetValue(name, out total);
            if (total <= 1)
                return name;

            int ordinal;
            ordinals.TryGetValue(name, out ordinal);
            ordinal++;
            ordinals[name] = ordinal;
            return name + "[" + ordinal + "]";
        }

        private static List<HierarchySummaryNode> ApplyHierarchyOptions(List<HierarchySummaryNode> roots, ReadYamlOptions options)
        {
            var scoped = roots;
            if (!string.IsNullOrEmpty(options.PathPrefix))
                scoped = SelectPathPrefixRoots(scoped, options.PathPrefix);

            if (!options.HasSearchFilters)
                return scoped;

            var filtered = new List<HierarchySummaryNode>();
            foreach (var root in scoped)
            {
                var match = FilterHierarchyNode(root, options);
                if (match != null)
                    filtered.Add(match);
            }
            return filtered;
        }

        private static List<HierarchySummaryNode> SelectPathPrefixRoots(List<HierarchySummaryNode> roots, string pathPrefix)
        {
            var parts = SplitHierarchyPath(pathPrefix);
            if (parts.Length == 0)
                return roots;

            var selected = new List<HierarchySummaryNode>();
            var node = FindSummaryNodeByPath(roots, parts, 0);
            if (node != null)
                selected.Add(CloneSummaryNode(node));
            return selected;
        }

        private static string[] SplitHierarchyPath(string path)
        {
            var raw = path.Split('/');
            var parts = new List<string>();
            foreach (var part in raw)
            {
                string trimmed = part.Trim();
                if (!string.IsNullOrEmpty(trimmed))
                    parts.Add(trimmed);
            }
            return parts.ToArray();
        }

        private static bool TryParseIndexedSegment(string segment, out string name, out int ordinal)
        {
            name = segment;
            ordinal = 0;

            if (string.IsNullOrEmpty(segment) || !segment.EndsWith("]", StringComparison.Ordinal))
                return false;

            int bracket = segment.LastIndexOf('[', segment.Length - 1);
            if (bracket <= 0 || bracket >= segment.Length - 2)
                return false;

            string number = segment.Substring(bracket + 1, segment.Length - bracket - 2);
            int parsed;
            if (!int.TryParse(number, out parsed) || parsed <= 0)
                return false;

            name = segment.Substring(0, bracket);
            ordinal = parsed;
            return true;
        }

        private static HierarchySummaryNode FindSummaryNodeByPath(
            List<HierarchySummaryNode> siblings,
            string[] parts,
            int index)
        {
            if (index >= parts.Length)
                return null;
            var node = FindSummaryNodeInSiblings(siblings, parts[index]);
            if (node == null)
                return null;
            if (index == parts.Length - 1)
                return node;

            return FindSummaryNodeByPath(node.Children, parts, index + 1);
        }

        private static HierarchySummaryNode FindSummaryNodeInSiblings(
            List<HierarchySummaryNode> siblings,
            string segment)
        {
            string name;
            int ordinal;
            if (TryParseIndexedSegment(segment, out name, out ordinal))
            {
                int seen = 0;
                foreach (var node in siblings)
                {
                    if (node.Name != name)
                        continue;
                    seen++;
                    if (seen == ordinal)
                        return node;
                }
            }

            foreach (var node in siblings)
            {
                if (node.Name == segment)
                    return node;
            }

            return null;
        }

        private static HierarchySummaryNode FilterHierarchyNode(
            HierarchySummaryNode node,
            ReadYamlOptions options)
        {
            var children = new List<HierarchySummaryNode>();
            foreach (var child in node.Children)
            {
                var match = FilterHierarchyNode(child, options);
                if (match != null)
                    children.Add(match);
            }

            if (!NodeMatchesFilters(node, options) && children.Count == 0)
                return null;

            var cloned = CloneSummaryNodeShallow(node);
            cloned.Children = children;
            cloned.StructureSignature = BuildNodeStructureSignature(cloned);
            return cloned;
        }

        private static bool NodeMatchesFilters(HierarchySummaryNode node, ReadYamlOptions options)
        {
            if (!string.IsNullOrEmpty(options.Query))
            {
                bool queryMatch = (options.MatchPath && QueryMatches(node.Path, options))
                    || (options.MatchName && QueryMatches(node.Name, options))
                    || (options.MatchComponent && QueryMatches(node.ComponentSignature, options))
                    || (options.MatchAnnotation && QueryMatches(node.Annotations, options))
                    || (options.MatchTag && QueryMatches(NodeTag(node), options))
                    || (options.MatchLayer && QueryMatches(NodeLayer(node), options))
                    || (options.MatchPrefabSource && QueryMatches(node.SourcePrefabPath, options))
                    || SerializedFieldsMatch(node.GameObject, options);
                if (!queryMatch)
                    return false;
            }

            if (options.ComponentFilters.Count == 0)
                return true;

            foreach (var filter in options.ComponentFilters)
            {
                if (ContainsIgnoreCase(node.ComponentSignature, filter.ToLowerInvariant()))
                    return true;
            }
            return false;
        }

        private static string NodeTag(HierarchySummaryNode node)
        {
            if (node == null || node.GameObject == null)
                return null;
            return node.GameObject.tag;
        }

        private static string NodeLayer(HierarchySummaryNode node)
        {
            if (node == null || node.GameObject == null)
                return null;

            string layerName = LayerMask.LayerToName(node.GameObject.layer);
            if (string.IsNullOrEmpty(layerName))
                return node.GameObject.layer.ToString(CultureInfo.InvariantCulture);
            return layerName + " " + node.GameObject.layer.ToString(CultureInfo.InvariantCulture);
        }

        private static bool QueryMatches(string value, ReadYamlOptions options)
        {
            if (string.IsNullOrEmpty(value) || string.IsNullOrEmpty(options.Query))
                return false;

            if (options.QueryRegex != null)
                return options.QueryRegex.IsMatch(value);

            return ContainsIgnoreCase(value, options.Query.ToLowerInvariant());
        }

        private static bool ContainsIgnoreCase(string value, string needleLower)
        {
            if (string.IsNullOrEmpty(value) || string.IsNullOrEmpty(needleLower))
                return false;
            return value.ToLowerInvariant().Contains(needleLower);
        }

        private static bool SerializedFieldsMatch(GameObject go, ReadYamlOptions options)
        {
            if (go == null || (!options.MatchFieldName && !options.MatchFieldValue))
                return false;

            var components = go.GetComponents<Component>();
            foreach (var comp in components)
            {
                if (comp == null)
                    continue;

                SerializedObject so;
                try
                {
                    so = new SerializedObject(comp);
                }
                catch
                {
                    continue;
                }

                var prop = so.GetIterator();
                bool enterChildren = true;
                while (prop.NextVisible(enterChildren))
                {
                    enterChildren = false;
                    if (SerializedPropertyMatchesSearch(prop, options, 0))
                        return true;
                }
            }

            return false;
        }

        private static bool SerializedPropertyMatchesSearch(
            SerializedProperty prop,
            ReadYamlOptions options,
            int genericDepth)
        {
            if (options.MatchFieldName
                && (QueryMatches(prop.name, options)
                    || QueryMatches(prop.displayName, options)
                    || QueryMatches(prop.propertyPath, options)))
            {
                return true;
            }

            if (options.MatchFieldValue)
            {
                string value;
                if (TryFormatSerializedPropertySearchValue(prop, out value)
                    && QueryMatches(value, options))
                    return true;
            }

            if (prop.propertyType != SerializedPropertyType.Generic
                || genericDepth >= options.SerializedFieldMaxDepth)
            {
                return false;
            }

            if (prop.isArray)
            {
                int arraySize;
                try
                {
                    arraySize = prop.arraySize;
                }
                catch
                {
                    return false;
                }

                int shown = Math.Min(arraySize, options.SerializedMaxArrayItems);
                for (int i = 0; i < shown; i++)
                {
                    try
                    {
                        if (SerializedPropertyMatchesSearch(
                            prop.GetArrayElementAtIndex(i),
                            options,
                            genericDepth + 1))
                            return true;
                    }
                    catch
                    {
                        continue;
                    }
                }
                return false;
            }

            var children = CollectVisibleChildProperties(prop);
            foreach (var child in children)
            {
                if (SerializedPropertyMatchesSearch(child, options, genericDepth + 1))
                    return true;
            }

            return false;
        }

        private static bool TryFormatSerializedPropertySearchValue(
            SerializedProperty prop,
            out string value)
        {
            value = null;
            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                    value = prop.intValue.ToString(CultureInfo.InvariantCulture);
                    return true;
                case SerializedPropertyType.Boolean:
                    value = prop.boolValue ? "true" : "false";
                    return true;
                case SerializedPropertyType.Float:
                    value = prop.floatValue.ToString("G5", CultureInfo.InvariantCulture);
                    return true;
                case SerializedPropertyType.String:
                    value = prop.stringValue;
                    return true;
                case SerializedPropertyType.Enum:
                    value = FormatSerializedEnumValue(prop);
                    return true;
                case SerializedPropertyType.ObjectReference:
                    value = prop.objectReferenceValue != null
                        ? FormatObjectReference(prop.objectReferenceValue)
                        : "None";
                    return true;
                case SerializedPropertyType.Vector2:
                    value = prop.vector2Value.ToString();
                    return true;
                case SerializedPropertyType.Vector3:
                    value = prop.vector3Value.ToString();
                    return true;
                case SerializedPropertyType.Vector4:
                    value = prop.vector4Value.ToString();
                    return true;
                case SerializedPropertyType.Quaternion:
                    value = prop.quaternionValue.eulerAngles.ToString();
                    return true;
                case SerializedPropertyType.Color:
                    value = prop.colorValue.ToString();
                    return true;
                case SerializedPropertyType.Rect:
                    value = prop.rectValue.ToString();
                    return true;
                case SerializedPropertyType.LayerMask:
                case SerializedPropertyType.ArraySize:
                    value = prop.intValue.ToString(CultureInfo.InvariantCulture);
                    return true;
                default:
                    return false;
            }
        }

        private static string FormatSerializedEnumValue(SerializedProperty prop)
        {
            int index = -1;
            try
            {
                index = prop.enumValueIndex;
            }
            catch
            {
            }

            try
            {
                var displayNames = prop.enumDisplayNames;
                if (displayNames != null
                    && index >= 0
                    && index < displayNames.Length
                    && !string.IsNullOrEmpty(displayNames[index]))
                    return displayNames[index];
            }
            catch
            {
            }

            try
            {
                var names = prop.enumNames;
                if (names != null
                    && index >= 0
                    && index < names.Length
                    && !string.IsNullOrEmpty(names[index]))
                    return names[index];
            }
            catch
            {
            }

            try
            {
                return "Unknown (" + prop.intValue.ToString(CultureInfo.InvariantCulture) + ")";
            }
            catch
            {
                return "Unknown";
            }
        }

        private static HierarchySummaryNode CloneSummaryNode(HierarchySummaryNode node)
        {
            var cloned = CloneSummaryNodeShallow(node);
            foreach (var child in node.Children)
                cloned.Children.Add(CloneSummaryNode(child));
            cloned.StructureSignature = BuildNodeStructureSignature(cloned);
            return cloned;
        }

        private static HierarchySummaryNode CloneSummaryNodeShallow(HierarchySummaryNode node)
        {
            return new HierarchySummaryNode
            {
                Name = node.Name,
                NormalizedName = node.NormalizedName,
                ComponentSuffix = node.ComponentSuffix,
                ComponentSignature = node.ComponentSignature,
                Annotations = node.Annotations,
                SourcePrefabPath = node.SourcePrefabPath,
                GameObject = node.GameObject,
                Path = node.Path,
                IsPrefabRoot = node.IsPrefabRoot,
                BoneFolded = node.BoneFolded,
                BoneDescCount = node.BoneDescCount,
                StructureSignature = node.StructureSignature,
            };
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
            string suffix = prefix + FormatGameObjectName(name) + node.ComponentSuffix + node.Annotations;
            if (!collapsed && PathNeedsHint(node))
                suffix += "  {object_path: " + node.Path + "}";
            if (node.BoneFolded)
                suffix += " [" + node.BoneDescCount + " bones]";
            return suffix;
        }

        private static string FormatGameObjectName(string name)
        {
            return "⟦" + name + "⟧";
        }

        private static bool PathNeedsHint(HierarchySummaryNode node)
        {
            if (string.IsNullOrEmpty(node.Path))
                return false;
            int slash = node.Path.LastIndexOf('/');
            string segment = slash >= 0 ? node.Path.Substring(slash + 1) : node.Path;
            return segment != node.Name;
        }

        private static string FormatInstanceSample(List<HierarchySummaryNode> members)
        {
            const int sampleLimit = 5;
            var sample = new List<string>();
            int count = members.Count < sampleLimit ? members.Count : sampleLimit;
            for (int i = 0; i < count; i++)
                sample.Add(string.IsNullOrEmpty(members[i].Path) ? members[i].Name : members[i].Path);

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
        private static string ReadGameObjectDetail(
            GameObject[] roots,
            string objectPath,
            string filePath,
            ReadYamlOptions options)
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

            AppendTransformHierarchySection(sb, target.transform);

            if (PrefabUtility.IsAnyPrefabInstanceRoot(target))
                AppendPrefabOverrideSummary(sb, target);

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
                    FormatSerializedProperty(sb, prop, 1, options, 0, null);
                }
            }

            return sb.ToString();
        }

        private static void AppendPrefabOverrideSummary(StringBuilder sb, GameObject instanceRoot)
        {
            PropertyModification[] modifications = PrefabUtility.GetPropertyModifications(instanceRoot);
            if (modifications == null || modifications.Length == 0)
                return;

            var grouped = new Dictionary<string, List<PropertyModification>>(StringComparer.Ordinal);
            foreach (var modification in modifications)
            {
                if (modification == null || modification.target == null)
                    continue;

                string label = FormatObjectReference(modification.target);
                List<PropertyModification> list;
                if (!grouped.TryGetValue(label, out list))
                {
                    list = new List<PropertyModification>();
                    grouped[label] = list;
                }
                list.Add(modification);
            }

            if (grouped.Count == 0)
                return;

            sb.AppendLine();
            sb.AppendLine("── Prefab Overrides ──");
            foreach (var entry in grouped)
            {
                sb.AppendLine();
                sb.AppendLine("--- " + entry.Key + " ---");
                foreach (var modification in entry.Value)
                {
                    if (modification.objectReference != null)
                    {
                        sb.AppendLine("  " + modification.propertyPath + " = {" + FormatObjectReference(modification.objectReference) + "}");
                    }
                    else if (!string.IsNullOrEmpty(modification.value))
                    {
                        sb.AppendLine("  " + modification.propertyPath + " = " + modification.value);
                    }
                    else
                    {
                        sb.AppendLine("  " + modification.propertyPath + " = {none}");
                    }
                }
            }
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

        private static void AppendTransformHierarchySection(StringBuilder sb, Transform transform)
        {
            if (transform == null)
                return;

            sb.AppendLine();
            sb.AppendLine("--- Hierarchy ---");

            if (transform.parent != null)
                sb.AppendLine("  parent: " + transform.parent.gameObject.name);
            else
                sb.AppendLine("  parent: none");

            sb.AppendLine("  " + FormatReadHierarchyNodeLabel(transform.gameObject));
            AppendReadHierarchyChildren(sb, transform);
        }

        private static void AppendReadHierarchyChildren(StringBuilder sb, Transform parent)
        {
            for (int i = 0; i < parent.childCount; i++)
            {
                var child = parent.GetChild(i);
                bool isLast = i + 1 == parent.childCount;
                sb.AppendLine("  " + (isLast ? "└─ " : "├─ ") + FormatReadHierarchyNodeLabel(child.gameObject));

                int hidden = CountDescendants(child);
                if (hidden > 0)
                    sb.AppendLine(
                        "  "
                        + (isLast ? "   " : "│  ")
                        + "... ("
                        + hidden
                        + " child nodes hidden by max_depth)"
                    );
            }
        }

        private static string FormatReadHierarchyNodeLabel(GameObject go)
        {
            return go.name + BuildComponentSuffix(go) + BuildGoAnnotations(go);
        }

        private static void AppendWorldTransformFields(StringBuilder sb, Component comp)
        {
            var transform = comp as Transform;
            if (transform == null)
                return;

            sb.AppendLine("  World Position: " + FormatVector3(transform.position));
            sb.AppendLine("  World Rotation: " + FormatVector3(transform.rotation.eulerAngles));
            sb.AppendLine("  World Scale: " + FormatVector3(transform.lossyScale));
        }

        /// <summary>
        /// </summary>
        private static void FormatSerializedProperty(
            StringBuilder sb,
            SerializedProperty prop,
            int indentLevel,
            ReadYamlOptions options,
            int genericDepth,
            string nameOverride)
        {
            string indent = new string(' ', indentLevel * 2);
            string name = string.IsNullOrEmpty(nameOverride) ? prop.displayName : nameOverride;

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Generic:
                    FormatGenericSerializedProperty(sb, prop, indentLevel, options, genericDepth, name);
                    break;
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
                    sb.AppendLine(indent + name + ": " + FormatSerializedEnumValue(prop));
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

        private static void FormatGenericSerializedProperty(
            StringBuilder sb,
            SerializedProperty prop,
            int indentLevel,
            ReadYamlOptions options,
            int genericDepth,
            string name)
        {
            string indent = new string(' ', indentLevel * 2);
            if (genericDepth >= options.SerializedFieldMaxDepth)
            {
                sb.AppendLine(
                    indent
                    + name
                    + ": [Generic] (children hidden by max_field_depth="
                    + options.SerializedFieldMaxDepth
                    + ")");
                return;
            }

            if (prop.isArray)
            {
                FormatArraySerializedProperty(sb, prop, indentLevel, options, genericDepth, name);
                return;
            }

            var children = CollectVisibleChildProperties(prop);
            if (children.Count == 0)
            {
                sb.AppendLine(indent + name + ": [Generic]");
                return;
            }

            sb.AppendLine(indent + name + ":");
            foreach (var child in children)
                FormatSerializedProperty(sb, child, indentLevel + 1, options, genericDepth + 1, null);
        }

        private static void FormatArraySerializedProperty(
            StringBuilder sb,
            SerializedProperty prop,
            int indentLevel,
            ReadYamlOptions options,
            int genericDepth,
            string name)
        {
            string indent = new string(' ', indentLevel * 2);
            int arraySize;
            try
            {
                arraySize = prop.arraySize;
            }
            catch
            {
                sb.AppendLine(indent + name + ": [Generic]");
                return;
            }

            sb.AppendLine(indent + name + ": [Array] count=" + arraySize);
            int shown = Math.Min(arraySize, options.SerializedMaxArrayItems);
            for (int i = 0; i < shown; i++)
            {
                SerializedProperty element;
                try
                {
                    element = prop.GetArrayElementAtIndex(i);
                }
                catch
                {
                    sb.AppendLine(new string(' ', (indentLevel + 1) * 2) + "[" + i + "]: [Unavailable]");
                    continue;
                }

                FormatSerializedProperty(
                    sb,
                    element,
                    indentLevel + 1,
                    options,
                    genericDepth + 1,
                    "[" + i + "]");
            }

            if (arraySize > shown)
            {
                sb.AppendLine(
                    new string(' ', (indentLevel + 1) * 2)
                    + "... ("
                    + (arraySize - shown)
                    + " more items hidden by max_array_items="
                    + options.SerializedMaxArrayItems
                    + ")");
            }
        }

        private static List<SerializedProperty> CollectVisibleChildProperties(SerializedProperty prop)
        {
            var children = new List<SerializedProperty>();
            SerializedProperty cursor = prop.Copy();
            SerializedProperty end = cursor.GetEndProperty();
            bool enterChildren = true;
            while (cursor.NextVisible(enterChildren)
                && (end == null || !SerializedProperty.EqualContents(cursor, end)))
            {
                children.Add(cursor.Copy());
                enterChildren = false;
            }
            return children;
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

            GameObject current = FindGameObjectInSiblings(roots, parts[0]);
            if (current == null) return null;

            for (int i = 1; i < parts.Length; i++)
            {
                current = FindGameObjectInSiblings(current.transform, parts[i]);
                if (current == null) return null;
            }

            return current;
        }

        private static GameObject FindGameObjectInSiblings(GameObject[] siblings, string segment)
        {
            string name;
            int ordinal;
            if (TryParseIndexedSegment(segment.Trim(), out name, out ordinal))
            {
                int seen = 0;
                foreach (var go in siblings)
                {
                    if (go.name != name)
                        continue;
                    seen++;
                    if (seen == ordinal)
                        return go;
                }
            }

            foreach (var go in siblings)
            {
                if (go.name == segment.Trim())
                    return go;
            }

            return null;
        }

        private static GameObject FindGameObjectInSiblings(Transform parent, string segment)
        {
            string name;
            int ordinal;
            if (TryParseIndexedSegment(segment.Trim(), out name, out ordinal))
            {
                int seen = 0;
                for (int i = 0; i < parent.childCount; i++)
                {
                    var child = parent.GetChild(i).gameObject;
                    if (child.name != name)
                        continue;
                    seen++;
                    if (seen == ordinal)
                        return child;
                }
            }

            for (int i = 0; i < parent.childCount; i++)
            {
                var child = parent.GetChild(i).gameObject;
                if (child.name == segment.Trim())
                    return child;
            }

            return null;
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

        private static string BuildSourcePrefabPath(GameObject go)
        {
            if (!PrefabUtility.IsPartOfAnyPrefab(go))
                return null;

            var source = PrefabUtility.GetCorrespondingObjectFromOriginalSource(go);
            if (source == null)
                return null;

            string path = AssetDatabase.GetAssetPath(source);
            return string.IsNullOrEmpty(path) ? null : path;
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
