using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int PropertyTreeFilteredDiscoverMaxResults = 500;
        private const int PropertyTreeShallowPathDiscoverMaxResults = 1001;
        private const int PropertyTreeIncludeAllDiscoverMaxResults = 50000;
        private const int PropertyTreeSubassetPreviewLimit = 32;
        // Filtered discovery stays streaming and bounded even when a scene has
        // thousands of components or a serialized array contains baked data.
        private const int PropertyTreeDiscoverSerializedObjectBudget = 20000;
        private const int PropertyTreeDiscoverSerializedPropertyBudget = 500000;

        [Serializable]
        private sealed class PropertyTreeTarget
        {
            public string kind;
            public string guid;
            public string path;
            public string scenePath;
            public string objectPath;
            public long objectFileId;
            public long targetFileId;
            public string componentType;
            public int componentIndex;
            public string targetTypeFullName;
            public string targetTypeAssembly;
            public string targetTypeName;
            public string propertyPath;
        }

        [Serializable]
        private sealed class PropertyTreeReadRequest
        {
            public string bindingId;
            public PropertyTreeTarget target;
            public int maxDepth;
            public int maxArrayItems;
            public int autoExpandCharLimit;
            public string schemaMode;
        }

        [Serializable]
        private sealed class PropertyTreeWriteRequest
        {
            public string bindingId;
            public PropertyTreeTarget target;
            public string valueJson;
            public string mode;
            public string schemaMode;
        }

        [Serializable]
        private sealed class PropertyTreeApplyRequest
        {
            public PropertyTreeWriteRequest[] writes;
        }

        [Serializable]
        private sealed class PropertyTreeDiscoverRequest
        {
            public string bindingId;
            public PropertyTreeTarget target;
            public string query;
            public string fieldName;
            public string fieldType;
            public string[] matchFields;
            public int maxDepth;
            public int maxResults;
            public bool includeAll;
            public bool shallowPathMatches;
            public string schemaMode;
        }

        private sealed class PropertyTreeDiscoverMatch
        {
            public string semanticPath;
            public string propertyPath;
            public string displayName;
            public string name;
            public string type;
            public string valueType;
            public string fieldTypeFullName;
            public string fieldTypeAssembly;
            public string displayValue;
            public bool editable;
            public bool hasChildren;
            public bool isArray;
            public bool isManagedReference;
            public long managedReferenceId;
            public SerializedPropertyBindingTarget referenceTarget;
            public int depth;
            public bool matchedPath;
            public bool matchedFieldName;
            public bool matchedFieldValue;
            public bool matchedType;
        }

        private sealed class PropertyTreeDiscoverResponse
        {
            public bool ok;
            public string bindingId;
            public string message;
            public PropertyTreeTarget target;
            public PropertyTreeDiscoverMatch[] matches;
            public bool truncated;
            public int scannedObjects;
            public int scannedProperties;
        }

        private sealed class PropertyTreeDiscoverTraversalState
        {
            public int scannedObjects;
            public int scannedProperties;
            public bool truncated;

            public bool TryBeginSerializedObject()
            {
                if (scannedObjects >= PropertyTreeDiscoverSerializedObjectBudget)
                {
                    truncated = true;
                    return false;
                }
                scannedObjects++;
                return true;
            }

            public bool TryVisitSerializedProperty()
            {
                if (scannedProperties >= PropertyTreeDiscoverSerializedPropertyBudget)
                {
                    truncated = true;
                    return false;
                }
                scannedProperties++;
                return true;
            }
        }

        private sealed class PropertyTreeSearchFieldSet
        {
            public bool path;
            public bool name;
            public bool value;
            public bool type;
        }

        private sealed class PropertyTreeSearchMatchEvidence
        {
            public bool path;
            public bool fieldName;
            public bool fieldValue;
            public bool type;

            public bool Any()
            {
                return path || fieldName || fieldValue || type;
            }
        }

        private sealed class PropertyTreeSubassetRecord
        {
            public UnityEngine.Object obj;
            public PropertyTreeSubassetEntry entry;
            public List<PropertyTreeSubassetRecord> children = new List<PropertyTreeSubassetRecord>();
        }

        private static async Task<PipeEnvelope> HandlePropertyTreeRead(string requestId, string message)
        {
            PropertyTreeReadRequest request;
            try
            {
                request = JsonUtility.FromJson<PropertyTreeReadRequest>(message ?? "{}");
                ValidatePropertyTreeObjectTarget(request != null ? request.target : null);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunPropertyTreeOnMainThread(
                requestId,
                "property_tree_read",
                delegate { return ReadPropertyTree(request.bindingId, request.target, request.maxDepth, request.maxArrayItems, request.autoExpandCharLimit, IsDynamicSchemaMode(request.schemaMode)); });
        }

        private static async Task<PipeEnvelope> HandlePropertyTreeWrite(string requestId, string message)
        {
            PropertyTreeWriteRequest request;
            try
            {
                request = JsonUtility.FromJson<PropertyTreeWriteRequest>(message ?? "{}");
                ValidatePropertyTreeTarget(request != null ? request.target : null);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunPropertyTreeOnMainThread(
                requestId,
                "property_tree_write",
                delegate { return WritePropertyTree(request.bindingId, request.target, request.valueJson, request.mode, IsDynamicSchemaMode(request.schemaMode)); });
        }

        private static async Task<PipeEnvelope> HandlePropertyTreeApply(string requestId, string message)
        {
            PropertyTreeApplyRequest request;
            try
            {
                request = JsonUtility.FromJson<PropertyTreeApplyRequest>(message ?? "{}");
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunPropertyTreeOnMainThread(
                requestId,
                "property_tree_apply",
                delegate { return ApplyPropertyTrees(request); });
        }

        private static async Task<PipeEnvelope> HandlePropertyTreeDiscover(string requestId, string message)
        {
            PropertyTreeDiscoverRequest request;
            try
            {
                request = JsonUtility.FromJson<PropertyTreeDiscoverRequest>(message ?? "{}");
                if (request == null)
                    throw new Exception("Property tree discover request is empty");
                ValidatePropertyTreeObjectTarget(request.target);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunPropertyTreeOnMainThread(
                requestId,
                "property_tree_discover",
                delegate { return DiscoverPropertyTreeProperties(request); });
        }

        private static async Task<PipeEnvelope> RunPropertyTreeOnMainThread(
            string requestId,
            string operation,
            Func<string> action)
        {
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    tcs.TrySetResult(OkResponse(requestId, action()));
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult(ErrorResponse(requestId, ex.Message));
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return ErrorResponse(requestId, operation + " timed out");

            return tcs.Task.Result;
        }

        private static void ValidatePropertyTreeTarget(PropertyTreeTarget target)
        {
            ValidatePropertyTreeObjectTarget(target);
            if (string.IsNullOrWhiteSpace(target.propertyPath))
                throw new Exception("Property tree target propertyPath is required");
        }

        private static void ValidatePropertyTreeObjectTarget(PropertyTreeTarget target)
        {
            if (target == null)
                throw new Exception("Property tree target is required");
            if (string.IsNullOrWhiteSpace(target.kind))
                throw new Exception("Property tree target kind is required");
        }

        private sealed class ResolvedPropertyTreeWrite
        {
            public int index;
            public string bindingId;
            public PropertyTreeTarget target;
            public string valueJson;
            public string mode;
            public bool dynamicSchema;
            public UnityEngine.Object obj;
        }

        private sealed class AppliedPropertyTreeWrite
        {
            public ResolvedPropertyTreeWrite write;
            public SerializedProperty prop;
        }

        private const string PropertyTreeComponentEnabledPropertyPath = "m_Enabled";
        private const string PropertyTreeGameObjectActivePropertyPath = "m_IsActive";
        private const string PropertyTreeGameObjectStaticPropertyPath = "__locus_static";

        private static string ApplyPropertyTrees(PropertyTreeApplyRequest request)
        {
            PropertyTreeWriteRequest[] writes = request != null && request.writes != null
                ? request.writes
                : new PropertyTreeWriteRequest[0];

            string[] resultItems = new string[writes.Length];
            bool ok = true;
            var objectCache = new Dictionary<string, UnityEngine.Object>(StringComparer.Ordinal);
            var groups = new Dictionary<int, List<ResolvedPropertyTreeWrite>>();
            var groupObjects = new Dictionary<int, UnityEngine.Object>();

            for (int i = 0; i < writes.Length; i++)
            {
                PropertyTreeWriteRequest write = writes[i];
                try
                {
                    if (write == null)
                        throw new Exception("Property tree write is required");
                    ValidatePropertyTreeTarget(write.target);

                    string objectKey = BuildPropertyTreeObjectKey(write.target);
                    UnityEngine.Object obj;
                    if (!objectCache.TryGetValue(objectKey, out obj))
                    {
                        obj = ResolvePropertyTreeObject(write.target);
                        objectCache[objectKey] = obj;
                    }

                    int groupKey = LocusObjectIdentity.InstanceId(obj);
                    List<ResolvedPropertyTreeWrite> group;
                    if (!groups.TryGetValue(groupKey, out group))
                    {
                        group = new List<ResolvedPropertyTreeWrite>();
                        groups[groupKey] = group;
                        groupObjects[groupKey] = obj;
                    }

                    group.Add(new ResolvedPropertyTreeWrite
                    {
                        index = i,
                        bindingId = write.bindingId,
                        target = PropertyTreeTargetWithLocalFileIds(write.target, obj),
                        valueJson = write.valueJson,
                        mode = write.mode,
                        dynamicSchema = IsDynamicSchemaMode(write.schemaMode),
                        obj = obj
                    });
                }
                catch (Exception ex)
                {
                    ok = false;
                    resultItems[i] = BuildBindingErrorJson(
                        write != null ? write.bindingId : null,
                        write != null ? write.target : null,
                        ex.Message);
                }
            }

            foreach (KeyValuePair<int, List<ResolvedPropertyTreeWrite>> entry in groups)
            {
                UnityEngine.Object obj = groupObjects[entry.Key];
                List<ResolvedPropertyTreeWrite> group = entry.Value;
                try
                {
                    var serialized = new SerializedObject(obj);
                    serialized.Update();
                    var applied = new List<AppliedPropertyTreeWrite>(group.Count);

                    for (int i = 0; i < group.Count; i++)
                    {
                        ResolvedPropertyTreeWrite write = group[i];
                        try
                        {
                            if (IsPropertyTreeSyntheticHeaderProperty(obj, write.target))
                            {
                                resultItems[write.index] = WritePropertyTreeSyntheticHeaderProperty(
                                    write.bindingId,
                                    write.target,
                                    obj,
                                    write.valueJson);
                                continue;
                            }

                            SerializedProperty prop = serialized.FindProperty(write.target.propertyPath);
                            if (prop == null)
                                throw new Exception("SerializedProperty not found: " + write.target.propertyPath);

                            if (IsPropertyTreePreviewMode(write.mode))
                            {
                                prop = ApplyPropertyTreePreviewValue(obj, serialized, prop, write);
                                resultItems[write.index] =
                                    BuildBindingReadJson(write.bindingId, write.target, prop, false, write.dynamicSchema);
                            }
                            else
                            {
                                SetSerializedPropertyValue(prop, write.valueJson);
                                applied.Add(new AppliedPropertyTreeWrite
                                {
                                    write = write,
                                    prop = prop
                                });
                            }
                        }
                        catch (Exception ex)
                        {
                            ok = false;
                            resultItems[write.index] =
                                BuildBindingErrorJson(write.bindingId, write.target, ex.Message);
                        }
                    }

                    if (applied.Count > 0)
                        ApplyPropertyTreeSerializedChanges(serialized, obj);

                    for (int i = 0; i < applied.Count; i++)
                    {
                        AppliedPropertyTreeWrite item = applied[i];
                        SerializedProperty freshProp = serialized.FindProperty(item.write.target.propertyPath);
                        resultItems[item.write.index] =
                            BuildBindingReadJson(item.write.bindingId, item.write.target, freshProp != null ? freshProp : item.prop, true, item.write.dynamicSchema);
                    }
                }
                catch (Exception ex)
                {
                    ok = false;
                    for (int i = 0; i < group.Count; i++)
                    {
                        ResolvedPropertyTreeWrite write = group[i];
                        if (resultItems[write.index] == null)
                            resultItems[write.index] =
                                BuildBindingErrorJson(write.bindingId, write.target, ex.Message);
                    }
                }
            }

            for (int i = 0; i < resultItems.Length; i++)
            {
                if (resultItems[i] == null)
                    resultItems[i] = BuildBindingErrorJson(null, null, "Property tree write did not run");
            }

            string json = "{" +
                          "\"ok\":" + (ok ? "true" : "false") + "," +
                          "\"message\":\"" + JsonEscape(ok ? "Applied bindings." : "Some bindings failed.") + "\"," +
                          "\"results\":[" + string.Join(",", resultItems) + "]" +
                          "}";
            return json;
        }

        private static string BuildPropertyTreeObjectKey(PropertyTreeTarget target)
        {
            return (target.kind ?? "").Trim().ToLowerInvariant() + "|" +
                   (target.guid ?? "").Trim().ToLowerInvariant() + "|" +
                   (target.path ?? "").Trim().Replace('\\', '/') + "|" +
                   (target.scenePath ?? "").Trim().Replace('\\', '/') + "|" +
                   (target.objectPath ?? "").Trim().Replace('\\', '/') + "|" +
                   target.objectFileId.ToString(CultureInfo.InvariantCulture) + "|" +
                   target.targetFileId.ToString(CultureInfo.InvariantCulture) + "|" +
                   (target.componentType ?? "").Trim() + "|" +
                   target.componentIndex.ToString(CultureInfo.InvariantCulture);
        }

        private static bool IsDynamicSchemaMode(string schemaMode)
        {
            return string.Equals((schemaMode ?? "").Trim(), "dynamic", StringComparison.OrdinalIgnoreCase);
        }

        private static string ReadPropertyTree(
            string bindingId,
            PropertyTreeTarget target,
            int maxDepth = 0,
            int maxArrayItems = 0,
            int autoExpandCharLimit = 0,
            bool dynamicSchema = false)
        {
            string sceneRead = TryReadPropertyTreeScene(
                bindingId,
                target,
                maxDepth,
                maxArrayItems,
                autoExpandCharLimit);
            if (sceneRead != null)
                return sceneRead;

            UnityEngine.Object obj = ResolvePropertyTreeObject(target);
            target = PropertyTreeTargetWithLocalFileIds(target, obj);
            var serialized = new SerializedObject(obj);
            serialized.Update();
            if (string.IsNullOrWhiteSpace(target.propertyPath))
            {
                int depthLimit = maxDepth > 0 ? Math.Min(maxDepth, 16) : 4;
                int arrayLimit = maxArrayItems > 0 ? Math.Min(maxArrayItems, 1024) : 64;
                SerializedPropertySnapshot[] properties = SnapshotPropertyTreeObjectProperties(
                    target,
                    obj,
                    depthLimit,
                    arrayLimit,
                    dynamicSchema);
                SerializedPropertySnapshot objectSnapshot = properties.Length == 1
                    ? properties[0]
                    : BuildPropertyTreeAggregateSnapshot(target, obj, properties);
                objectSnapshot.subassets = BuildPropertyTreeSubassetRecords(
                    target,
                    obj,
                    objectSnapshot.children)
                    .Select(record => record.entry)
                    .ToArray();
                objectSnapshot.displaySections = BuildPropertyTreeDisplaySections(obj);
                return BuildBindingReadJson(bindingId, target, objectSnapshot, false, properties.Length > 1 ? properties : null);
            }
            if (IsPropertyTreeSyntheticHeaderProperty(obj, target))
            {
                SerializedPropertySnapshot syntheticSnapshot = BuildPropertyTreeSyntheticHeaderPropertySnapshot(
                    obj,
                    ToSerializedPropertyBindingTarget(target));
                return BuildBindingReadJson(bindingId, target, syntheticSnapshot, false);
            }
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            int propertyDepthLimit = maxDepth > 0 ? Math.Min(maxDepth, 16) : 4;
            int propertyArrayLimit = maxArrayItems > 0 ? Math.Min(maxArrayItems, 1024) : 64;
            SerializedPropertySnapshot propertySnapshot = SnapshotSerializedProperty(prop, propertyDepthLimit, propertyArrayLimit, dynamicSchema);
            ApplyPropertyTreeTargetToSnapshotTree(propertySnapshot, ToSerializedPropertyBindingTarget(target));
            return BuildBindingReadJson(
                bindingId,
                target,
                propertySnapshot,
                false);
        }

        /// <summary>
        /// A .unity asset path represents the loaded Scene container rather
        /// than the serialized SceneAsset importer object.  The container's
        /// children are real, addressable GameObjects, so read/search can use
        /// the same asset-qualified paths while observing unsaved hierarchy
        /// changes in the connected Editor.
        /// </summary>
        private static string TryReadPropertyTreeScene(
            string bindingId,
            PropertyTreeTarget target,
            int maxDepth,
            int maxArrayItems,
            int autoExpandCharLimit)
        {
            if (target == null
                || !string.Equals((target.kind ?? "").Trim(), "asset", StringComparison.OrdinalIgnoreCase)
                || !string.IsNullOrWhiteSpace(target.propertyPath))
            {
                return null;
            }

            string scenePath = ResolvePropertyTreeAssetPath(target);
            if (!IsSceneAssetPath(scenePath))
                return null;

            Scene scene = ResolveScene(scenePath);
            int depthLimit = maxDepth > 0 ? Math.Min(maxDepth, 16) : 4;
            GameObject[] roots = scene.GetRootGameObjects();
            if (autoExpandCharLimit > 0
                && PropertyTreeSceneHierarchyFitsBudget(
                    roots,
                    Math.Max(256, autoExpandCharLimit)))
            {
                depthLimit = 16;
            }
            int arrayLimit = maxArrayItems > 0 ? Math.Min(maxArrayItems, 4) : 4;
            target.scenePath = scenePath;
            target.targetTypeFullName = "UnityEngine.SceneManagement.Scene";
            target.targetTypeAssembly = typeof(Scene).Assembly.GetName().Name;
            target.targetTypeName = "Scene";

            SerializedPropertySnapshot snapshot = BuildPropertyTreeSceneSnapshot(
                target,
                scene,
                roots,
                depthLimit,
                arrayLimit);
            return BuildBindingReadJson(bindingId, target, snapshot, false);
        }

        private static SerializedPropertySnapshot BuildPropertyTreeSceneSnapshot(
            PropertyTreeTarget target,
            Scene scene,
            GameObject[] roots,
            int maxDepth,
            int maxArrayItems)
        {
            SerializedPropertySnapshot[] children = maxDepth > 0
                ? BuildPropertyTreeSceneHierarchyChildren(
                    target,
                    roots,
                    "",
                    maxDepth - 1,
                    maxArrayItems)
                : new SerializedPropertySnapshot[0];
            bool hasChildren = roots.Length > 0;
            string sceneName = !string.IsNullOrWhiteSpace(scene.name)
                ? scene.name
                : ResolvePropertyTreeAssetPath(target);

            return new SerializedPropertySnapshot
            {
                propertyPath = "",
                semanticPath = "",
                nodeKind = "scene",
                canonicalPath = "",
                bindingTarget = ToSerializedPropertyBindingTarget(target),
                referenceTarget = null,
                displayName = sceneName,
                name = sceneName,
                type = "Scene",
                valueType = "Object",
                fieldTypeFullName = "UnityEngine.SceneManagement.Scene",
                fieldTypeAssembly = typeof(Scene).Assembly.GetName().Name,
                value = sceneName,
                displayValue = sceneName,
                editable = false,
                hasChildren = hasChildren,
                isArray = false,
                arraySize = -1,
                visibleChildCount = children.Length,
                childrenTruncated = maxDepth <= 0 && hasChildren,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = children,
                isManagedReference = false,
                managedReferenceId = 0,
                managedReferenceFullTypename = "",
                managedReferenceFieldTypename = "",
                managedReferenceDisplayName = "",
                managedReferenceTypes = new SerializedManagedReferenceTypeOption[0],
                tooltip = "",
                header = "",
                hasRange = false,
                rangeMin = 0f,
                rangeMax = 0f,
                numberStep = 0f,
                multiline = false,
                minLines = 0,
                maxLines = 0,
                referenceTypeFullName = "UnityEngine.SceneManagement.Scene",
                referenceTypeAssembly = typeof(Scene).Assembly.GetName().Name,
                attributes = new SerializedPropertyAttributeInfo[0],
                displaySections = new[]
                {
                    new PropertyTreeDisplaySection
                    {
                        title = "Scene",
                        lines = new[]
                        {
                            "Active: " + (scene == SceneManager.GetActiveScene() ? "true" : "false"),
                            "Loaded: " + (scene.isLoaded ? "true" : "false"),
                            "Dirty: " + (scene.isDirty ? "true" : "false"),
                            "Root GameObjects: " + roots.Length.ToString(CultureInfo.InvariantCulture)
                        }
                    }
                }
            };
        }

        private static bool PropertyTreeSceneHierarchyFitsBudget(
            GameObject[] roots,
            int charLimit)
        {
            int remaining = Math.Max(0, charLimit) - 256;
            return ConsumePropertyTreeSceneHierarchyBudget(roots, 1, ref remaining);
        }

        private static bool ConsumePropertyTreeSceneHierarchyBudget(
            GameObject[] objects,
            int depth,
            ref int remaining)
        {
            if (depth > 16)
                return false;
            for (int i = 0; i < objects.Length; i++)
            {
                GameObject go = objects[i];
                if (go == null)
                    continue;
                string summary = BuildComponentSuffix(go) + BuildGoAnnotations(go);
                remaining -= depth * 3
                    + (go.name ?? "GameObject").Length
                    + summary.Length
                    + 8;
                if (remaining < 0)
                    return false;
                if (go.transform == null || go.transform.childCount == 0)
                    continue;
                var children = new GameObject[go.transform.childCount];
                for (int j = 0; j < children.Length; j++)
                    children[j] = go.transform.GetChild(j).gameObject;
                if (!ConsumePropertyTreeSceneHierarchyBudget(
                    children,
                    depth + 1,
                    ref remaining))
                {
                    return false;
                }
            }
            return true;
        }

        private static SerializedPropertySnapshot[] BuildPropertyTreeSceneHierarchyChildren(
            PropertyTreeTarget sceneTarget,
            GameObject[] siblings,
            string parentObjectPath,
            int remainingDepth,
            int maxArrayItems)
        {
            var totals = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < siblings.Length; i++)
            {
                GameObject sibling = siblings[i];
                if (sibling == null)
                    continue;
                int count;
                totals.TryGetValue(sibling.name ?? "GameObject", out count);
                totals[sibling.name ?? "GameObject"] = count + 1;
            }

            var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);
            var children = new List<SerializedPropertySnapshot>(siblings.Length);
            for (int i = 0; i < siblings.Length; i++)
            {
                GameObject child = siblings[i];
                if (child == null)
                    continue;
                string segment = PropertyTreeUniqueHierarchySegment(
                    child.name ?? "GameObject",
                    totals,
                    ordinals);
                string objectPath = string.IsNullOrWhiteSpace(parentObjectPath)
                    ? segment
                    : parentObjectPath + "/" + segment;
                children.Add(BuildPropertyTreeSceneHierarchyNode(
                    sceneTarget,
                    child,
                    segment,
                    objectPath,
                    remainingDepth,
                    maxArrayItems));
            }
            return children.ToArray();
        }

        private static SerializedPropertySnapshot BuildPropertyTreeSceneHierarchyNode(
            PropertyTreeTarget sceneTarget,
            GameObject go,
            string segment,
            string objectPath,
            int remainingDepth,
            int maxArrayItems)
        {
            var objectTarget = new PropertyTreeTarget
            {
                kind = "gameobject",
                guid = sceneTarget.guid ?? "",
                path = sceneTarget.path ?? sceneTarget.scenePath ?? "",
                scenePath = sceneTarget.scenePath ?? sceneTarget.path ?? "",
                objectPath = objectPath,
                componentType = "",
                componentIndex = 0,
                targetTypeFullName = "UnityEngine.GameObject",
                targetTypeAssembly = typeof(GameObject).Assembly.GetName().Name,
                targetTypeName = "GameObject",
                propertyPath = ""
            };
            objectTarget = PropertyTreeTargetWithLocalFileIds(objectTarget, go);

            int childCount = go.transform != null ? go.transform.childCount : 0;
            SerializedPropertySnapshot[] children = new SerializedPropertySnapshot[0];
            if (remainingDepth > 0 && childCount > 0)
            {
                var childObjects = new GameObject[childCount];
                for (int i = 0; i < childCount; i++)
                    childObjects[i] = go.transform.GetChild(i).gameObject;
                children = BuildPropertyTreeSceneHierarchyChildren(
                    sceneTarget,
                    childObjects,
                    objectPath,
                    remainingDepth - 1,
                    maxArrayItems);
            }

            string summary = BuildComponentSuffix(go) + BuildGoAnnotations(go);
            return new SerializedPropertySnapshot
            {
                propertyPath = "",
                semanticPath = "",
                nodeKind = "hierarchy",
                canonicalPath = "",
                bindingTarget = ToSerializedPropertyBindingTarget(objectTarget),
                referenceTarget = null,
                displayName = segment,
                name = segment,
                type = "GameObject",
                valueType = "Object",
                fieldTypeFullName = "UnityEngine.GameObject",
                fieldTypeAssembly = typeof(GameObject).Assembly.GetName().Name,
                value = go.name ?? "",
                displayValue = summary,
                editable = false,
                hasChildren = childCount > 0,
                isArray = false,
                arraySize = -1,
                visibleChildCount = children.Length,
                childrenTruncated = remainingDepth <= 0 && childCount > 0,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = children,
                isManagedReference = false,
                managedReferenceId = 0,
                managedReferenceFullTypename = "",
                managedReferenceFieldTypename = "",
                managedReferenceDisplayName = "",
                managedReferenceTypes = new SerializedManagedReferenceTypeOption[0],
                tooltip = "",
                header = "",
                hasRange = false,
                rangeMin = 0f,
                rangeMax = 0f,
                numberStep = 0f,
                multiline = false,
                minLines = 0,
                maxLines = 0,
                referenceTypeFullName = "UnityEngine.GameObject",
                referenceTypeAssembly = typeof(GameObject).Assembly.GetName().Name,
                attributes = new SerializedPropertyAttributeInfo[0],
                displaySections = new PropertyTreeDisplaySection[0]
            };
        }

        private static string PropertyTreeUniqueHierarchySegment(
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
            return ordinal <= 1
                ? name
                : name + "[" + ordinal.ToString(CultureInfo.InvariantCulture) + "]";
        }

        private static SerializedPropertySnapshot[] SnapshotPropertyTreeObjectProperties(
            PropertyTreeTarget target,
            UnityEngine.Object obj,
            int maxDepth,
            int maxArrayItems,
            bool dynamicSchema)
        {
            GameObject go = obj as GameObject;
            if (go == null)
                return new[] { SnapshotPropertyTreeObject(target, obj, maxDepth, maxArrayItems, dynamicSchema) };

            var properties = new List<SerializedPropertySnapshot>();
            properties.Add(SnapshotPropertyTreeObject(
                PropertyTreeGameObjectTarget(target),
                go,
                maxDepth,
                maxArrayItems,
                dynamicSchema));
            var componentIndexes = new Dictionary<string, int>(StringComparer.Ordinal);
            Component[] components = go.GetComponents<Component>();
            for (int i = 0; i < components.Length; i++)
            {
                Component component = components[i];
                if (component == null)
                    continue;

                string componentType = ComponentBindingTypeName(component);
                int componentIndex = 0;
                componentIndexes.TryGetValue(componentType, out componentIndex);
                componentIndexes[componentType] = componentIndex + 1;

                properties.Add(SnapshotPropertyTreeObject(
                    PropertyTreeComponentTarget(target, componentType, componentIndex),
                    component,
                    maxDepth,
                    maxArrayItems,
                    dynamicSchema));
            }

            return properties.ToArray();
        }

        private static SerializedPropertySnapshot SnapshotPropertyTreeObject(
            PropertyTreeTarget target,
            UnityEngine.Object obj,
            int maxDepth,
            int maxArrayItems,
            bool dynamicSchema)
        {
            target = PropertyTreeTargetWithLocalFileIds(target, obj);
            SerializedPropertySnapshot snapshot = SnapshotSerializedObject(obj, maxDepth, maxArrayItems, dynamicSchema);
            if (snapshot == null)
                return null;

            SerializedPropertyBindingTarget bindingTarget = ToSerializedPropertyBindingTarget(target);
            ApplyPropertyTreeTargetToSnapshotTree(snapshot, bindingTarget);
            snapshot.displayName = PropertyTreeObjectDisplayName(obj);
            snapshot.name = snapshot.displayName;
            snapshot.children = WithPropertyTreeSyntheticHeaderProperties(
                obj,
                bindingTarget,
                snapshot.children);
            snapshot.hasChildren = snapshot.children != null && snapshot.children.Length > 0;
            return snapshot;
        }

        private static void ApplyPropertyTreeTargetToSnapshotTree(
            SerializedPropertySnapshot snapshot,
            SerializedPropertyBindingTarget bindingTarget)
        {
            if (snapshot == null || bindingTarget == null)
                return;

            SerializedPropertyBindingTarget propertyTarget = CloneSerializedPropertyBindingTarget(bindingTarget);
            propertyTarget.propertyPath = snapshot.propertyPath ?? "";
            snapshot.bindingTarget = propertyTarget;

            SerializedPropertySnapshot[] children = snapshot.children ?? new SerializedPropertySnapshot[0];
            for (int i = 0; i < children.Length; i++)
                ApplyPropertyTreeTargetToSnapshotTree(children[i], bindingTarget);
        }

        private static SerializedPropertySnapshot[] WithPropertyTreeSyntheticHeaderProperties(
            UnityEngine.Object obj,
            SerializedPropertyBindingTarget bindingTarget,
            SerializedPropertySnapshot[] children)
        {
            var remaining = new List<SerializedPropertySnapshot>(
                (children ?? new SerializedPropertySnapshot[0]).Where(child => child != null));
            var semantic = new List<SerializedPropertySnapshot>();

            GameObject go = obj as GameObject;
            if (go != null)
            {
                semantic.Add(TakeOrBuildPropertyTreeSemanticProperty(
                    remaining,
                    "m_Name",
                    "Name",
                    "String",
                    typeof(string),
                    go.name ?? "",
                    go.name ?? ""));

                RemovePropertyTreeProperty(remaining, PropertyTreeGameObjectActivePropertyPath);
                semantic.Add(BuildPropertyTreeSyntheticHeaderPropertySnapshot(obj, bindingTarget));

                // Unity serializes several static-editor flags as an integer.
                // The semantic Property Tree exposes the effective static state
                // as an editable bool and keeps the implementation flags out of
                // both the agent outline and the shared inspector.
                RemovePropertyTreeProperty(remaining, "m_StaticEditorFlags");
                SerializedPropertyBindingTarget staticTarget = CloneSerializedPropertyBindingTarget(bindingTarget);
                if (staticTarget != null)
                    staticTarget.propertyPath = PropertyTreeGameObjectStaticPropertyPath;
                semantic.Add(BuildPropertyTreeSyntheticBooleanPropertySnapshot(
                    staticTarget,
                    PropertyTreeGameObjectStaticPropertyPath,
                    "Static",
                    go.isStatic,
                    true));

                string layerName = LayerMask.LayerToName(go.layer);
                string layerDisplay = go.layer.ToString(CultureInfo.InvariantCulture);
                if (!string.IsNullOrWhiteSpace(layerName))
                    layerDisplay += " (" + layerName + ")";
                semantic.Add(TakeOrBuildPropertyTreeSemanticProperty(
                    remaining,
                    "m_Layer",
                    "Layer",
                    "Integer",
                    typeof(int),
                    go.layer,
                    layerDisplay));
                semantic.Add(TakeOrBuildPropertyTreeSemanticProperty(
                    remaining,
                    "m_TagString",
                    "Tag",
                    "String",
                    typeof(string),
                    go.tag ?? "",
                    go.tag ?? ""));
            }
            else
            {
                SerializedPropertySnapshot header = BuildPropertyTreeSyntheticHeaderPropertySnapshot(obj, bindingTarget);
                if (header != null)
                {
                    RemovePropertyTreeProperty(remaining, header.propertyPath);
                    semantic.Add(header);
                }

                Transform transform = obj as Transform;
                if (transform != null)
                {
                    AddPropertyTreeSemanticPropertyIfPresent(semantic, remaining, "m_LocalRotation", "Local Rotation");
                    AddPropertyTreeSemanticPropertyIfPresent(semantic, remaining, "m_LocalPosition", "Local Position");
                    AddPropertyTreeSemanticPropertyIfPresent(semantic, remaining, "m_LocalScale", "Local Scale");
                    AddPropertyTreeSemanticPropertyIfPresent(
                        semantic,
                        remaining,
                        "m_ConstrainProportionsScale",
                        "Constrain Proportions Scale");
                }

                if (obj is MonoBehaviour || obj is ScriptableObject)
                    AddPropertyTreeSemanticPropertyIfPresent(semantic, remaining, "m_Script", "Script");
            }

            semantic.AddRange(remaining);
            return semantic.Where(property => property != null).ToArray();
        }

        private static SerializedPropertySnapshot TakeOrBuildPropertyTreeSemanticProperty(
            List<SerializedPropertySnapshot> properties,
            string propertyPath,
            string displayName,
            string propertyType,
            Type fieldType,
            object value,
            string displayValue)
        {
            SerializedPropertySnapshot property = TakePropertyTreeProperty(properties, propertyPath);
            if (property == null)
            {
                return BuildPropertyTreeSyntheticValuePropertySnapshot(
                    null,
                    propertyPath,
                    displayName,
                    propertyType,
                    fieldType,
                    value,
                    displayValue,
                    false,
                    null);
            }
            property.name = displayName;
            property.displayName = displayName;
            property.displayValue = displayValue ?? "";
            return property;
        }

        private static void AddPropertyTreeSemanticPropertyIfPresent(
            List<SerializedPropertySnapshot> destination,
            List<SerializedPropertySnapshot> source,
            string propertyPath,
            string displayName)
        {
            SerializedPropertySnapshot property = TakePropertyTreeProperty(source, propertyPath);
            if (property == null)
                return;
            property.name = displayName;
            property.displayName = displayName;
            destination.Add(property);
        }

        private static SerializedPropertySnapshot TakePropertyTreeProperty(
            List<SerializedPropertySnapshot> properties,
            string propertyPath)
        {
            int index = properties.FindIndex(property => property != null
                && string.Equals(property.propertyPath, propertyPath, StringComparison.Ordinal));
            if (index < 0)
                return null;
            SerializedPropertySnapshot property = properties[index];
            properties.RemoveAt(index);
            return property;
        }

        private static void RemovePropertyTreeProperty(
            List<SerializedPropertySnapshot> properties,
            string propertyPath)
        {
            properties.RemoveAll(property => property != null
                && string.Equals(property.propertyPath, propertyPath, StringComparison.Ordinal));
        }

        /// <summary>
        /// Builds an addressable forest of every non-main Unity object stored
        /// in the same asset file. Ownership follows the first same-file
        /// ObjectReference reached from the main object in SerializedProperty
        /// order. SerializedProperty is deliberately the only source here:
        /// ordinary CLR fields, properties, and runtime caches never
        /// participate in the forest.
        /// </summary>
        private static List<PropertyTreeSubassetRecord> BuildPropertyTreeSubassetRecords(
            PropertyTreeTarget source,
            UnityEngine.Object obj,
            SerializedPropertySnapshot[] rootChildren)
        {
            var records = new List<PropertyTreeSubassetRecord>();
            if (source == null || obj == null || !string.IsNullOrWhiteSpace(source.propertyPath))
                return records;

            string path = ResolvePropertyTreeAssetPath(source);
            if (string.IsNullOrWhiteSpace(path)
                || IsSceneAssetPath(path)
                || IsPrefabAssetPath(path))
            {
                return records;
            }

            UnityEngine.Object main = AssetDatabase.LoadMainAssetAtPath(path);
            if (main == null || main != obj)
                return records;

            var used = new HashSet<string>(StringComparer.Ordinal);
            SerializedPropertySnapshot[] children = rootChildren ?? new SerializedPropertySnapshot[0];
            for (int i = 0; i < children.Length; i++)
            {
                SerializedPropertySnapshot child = children[i];
                if (child == null)
                    continue;
                string name = !string.IsNullOrWhiteSpace(child.name)
                    ? child.name
                    : child.displayName;
                if (!string.IsNullOrWhiteSpace(name))
                    used.Add(name);
            }

            var candidatesByFileId = new Dictionary<long, PropertyTreeSubassetRecord>();
            var candidateOrder = new List<long>();
            UnityEngine.Object[] candidates = AssetDatabase.LoadAllAssetsAtPath(path);
            for (int i = 0; i < candidates.Length; i++)
            {
                UnityEngine.Object candidate = candidates[i];
                if (candidate == null || candidate == main)
                    continue;

                long localFileId;
                if (!TryGetLocalFileId(candidate, out localFileId) || localFileId == 0)
                    continue;

                Type type = candidate.GetType();
                string displayName = (candidate.name ?? "").Trim();
                var target = new PropertyTreeTarget
                {
                    kind = "asset",
                    guid = !string.IsNullOrWhiteSpace(source.guid)
                        ? source.guid
                        : AssetDatabase.AssetPathToGUID(path),
                    path = path,
                    targetFileId = localFileId,
                    targetTypeFullName = FieldTypeFullName(type),
                    targetTypeAssembly = FieldTypeAssembly(type),
                    targetTypeName = type.Name,
                    propertyPath = ""
                };
                var record = new PropertyTreeSubassetRecord
                {
                    obj = candidate,
                    entry = new PropertyTreeSubassetEntry
                    {
                        segment = "",
                        displayName = !string.IsNullOrWhiteSpace(displayName)
                            ? displayName
                            : type.Name,
                        type = type.Name,
                        typeFullName = FieldTypeFullName(type),
                        target = ToSerializedPropertyBindingTarget(target),
                        children = new PropertyTreeSubassetEntry[0]
                    }
                };
                candidatesByFileId[localFileId] = record;
                candidateOrder.Add(localFileId);
            }

            var claimed = new HashSet<long>();
            long mainFileId;
            if (TryGetLocalFileId(main, out mainFileId) && mainFileId != 0)
                claimed.Add(mainFileId);

            var nextOrdinals = new Dictionary<string, int>(StringComparer.Ordinal);
            AppendPropertyTreeOwnedSubassets(
                main,
                path,
                candidatesByFileId,
                claimed,
                records,
                used,
                nextOrdinals);

            // Objects without a serialized owner remain independently
            // addressable roots, in AssetDatabase order. Their own serialized
            // references can still define descendants.
            for (int i = 0; i < candidateOrder.Count; i++)
            {
                long fileId = candidateOrder[i];
                if (!claimed.Add(fileId))
                    continue;
                PropertyTreeSubassetRecord record = candidatesByFileId[fileId];
                AssignPropertyTreeSubassetSegment(record, used, nextOrdinals);
                PopulatePropertyTreeSubassetChildren(
                    record,
                    path,
                    candidatesByFileId,
                    claimed);
                records.Add(record);
            }
            return records;
        }

        private static void AppendPropertyTreeOwnedSubassets(
            UnityEngine.Object owner,
            string assetPath,
            Dictionary<long, PropertyTreeSubassetRecord> candidatesByFileId,
            HashSet<long> claimed,
            List<PropertyTreeSubassetRecord> destination,
            HashSet<string> used,
            Dictionary<string, int> nextOrdinals)
        {
            List<long> references = PropertyTreeSerializedLocalReferences(
                owner,
                assetPath,
                candidatesByFileId);
            for (int i = 0; i < references.Count; i++)
            {
                long fileId = references[i];
                if (!claimed.Add(fileId))
                    continue;
                PropertyTreeSubassetRecord record;
                if (!candidatesByFileId.TryGetValue(fileId, out record))
                    continue;
                AssignPropertyTreeSubassetSegment(record, used, nextOrdinals);
                PopulatePropertyTreeSubassetChildren(
                    record,
                    assetPath,
                    candidatesByFileId,
                    claimed);
                destination.Add(record);
            }
        }

        private static void PopulatePropertyTreeSubassetChildren(
            PropertyTreeSubassetRecord record,
            string assetPath,
            Dictionary<long, PropertyTreeSubassetRecord> candidatesByFileId,
            HashSet<long> claimed)
        {
            var used = PropertyTreeSerializedRootSegments(record.obj);
            var nextOrdinals = new Dictionary<string, int>(StringComparer.Ordinal);
            AppendPropertyTreeOwnedSubassets(
                record.obj,
                assetPath,
                candidatesByFileId,
                claimed,
                record.children,
                used,
                nextOrdinals);
            record.entry.children = record.children
                .Select(child => child.entry)
                .ToArray();
        }

        private static void AssignPropertyTreeSubassetSegment(
            PropertyTreeSubassetRecord record,
            HashSet<string> used,
            Dictionary<string, int> nextOrdinals)
        {
            string baseName = record != null && record.entry != null
                ? record.entry.displayName
                : "";
            if (string.IsNullOrWhiteSpace(baseName))
                baseName = record != null && record.entry != null
                    ? record.entry.type
                    : "Subasset";
            record.entry.segment = PropertyTreeUniqueSubassetSegment(
                baseName,
                used,
                nextOrdinals);
        }

        private static List<long> PropertyTreeSerializedLocalReferences(
            UnityEngine.Object owner,
            string assetPath,
            Dictionary<long, PropertyTreeSubassetRecord> candidatesByFileId)
        {
            var references = new List<long>();
            if (owner == null || candidatesByFileId == null || candidatesByFileId.Count == 0)
                return references;

            try
            {
                var serialized = new SerializedObject(owner);
                serialized.Update();
                SerializedProperty cursor = serialized.GetIterator();
                bool enterChildren = true;
                while (cursor.Next(enterChildren))
                {
                    enterChildren = true;
                    if (cursor.propertyType != SerializedPropertyType.ObjectReference)
                        continue;
                    UnityEngine.Object referenced = cursor.objectReferenceValue;
                    if (referenced == null
                        || !string.Equals(
                            (AssetDatabase.GetAssetPath(referenced) ?? "").Replace('\\', '/'),
                            assetPath,
                            StringComparison.OrdinalIgnoreCase))
                    {
                        continue;
                    }
                    long fileId;
                    if (TryGetLocalFileId(referenced, out fileId)
                        && fileId != 0
                        && candidatesByFileId.ContainsKey(fileId))
                    {
                        references.Add(fileId);
                    }
                }
            }
            catch
            {
                // Some importer-owned objects cannot create a
                // SerializedObject. They remain independent roots.
            }
            return references;
        }

        private static HashSet<string> PropertyTreeSerializedRootSegments(
            UnityEngine.Object owner)
        {
            var used = new HashSet<string>(StringComparer.Ordinal);
            if (owner == null)
                return used;
            try
            {
                var serialized = new SerializedObject(owner);
                serialized.Update();
                SerializedProperty cursor = serialized.GetIterator();
                bool enterChildren = true;
                while (cursor.Next(enterChildren))
                {
                    if (cursor.depth == 0)
                    {
                        if (!string.IsNullOrWhiteSpace(cursor.name))
                            used.Add(cursor.name);
                        if (!string.IsNullOrWhiteSpace(cursor.displayName))
                            used.Add(cursor.displayName);
                    }
                    enterChildren = false;
                }
            }
            catch
            {
            }
            return used;
        }

        private static string PropertyTreeUniqueSubassetSegment(
            string baseName,
            HashSet<string> used,
            Dictionary<string, int> nextOrdinals)
        {
            string normalizedBase = !string.IsNullOrWhiteSpace(baseName)
                ? baseName.Trim()
                : "Subasset";
            int ordinal;
            nextOrdinals.TryGetValue(normalizedBase, out ordinal);
            ordinal = Math.Max(1, ordinal);
            string candidate = ordinal == 1
                ? normalizedBase
                : normalizedBase + "[" + ordinal.ToString(CultureInfo.InvariantCulture) + "]";
            while (used.Contains(candidate))
            {
                ordinal++;
                candidate = normalizedBase + "[" + ordinal.ToString(CultureInfo.InvariantCulture) + "]";
            }
            used.Add(candidate);
            nextOrdinals[normalizedBase] = ordinal + 1;
            return candidate;
        }

        private static PropertyTreeDisplaySection[] BuildPropertyTreeDisplaySections(
            UnityEngine.Object obj)
        {
            var sections = new List<PropertyTreeDisplaySection>();
            GameObject go = obj as GameObject;
            Transform transform = obj as Transform;
            if (go != null)
            {
                sections.Add(BuildPropertyTreeHierarchyDisplaySection(go));
                sections.Add(BuildPropertyTreeTransformDisplaySection(go.transform));

                PropertyTreeDisplaySection prefab = BuildPropertyTreePrefabDisplaySection(go);
                if (prefab != null)
                    sections.Add(prefab);

                int missingScripts = go.GetComponents<Component>().Count(component => component == null);
                if (missingScripts > 0)
                {
                    sections.Add(new PropertyTreeDisplaySection
                    {
                        title = "Diagnostics",
                        lines = new[]
                        {
                            "Missing Scripts: " + missingScripts.ToString(CultureInfo.InvariantCulture)
                        }
                    });
                }
            }
            else if (transform != null)
            {
                sections.Add(BuildPropertyTreeTransformDisplaySection(transform));
            }

            return sections.Where(section => section != null && section.lines != null && section.lines.Length > 0).ToArray();
        }

        private static PropertyTreeDisplaySection BuildPropertyTreeHierarchyDisplaySection(GameObject go)
        {
            var lines = new List<string>();
            Transform transform = go.transform;
            lines.Add(transform.parent != null
                ? "Parent: " + FormatReadHierarchyNodeLabel(transform.parent.gameObject)
                : "Parent: none");
            lines.Add(FormatReadHierarchyNodeLabel(go));
            for (int i = 0; i < transform.childCount; i++)
            {
                Transform child = transform.GetChild(i);
                bool isLast = i + 1 == transform.childCount;
                string line = (isLast ? "└─ " : "├─ ") + FormatReadHierarchyNodeLabel(child.gameObject);
                int descendants = CountDescendants(child);
                if (descendants > 0)
                    line += " … +" + descendants.ToString(CultureInfo.InvariantCulture) + " descendants";
                lines.Add(line);
            }
            return new PropertyTreeDisplaySection
            {
                title = "Hierarchy",
                lines = lines.ToArray()
            };
        }

        private static PropertyTreeDisplaySection BuildPropertyTreeTransformDisplaySection(Transform transform)
        {
            return new PropertyTreeDisplaySection
            {
                title = "Transform",
                lines = new[]
                {
                    "World Position: " + FormatVector3(transform.position),
                    "World Rotation: " + FormatVector3(transform.rotation.eulerAngles),
                    "World Scale: " + FormatVector3(transform.lossyScale)
                }
            };
        }

        private static PropertyTreeDisplaySection BuildPropertyTreePrefabDisplaySection(GameObject go)
        {
            if (go == null || !PrefabUtility.IsPartOfAnyPrefab(go))
                return null;

            var lines = new List<string>();
            UnityEngine.Object source = PrefabUtility.GetCorrespondingObjectFromSource(go);
            string sourcePath = source != null ? AssetDatabase.GetAssetPath(source) : "";
            if (!string.IsNullOrWhiteSpace(sourcePath))
                lines.Add("Source Prefab: " + sourcePath);

            GameObject nearestRoot = PrefabUtility.GetNearestPrefabInstanceRoot(go);
            if (nearestRoot != null && nearestRoot != go)
                lines.Add("Prefab Instance Root: " + PropertyTreeHierarchyObjectPath(nearestRoot));

            return lines.Count > 0
                ? new PropertyTreeDisplaySection { title = "Prefab", lines = lines.ToArray() }
                : null;
        }

        private static string PropertyTreeHierarchyObjectPath(GameObject go)
        {
            if (go == null)
                return "";

            var segments = new List<string>();
            Transform current = go.transform;
            while (current != null)
            {
                int ordinal = 1;
                Transform parent = current.parent;
                if (parent != null)
                {
                    for (int i = 0; i < parent.childCount; i++)
                    {
                        Transform sibling = parent.GetChild(i);
                        if (sibling == current)
                            break;
                        if (string.Equals(sibling.name, current.name, StringComparison.Ordinal))
                            ordinal++;
                    }
                }
                else if (current.gameObject.scene.IsValid())
                {
                    GameObject[] roots = current.gameObject.scene.GetRootGameObjects();
                    for (int i = 0; i < roots.Length; i++)
                    {
                        if (roots[i] == current.gameObject)
                            break;
                        if (string.Equals(roots[i].name, current.name, StringComparison.Ordinal))
                            ordinal++;
                    }
                }

                string segment = current.name ?? "GameObject";
                if (ordinal > 1)
                    segment += "[" + ordinal.ToString(CultureInfo.InvariantCulture) + "]";
                segments.Add(segment);
                current = parent;
            }
            segments.Reverse();
            return string.Join("/", segments.ToArray());
        }

        private static SerializedPropertySnapshot BuildPropertyTreeSyntheticHeaderPropertySnapshot(
            UnityEngine.Object obj,
            SerializedPropertyBindingTarget bindingTarget)
        {
            GameObject go = obj as GameObject;
            if (go != null)
            {
                string requestedPath = bindingTarget != null
                    ? (bindingTarget.propertyPath ?? "").Trim()
                    : "";
                if (string.Equals(
                    requestedPath,
                    PropertyTreeGameObjectStaticPropertyPath,
                    StringComparison.Ordinal))
                {
                    return BuildPropertyTreeSyntheticBooleanPropertySnapshot(
                        bindingTarget,
                        PropertyTreeGameObjectStaticPropertyPath,
                        "Static",
                        go.isStatic,
                        true);
                }
                return BuildPropertyTreeSyntheticBooleanPropertySnapshot(
                    bindingTarget,
                    PropertyTreeGameObjectActivePropertyPath,
                    "Active",
                    go.activeSelf,
                    true);
            }

            Component component = obj as Component;
            bool enabled;
            if (component != null && TryGetPropertyTreeComponentEnabledState(component, out enabled))
            {
                return BuildPropertyTreeSyntheticBooleanPropertySnapshot(
                    bindingTarget,
                    PropertyTreeComponentEnabledPropertyPath,
                    "Enabled",
                    enabled,
                    CanSetPropertyTreeComponentEnabledState(component));
            }

            return null;
        }

        private static SerializedPropertySnapshot BuildPropertyTreeSyntheticBooleanPropertySnapshot(
            SerializedPropertyBindingTarget bindingTarget,
            string propertyPath,
            string displayName,
            bool value,
            bool editable)
        {
            return BuildPropertyTreeSyntheticValuePropertySnapshot(
                bindingTarget,
                propertyPath,
                displayName,
                "Boolean",
                typeof(bool),
                value,
                value ? "true" : "false",
                editable,
                null);
        }

        private static SerializedPropertySnapshot BuildPropertyTreeSyntheticValuePropertySnapshot(
            SerializedPropertyBindingTarget bindingTarget,
            string propertyPath,
            string displayName,
            string propertyType,
            Type fieldType,
            object value,
            string displayValue,
            bool editable,
            SerializedPropertySnapshot[] children)
        {
            SerializedPropertyBindingTarget propertyTarget = CloneSerializedPropertyBindingTarget(bindingTarget);
            if (propertyTarget != null)
                propertyTarget.propertyPath = propertyPath ?? "";

            SerializedPropertySnapshot[] childValues = children ?? new SerializedPropertySnapshot[0];
            bool hasChildren = childValues.Length > 0;
            return new SerializedPropertySnapshot
            {
                propertyPath = propertyPath ?? "",
                semanticPath = "",
                nodeKind = hasChildren ? "object" : "property",
                canonicalPath = "",
                bindingTarget = propertyTarget,
                referenceTarget = null,
                displayName = displayName ?? "",
                name = displayName ?? "",
                type = propertyType ?? "Generic",
                valueType = propertyType ?? "Generic",
                fieldTypeFullName = FieldTypeFullName(fieldType),
                fieldTypeAssembly = FieldTypeAssembly(fieldType),
                value = value,
                displayValue = displayValue ?? "",
                editable = editable,
                hasChildren = hasChildren,
                isArray = false,
                arraySize = -1,
                visibleChildCount = childValues.Length,
                childrenTruncated = false,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = childValues,
                isManagedReference = false,
                managedReferenceId = 0,
                managedReferenceFullTypename = "",
                managedReferenceFieldTypename = "",
                managedReferenceDisplayName = "",
                managedReferenceTypes = new SerializedManagedReferenceTypeOption[0],
                tooltip = "",
                header = "",
                hasRange = false,
                rangeMin = 0f,
                rangeMax = 0f,
                numberStep = 0f,
                multiline = false,
                minLines = 0,
                maxLines = 0,
                referenceTypeFullName = "",
                referenceTypeAssembly = "",
                attributes = new SerializedPropertyAttributeInfo[0]
            };
        }

        private static SerializedPropertyBindingTarget CloneSerializedPropertyBindingTarget(
            SerializedPropertyBindingTarget source)
        {
            if (source == null)
                return null;

            return new SerializedPropertyBindingTarget
            {
                kind = source.kind ?? "",
                guid = source.guid ?? "",
                path = source.path ?? "",
                scenePath = source.scenePath ?? "",
                objectPath = source.objectPath ?? "",
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType ?? "",
                componentIndex = source.componentIndex,
                targetTypeFullName = source.targetTypeFullName ?? "",
                targetTypeAssembly = source.targetTypeAssembly ?? "",
                targetTypeName = source.targetTypeName ?? "",
                propertyPath = source.propertyPath ?? ""
            };
        }

        private static SerializedPropertySnapshot BuildPropertyTreeAggregateSnapshot(
            PropertyTreeTarget target,
            UnityEngine.Object obj,
            SerializedPropertySnapshot[] properties)
        {
            string displayName = obj != null && !string.IsNullOrWhiteSpace(obj.name)
                ? obj.name
                : "Unity Object";
            Type type = obj != null ? obj.GetType() : typeof(UnityEngine.Object);
            return new SerializedPropertySnapshot
            {
                propertyPath = "",
                semanticPath = "",
                nodeKind = "object",
                canonicalPath = "",
                bindingTarget = ToSerializedPropertyBindingTarget(target),
                referenceTarget = null,
                displayName = displayName,
                name = displayName,
                type = "Object",
                valueType = "Object",
                fieldTypeFullName = FieldTypeFullName(type),
                fieldTypeAssembly = FieldTypeAssembly(type),
                value = displayName,
                displayValue = displayName,
                editable = false,
                hasChildren = properties != null && properties.Length > 0,
                isArray = false,
                arraySize = -1,
                visibleChildCount = properties != null ? properties.Length : 0,
                childrenTruncated = false,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = properties ?? new SerializedPropertySnapshot[0],
                isManagedReference = false,
                managedReferenceId = 0,
                managedReferenceFullTypename = "",
                managedReferenceFieldTypename = "",
                managedReferenceDisplayName = "",
                managedReferenceTypes = new SerializedManagedReferenceTypeOption[0],
                tooltip = "",
                header = "",
                hasRange = false,
                rangeMin = 0f,
                rangeMax = 0f,
                numberStep = 0f,
                multiline = false,
                minLines = 0,
                maxLines = 0,
                referenceTypeFullName = FieldTypeFullName(type),
                referenceTypeAssembly = FieldTypeAssembly(type),
                attributes = new SerializedPropertyAttributeInfo[0]
            };
        }

        internal static string FormatPropertyTreeForExecute(
            UnityEngine.Object obj,
            int depth,
            int maxArrayItems)
        {
            if (obj == null)
                return "null";

            int depthLimit = Math.Max(0, Math.Min(depth, 16));
            int arrayLimit = Math.Max(1, Math.Min(maxArrayItems, 1024));
            PropertyTreeTarget target = PropertyTreeTargetForObject(obj);
            SerializedPropertySnapshot[] properties = SnapshotPropertyTreeObjectProperties(
                target,
                obj,
                Math.Max(1, depthLimit),
                arrayLimit,
                false);
            SerializedPropertySnapshot snapshot = properties.Length == 1
                ? properties[0]
                : BuildPropertyTreeAggregateSnapshot(target, obj, properties);
            snapshot.subassets = BuildPropertyTreeSubassetRecords(
                target,
                obj,
                snapshot.children)
                .Select(record => record.entry)
                .ToArray();
            snapshot.displaySections = BuildPropertyTreeDisplaySections(obj);

            string rootPath = PropertyTreeExecuteRootPath(target, obj);
            var output = new StringBuilder();
            output.AppendLine(FormatPropertyTreeExecuteNode(snapshot, rootPath, true, depthLimit == 0));
            if (depthLimit > 0)
                AppendPropertyTreeExecuteChildren(output, snapshot, "", 0, depthLimit);
            AppendPropertyTreeExecuteSubassets(output, snapshot.subassets);
            AppendPropertyTreeExecuteSections(output, snapshot.displaySections);
            return output.ToString();
        }

        private static PropertyTreeTarget PropertyTreeTargetForObject(UnityEngine.Object obj)
        {
            Component component = obj as Component;
            GameObject go = obj as GameObject;
            GameObject owner = component != null ? component.gameObject : go;
            string assetPath = AssetDatabase.GetAssetPath(obj);
            if (string.IsNullOrWhiteSpace(assetPath) && owner != null)
                assetPath = AssetDatabase.GetAssetPath(owner);
            if (string.IsNullOrWhiteSpace(assetPath)
                && owner != null
                && owner.scene.IsValid())
            {
                assetPath = owner.scene.path;
            }
            assetPath = (assetPath ?? "").Replace('\\', '/');

            var target = new PropertyTreeTarget
            {
                kind = component != null ? "component" : (go != null ? "gameobject" : "asset"),
                guid = !string.IsNullOrWhiteSpace(assetPath)
                    ? AssetDatabase.AssetPathToGUID(assetPath)
                    : "",
                path = assetPath,
                scenePath = owner != null
                    && owner.scene.IsValid()
                    && !string.IsNullOrWhiteSpace(owner.scene.path)
                    ? owner.scene.path.Replace('\\', '/')
                    : "",
                objectPath = owner != null ? PropertyTreeHierarchyObjectPath(owner) : "",
                componentType = component != null ? ComponentBindingTypeName(component) : "",
                componentIndex = component != null ? PropertyTreeComponentIndex(component) : 0,
                propertyPath = ""
            };
            return PropertyTreeTargetWithLocalFileIds(target, obj);
        }

        private static int PropertyTreeComponentIndex(Component component)
        {
            if (component == null || component.gameObject == null)
                return 0;
            Component[] matches = component.gameObject.GetComponents(component.GetType());
            for (int i = 0; i < matches.Length; i++)
            {
                if (matches[i] == component)
                    return i;
            }
            return 0;
        }

        private static string PropertyTreeExecuteRootPath(
            PropertyTreeTarget target,
            UnityEngine.Object obj)
        {
            string assetPath = (target.path ?? target.scenePath ?? "").Trim().TrimEnd('/');
            Component component = obj as Component;
            GameObject go = component != null ? component.gameObject : obj as GameObject;
            if (go == null)
            {
                if (!string.IsNullOrWhiteSpace(assetPath) && AssetDatabase.IsSubAsset(obj))
                    return assetPath + "/" + (obj.name ?? obj.GetType().Name);
                return !string.IsNullOrWhiteSpace(assetPath)
                    ? assetPath
                    : (obj.name ?? obj.GetType().Name);
            }

            string objectPath = (target.objectPath ?? PropertyTreeHierarchyObjectPath(go)).Trim('/');
            if (assetPath.EndsWith(".prefab", StringComparison.OrdinalIgnoreCase))
            {
                int separator = objectPath.IndexOf('/');
                objectPath = separator >= 0 ? objectPath.Substring(separator + 1) : "";
            }
            string path = string.IsNullOrWhiteSpace(assetPath)
                ? objectPath
                : (string.IsNullOrWhiteSpace(objectPath) ? assetPath : assetPath + "/" + objectPath);
            if (component == null)
                return path;

            string componentName = component.GetType().Name;
            if (target.componentIndex > 0)
                componentName += "[" + (target.componentIndex + 1).ToString(CultureInfo.InvariantCulture) + "]";
            return string.IsNullOrWhiteSpace(path) ? componentName : path + "/" + componentName;
        }

        private static void AppendPropertyTreeExecuteChildren(
            StringBuilder output,
            SerializedPropertySnapshot parent,
            string prefix,
            int parentDepth,
            int maxDepth)
        {
            if (parent == null || IsPropertyTreeExecuteCompactValue(parent) || parentDepth >= maxDepth)
                return;

            SerializedPropertySnapshot[] children = parent.children ?? new SerializedPropertySnapshot[0];
            bool arrayOmission = parent.isArray && parent.childrenTruncated;
            for (int i = 0; i < children.Length; i++)
            {
                SerializedPropertySnapshot child = children[i];
                bool last = i + 1 == children.Length && !arrayOmission;
                int childDepth = parentDepth + 1;
                output.Append(prefix);
                output.Append(last ? "└─ " : "├─ ");
                output.AppendLine(FormatPropertyTreeExecuteNode(
                    child,
                    null,
                    false,
                    childDepth >= maxDepth));

                if (childDepth < maxDepth)
                {
                    AppendPropertyTreeExecuteChildren(
                        output,
                        child,
                        prefix + (last ? "   " : "│  "),
                        childDepth,
                        maxDepth);
                }
            }

            if (arrayOmission)
            {
                int omitted = Math.Max(0, parent.arraySize - children.Length);
                output.Append(prefix).Append("└─ …");
                if (omitted > 0)
                    output.Append(" +").Append(omitted.ToString(CultureInfo.InvariantCulture));
                output.AppendLine();
            }
        }

        private static string FormatPropertyTreeExecuteNode(
            SerializedPropertySnapshot node,
            string rootPath,
            bool root,
            bool depthBoundary)
        {
            string name = root
                ? (!string.IsNullOrWhiteSpace(rootPath) ? rootPath : node.displayName)
                : (!string.IsNullOrWhiteSpace(node.name) ? node.name : node.displayName);
            var output = new StringBuilder(root ? name : PropertyTreeEncodePathSegment(name));
            string type = PropertyTreeExecuteDisplayType(node);

            if (node.isArray)
            {
                output.Append(" [").Append(Math.Max(0, node.arraySize)).Append(']');
                if (!string.IsNullOrWhiteSpace(type) && type != "Array" && type != "Generic")
                    output.Append(" (").Append(type).Append(')');
            }
            else if (IsPropertyTreeExecuteCompactValue(node)
                || (!node.hasChildren && (node.children == null || node.children.Length == 0)))
            {
                if (!string.IsNullOrWhiteSpace(node.displayValue))
                    output.Append(": ").Append(PropertyTreeCompactScalar(node.displayValue));
            }
            else
            {
                if (!string.IsNullOrWhiteSpace(node.displayValue)
                    && string.Equals(node.nodeKind, "reference", StringComparison.OrdinalIgnoreCase))
                {
                    output.Append(": ").Append(PropertyTreeCompactScalar(node.displayValue));
                }
                if (!string.IsNullOrWhiteSpace(type) && type != "Object" && type != "Generic")
                    output.Append(" (").Append(type).Append(')');
            }

            if ((depthBoundary || node.childrenTruncated)
                && !node.isArray
                && !IsPropertyTreeExecuteCompactValue(node)
                && node.hasChildren)
            {
                output.Append(" …");
            }
            return output.ToString();
        }

        private static void AppendPropertyTreeExecuteSections(
            StringBuilder output,
            PropertyTreeDisplaySection[] sections)
        {
            if (sections == null)
                return;
            for (int i = 0; i < sections.Length; i++)
            {
                PropertyTreeDisplaySection section = sections[i];
                if (section == null || string.IsNullOrWhiteSpace(section.title) || section.lines == null)
                    continue;
                output.AppendLine();
                output.Append("--- ").Append(section.title.Trim()).AppendLine(" ---");
                for (int j = 0; j < section.lines.Length; j++)
                {
                    if (!string.IsNullOrWhiteSpace(section.lines[j]))
                        output.Append("  ").AppendLine(section.lines[j].TrimEnd());
                }
            }
        }

        private static void AppendPropertyTreeExecuteSubassets(
            StringBuilder output,
            PropertyTreeSubassetEntry[] subassets)
        {
            if (subassets == null || subassets.Length == 0)
                return;

            int total = CountPropertyTreeSubassets(subassets);
            output.AppendLine();
            output.Append("--- Subassets [")
                .Append(total.ToString(CultureInfo.InvariantCulture))
                .AppendLine("] ---");
            int visible = 0;
            for (int i = 0; i < subassets.Length && visible < PropertyTreeSubassetPreviewLimit; i++)
            {
                PropertyTreeSubassetEntry entry = subassets[i];
                if (entry == null || string.IsNullOrWhiteSpace(entry.segment))
                    continue;
                output.Append("  ");
                AppendPropertyTreeExecuteSubassetLabel(output, entry);
                output.AppendLine();
                visible++;
                AppendPropertyTreeExecuteSubassetChildren(
                    output,
                    entry.children,
                    "  ",
                    ref visible);
            }
            if (visible < total)
            {
                output.Append("  … +")
                    .Append((total - visible).ToString(CultureInfo.InvariantCulture))
                    .AppendLine();
            }
        }

        private static void AppendPropertyTreeExecuteSubassetChildren(
            StringBuilder output,
            PropertyTreeSubassetEntry[] children,
            string prefix,
            ref int visible)
        {
            if (children == null)
                return;
            for (int i = 0; i < children.Length && visible < PropertyTreeSubassetPreviewLimit; i++)
            {
                PropertyTreeSubassetEntry child = children[i];
                if (child == null || string.IsNullOrWhiteSpace(child.segment))
                    continue;
                bool last = i + 1 == children.Length;
                output.Append(prefix).Append(last ? "└─ " : "├─ ");
                AppendPropertyTreeExecuteSubassetLabel(output, child);
                output.AppendLine();
                visible++;
                AppendPropertyTreeExecuteSubassetChildren(
                    output,
                    child.children,
                    prefix + (last ? "   " : "│  "),
                    ref visible);
            }
        }

        private static void AppendPropertyTreeExecuteSubassetLabel(
            StringBuilder output,
            PropertyTreeSubassetEntry entry)
        {
            output.Append(PropertyTreeEncodePathSegment(entry.segment));
            if (!string.IsNullOrWhiteSpace(entry.type))
                output.Append(" (").Append(entry.type).Append(')');
        }

        private static int CountPropertyTreeSubassets(PropertyTreeSubassetEntry[] entries)
        {
            if (entries == null)
                return 0;
            int count = 0;
            for (int i = 0; i < entries.Length; i++)
            {
                PropertyTreeSubassetEntry entry = entries[i];
                if (entry == null || string.IsNullOrWhiteSpace(entry.segment))
                    continue;
                count++;
                count += CountPropertyTreeSubassets(entry.children);
            }
            return count;
        }

        private static bool IsPropertyTreeExecuteCompactValue(SerializedPropertySnapshot node)
        {
            if (node == null || string.IsNullOrWhiteSpace(node.displayValue))
                return false;
            string type = PropertyTreeExecuteDisplayType(node);
            switch (type)
            {
                case "Vector2":
                case "Vector3":
                case "Vector4":
                case "Vector2Int":
                case "Vector3Int":
                case "Quaternion":
                case "Color":
                case "Rect":
                case "RectInt":
                case "Bounds":
                case "BoundsInt":
                case "AnimationCurve":
                case "Gradient":
                case "Hash128":
                    return true;
                default:
                    return false;
            }
        }

        private static string PropertyTreeExecuteDisplayType(SerializedPropertySnapshot node)
        {
            string type = !string.IsNullOrWhiteSpace(node.fieldTypeFullName)
                ? node.fieldTypeFullName
                : (!string.IsNullOrWhiteSpace(node.valueType) ? node.valueType : node.type);
            int assemblySeparator = type.IndexOf(',');
            if (assemblySeparator >= 0)
                type = type.Substring(0, assemblySeparator);
            int namespaceSeparator = type.LastIndexOf('.');
            return namespaceSeparator >= 0 ? type.Substring(namespaceSeparator + 1) : type;
        }

        private static string PropertyTreeEncodePathSegment(string value)
        {
            return (value ?? "").Replace("~", "~0").Replace("/", "~1");
        }

        private static string PropertyTreeCompactScalar(string value)
        {
            string normalized = (value ?? "").Replace('\r', ' ').Replace('\n', ' ').Replace('\t', ' ');
            return normalized.Length <= 160 ? normalized : normalized.Substring(0, 157) + "...";
        }

        private static PropertyTreeTarget PropertyTreeTargetWithLocalFileIds(
            PropertyTreeTarget source,
            UnityEngine.Object obj)
        {
            if (source == null)
                return null;

            var target = new PropertyTreeTarget
            {
                kind = source.kind,
                guid = source.guid,
                path = source.path,
                scenePath = source.scenePath,
                objectPath = source.objectPath,
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType,
                componentIndex = source.componentIndex,
                targetTypeFullName = source.targetTypeFullName,
                targetTypeAssembly = source.targetTypeAssembly,
                targetTypeName = source.targetTypeName,
                propertyPath = source.propertyPath
            };

            Type objectType = obj != null ? obj.GetType() : null;
            if (objectType != null)
            {
                target.targetTypeFullName = FieldTypeFullName(objectType);
                target.targetTypeAssembly = FieldTypeAssembly(objectType);
                target.targetTypeName = objectType.Name ?? "";
            }

            long objectFileId;
            GameObject go = obj as GameObject;
            Component component = obj as Component;
            if (go != null && TryGetLocalFileId(go, out objectFileId))
            {
                target.objectFileId = objectFileId;
                if (target.targetFileId == 0)
                    target.targetFileId = objectFileId;
            }
            else if (component != null)
            {
                if (component.gameObject != null && TryGetLocalFileId(component.gameObject, out objectFileId))
                    target.objectFileId = objectFileId;
                long componentFileId;
                if (TryGetLocalFileId(component, out componentFileId))
                    target.targetFileId = componentFileId;
            }
            else if (obj != null)
            {
                long assetFileId;
                if (TryGetLocalFileId(obj, out assetFileId))
                    target.targetFileId = assetFileId;
            }

            return target;
        }

        private static PropertyTreeTarget PropertyTreeGameObjectTarget(PropertyTreeTarget source)
        {
            if (source == null)
                return null;
            return new PropertyTreeTarget
            {
                kind = source.kind,
                guid = source.guid,
                path = source.path,
                scenePath = source.scenePath,
                objectPath = source.objectPath,
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = "",
                componentIndex = 0,
                targetTypeFullName = source.targetTypeFullName,
                targetTypeAssembly = source.targetTypeAssembly,
                targetTypeName = source.targetTypeName,
                propertyPath = ""
            };
        }

        private static PropertyTreeTarget PropertyTreeComponentTarget(
            PropertyTreeTarget source,
            string componentType,
            int componentIndex)
        {
            return new PropertyTreeTarget
            {
                kind = "component",
                guid = source != null ? source.guid : "",
                path = source != null ? source.path : "",
                scenePath = source != null ? source.scenePath : "",
                objectPath = source != null ? source.objectPath : "",
                objectFileId = source != null ? source.objectFileId : 0,
                targetFileId = 0,
                componentType = componentType,
                componentIndex = componentIndex,
                targetTypeFullName = "",
                targetTypeAssembly = "",
                targetTypeName = "",
                propertyPath = ""
            };
        }

        private static SerializedPropertyBindingTarget ToSerializedPropertyBindingTarget(PropertyTreeTarget source)
        {
            if (source == null)
                return null;
            return new SerializedPropertyBindingTarget
            {
                kind = source.kind ?? "",
                guid = source.guid ?? "",
                path = source.path ?? "",
                scenePath = source.scenePath ?? "",
                objectPath = source.objectPath ?? "",
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType ?? "",
                componentIndex = source.componentIndex,
                targetTypeFullName = source.targetTypeFullName ?? "",
                targetTypeAssembly = source.targetTypeAssembly ?? "",
                targetTypeName = source.targetTypeName ?? "",
                propertyPath = source.propertyPath ?? ""
            };
        }

        private static string ComponentBindingTypeName(Component component)
        {
            Type type = component != null ? component.GetType() : null;
            return type != null ? type.FullName ?? type.Name ?? "" : "";
        }

        private static string PropertyTreeObjectDisplayName(UnityEngine.Object obj)
        {
            if (obj is GameObject)
                return "GameObject";

            Component component = obj as Component;
            if (component != null)
                return component.GetType().Name;

            if (obj == null)
                return "Unity Object";

            Type objectType = obj.GetType();
            return ObjectNames.NicifyVariableName(objectType.Name);
        }

        private static bool IsPropertyTreePreviewMode(string mode)
        {
            return string.Equals((mode ?? "").Trim(), "preview", StringComparison.OrdinalIgnoreCase);
        }

        private static SerializedProperty ApplyPropertyTreePreviewValue(
            UnityEngine.Object obj,
            SerializedObject serialized,
            SerializedProperty prop,
            ResolvedPropertyTreeWrite write)
        {
            if (obj == null)
                throw new Exception("Preview write target object is required");
            if (prop == null)
                throw new Exception("Preview write property is required");
            if (!CanPreviewWriteSerializedProperty(prop))
                throw new Exception("Preview write is only supported for numeric leaf fields: " + prop.propertyPath);

            string propertyPath = prop.propertyPath ?? "";
            string[] parts = propertyPath.Replace(".Array.data[", "[").Split('.');
            if (parts.Any(part => part.IndexOf('[') >= 0))
                throw new Exception("Preview write does not support array paths: " + propertyPath);

            object boxedTarget = obj;
            string error;
            if (!TrySetDirectPreviewPathValue(
                ref boxedTarget,
                obj.GetType(),
                parts,
                0,
                prop.propertyType,
                write.valueJson,
                out error))
            {
                throw new Exception(error);
            }

            serialized.Update();
            SerializedProperty updated = serialized.FindProperty(write.target.propertyPath);
            return updated != null ? updated : prop;
        }

        private static bool CanPreviewWriteSerializedProperty(SerializedProperty prop)
        {
            if (prop == null || !IsSerializedPropertyWritable(prop))
                return false;
            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                case SerializedPropertyType.LayerMask:
                case SerializedPropertyType.Float:
                    return !prop.hasVisibleChildren;
                default:
                    return false;
            }
        }

        private static bool TrySetDirectPreviewPathValue(
            ref object container,
            Type containerType,
            string[] parts,
            int partIndex,
            SerializedPropertyType propertyType,
            string valueJson,
            out string error)
        {
            error = "";
            if (container == null || containerType == null)
            {
                error = "Preview write target path contains null object";
                return false;
            }
            if (partIndex < 0 || partIndex >= parts.Length)
            {
                error = "Preview write target path is empty";
                return false;
            }

            string memberName = parts[partIndex];
            if (string.IsNullOrWhiteSpace(memberName))
            {
                error = "Preview write target path contains an empty segment";
                return false;
            }

            FieldInfo field = SerializedMemberField(containerType, memberName);
            if (field == null)
            {
                error = "Preview write field not found: " + memberName;
                return false;
            }

            bool isLeaf = partIndex == parts.Length - 1;
            if (isLeaf)
            {
                object nextValue;
                if (!TryParseDirectPreviewValue(field.FieldType, propertyType, valueJson, out nextValue, out error))
                    return false;
                field.SetValue(container, nextValue);
                return true;
            }

            object child = field.GetValue(container);
            Type childType = field.FieldType;
            if (child == null)
            {
                error = "Preview write target path contains null field: " + memberName;
                return false;
            }

            object boxedChild = child;
            if (!TrySetDirectPreviewPathValue(
                ref boxedChild,
                childType,
                parts,
                partIndex + 1,
                propertyType,
                valueJson,
                out error))
            {
                return false;
            }

            field.SetValue(container, boxedChild);
            return true;
        }

        private static bool TryParseDirectPreviewValue(
            Type fieldType,
            SerializedPropertyType propertyType,
            string valueJson,
            out object value,
            out string error)
        {
            value = null;
            error = "";
            try
            {
                Type targetType = Nullable.GetUnderlyingType(fieldType) ?? fieldType;
                if (propertyType == SerializedPropertyType.Float)
                {
                    float parsed = ParseFloatJson(valueJson);
                    if (targetType == typeof(float))
                        value = parsed;
                    else if (targetType == typeof(double))
                        value = (double)parsed;
                    else
                        value = Convert.ChangeType(parsed, targetType, CultureInfo.InvariantCulture);
                    return true;
                }

                int intValue = ParseIntJson(valueJson);
                if (targetType == typeof(int))
                    value = intValue;
                else if (targetType == typeof(long))
                    value = (long)intValue;
                else if (targetType == typeof(short))
                    value = (short)intValue;
                else if (targetType == typeof(byte))
                    value = (byte)Math.Max(byte.MinValue, Math.Min(byte.MaxValue, intValue));
                else if (targetType == typeof(uint))
                    value = (uint)Math.Max(0, intValue);
                else if (targetType == typeof(ulong))
                    value = (ulong)Math.Max(0, intValue);
                else if (targetType == typeof(ushort))
                    value = (ushort)Math.Max(ushort.MinValue, Math.Min(ushort.MaxValue, intValue));
                else
                    value = Convert.ChangeType(intValue, targetType, CultureInfo.InvariantCulture);
                return true;
            }
            catch (Exception ex)
            {
                error = "Preview write failed to parse direct value: " + ex.Message;
                return false;
            }
        }

        private static string WritePropertyTree(
            string bindingId,
            PropertyTreeTarget target,
            string valueJson,
            string mode = null,
            bool dynamicSchema = false)
        {
            UnityEngine.Object obj = ResolvePropertyTreeObject(target);
            if (IsPropertyTreeSyntheticHeaderProperty(obj, target))
                return WritePropertyTreeSyntheticHeaderProperty(bindingId, target, obj, valueJson);

            var serialized = new SerializedObject(obj);
            serialized.Update();
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);

            if (IsPropertyTreePreviewMode(mode))
            {
                var write = new ResolvedPropertyTreeWrite
                {
                    index = 0,
                    bindingId = bindingId,
                    target = target,
                    valueJson = valueJson,
                    mode = mode,
                    dynamicSchema = dynamicSchema,
                    obj = obj
                };
                prop = ApplyPropertyTreePreviewValue(obj, serialized, prop, write);
                return BuildBindingReadJson(bindingId, target, prop, false, dynamicSchema);
            }

            SetSerializedPropertyValue(prop, valueJson);
            ApplyPropertyTreeSerializedChanges(serialized, obj);
            SerializedProperty updated = serialized.FindProperty(target.propertyPath);
            return BuildBindingReadJson(bindingId, target, updated != null ? updated : prop, true, dynamicSchema);
        }

        private static string DiscoverPropertyTreeProperties(PropertyTreeDiscoverRequest request)
        {
            string query = NormalizeSearchText(request.query);
            string rawQuery = (request.query ?? "").Trim();
            string fieldName = (request.fieldName ?? "").Trim();
            string fieldType = (request.fieldType ?? "").Trim();
            bool dynamicSchema = IsDynamicSchemaMode(request.schemaMode);
            bool includeAll = request.includeAll || dynamicSchema;
            if (!includeAll && string.IsNullOrEmpty(query) && string.IsNullOrEmpty(fieldName) && string.IsNullOrEmpty(fieldType))
                throw new Exception("Property tree discover requires query, fieldName, or fieldType");

            int maxDepth = request.maxDepth > 0 ? Math.Min(request.maxDepth, 32) : 8;
            int filteredMaxResults = request.shallowPathMatches
                ? PropertyTreeShallowPathDiscoverMaxResults
                : PropertyTreeFilteredDiscoverMaxResults;
            int maxResults = request.maxResults > 0
                ? Math.Min(request.maxResults, includeAll ? PropertyTreeIncludeAllDiscoverMaxResults : filteredMaxResults)
                : includeAll ? PropertyTreeIncludeAllDiscoverMaxResults : 100;
            PropertyTreeSearchFieldSet searchFields = BuildPropertyTreeSearchFieldSet(
                request.matchFields);
            var traversal = new PropertyTreeDiscoverTraversalState();

            PropertyTreeDiscoverResponse sceneResponse;
            if (TryDiscoverPropertyTreeScene(
                request,
                (request.query ?? "").Trim(),
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                traversal,
                out sceneResponse))
            {
                return ToJsonValue(sceneResponse, 0);
            }

            PropertyTreeDiscoverResponse subassetResponse;
            if (!includeAll && TryDiscoverPropertyTreeAssetWithSubassets(
                request,
                rawQuery,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                traversal,
                out subassetResponse))
            {
                return ToJsonValue(subassetResponse, 0);
            }

            UnityEngine.Object obj = ResolvePropertyTreeObject(request.target);
            PropertyTreeTarget resolvedTarget = PropertyTreeTargetWithLocalFileIds(request.target, obj);
            GameObject hierarchyRoot = obj as GameObject;
            if (hierarchyRoot != null)
            {
                PropertyTreeDiscoverResponse hierarchyResponse = DiscoverPropertyTreeGameObjectHierarchy(
                    request,
                    resolvedTarget,
                    hierarchyRoot,
                    rawQuery,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    traversal);
                return ToJsonValue(hierarchyResponse, 0);
            }

            if (string.IsNullOrWhiteSpace(resolvedTarget.propertyPath))
            {
                var rootMatches = new List<PropertyTreeDiscoverMatch>();
                CollectPropertyTreeObjectDiscoverMatches(
                    obj,
                    resolvedTarget,
                    PropertyTreeDiscoverObjectSemanticRoot(resolvedTarget, obj),
                    0,
                    rawQuery,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    request.shallowPathMatches,
                    false,
                    rootMatches,
                    traversal);
                return ToJsonValue(BuildPropertyTreeDiscoverResponse(
                    request,
                    resolvedTarget,
                    rootMatches,
                    traversal), 0);
            }

            if (!traversal.TryBeginSerializedObject())
            {
                return ToJsonValue(BuildPropertyTreeDiscoverResponse(
                    request,
                    resolvedTarget,
                    new List<PropertyTreeDiscoverMatch>(),
                    traversal), 0);
            }
            var serialized = new SerializedObject(obj);
            serialized.Update();
            string scopePropertyPath = (resolvedTarget.propertyPath ?? "").Trim();
            int scopeDepth = SerializedPropertyDepth(scopePropertyPath);

            var matches = new List<PropertyTreeDiscoverMatch>();
            SerializedProperty cursor = serialized.GetIterator();
            bool enterChildren = true;
            string shallowPathRoot = "";
            while (cursor.NextVisible(enterChildren))
            {
                if (!traversal.TryVisitSerializedProperty())
                    break;
                // Discovery is a filtered traversal, so arrays and deeply
                // nested objects are scanned in-place without first creating
                // a large snapshot. Scope and depth are applied relative to
                // the selected Property Tree path.
                enterChildren = !IsSerializedPropertyCompactValue(cursor.propertyType);
                if (!PropertyTreePropertyPathIsWithinScope(
                    cursor.propertyPath,
                    scopePropertyPath))
                {
                    continue;
                }
                if (!string.IsNullOrWhiteSpace(shallowPathRoot)
                    && !PropertyTreePropertyPathIsWithinScope(cursor.propertyPath, shallowPathRoot))
                {
                    shallowPathRoot = "";
                }
                int depth = Math.Max(
                    0,
                    SerializedPropertyDepth(cursor.propertyPath) - scopeDepth);
                if (depth > maxDepth)
                {
                    enterChildren = false;
                    continue;
                }

                Type resolvedType = null;
                var evidence = new PropertyTreeSearchMatchEvidence();
                if (!includeAll)
                {
                    resolvedType = ResolveSerializedPropertyFieldType(cursor);
                    if (!MatchesPropertyTreeDiscoveryName(cursor, fieldName))
                        continue;
                    evidence = PropertyTreeDiscoveryQueryEvidence(
                        cursor,
                        resolvedType,
                        rawQuery,
                        searchFields);
                    if (!string.IsNullOrEmpty(rawQuery) && !evidence.Any())
                        continue;
                    if (!string.IsNullOrEmpty(fieldType) && !TypeMatches(resolvedType, fieldType))
                        continue;

                    if (request.shallowPathMatches && evidence.path)
                    {
                        if (!string.IsNullOrWhiteSpace(shallowPathRoot))
                            evidence.path = false;
                        else
                            shallowPathRoot = cursor.propertyPath;
                    }
                    if (!string.IsNullOrEmpty(rawQuery) && !evidence.Any())
                        continue;
                }

                matches.Add(BuildPropertyTreeDiscoverMatch(cursor, resolvedType, depth, "", evidence));
                if (matches.Count >= maxResults)
                    break;
                if (request.shallowPathMatches
                    && evidence.path
                    && PropertyTreeSearchFieldsOnlyPath(searchFields))
                {
                    enterChildren = false;
                }
            }

            return ToJsonValue(BuildPropertyTreeDiscoverResponse(
                request,
                resolvedTarget,
                matches,
                traversal), 0);
        }

        private static PropertyTreeDiscoverResponse BuildPropertyTreeDiscoverResponse(
            PropertyTreeDiscoverRequest request,
            PropertyTreeTarget target,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            bool truncated = traversal != null && traversal.truncated;
            string message;
            if (truncated)
            {
                message = "Property Tree search scan budget reached; results may be incomplete.";
            }
            else
            {
                message = matches != null && matches.Count > 0
                    ? "ok"
                    : "No matching properties.";
            }
            return new PropertyTreeDiscoverResponse
            {
                ok = true,
                bindingId = request != null ? request.bindingId ?? "" : "",
                message = message,
                target = target,
                matches = (matches ?? new List<PropertyTreeDiscoverMatch>()).ToArray(),
                truncated = truncated,
                scannedObjects = traversal != null ? traversal.scannedObjects : 0,
                scannedProperties = traversal != null ? traversal.scannedProperties : 0
            };
        }

        private static string PropertyTreeDiscoverObjectSemanticRoot(
            PropertyTreeTarget target,
            UnityEngine.Object obj)
        {
            Component component = obj as Component;
            if (component != null)
                return PropertyTreeExecuteRootPath(target, component);

            GameObject go = obj as GameObject;
            if (go != null)
                return PropertyTreeHierarchySemanticPath(target, go);

            string assetPath = ResolvePropertyTreeAssetPath(target);
            if (obj != null && AssetDatabase.IsSubAsset(obj))
            {
                string segment = !string.IsNullOrWhiteSpace(obj.name)
                    ? obj.name
                    : obj.GetType().Name;
                return PropertyTreeAppendSemanticSegment(assetPath, segment);
            }
            return assetPath;
        }

        private static PropertyTreeDiscoverResponse DiscoverPropertyTreeGameObjectHierarchy(
            PropertyTreeDiscoverRequest request,
            PropertyTreeTarget resolvedTarget,
            GameObject scopeRoot,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            PropertyTreeDiscoverTraversalState traversal)
        {
            var matches = new List<PropertyTreeDiscoverMatch>();
            PropertyTreeTarget gameObjectTarget = PropertyTreeHierarchyGameObjectTarget(
                resolvedTarget,
                scopeRoot);
            string semanticRoot = PropertyTreeHierarchySemanticPath(
                gameObjectTarget,
                scopeRoot);

            CollectPropertyTreeGameObjectObjectRootsDiscoverMatches(
                gameObjectTarget,
                scopeRoot,
                semanticRoot,
                0,
                query,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                request.shallowPathMatches,
                false,
                matches,
                traversal);
            if (matches.Count < maxResults && !traversal.truncated && maxDepth >= 0)
            {
                CollectPropertyTreeGameObjectChildrenDiscoverMatches(
                    gameObjectTarget,
                    scopeRoot,
                    semanticRoot,
                    0,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    request.shallowPathMatches,
                    false,
                    matches,
                    traversal);
            }

            return BuildPropertyTreeDiscoverResponse(
                request,
                resolvedTarget,
                matches,
                traversal);
        }

        private static void CollectPropertyTreeGameObjectChildrenDiscoverMatches(
            PropertyTreeTarget sourceTarget,
            GameObject parent,
            string parentSemanticPath,
            int childDepth,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (parent == null
                || parent.transform == null
                || childDepth > maxDepth
                || matches.Count >= maxResults
                || (traversal != null && traversal.truncated))
            {
                return;
            }

            int childCount = parent.transform.childCount;
            var totals = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < childCount; i++)
            {
                Transform child = parent.transform.GetChild(i);
                if (child == null)
                    continue;
                string name = child.name ?? "GameObject";
                int count;
                totals.TryGetValue(name, out count);
                totals[name] = count + 1;
            }
            var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);

            for (int i = 0; i < childCount && matches.Count < maxResults; i++)
            {
                Transform childTransform = parent.transform.GetChild(i);
                if (childTransform == null)
                    continue;
                GameObject child = childTransform.gameObject;
                string segment = PropertyTreeUniqueHierarchySegment(
                    child.name ?? "GameObject",
                    totals,
                    ordinals);
                string semanticPath = PropertyTreeAppendSemanticSegment(
                    parentSemanticPath,
                    segment);
                PropertyTreeTarget childTarget = PropertyTreeHierarchyGameObjectTarget(
                    sourceTarget,
                    child);
                CollectPropertyTreeGameObjectNodeDiscoverMatches(
                    childTarget,
                    child,
                    segment,
                    semanticPath,
                    childDepth,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    shallowPathMatches,
                    pathAncestorMatched,
                    matches,
                    traversal);
                if (traversal != null && traversal.truncated)
                    return;
            }
        }

        private static void CollectPropertyTreeGameObjectNodeDiscoverMatches(
            PropertyTreeTarget gameObjectTarget,
            GameObject go,
            string segment,
            string semanticPath,
            int depth,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (go == null || depth > maxDepth || matches.Count >= maxResults)
                return;

            Type objectType = typeof(GameObject);
            bool nameMatches = string.IsNullOrWhiteSpace(fieldName)
                || string.Equals(go.name ?? "", fieldName, StringComparison.OrdinalIgnoreCase)
                || string.Equals(segment, fieldName, StringComparison.OrdinalIgnoreCase);
            bool typeMatches = string.IsNullOrWhiteSpace(fieldType)
                || TypeMatches(objectType, fieldType);
            PropertyTreeSearchMatchEvidence evidence = PropertyTreeObjectSearchEvidence(
                query,
                semanticPath,
                segment,
                go.name ?? "",
                objectType,
                searchFields);
            bool ownPathMatched = evidence.path;
            if (shallowPathMatches && pathAncestorMatched)
                evidence.path = false;
            bool nodeMatches = nameMatches
                && typeMatches
                && (string.IsNullOrWhiteSpace(query) || evidence.Any());
            if (nodeMatches)
            {
                string summary = BuildComponentSuffix(go) + BuildGoAnnotations(go);
                matches.Add(new PropertyTreeDiscoverMatch
                {
                    semanticPath = semanticPath,
                    propertyPath = "",
                    displayName = segment,
                    name = segment,
                    type = "GameObject",
                    valueType = "Object",
                    fieldTypeFullName = FieldTypeFullName(objectType),
                    fieldTypeAssembly = FieldTypeAssembly(objectType),
                    displayValue = summary.Trim(),
                    editable = false,
                    hasChildren = true,
                    isArray = false,
                    isManagedReference = false,
                    managedReferenceId = 0,
                    referenceTarget = ToSerializedPropertyBindingTarget(gameObjectTarget),
                    depth = depth,
                    matchedPath = evidence.path,
                    matchedFieldName = evidence.fieldName,
                    matchedFieldValue = evidence.fieldValue,
                    matchedType = evidence.type
                });
            }
            if (matches.Count >= maxResults)
                return;

            bool descendantPathMatched = pathAncestorMatched || ownPathMatched;
            if (!(shallowPathMatches
                && descendantPathMatched
                && PropertyTreeSearchFieldsOnlyPath(searchFields)))
            {
                CollectPropertyTreeGameObjectObjectRootsDiscoverMatches(
                    gameObjectTarget,
                    go,
                    semanticPath,
                    depth + 1,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    shallowPathMatches,
                    descendantPathMatched,
                    matches,
                    traversal);
            }
            if (matches.Count >= maxResults || depth >= maxDepth)
                return;

            CollectPropertyTreeGameObjectChildrenDiscoverMatches(
                gameObjectTarget,
                go,
                semanticPath,
                depth + 1,
                query,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                shallowPathMatches,
                descendantPathMatched,
                matches,
                traversal);
        }

        private static void CollectPropertyTreeGameObjectObjectRootsDiscoverMatches(
            PropertyTreeTarget gameObjectTarget,
            GameObject go,
            string semanticRoot,
            int directoryDepth,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (go == null
                || directoryDepth > maxDepth
                || matches.Count >= maxResults
                || (traversal != null && traversal.truncated))
            {
                return;
            }

            var objects = new List<UnityEngine.Object>();
            var targets = new List<PropertyTreeTarget>();
            objects.Add(go);
            targets.Add(PropertyTreeTargetWithLocalFileIds(
                PropertyTreeGameObjectTarget(gameObjectTarget),
                go));

            var componentIndexes = new Dictionary<string, int>(StringComparer.Ordinal);
            Component[] components = go.GetComponents<Component>();
            for (int i = 0; i < components.Length; i++)
            {
                Component component = components[i];
                if (component == null)
                    continue;
                string componentType = ComponentBindingTypeName(component);
                int componentIndex = 0;
                componentIndexes.TryGetValue(componentType, out componentIndex);
                componentIndexes[componentType] = componentIndex + 1;
                objects.Add(component);
                targets.Add(PropertyTreeTargetWithLocalFileIds(
                    PropertyTreeComponentTarget(
                        gameObjectTarget,
                        componentType,
                        componentIndex),
                    component));
            }

            var totals = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < objects.Count; i++)
            {
                string name = PropertyTreeObjectDisplayName(objects[i]);
                int count;
                totals.TryGetValue(name, out count);
                totals[name] = count + 1;
            }
            var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);

            for (int i = 0; i < objects.Count && matches.Count < maxResults; i++)
            {
                UnityEngine.Object obj = objects[i];
                PropertyTreeTarget target = targets[i];
                string segment = PropertyTreeUniqueHierarchySegment(
                    PropertyTreeObjectDisplayName(obj),
                    totals,
                    ordinals);
                string semanticPath = PropertyTreeAppendSemanticSegment(
                    semanticRoot,
                    segment);
                Type objectType = obj.GetType();
                bool nameMatches = string.IsNullOrWhiteSpace(fieldName)
                    || string.Equals(segment, fieldName, StringComparison.OrdinalIgnoreCase)
                    || string.Equals(PropertyTreeObjectDisplayName(obj), fieldName, StringComparison.OrdinalIgnoreCase);
                bool typeMatches = string.IsNullOrWhiteSpace(fieldType)
                    || TypeMatches(objectType, fieldType);
                PropertyTreeSearchMatchEvidence evidence = PropertyTreeObjectSearchEvidence(
                    query,
                    semanticPath,
                    segment,
                    PropertyTreeObjectDisplayName(obj),
                    objectType,
                    searchFields);
                bool ownPathMatched = evidence.path;
                if (shallowPathMatches && pathAncestorMatched)
                    evidence.path = false;
                bool directoryMatches = nameMatches
                    && typeMatches
                    && (string.IsNullOrWhiteSpace(query) || evidence.Any());
                if (directoryMatches)
                {
                    matches.Add(new PropertyTreeDiscoverMatch
                    {
                        semanticPath = semanticPath,
                        propertyPath = "",
                        displayName = segment,
                        name = segment,
                        type = objectType.Name,
                        valueType = "Object",
                        fieldTypeFullName = FieldTypeFullName(objectType),
                        fieldTypeAssembly = FieldTypeAssembly(objectType),
                        displayValue = "",
                        editable = false,
                        hasChildren = true,
                        isArray = false,
                        isManagedReference = false,
                        managedReferenceId = 0,
                        referenceTarget = ToSerializedPropertyBindingTarget(target),
                        depth = directoryDepth,
                        matchedPath = evidence.path,
                        matchedFieldName = evidence.fieldName,
                        matchedFieldValue = evidence.fieldValue,
                        matchedType = evidence.type
                    });
                }
                if (matches.Count >= maxResults)
                    return;

                bool descendantPathMatched = pathAncestorMatched || ownPathMatched;
                if (!(shallowPathMatches
                    && descendantPathMatched
                    && PropertyTreeSearchFieldsOnlyPath(searchFields)))
                {
                    CollectPropertyTreeObjectDiscoverMatches(
                        obj,
                        target,
                        semanticPath,
                        directoryDepth + 1,
                        query,
                        fieldName,
                        fieldType,
                        searchFields,
                        maxDepth,
                        maxResults,
                        shallowPathMatches,
                        descendantPathMatched,
                        matches,
                        traversal);
                }
                if (traversal != null && traversal.truncated)
                    return;
            }
        }

        private static PropertyTreeTarget PropertyTreeHierarchyGameObjectTarget(
            PropertyTreeTarget source,
            GameObject go)
        {
            string assetPath = source != null
                ? ResolvePropertyTreeAssetPath(source)
                : "";
            var target = new PropertyTreeTarget
            {
                kind = "gameobject",
                guid = source != null ? source.guid ?? "" : "",
                path = assetPath,
                scenePath = IsSceneAssetPath(assetPath) ? assetPath : "",
                objectPath = PropertyTreeHierarchyObjectPath(go),
                componentType = "",
                componentIndex = 0,
                targetTypeFullName = "UnityEngine.GameObject",
                targetTypeAssembly = typeof(GameObject).Assembly.GetName().Name,
                targetTypeName = "GameObject",
                propertyPath = ""
            };
            return PropertyTreeTargetWithLocalFileIds(target, go);
        }

        private static string PropertyTreeHierarchySemanticPath(
            PropertyTreeTarget target,
            GameObject go)
        {
            string assetPath = ResolvePropertyTreeAssetPath(target);
            string objectPath = PropertyTreeHierarchyObjectPath(go);
            if (IsPrefabAssetPath(assetPath))
            {
                int separator = objectPath.IndexOf('/');
                objectPath = separator >= 0
                    ? objectPath.Substring(separator + 1)
                    : "";
            }
            return PropertyTreeAssetQualifiedHierarchyPath(assetPath, objectPath);
        }

        private static PropertyTreeSearchMatchEvidence PropertyTreeObjectSearchEvidence(
            string query,
            string semanticPath,
            string name,
            string displayName,
            Type objectType,
            PropertyTreeSearchFieldSet fields)
        {
            var evidence = new PropertyTreeSearchMatchEvidence();
            if (string.IsNullOrWhiteSpace(query))
                return evidence;

            string typeName = objectType != null ? objectType.Name : "";
            string typeFullName = FieldTypeFullName(objectType);
            string typeAssembly = FieldTypeAssembly(objectType);
            if (query.StartsWith("re:", StringComparison.OrdinalIgnoreCase))
            {
                string pattern = query.Substring(3);
                if (string.IsNullOrWhiteSpace(pattern))
                    throw new Exception("Property Tree search regex is empty");
                var regex = new System.Text.RegularExpressions.Regex(
                    pattern,
                    System.Text.RegularExpressions.RegexOptions.IgnoreCase
                    | System.Text.RegularExpressions.RegexOptions.CultureInvariant);
                evidence.path = fields.path && regex.IsMatch(semanticPath ?? "");
                evidence.fieldName = fields.name && (regex.IsMatch(name ?? "")
                    || regex.IsMatch(displayName ?? ""));
                evidence.fieldValue = false;
                evidence.type = fields.type && (regex.IsMatch(typeName)
                    || regex.IsMatch(typeFullName)
                    || regex.IsMatch(typeAssembly));
                return evidence;
            }

            string normalized = query.ToLowerInvariant();
            evidence.path = fields.path && ContainsNormalized(semanticPath, normalized);
            evidence.fieldName = fields.name && (ContainsNormalized(name, normalized)
                || ContainsNormalized(displayName, normalized));
            evidence.fieldValue = false;
            evidence.type = fields.type && (ContainsNormalized(typeName, normalized)
                || ContainsNormalized(typeFullName, normalized)
                || ContainsNormalized(typeAssembly, normalized));
            return evidence;
        }

        private static bool TryDiscoverPropertyTreeAssetWithSubassets(
            PropertyTreeDiscoverRequest request,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            PropertyTreeDiscoverTraversalState traversal,
            out PropertyTreeDiscoverResponse response)
        {
            response = null;
            PropertyTreeTarget target = request != null ? request.target : null;
            if (target == null
                || !string.Equals((target.kind ?? "").Trim(), "asset", StringComparison.OrdinalIgnoreCase)
                || !string.IsNullOrWhiteSpace(target.propertyPath))
            {
                return false;
            }

            string assetPath = ResolvePropertyTreeAssetPath(target);
            if (string.IsNullOrWhiteSpace(assetPath)
                || IsSceneAssetPath(assetPath)
                || IsPrefabAssetPath(assetPath))
            {
                return false;
            }

            UnityEngine.Object selected = ResolveAssetTarget(target);
            UnityEngine.Object mainAtPath = AssetDatabase.LoadMainAssetAtPath(assetPath);
            if (selected == null || mainAtPath == null)
                return false;

            PropertyTreeTarget mainTarget = PropertyTreeTargetWithLocalFileIds(target, mainAtPath);
            PropertyTreeTarget resolvedTarget = PropertyTreeTargetWithLocalFileIds(target, selected);
            SerializedPropertySnapshot mainRoot = SnapshotPropertyTreeObject(
                mainTarget,
                mainAtPath,
                0,
                0,
                false);
            List<PropertyTreeSubassetRecord> subassets = BuildPropertyTreeSubassetRecords(
                mainTarget,
                mainAtPath,
                mainRoot != null ? mainRoot.children : null);
            if (subassets.Count == 0)
                return false;

            string semanticRoot = assetPath;
            List<PropertyTreeSubassetRecord> scopedSubassets = subassets;
            if (selected != mainAtPath)
            {
                PropertyTreeSubassetRecord selectedRecord;
                if (!TryFindPropertyTreeSubassetRecord(
                    subassets,
                    assetPath,
                    selected,
                    out selectedRecord,
                    out semanticRoot))
                {
                    return false;
                }
                scopedSubassets = selectedRecord.children;
            }

            var matches = new List<PropertyTreeDiscoverMatch>();
            CollectPropertyTreeObjectDiscoverMatches(
                selected,
                semanticRoot,
                query,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                request.shallowPathMatches,
                false,
                matches,
                traversal);

            CollectPropertyTreeSubassetDiscoverMatches(
                scopedSubassets,
                semanticRoot,
                query,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                request.shallowPathMatches,
                false,
                0,
                matches,
                traversal);

            response = BuildPropertyTreeDiscoverResponse(
                request,
                resolvedTarget,
                matches,
                traversal);
            return true;
        }

        private static void CollectPropertyTreeSubassetDiscoverMatches(
            List<PropertyTreeSubassetRecord> records,
            string parentSemanticPath,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            int directoryDepth,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (records == null)
                return;
            for (int i = 0; i < records.Count && matches.Count < maxResults; i++)
            {
                PropertyTreeSubassetRecord record = records[i];
                PropertyTreeSubassetEntry entry = record != null ? record.entry : null;
                if (entry == null || string.IsNullOrWhiteSpace(entry.segment))
                    continue;

                string semanticPath = PropertyTreeAppendSemanticSegment(
                    parentSemanticPath,
                    entry.segment);
                Type objectType = record.obj != null ? record.obj.GetType() : null;
                bool nameMatches = string.IsNullOrWhiteSpace(fieldName)
                    || string.Equals(entry.segment, fieldName, StringComparison.OrdinalIgnoreCase)
                    || string.Equals(entry.displayName, fieldName, StringComparison.OrdinalIgnoreCase);
                bool typeMatches = string.IsNullOrWhiteSpace(fieldType)
                    || TypeMatches(objectType, fieldType);
                PropertyTreeSearchMatchEvidence evidence = PropertyTreeSubassetSearchEvidence(
                    query,
                    semanticPath,
                    entry,
                    searchFields);
                bool ownPathMatched = evidence.path;
                if (shallowPathMatches && pathAncestorMatched)
                    evidence.path = false;
                bool subassetMatches = nameMatches
                    && typeMatches
                    && (string.IsNullOrWhiteSpace(query) || evidence.Any());
                if (subassetMatches)
                {
                    matches.Add(new PropertyTreeDiscoverMatch
                    {
                        semanticPath = semanticPath,
                        propertyPath = "",
                        displayName = entry.segment,
                        name = entry.segment,
                        type = entry.type ?? "Object",
                        valueType = "Object",
                        fieldTypeFullName = entry.typeFullName ?? "",
                        fieldTypeAssembly = objectType != null
                            ? FieldTypeAssembly(objectType)
                            : "",
                        displayValue = "",
                        editable = false,
                        hasChildren = true,
                        isArray = false,
                        isManagedReference = false,
                        managedReferenceId = 0,
                        referenceTarget = entry.target,
                        depth = directoryDepth,
                        matchedPath = evidence.path,
                        matchedFieldName = evidence.fieldName,
                        matchedFieldValue = evidence.fieldValue,
                        matchedType = evidence.type
                    });
                }
                if (matches.Count >= maxResults)
                    return;

                bool descendantPathMatched = pathAncestorMatched || ownPathMatched;
                CollectPropertyTreeObjectDiscoverMatches(
                    record.obj,
                    semanticPath,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    shallowPathMatches,
                    descendantPathMatched,
                    matches,
                    traversal);
                CollectPropertyTreeSubassetDiscoverMatches(
                    record.children,
                    semanticPath,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    shallowPathMatches,
                    descendantPathMatched,
                    directoryDepth + 1,
                    matches,
                    traversal);
            }
        }

        private static bool TryFindPropertyTreeSubassetRecord(
            List<PropertyTreeSubassetRecord> records,
            string parentSemanticPath,
            UnityEngine.Object target,
            out PropertyTreeSubassetRecord match,
            out string semanticPath)
        {
            match = null;
            semanticPath = "";
            if (records == null || target == null)
                return false;
            for (int i = 0; i < records.Count; i++)
            {
                PropertyTreeSubassetRecord record = records[i];
                if (record == null || record.entry == null)
                    continue;
                string candidatePath = PropertyTreeAppendSemanticSegment(
                    parentSemanticPath,
                    record.entry.segment);
                if (record.obj == target)
                {
                    match = record;
                    semanticPath = candidatePath;
                    return true;
                }
                if (TryFindPropertyTreeSubassetRecord(
                    record.children,
                    candidatePath,
                    target,
                    out match,
                    out semanticPath))
                {
                    return true;
                }
            }
            return false;
        }

        private static void CollectPropertyTreeObjectDiscoverMatches(
            UnityEngine.Object obj,
            string semanticRoot,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            CollectPropertyTreeObjectDiscoverMatches(
                obj,
                PropertyTreeTargetForObject(obj),
                semanticRoot,
                0,
                query,
                fieldName,
                fieldType,
                searchFields,
                maxDepth,
                maxResults,
                shallowPathMatches,
                pathAncestorMatched,
                matches,
                traversal);
        }

        private static void CollectPropertyTreeObjectDiscoverMatches(
            UnityEngine.Object obj,
            PropertyTreeTarget target,
            string semanticRoot,
            int baseDepth,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (obj == null
                || baseDepth > maxDepth
                || matches.Count >= maxResults
                || (traversal != null && traversal.truncated))
            {
                return;
            }

            if (traversal != null && !traversal.TryBeginSerializedObject())
                return;

            target = PropertyTreeTargetWithLocalFileIds(target, obj);
            SerializedPropertySnapshot semanticSnapshot = SnapshotPropertyTreeObject(
                target,
                obj,
                0,
                0,
                false);
            var semanticRoots = new Dictionary<string, SerializedPropertySnapshot>(StringComparer.Ordinal);
            SerializedPropertySnapshot[] semanticChildren = semanticSnapshot != null
                ? semanticSnapshot.children ?? new SerializedPropertySnapshot[0]
                : new SerializedPropertySnapshot[0];
            for (int i = 0; i < semanticChildren.Length; i++)
            {
                SerializedPropertySnapshot child = semanticChildren[i];
                if (child == null || PropertyTreeDiscoverRootMetadataHidden(child.name))
                    continue;
                string propertyPath = child.propertyPath ?? "";
                if (!string.IsNullOrWhiteSpace(propertyPath))
                    semanticRoots[propertyPath] = child;
            }

            var serialized = new SerializedObject(obj);
            serialized.Update();

            // Semantic headers such as GameObject.Static can be backed by a
            // synthetic property path. They belong to the same addressable
            // tree as read(), even though SerializedObject has no raw cursor
            // entry for them.
            for (int i = 0; i < semanticChildren.Length && matches.Count < maxResults; i++)
            {
                SerializedPropertySnapshot child = semanticChildren[i];
                if (child == null || PropertyTreeDiscoverRootMetadataHidden(child.name))
                    continue;
                string propertyPath = child.propertyPath ?? "";
                if (string.IsNullOrWhiteSpace(propertyPath)
                    || serialized.FindProperty(propertyPath) != null)
                {
                    continue;
                }
                if (traversal != null && !traversal.TryVisitSerializedProperty())
                    return;
                string semanticPath = PropertyTreeAppendSemanticSegment(
                    semanticRoot,
                    child.name ?? child.displayName ?? propertyPath);
                CollectPropertyTreeSnapshotDiscoverMatch(
                    child,
                    semanticPath,
                    baseDepth,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxResults,
                    shallowPathMatches,
                    pathAncestorMatched,
                    matches);
            }

            SerializedProperty cursor = serialized.GetIterator();
            bool enterChildren = true;
            string shallowPathRoot = "";
            while (cursor.NextVisible(enterChildren) && matches.Count < maxResults)
            {
                if (traversal != null && !traversal.TryVisitSerializedProperty())
                    break;
                enterChildren = !IsSerializedPropertyCompactValue(cursor.propertyType);
                if (!string.IsNullOrWhiteSpace(shallowPathRoot)
                    && !PropertyTreePropertyPathIsWithinScope(cursor.propertyPath, shallowPathRoot))
                {
                    shallowPathRoot = "";
                }
                string rootPropertyPath = PropertyTreeSerializedRootPropertyPath(cursor.propertyPath);
                SerializedPropertySnapshot rootSemantic;
                if (!semanticRoots.TryGetValue(rootPropertyPath, out rootSemantic))
                {
                    if (SerializedPropertyDepth(cursor.propertyPath) == 0)
                        enterChildren = false;
                    continue;
                }

                int depth = baseDepth + SerializedPropertyDepth(cursor.propertyPath);
                if (depth > maxDepth)
                {
                    enterChildren = false;
                    continue;
                }

                Type resolvedType = ResolveSerializedPropertyFieldType(cursor);
                string semanticPath = PropertyTreeSerializedSemanticPath(
                    semanticRoot,
                    cursor.propertyPath,
                    rootPropertyPath,
                    rootSemantic.name ?? rootSemantic.displayName ?? rootPropertyPath);
                bool directRootProperty = string.Equals(
                    cursor.propertyPath,
                    rootPropertyPath,
                    StringComparison.Ordinal);
                string semanticName = directRootProperty
                    ? rootSemantic.name ?? cursor.name ?? ""
                    : cursor.name ?? "";
                string semanticDisplayName = directRootProperty
                    ? rootSemantic.displayName ?? semanticName
                    : cursor.displayName ?? semanticName;
                string semanticDisplayValue = directRootProperty
                    ? rootSemantic.displayValue ?? ""
                    : SerializedPropertyDisplayValue(cursor);
                PropertyTreeSearchMatchEvidence evidence = PropertyTreeDiscoveryQueryEvidence(
                    cursor,
                    resolvedType,
                    query,
                    searchFields,
                    semanticPath,
                    semanticName,
                    semanticDisplayName,
                    semanticDisplayValue);
                if (!MatchesPropertyTreeDiscoveryName(cursor, fieldName, semanticName, semanticDisplayName))
                    continue;
                if (!string.IsNullOrWhiteSpace(query) && !evidence.Any())
                    continue;
                if (!string.IsNullOrEmpty(fieldType) && !TypeMatches(resolvedType, fieldType))
                    continue;

                if (shallowPathMatches && evidence.path)
                {
                    if (pathAncestorMatched || !string.IsNullOrWhiteSpace(shallowPathRoot))
                        evidence.path = false;
                    else
                        shallowPathRoot = cursor.propertyPath;
                }
                if (!string.IsNullOrWhiteSpace(query) && !evidence.Any())
                    continue;

                matches.Add(BuildPropertyTreeDiscoverMatch(
                    cursor,
                    resolvedType,
                    depth,
                    semanticPath,
                    evidence,
                    semanticName,
                    semanticDisplayName,
                    semanticDisplayValue));
                if (shallowPathMatches
                    && evidence.path
                    && PropertyTreeSearchFieldsOnlyPath(searchFields))
                {
                    enterChildren = false;
                }
            }
        }

        private static void CollectPropertyTreeSnapshotDiscoverMatch(
            SerializedPropertySnapshot snapshot,
            string semanticPath,
            int depth,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxResults,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches)
        {
            if (snapshot == null || matches.Count >= maxResults)
                return;

            string name = snapshot.name ?? "";
            string displayName = snapshot.displayName ?? name;
            bool nameMatches = string.IsNullOrWhiteSpace(fieldName)
                || string.Equals(name, fieldName, StringComparison.OrdinalIgnoreCase)
                || string.Equals(displayName, fieldName, StringComparison.OrdinalIgnoreCase);
            bool typeMatches = string.IsNullOrWhiteSpace(fieldType)
                || ContainsNormalized(snapshot.fieldTypeFullName, fieldType.ToLowerInvariant())
                || ContainsNormalized(snapshot.type, fieldType.ToLowerInvariant());
            PropertyTreeSearchMatchEvidence evidence = PropertyTreeSnapshotSearchEvidence(
                snapshot,
                semanticPath,
                query,
                searchFields);
            if (shallowPathMatches && pathAncestorMatched)
                evidence.path = false;
            if (!nameMatches
                || !typeMatches
                || (!string.IsNullOrWhiteSpace(query) && !evidence.Any()))
            {
                return;
            }

            matches.Add(new PropertyTreeDiscoverMatch
            {
                semanticPath = semanticPath,
                propertyPath = snapshot.propertyPath ?? "",
                displayName = displayName,
                name = name,
                type = snapshot.type ?? "Generic",
                valueType = snapshot.valueType ?? snapshot.type ?? "Generic",
                fieldTypeFullName = snapshot.fieldTypeFullName ?? "",
                fieldTypeAssembly = snapshot.fieldTypeAssembly ?? "",
                displayValue = snapshot.displayValue ?? "",
                editable = snapshot.editable,
                hasChildren = snapshot.hasChildren,
                isArray = snapshot.isArray,
                isManagedReference = snapshot.isManagedReference,
                managedReferenceId = snapshot.managedReferenceId,
                referenceTarget = snapshot.referenceTarget,
                depth = depth,
                matchedPath = evidence.path,
                matchedFieldName = evidence.fieldName,
                matchedFieldValue = evidence.fieldValue,
                matchedType = evidence.type
            });
        }

        private static PropertyTreeSearchMatchEvidence PropertyTreeSnapshotSearchEvidence(
            SerializedPropertySnapshot snapshot,
            string semanticPath,
            string query,
            PropertyTreeSearchFieldSet fields)
        {
            var evidence = new PropertyTreeSearchMatchEvidence();
            if (snapshot == null || string.IsNullOrWhiteSpace(query))
                return evidence;

            string name = snapshot.name ?? "";
            string displayName = snapshot.displayName ?? "";
            string displayValue = snapshot.displayValue ?? "";
            string propertyType = snapshot.type ?? "";
            string typeFullName = snapshot.fieldTypeFullName ?? "";
            string typeAssembly = snapshot.fieldTypeAssembly ?? "";
            if (query.StartsWith("re:", StringComparison.OrdinalIgnoreCase))
            {
                string pattern = query.Substring(3);
                if (string.IsNullOrWhiteSpace(pattern))
                    throw new Exception("Property Tree search regex is empty");
                var regex = new System.Text.RegularExpressions.Regex(
                    pattern,
                    System.Text.RegularExpressions.RegexOptions.IgnoreCase
                    | System.Text.RegularExpressions.RegexOptions.CultureInvariant);
                evidence.path = fields.path && regex.IsMatch(semanticPath ?? "");
                evidence.fieldName = fields.name && (regex.IsMatch(name)
                    || regex.IsMatch(displayName));
                evidence.fieldValue = fields.value && regex.IsMatch(displayValue);
                evidence.type = fields.type && (regex.IsMatch(propertyType)
                    || regex.IsMatch(typeFullName)
                    || regex.IsMatch(typeAssembly));
                return evidence;
            }

            string normalized = query.ToLowerInvariant();
            evidence.path = fields.path && ContainsNormalized(semanticPath, normalized);
            evidence.fieldName = fields.name && (ContainsNormalized(name, normalized)
                || ContainsNormalized(displayName, normalized));
            evidence.fieldValue = fields.value && ContainsNormalized(displayValue, normalized);
            evidence.type = fields.type && (ContainsNormalized(propertyType, normalized)
                || ContainsNormalized(typeFullName, normalized)
                || ContainsNormalized(typeAssembly, normalized));
            return evidence;
        }

        private static bool PropertyTreeDiscoverRootMetadataHidden(string name)
        {
            switch (name ?? "")
            {
                case "serializedVersion":
                case "m_ObjectHideFlags":
                case "m_CorrespondingSourceObject":
                case "m_PrefabInstance":
                case "m_PrefabAsset":
                case "m_GameObject":
                case "m_Component":
                case "m_Children":
                case "m_Father":
                case "m_EditorHideFlags":
                case "m_EditorClassIdentifier":
                case "references":
                    return true;
                default:
                    return false;
            }
        }

        private static string PropertyTreeSerializedRootPropertyPath(string propertyPath)
        {
            string value = propertyPath ?? "";
            int dot = value.IndexOf('.');
            return dot >= 0 ? value.Substring(0, dot) : value;
        }

        private static PropertyTreeSearchMatchEvidence PropertyTreeSubassetSearchEvidence(
            string query,
            string semanticPath,
            PropertyTreeSubassetEntry entry,
            PropertyTreeSearchFieldSet fields)
        {
            var evidence = new PropertyTreeSearchMatchEvidence();
            if (string.IsNullOrWhiteSpace(query))
                return evidence;

            string displayName = entry != null ? entry.displayName ?? "" : "";
            string segment = entry != null ? entry.segment ?? "" : "";
            string type = entry != null
                ? (!string.IsNullOrWhiteSpace(entry.typeFullName)
                    ? entry.typeFullName
                    : entry.type ?? "")
                : "";
            if (query.StartsWith("re:", StringComparison.OrdinalIgnoreCase))
            {
                string pattern = query.Substring(3);
                if (string.IsNullOrWhiteSpace(pattern))
                    throw new Exception("Property Tree search regex is empty");
                var regex = new System.Text.RegularExpressions.Regex(
                    pattern,
                    System.Text.RegularExpressions.RegexOptions.IgnoreCase
                    | System.Text.RegularExpressions.RegexOptions.CultureInvariant);
                evidence.path = fields.path && regex.IsMatch(semanticPath ?? "");
                evidence.fieldName = fields.name && (regex.IsMatch(segment) || regex.IsMatch(displayName));
                evidence.fieldValue = false;
                evidence.type = fields.type && regex.IsMatch(type);
                return evidence;
            }

            string normalized = query.ToLowerInvariant();
            evidence.path = fields.path && ContainsNormalized(semanticPath, normalized);
            evidence.fieldName = fields.name && (ContainsNormalized(segment, normalized)
                || ContainsNormalized(displayName, normalized));
            evidence.fieldValue = false;
            evidence.type = fields.type && ContainsNormalized(type, normalized);
            return evidence;
        }

        private static string PropertyTreeSerializedSemanticPath(
            string semanticRoot,
            string propertyPath,
            string rootPropertyPath = "",
            string rootSemanticName = "")
        {
            string current = (semanticRoot ?? "").Trim().TrimEnd('/');
            string normalized = (propertyPath ?? "").Replace(".Array.data[", "[");
            string[] parts = normalized.Split(new[] { '.' }, StringSplitOptions.RemoveEmptyEntries);
            for (int i = 0; i < parts.Length; i++)
            {
                string part = parts[i];
                int bracket = part.IndexOf('[');
                string name = bracket >= 0 ? part.Substring(0, bracket) : part;
                if (i == 0
                    && !string.IsNullOrWhiteSpace(rootPropertyPath)
                    && string.Equals(name, rootPropertyPath, StringComparison.Ordinal)
                    && !string.IsNullOrWhiteSpace(rootSemanticName))
                {
                    name = rootSemanticName;
                }
                if (!string.IsNullOrWhiteSpace(name) && name != "Array")
                    current = PropertyTreeAppendSemanticSegment(current, name);
                while (bracket >= 0)
                {
                    int close = part.IndexOf(']', bracket + 1);
                    if (close < 0)
                        break;
                    string index = part.Substring(bracket + 1, close - bracket - 1);
                    if (!string.IsNullOrWhiteSpace(index))
                        current = PropertyTreeAppendSemanticSegment(current, index);
                    bracket = part.IndexOf('[', close + 1);
                }
            }
            return current;
        }

        private static string PropertyTreeAppendSemanticSegment(string path, string segment)
        {
            string encoded = PropertyTreeEncodePathSegment(segment);
            return string.IsNullOrWhiteSpace(path)
                ? encoded
                : path.TrimEnd('/') + "/" + encoded;
        }

        private static bool TryDiscoverPropertyTreeScene(
            PropertyTreeDiscoverRequest request,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            int maxDepth,
            int maxResults,
            PropertyTreeDiscoverTraversalState traversal,
            out PropertyTreeDiscoverResponse response)
        {
            response = null;
            PropertyTreeTarget target = request != null ? request.target : null;
            if (target == null
                || !string.Equals((target.kind ?? "").Trim(), "asset", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            string scenePath = ResolvePropertyTreeAssetPath(target);
            if (!IsSceneAssetPath(scenePath))
                return false;

            Scene scene = ResolveScene(scenePath);
            target.scenePath = scenePath;
            target.targetTypeFullName = "UnityEngine.SceneManagement.Scene";
            target.targetTypeAssembly = typeof(Scene).Assembly.GetName().Name;
            target.targetTypeName = "Scene";

            var matches = new List<PropertyTreeDiscoverMatch>();
            CollectPropertyTreeSceneDiscoverMatches(
                target,
                scene.GetRootGameObjects(),
                scenePath,
                0,
                maxDepth,
                maxResults,
                query,
                fieldName,
                fieldType,
                searchFields,
                request.shallowPathMatches,
                false,
                matches,
                traversal);
            response = BuildPropertyTreeDiscoverResponse(
                request,
                target,
                matches,
                traversal);
            return true;
        }

        private static void CollectPropertyTreeSceneDiscoverMatches(
            PropertyTreeTarget sceneTarget,
            GameObject[] siblings,
            string parentSemanticPath,
            int depth,
            int maxDepth,
            int maxResults,
            string query,
            string fieldName,
            string fieldType,
            PropertyTreeSearchFieldSet searchFields,
            bool shallowPathMatches,
            bool pathAncestorMatched,
            List<PropertyTreeDiscoverMatch> matches,
            PropertyTreeDiscoverTraversalState traversal)
        {
            if (siblings == null
                || matches.Count >= maxResults
                || depth > maxDepth
                || (traversal != null && traversal.truncated))
            {
                return;
            }

            var totals = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < siblings.Length; i++)
            {
                GameObject sibling = siblings[i];
                if (sibling == null)
                    continue;
                string name = sibling.name ?? "GameObject";
                int count;
                totals.TryGetValue(name, out count);
                totals[name] = count + 1;
            }
            var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);

            for (int i = 0; i < siblings.Length && matches.Count < maxResults; i++)
            {
                GameObject go = siblings[i];
                if (go == null)
                    continue;
                string segment = PropertyTreeUniqueHierarchySegment(
                    go.name ?? "GameObject",
                    totals,
                    ordinals);
                string semanticPath = PropertyTreeAppendSemanticSegment(
                    parentSemanticPath,
                    segment);
                PropertyTreeTarget gameObjectTarget = PropertyTreeHierarchyGameObjectTarget(
                    sceneTarget,
                    go);
                CollectPropertyTreeGameObjectNodeDiscoverMatches(
                    gameObjectTarget,
                    go,
                    segment,
                    semanticPath,
                    depth,
                    query,
                    fieldName,
                    fieldType,
                    searchFields,
                    maxDepth,
                    maxResults,
                    shallowPathMatches,
                    pathAncestorMatched,
                    matches,
                    traversal);
                if (traversal != null && traversal.truncated)
                    return;
            }
        }

        private static PropertyTreeSearchFieldSet BuildPropertyTreeSearchFieldSet(
            string[] matchFields)
        {
            var values = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            if (matchFields != null)
            {
                for (int i = 0; i < matchFields.Length; i++)
                {
                    string[] parts = (matchFields[i] ?? "").Split(new[] { ',', '|' });
                    for (int j = 0; j < parts.Length; j++)
                    {
                        string value = parts[j].Trim();
                        if (!string.IsNullOrWhiteSpace(value))
                            values.Add(value);
                    }
                }
            }
            bool all = values.Count == 0 || values.Contains("all");
            return new PropertyTreeSearchFieldSet
            {
                path = all || values.Contains("path"),
                name = all || values.Contains("name") || values.Contains("field_name"),
                value = all || values.Contains("value") || values.Contains("field_value"),
                type = all || values.Contains("type")
            };
        }

        private static bool PropertyTreePropertyPathIsWithinScope(
            string propertyPath,
            string scopePropertyPath)
        {
            if (string.IsNullOrWhiteSpace(scopePropertyPath))
                return true;
            if (string.Equals(propertyPath, scopePropertyPath, StringComparison.Ordinal))
                return true;
            return (propertyPath ?? "").StartsWith(
                scopePropertyPath + ".",
                StringComparison.Ordinal);
        }

        private static string PropertyTreeAssetQualifiedHierarchyPath(
            string assetPath,
            string objectPath)
        {
            string[] segments = (objectPath ?? "")
                .Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries);
            string suffix = string.Join(
                "/",
                segments.Select(PropertyTreeEncodePathSegment).ToArray());
            string normalizedAsset = (assetPath ?? "").Trim().TrimEnd('/');
            return string.IsNullOrWhiteSpace(suffix)
                ? normalizedAsset
                : normalizedAsset + "/" + suffix;
        }

        private static UnityEngine.Object ResolvePropertyTreeObject(PropertyTreeTarget target)
        {
            string kind = (target.kind ?? "").Trim().ToLowerInvariant();
            switch (kind)
            {
                case "selection":
                    if (Selection.activeObject == null)
                        throw new Exception("Unity selection is empty");
                    return Selection.activeObject;
                case "asset":
                case "scriptableobject":
                case "material":
                    return ResolveAssetTarget(target);
                case "gameobject":
                    return ResolveGameObjectTarget(target);
                case "component":
                    return ResolveComponentTarget(target);
                default:
                    throw new Exception("Unsupported Property tree target kind: " + target.kind);
            }
        }

        private static UnityEngine.Object ResolveAssetTarget(PropertyTreeTarget target)
        {
            string path = ResolvePropertyTreeAssetPath(target);
            UnityEngine.Object obj = null;
            if (!string.IsNullOrWhiteSpace(path) && target.targetFileId != 0)
            {
                UnityEngine.Object[] candidates = AssetDatabase.LoadAllAssetsAtPath(path);
                for (int i = 0; i < candidates.Length; i++)
                {
                    long localFileId;
                    if (candidates[i] != null
                        && TryGetLocalFileId(candidates[i], out localFileId)
                        && localFileId == target.targetFileId)
                    {
                        obj = candidates[i];
                        break;
                    }
                }
            }
            if (obj == null)
            {
                obj = !string.IsNullOrWhiteSpace(path)
                    ? AssetDatabase.LoadMainAssetAtPath(path)
                    : Selection.activeObject;
            }
            if (obj == null)
                throw new Exception("Asset target not found: " + (!string.IsNullOrWhiteSpace(path) ? path : "<selection>"));
            return obj;
        }

        private static string ResolvePropertyTreeAssetPath(PropertyTreeTarget target)
        {
            string path = (target.path ?? "").Trim().Replace('\\', '/');
            if (string.IsNullOrWhiteSpace(path) && !string.IsNullOrWhiteSpace(target.guid))
            {
                string guid = (target.guid ?? "").Trim();
                path = AssetDatabase.GUIDToAssetPath(guid);
                if (string.IsNullOrWhiteSpace(path))
                    throw new Exception("Asset GUID target not found: " + guid);
            }

            if (!string.IsNullOrWhiteSpace(path))
                target.path = path;
            return path;
        }

        private static GameObject ResolveGameObjectTarget(PropertyTreeTarget target)
        {
            string assetPath = ResolvePropertyTreeAssetPath(target);
            if (IsPrefabAssetPath(assetPath))
                return ResolvePrefabAssetGameObjectTarget(target);
            if (string.IsNullOrWhiteSpace(target.scenePath) && IsSceneAssetPath(assetPath))
                target.scenePath = assetPath;

            Scene scene = ResolveScene(target.scenePath);
            bool componentTarget = string.Equals((target.kind ?? "").Trim(), "component", StringComparison.OrdinalIgnoreCase);
            long sceneObjectFileId = componentTarget ? target.objectFileId : FirstNonZero(target.objectFileId, target.targetFileId);
            if (sceneObjectFileId != 0)
            {
                GameObject byFileId = ResolveSceneGameObjectByFileId(scene, sceneObjectFileId);
                if (byFileId != null)
                    return byFileId;
                if (string.IsNullOrWhiteSpace(target.objectPath))
                    throw new Exception("Scene GameObject fileID not found: " + sceneObjectFileId.ToString(CultureInfo.InvariantCulture));
            }

            if (string.IsNullOrWhiteSpace(target.objectPath))
            {
                GameObject selected = Selection.activeGameObject;
                if (selected == null)
                    throw new Exception("GameObject target objectPath is required when no GameObject is selected");
                return selected;
            }

            string[] parts = target.objectPath.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length == 0)
                throw new Exception("GameObject target objectPath is empty");

            ObjectPathSegment rootSegment = ParseObjectPathSegment(parts[0]);
            GameObject current = scene.GetRootGameObjects()
                .Where(root => string.Equals(root.name, rootSegment.name, StringComparison.Ordinal))
                .Skip(rootSegment.zeroBasedIndex)
                .FirstOrDefault();
            if (current == null)
                throw new Exception("Root GameObject not found: " + parts[0]);

            for (int i = 1; i < parts.Length; i++)
            {
                ObjectPathSegment segment = ParseObjectPathSegment(parts[i]);
                Transform child = null;
                int matchIndex = 0;
                for (int j = 0; j < current.transform.childCount; j++)
                {
                    Transform candidate = current.transform.GetChild(j);
                    if (string.Equals(candidate.name, segment.name, StringComparison.Ordinal))
                    {
                        if (matchIndex == segment.zeroBasedIndex)
                        {
                            child = candidate;
                            break;
                        }
                        matchIndex++;
                    }
                }
                if (child == null)
                    throw new Exception("GameObject child not found: " + parts[i]);
                current = child.gameObject;
            }

            return current;
        }

        private static bool IsPrefabAssetPath(string path)
        {
            return !string.IsNullOrWhiteSpace(path) &&
                   path.Trim().Replace('\\', '/').EndsWith(".prefab", StringComparison.OrdinalIgnoreCase);
        }

        private static bool IsSceneAssetPath(string path)
        {
            return !string.IsNullOrWhiteSpace(path) &&
                   path.Trim().Replace('\\', '/').EndsWith(".unity", StringComparison.OrdinalIgnoreCase);
        }

        private static GameObject ResolvePrefabAssetGameObjectTarget(PropertyTreeTarget target)
        {
            string path = (target.path ?? "").Trim().Replace('\\', '/');
            GameObject root = AssetDatabase.LoadAssetAtPath<GameObject>(path);
            if (root == null)
                throw new Exception("Prefab asset target not found: " + path);

            string objectPath = (target.objectPath ?? "").Trim();
            if (string.IsNullOrWhiteSpace(objectPath))
                return root;

            string[] parts = objectPath.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length == 0)
                return root;

            int index = 0;
            ObjectPathSegment rootSegment = ParseObjectPathSegment(parts[0]);
            if (string.Equals(root.name, rootSegment.name, StringComparison.Ordinal) && rootSegment.zeroBasedIndex == 0)
                index = 1;

            GameObject current = root;
            for (int i = index; i < parts.Length; i++)
            {
                ObjectPathSegment segment = ParseObjectPathSegment(parts[i]);
                Transform child = null;
                int matchIndex = 0;
                for (int j = 0; j < current.transform.childCount; j++)
                {
                    Transform candidate = current.transform.GetChild(j);
                    if (string.Equals(candidate.name, segment.name, StringComparison.Ordinal))
                    {
                        if (matchIndex == segment.zeroBasedIndex)
                        {
                            child = candidate;
                            break;
                        }
                        matchIndex++;
                    }
                }
                if (child == null)
                    throw new Exception("Prefab GameObject child not found: " + parts[i]);
                current = child.gameObject;
            }

            return current;
        }

        private static Component ResolveComponentTarget(PropertyTreeTarget target)
        {
            string assetPath = ResolvePropertyTreeAssetPath(target);
            if (string.IsNullOrWhiteSpace(target.scenePath) && IsSceneAssetPath(assetPath))
                target.scenePath = assetPath;

            if (target.targetFileId != 0 && !IsPrefabAssetPath(assetPath))
            {
                if (target.objectFileId != 0 || !string.IsNullOrWhiteSpace(target.objectPath))
                {
                    GameObject scopedGo = ResolveGameObjectTarget(target);
                    Component scopedComponent = ResolveGameObjectComponentByFileId(scopedGo, target.targetFileId);
                    if (scopedComponent != null)
                        return scopedComponent;
                }
                else
                {
                    Scene scene = ResolveScene(target.scenePath);
                    Component byFileId = ResolveSceneComponentByFileId(scene, target.targetFileId);
                    if (byFileId != null)
                        return byFileId;
                    throw new Exception("Scene component fileID not found: " + target.targetFileId.ToString(CultureInfo.InvariantCulture));
                }
            }

            GameObject go = ResolveGameObjectTarget(target);
            string typeName = target.componentType;
            if (string.IsNullOrWhiteSpace(typeName))
                throw new Exception("Component target componentType is required");
            if (target.componentIndex < 0)
                throw new Exception("Component target componentIndex cannot be negative");

            Component[] components = go.GetComponents<Component>()
                .Where(candidate =>
                    candidate != null &&
                    TypeMatches(candidate.GetType(), typeName))
                .ToArray();
            Component component = target.componentIndex < components.Length
                ? components[target.componentIndex]
                : null;
            if (component == null)
                throw new Exception("Component not found: " + typeName + "[" + target.componentIndex.ToString(CultureInfo.InvariantCulture) + "]");
            return component;
        }

        private static Scene ResolveScene(string scenePath)
        {
            if (string.IsNullOrWhiteSpace(scenePath))
                return SceneManager.GetActiveScene();

            for (int i = 0; i < SceneManager.sceneCount; i++)
            {
                Scene scene = SceneManager.GetSceneAt(i);
                if (string.Equals(scene.path, scenePath, StringComparison.OrdinalIgnoreCase))
                    return scene;
            }
            throw new Exception("Scene is not loaded: " + scenePath);
        }

        private struct ObjectPathSegment
        {
            public string name;
            public int zeroBasedIndex;
        }

        private static ObjectPathSegment ParseObjectPathSegment(string segment)
        {
            string source = segment ?? "";
            int ordinal = source.LastIndexOf('[');
            if (ordinal > 0 && source.EndsWith("]", StringComparison.Ordinal))
            {
                string indexText = source.Substring(ordinal + 1, source.Length - ordinal - 2);
                int index;
                if (int.TryParse(indexText, NumberStyles.Integer, CultureInfo.InvariantCulture, out index))
                {
                    if (index <= 0)
                        throw new Exception("GameObject path ordinal must be 1 or greater: " + segment);
                    return new ObjectPathSegment
                    {
                        name = source.Substring(0, ordinal),
                        zeroBasedIndex = index - 1
                    };
                }
            }

            return new ObjectPathSegment
            {
                name = source,
                zeroBasedIndex = 0
            };
        }

        private static long FirstNonZero(long first, long second)
        {
            return first != 0 ? first : second;
        }

        private static GameObject ResolveSceneGameObjectByFileId(Scene scene, long fileId)
        {
            foreach (GameObject root in scene.GetRootGameObjects())
            {
                GameObject found = FindSceneGameObjectByFileId(root, fileId);
                if (found != null)
                    return found;
            }
            return null;
        }

        private static GameObject FindSceneGameObjectByFileId(GameObject current, long fileId)
        {
            long currentFileId;
            if (current != null && TryGetLocalFileId(current, out currentFileId) && currentFileId == fileId)
                return current;

            if (current == null)
                return null;

            Transform transform = current.transform;
            for (int i = 0; i < transform.childCount; i++)
            {
                GameObject found = FindSceneGameObjectByFileId(transform.GetChild(i).gameObject, fileId);
                if (found != null)
                    return found;
            }
            return null;
        }

        private static Component ResolveSceneComponentByFileId(Scene scene, long fileId)
        {
            foreach (GameObject root in scene.GetRootGameObjects())
            {
                Component found = FindSceneComponentByFileId(root, fileId);
                if (found != null)
                    return found;
            }
            return null;
        }

        private static Component FindSceneComponentByFileId(GameObject current, long fileId)
        {
            if (current == null)
                return null;

            Component componentOnCurrent = ResolveGameObjectComponentByFileId(current, fileId);
            if (componentOnCurrent != null)
                return componentOnCurrent;

            Transform transform = current.transform;
            for (int i = 0; i < transform.childCount; i++)
            {
                Component found = FindSceneComponentByFileId(transform.GetChild(i).gameObject, fileId);
                if (found != null)
                    return found;
            }
            return null;
        }

        private static Component ResolveGameObjectComponentByFileId(GameObject go, long fileId)
        {
            if (go == null)
                return null;

            Component[] components = go.GetComponents<Component>();
            foreach (Component component in components)
            {
                long componentFileId;
                if (component != null && TryGetLocalFileId(component, out componentFileId) && componentFileId == fileId)
                    return component;
            }
            return null;
        }

        private static bool TryGetLocalFileId(UnityEngine.Object obj, out long fileId)
        {
            fileId = 0;
            if (obj == null)
                return false;

            try
            {
                GlobalObjectId globalId = GlobalObjectId.GetGlobalObjectIdSlow(obj);
                fileId = unchecked((long)globalId.targetObjectId);
                if (fileId != 0)
                    return true;
            }
            catch
            {
            }

            try
            {
                string guid;
                long localId;
                if (AssetDatabase.TryGetGUIDAndLocalFileIdentifier(obj, out guid, out localId) && localId != 0)
                {
                    fileId = localId;
                    return true;
                }
            }
            catch
            {
            }

            return false;
        }

        private static bool ApplyPropertyTreeSerializedChanges(SerializedObject serialized, UnityEngine.Object obj)
        {
            int undoGroup = Undo.GetCurrentGroup();
            Undo.SetCurrentGroupName("Locus Property Tree");
            bool changed = serialized.ApplyModifiedProperties();
            if (changed)
            {
                RecordPropertyTreePrefabModifications(obj);
                MarkPropertyTreeObjectDirty(obj);
                Undo.CollapseUndoOperations(undoGroup);
            }
            serialized.Update();
            return changed;
        }

        private static bool IsPropertyTreeSyntheticHeaderProperty(
            UnityEngine.Object obj,
            PropertyTreeTarget target)
        {
            string propertyPath = (target != null ? target.propertyPath : "") ?? "";
            propertyPath = propertyPath.Trim();

            if (string.Equals(propertyPath, PropertyTreeGameObjectActivePropertyPath, StringComparison.Ordinal))
                return obj is GameObject;

            if (string.Equals(propertyPath, PropertyTreeGameObjectStaticPropertyPath, StringComparison.Ordinal))
                return obj is GameObject;

            if (string.Equals(propertyPath, PropertyTreeComponentEnabledPropertyPath, StringComparison.Ordinal))
                return obj is Component && HasPropertyTreeComponentEnabledState((Component)obj);

            return false;
        }

        private static string WritePropertyTreeSyntheticHeaderProperty(
            string bindingId,
            PropertyTreeTarget target,
            UnityEngine.Object obj,
            string valueJson)
        {
            bool value = ParseBoolJson(string.IsNullOrWhiteSpace(valueJson) ? "false" : valueJson);
            int undoGroup = Undo.GetCurrentGroup();
            Undo.SetCurrentGroupName("Locus Property Tree");
            Undo.RecordObject(obj, "Locus Property Tree");

            string propertyPath = (target != null ? target.propertyPath : "") ?? "";
            propertyPath = propertyPath.Trim();

            GameObject go = obj as GameObject;
            if (go != null && string.Equals(propertyPath, PropertyTreeGameObjectActivePropertyPath, StringComparison.Ordinal))
            {
                go.SetActive(value);
            }
            else if (go != null && string.Equals(propertyPath, PropertyTreeGameObjectStaticPropertyPath, StringComparison.Ordinal))
            {
                go.isStatic = value;
            }
            else
            {
                Component component = obj as Component;
                if (component == null
                    || !string.Equals(propertyPath, PropertyTreeComponentEnabledPropertyPath, StringComparison.Ordinal)
                    || !TrySetPropertyTreeComponentEnabledState(component, value))
                {
                    throw new Exception("Synthetic Property tree property is not writable: " + propertyPath);
                }
            }

            RecordPropertyTreePrefabModifications(obj);
            MarkPropertyTreeObjectDirty(obj);
            Undo.CollapseUndoOperations(undoGroup);

            SerializedPropertySnapshot snapshot = BuildPropertyTreeSyntheticHeaderPropertySnapshot(
                obj,
                ToSerializedPropertyBindingTarget(target));
            return BuildBindingReadJson(bindingId, target, snapshot, true);
        }

        private static bool HasPropertyTreeComponentEnabledState(Component component)
        {
            bool enabled;
            return TryGetPropertyTreeComponentEnabledState(component, out enabled);
        }

        private static bool TryGetPropertyTreeComponentEnabledState(Component component, out bool enabled)
        {
            enabled = false;
            PropertyInfo property = PropertyTreeComponentEnabledProperty(component);
            if (property == null || !property.CanRead)
                return false;

            try
            {
                enabled = (bool)property.GetValue(component, null);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static bool CanSetPropertyTreeComponentEnabledState(Component component)
        {
            PropertyInfo property = PropertyTreeComponentEnabledProperty(component);
            return property != null && property.CanWrite;
        }

        private static bool TrySetPropertyTreeComponentEnabledState(Component component, bool enabled)
        {
            PropertyInfo property = PropertyTreeComponentEnabledProperty(component);
            if (property == null || !property.CanWrite)
                return false;

            try
            {
                property.SetValue(component, enabled, null);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static PropertyInfo PropertyTreeComponentEnabledProperty(Component component)
        {
            if (component == null)
                return null;

            PropertyInfo property = component.GetType().GetProperty(
                "enabled",
                BindingFlags.Instance | BindingFlags.Public);
            if (property == null
                || property.PropertyType != typeof(bool)
                || property.GetIndexParameters().Length != 0)
                return null;

            return property;
        }

        private static void RecordPropertyTreePrefabModifications(UnityEngine.Object obj)
        {
            if (obj == null)
                return;

            try
            {
                Component component = obj as Component;
                GameObject go = obj as GameObject;
                if (go == null && component != null)
                    go = component.gameObject;
                if (go != null && PrefabUtility.GetNearestPrefabInstanceRoot(go) != null)
                    PrefabUtility.RecordPrefabInstancePropertyModifications(obj);
            }
            catch
            {
            }
        }

        private static void MarkPropertyTreeObjectDirty(UnityEngine.Object obj)
        {
            if (obj == null)
                return;

            EditorUtility.SetDirty(obj);
            Component component = obj as Component;
            GameObject go = obj as GameObject;
            if (IsPropertyTreePrefabAssetObject(obj))
            {
                GameObject prefabRoot = PropertyTreePrefabAssetRoot(obj);
                if (prefabRoot != null)
                    EditorUtility.SetDirty(prefabRoot);
            }
            else if (component != null)
                EditorSceneManager.MarkSceneDirty(component.gameObject.scene);
            else if (go != null)
                EditorSceneManager.MarkSceneDirty(go.scene);
        }

        private static bool IsPropertyTreePrefabAssetObject(UnityEngine.Object obj)
        {
            return PropertyTreePrefabAssetRoot(obj) != null;
        }

        private static GameObject PropertyTreePrefabAssetRoot(UnityEngine.Object obj)
        {
            if (obj == null)
                return null;

            Component component = obj as Component;
            GameObject go = obj as GameObject;
            if (go == null && component != null)
                go = component.gameObject;
            if (go == null)
                return null;

            string path = AssetDatabase.GetAssetPath(obj);
            if (string.IsNullOrWhiteSpace(path))
                path = AssetDatabase.GetAssetPath(go);
            if (!IsPrefabAssetPath(path))
                return null;

            GameObject root = AssetDatabase.LoadAssetAtPath<GameObject>(path);
            return root != null ? root : go;
        }

        private static PropertyTreeDiscoverMatch BuildPropertyTreeDiscoverMatch(
            SerializedProperty prop,
            Type resolvedType,
            int depth,
            string semanticPath = "",
            PropertyTreeSearchMatchEvidence evidence = null,
            string semanticName = "",
            string semanticDisplayName = "",
            string semanticDisplayValue = "")
        {
            evidence = evidence ?? new PropertyTreeSearchMatchEvidence();
            string name = !string.IsNullOrWhiteSpace(semanticName)
                ? semanticName
                : prop.name ?? "";
            string displayName = !string.IsNullOrWhiteSpace(semanticDisplayName)
                ? semanticDisplayName
                : prop.displayName ?? name;
            string displayValue = !string.IsNullOrEmpty(semanticDisplayValue)
                ? semanticDisplayValue
                : SerializedPropertyDisplayValue(prop);
            return new PropertyTreeDiscoverMatch
            {
                semanticPath = semanticPath ?? "",
                propertyPath = prop.propertyPath,
                displayName = displayName,
                name = name,
                type = prop.propertyType.ToString(),
                valueType = prop.propertyType.ToString(),
                fieldTypeFullName = FieldTypeFullName(resolvedType),
                fieldTypeAssembly = FieldTypeAssembly(resolvedType),
                displayValue = displayValue,
                editable = IsSerializedPropertyWritable(prop),
                hasChildren = prop.hasVisibleChildren,
                isArray = prop.isArray && prop.propertyType == SerializedPropertyType.Generic,
                isManagedReference = prop.propertyType == SerializedPropertyType.ManagedReference,
                managedReferenceId = prop.propertyType == SerializedPropertyType.ManagedReference
                    ? SerializedManagedReferenceId(prop)
                    : 0,
                referenceTarget = prop.propertyType == SerializedPropertyType.ObjectReference
                    ? SerializedObjectReferenceTarget(prop)
                    : null,
                depth = depth,
                matchedPath = evidence.path,
                matchedFieldName = evidence.fieldName,
                matchedFieldValue = evidence.fieldValue,
                matchedType = evidence.type
            };
        }

        private static bool MatchesPropertyTreeDiscoveryName(
            SerializedProperty prop,
            string fieldName,
            string semanticName = "",
            string semanticDisplayName = "")
        {
            if (string.IsNullOrWhiteSpace(fieldName))
                return true;

            string expected = fieldName.Trim();
            return string.Equals(prop.name ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(prop.displayName ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(semanticName ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(semanticDisplayName ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(SerializedPropertyLeafName(prop.propertyPath), expected, StringComparison.OrdinalIgnoreCase) ||
                   (prop.propertyPath ?? "").EndsWith("." + expected, StringComparison.OrdinalIgnoreCase);
        }

        private static PropertyTreeSearchMatchEvidence PropertyTreeDiscoveryQueryEvidence(
            SerializedProperty prop,
            Type resolvedType,
            string query,
            PropertyTreeSearchFieldSet fields,
            string semanticPath = "",
            string semanticName = "",
            string semanticDisplayName = "",
            string semanticDisplayValue = "")
        {
            var evidence = new PropertyTreeSearchMatchEvidence();
            if (string.IsNullOrWhiteSpace(query) || prop == null)
                return evidence;

            string propertyPath = prop.propertyPath ?? "";
            string displayName = !string.IsNullOrWhiteSpace(semanticDisplayName)
                ? semanticDisplayName
                : prop.displayName ?? "";
            string name = !string.IsNullOrWhiteSpace(semanticName)
                ? semanticName
                : prop.name ?? "";
            string displayValue = !string.IsNullOrEmpty(semanticDisplayValue)
                ? semanticDisplayValue
                : SerializedPropertyDisplayValue(prop);
            string propertyType = prop.propertyType.ToString();
            string typeFullName = FieldTypeFullName(resolvedType);
            string typeAssembly = FieldTypeAssembly(resolvedType);
            if (query.StartsWith("re:", StringComparison.OrdinalIgnoreCase))
            {
                string pattern = query.Substring(3);
                if (string.IsNullOrWhiteSpace(pattern))
                    throw new Exception("Property Tree search regex is empty");
                var regex = new System.Text.RegularExpressions.Regex(
                    pattern,
                    System.Text.RegularExpressions.RegexOptions.IgnoreCase
                    | System.Text.RegularExpressions.RegexOptions.CultureInvariant);
                evidence.path = fields.path && (regex.IsMatch(propertyPath)
                    || regex.IsMatch(semanticPath ?? ""));
                evidence.fieldName = fields.name && (regex.IsMatch(displayName)
                    || regex.IsMatch(name)
                    || regex.IsMatch(prop.displayName ?? "")
                    || regex.IsMatch(prop.name ?? ""));
                evidence.fieldValue = fields.value && regex.IsMatch(displayValue);
                evidence.type = fields.type && (regex.IsMatch(propertyType)
                    || regex.IsMatch(typeFullName)
                    || regex.IsMatch(typeAssembly));
                return evidence;
            }

            string normalized = query.ToLowerInvariant();
            evidence.path = fields.path && (ContainsNormalized(propertyPath, normalized)
                || ContainsNormalized(semanticPath, normalized));
            evidence.fieldName = fields.name && (ContainsNormalized(displayName, normalized)
                || ContainsNormalized(name, normalized)
                || ContainsNormalized(prop.displayName, normalized)
                || ContainsNormalized(prop.name, normalized));
            evidence.fieldValue = fields.value && ContainsNormalized(displayValue, normalized);
            evidence.type = fields.type && (ContainsNormalized(propertyType, normalized)
                || ContainsNormalized(typeFullName, normalized)
                || ContainsNormalized(typeAssembly, normalized));
            return evidence;
        }

        private static bool PropertyTreeSearchFieldsOnlyPath(PropertyTreeSearchFieldSet fields)
        {
            return fields != null
                && fields.path
                && !fields.name
                && !fields.value
                && !fields.type;
        }

        private static string SerializedPropertyLeafName(string propertyPath)
        {
            if (string.IsNullOrEmpty(propertyPath))
                return "";
            int dot = propertyPath.LastIndexOf('.');
            return dot >= 0 ? propertyPath.Substring(dot + 1) : propertyPath;
        }

        private static int SerializedPropertyDepth(string propertyPath)
        {
            if (string.IsNullOrEmpty(propertyPath))
                return 0;

            string normalized = propertyPath.Replace(".Array.data[", "[");
            int depth = 0;
            for (int i = 0; i < normalized.Length; i++)
            {
                if (normalized[i] == '.')
                    depth++;
                else if (normalized[i] == '[')
                    depth++;
            }
            return depth;
        }

        private static string NormalizeSearchText(string value)
        {
            return (value ?? "").Trim().ToLowerInvariant();
        }

        private static bool ContainsNormalized(string source, string query)
        {
            return !string.IsNullOrEmpty(source) &&
                   source.ToLowerInvariant().IndexOf(query, StringComparison.Ordinal) >= 0;
        }

        private static string BuildBindingReadJson(
            string bindingId,
            PropertyTreeTarget target,
            SerializedProperty prop,
            bool saved,
            bool dynamicSchema = false)
        {
            SerializedPropertySnapshot snapshot = SnapshotSerializedProperty(prop, 4, 64, dynamicSchema);
            UnityEngine.Object obj = prop != null && prop.serializedObject != null
                ? prop.serializedObject.targetObject
                : null;
            if (obj != null)
                target = PropertyTreeTargetWithLocalFileIds(target, obj);
            ApplyPropertyTreeTargetToSnapshotTree(snapshot, ToSerializedPropertyBindingTarget(target));
            return BuildBindingReadJson(bindingId, target, snapshot, saved);
        }

        private static string BuildBindingReadJson(
            string bindingId,
            PropertyTreeTarget target,
            SerializedPropertySnapshot snapshot,
            bool saved,
            SerializedPropertySnapshot[] properties = null)
        {
            string snapshotFields = SerializedPropertySnapshotFieldsToJson(snapshot);
            return "{" +
                   "\"ok\":true," +
                   "\"bindingId\":" + NullableJsonString(bindingId) + "," +
                   "\"message\":\"ok\"," +
                   "\"target\":" + TargetToJson(target) + "," +
                   snapshotFields + "," +
                   (properties != null ? "\"properties\":" + ToJsonValue(properties, 0, SnapshotJsonDepthLimit, true) + "," : "") +
                   "\"saved\":" + (saved ? "true" : "false") +
                   "}";
        }

        private static string BuildBindingErrorJson(string bindingId, PropertyTreeTarget target, string message)
        {
            return "{" +
                   "\"ok\":false," +
                   "\"bindingId\":" + NullableJsonString(bindingId) + "," +
                   "\"message\":\"" + JsonEscape(message) + "\"," +
                   "\"target\":" + TargetToJson(target) + "," +
                   "\"propertyPath\":\"" + JsonEscape(target != null ? target.propertyPath : "") + "\"," +
                   "\"displayName\":\"\"," +
                   "\"name\":\"\"," +
                   "\"type\":\"Error\"," +
                   "\"valueType\":\"Error\"," +
                   "\"fieldTypeFullName\":\"\"," +
                   "\"fieldTypeAssembly\":\"\"," +
                   "\"value\":null," +
                   "\"displayValue\":\"\"," +
                   "\"editable\":false," +
                   "\"hasChildren\":false," +
                   "\"isArray\":false," +
                   "\"arraySize\":-1," +
                   "\"isFlagsEnum\":false," +
                   "\"enumValueIndex\":-1," +
                   "\"enumValueFlag\":0," +
                   "\"enumOptions\":[]," +
                   "\"children\":[]," +
                   "\"isManagedReference\":false," +
                   "\"managedReferenceFullTypename\":\"\"," +
                   "\"managedReferenceFieldTypename\":\"\"," +
                   "\"managedReferenceDisplayName\":\"\"," +
                   "\"managedReferenceTypes\":[]," +
                   "\"saved\":false" +
                   "}";
        }

        private static string SerializedPropertySnapshotFieldsToJson(SerializedPropertySnapshot snapshot)
        {
            string json = SerializedPropertySnapshotToJson(snapshot);
            if (string.IsNullOrWhiteSpace(json) || json.Length < 2)
                return "";
            json = json.Trim();
            if (json[0] == '{' && json[json.Length - 1] == '}')
                return json.Substring(1, json.Length - 2);
            return json;
        }

        private static string TargetToJson(PropertyTreeTarget target)
        {
            if (target == null)
                return "null";
            return "{" +
                   "\"kind\":\"" + JsonEscape(target.kind) + "\"," +
                   "\"guid\":" + NullableJsonString(target.guid) + "," +
                   "\"path\":" + NullableJsonString(target.path) + "," +
                   "\"scenePath\":" + NullableJsonString(target.scenePath) + "," +
                   "\"objectPath\":" + NullableJsonString(target.objectPath) + "," +
                   "\"objectFileId\":" + NullableJsonLong(target.objectFileId) + "," +
                   "\"targetFileId\":" + NullableJsonLong(target.targetFileId) + "," +
                   "\"componentType\":" + NullableJsonString(target.componentType) + "," +
                   "\"componentIndex\":" + target.componentIndex.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"targetTypeFullName\":" + NullableJsonString(target.targetTypeFullName) + "," +
                   "\"targetTypeAssembly\":" + NullableJsonString(target.targetTypeAssembly) + "," +
                   "\"targetTypeName\":" + NullableJsonString(target.targetTypeName) + "," +
                   "\"propertyPath\":" + NullableJsonString(target.propertyPath) +
                   "}";
        }

        private static string NullableJsonLong(long value)
        {
            return value == 0 ? "null" : value.ToString(CultureInfo.InvariantCulture);
        }

        private static string NullableJsonString(string value)
        {
            return string.IsNullOrEmpty(value) ? "null" : "\"" + JsonEscape(value) + "\"";
        }
    }
}
