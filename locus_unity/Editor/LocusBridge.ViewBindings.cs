using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private sealed class ViewBindingTarget
        {
            public string kind;
            public string path;
            public string scenePath;
            public string objectPath;
            public string componentType;
            public int componentIndex;
            public string propertyPath;
        }

        [Serializable]
        private sealed class ViewBindingReadRequest
        {
            public string bindingId;
            public ViewBindingTarget target;
        }

        [Serializable]
        private sealed class ViewBindingWriteRequest
        {
            public string bindingId;
            public ViewBindingTarget target;
            public string valueJson;
        }

        [Serializable]
        private sealed class ViewBindingApplyRequest
        {
            public ViewBindingWriteRequest[] writes;
        }

        [Serializable]
        private sealed class ViewBindingDiscoverRequest
        {
            public string bindingId;
            public ViewBindingTarget target;
            public string query;
            public string fieldName;
            public string fieldType;
            public int maxDepth;
            public int maxResults;
        }

        private sealed class ViewBindingDiscoverMatch
        {
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
            public int depth;
        }

        private sealed class ViewBindingDiscoverResponse
        {
            public bool ok;
            public string bindingId;
            public string message;
            public ViewBindingTarget target;
            public ViewBindingDiscoverMatch[] matches;
        }

        private static async Task<PipeEnvelope> HandleViewBindingRead(string requestId, string message)
        {
            ViewBindingReadRequest request;
            try
            {
                request = JsonUtility.FromJson<ViewBindingReadRequest>(message ?? "{}");
                ValidateViewBindingTarget(request != null ? request.target : null);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunViewBindingOnMainThread(
                requestId,
                "view_binding_read",
                delegate { return ReadViewBinding(request.bindingId, request.target); });
        }

        private static async Task<PipeEnvelope> HandleViewBindingWrite(string requestId, string message)
        {
            ViewBindingWriteRequest request;
            try
            {
                request = JsonUtility.FromJson<ViewBindingWriteRequest>(message ?? "{}");
                ValidateViewBindingTarget(request != null ? request.target : null);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunViewBindingOnMainThread(
                requestId,
                "view_binding_write",
                delegate { return WriteViewBinding(request.bindingId, request.target, request.valueJson); });
        }

        private static async Task<PipeEnvelope> HandleViewBindingApply(string requestId, string message)
        {
            ViewBindingApplyRequest request;
            try
            {
                request = JsonUtility.FromJson<ViewBindingApplyRequest>(message ?? "{}");
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunViewBindingOnMainThread(
                requestId,
                "view_binding_apply",
                delegate { return ApplyViewBindings(request); });
        }

        private static async Task<PipeEnvelope> HandleViewBindingDiscover(string requestId, string message)
        {
            ViewBindingDiscoverRequest request;
            try
            {
                request = JsonUtility.FromJson<ViewBindingDiscoverRequest>(message ?? "{}");
                if (request == null)
                    throw new Exception("View binding discover request is empty");
                ValidateViewBindingObjectTarget(request.target);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunViewBindingOnMainThread(
                requestId,
                "view_binding_discover",
                delegate { return DiscoverViewBindingProperties(request); });
        }

        private static async Task<PipeEnvelope> RunViewBindingOnMainThread(
            string requestId,
            string operation,
            Func<string> action)
        {
            var tcs = new TaskCompletionSource<PipeEnvelope>();
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

        private static void ValidateViewBindingTarget(ViewBindingTarget target)
        {
            ValidateViewBindingObjectTarget(target);
            if (string.IsNullOrWhiteSpace(target.propertyPath))
                throw new Exception("View binding target propertyPath is required");
        }

        private static void ValidateViewBindingObjectTarget(ViewBindingTarget target)
        {
            if (target == null)
                throw new Exception("View binding target is required");
            if (string.IsNullOrWhiteSpace(target.kind))
                throw new Exception("View binding target kind is required");
        }

        private sealed class ResolvedViewBindingWrite
        {
            public int index;
            public string bindingId;
            public ViewBindingTarget target;
            public string valueJson;
            public UnityEngine.Object obj;
        }

        private sealed class AppliedViewBindingWrite
        {
            public ResolvedViewBindingWrite write;
            public SerializedProperty prop;
        }

        private static string ApplyViewBindings(ViewBindingApplyRequest request)
        {
            ViewBindingWriteRequest[] writes = request != null && request.writes != null
                ? request.writes
                : new ViewBindingWriteRequest[0];

            string[] resultItems = new string[writes.Length];
            bool ok = true;
            var objectCache = new Dictionary<string, UnityEngine.Object>(StringComparer.Ordinal);
            var groups = new Dictionary<int, List<ResolvedViewBindingWrite>>();
            var groupObjects = new Dictionary<int, UnityEngine.Object>();

            for (int i = 0; i < writes.Length; i++)
            {
                ViewBindingWriteRequest write = writes[i];
                try
                {
                    if (write == null)
                        throw new Exception("View binding write is required");
                    ValidateViewBindingTarget(write.target);

                    string objectKey = BuildViewBindingObjectKey(write.target);
                    UnityEngine.Object obj;
                    if (!objectCache.TryGetValue(objectKey, out obj))
                    {
                        obj = ResolveViewBindingObject(write.target);
                        objectCache[objectKey] = obj;
                    }

                    int groupKey = obj.GetInstanceID();
                    List<ResolvedViewBindingWrite> group;
                    if (!groups.TryGetValue(groupKey, out group))
                    {
                        group = new List<ResolvedViewBindingWrite>();
                        groups[groupKey] = group;
                        groupObjects[groupKey] = obj;
                    }

                    group.Add(new ResolvedViewBindingWrite
                    {
                        index = i,
                        bindingId = write.bindingId,
                        target = write.target,
                        valueJson = write.valueJson,
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

            foreach (KeyValuePair<int, List<ResolvedViewBindingWrite>> entry in groups)
            {
                UnityEngine.Object obj = groupObjects[entry.Key];
                List<ResolvedViewBindingWrite> group = entry.Value;
                try
                {
                    var serialized = new SerializedObject(obj);
                    serialized.Update();
                    var applied = new List<AppliedViewBindingWrite>(group.Count);

                    for (int i = 0; i < group.Count; i++)
                    {
                        ResolvedViewBindingWrite write = group[i];
                        try
                        {
                            SerializedProperty prop = serialized.FindProperty(write.target.propertyPath);
                            if (prop == null)
                                throw new Exception("SerializedProperty not found: " + write.target.propertyPath);

                            SetSerializedPropertyValue(prop, write.valueJson);
                            applied.Add(new AppliedViewBindingWrite
                            {
                                write = write,
                                prop = prop
                            });
                        }
                        catch (Exception ex)
                        {
                            ok = false;
                            resultItems[write.index] =
                                BuildBindingErrorJson(write.bindingId, write.target, ex.Message);
                        }
                    }

                    if (applied.Count > 0)
                        ApplyViewBindingSerializedChanges(serialized, obj);

                    for (int i = 0; i < applied.Count; i++)
                    {
                        AppliedViewBindingWrite item = applied[i];
                        SerializedProperty freshProp = serialized.FindProperty(item.write.target.propertyPath);
                        resultItems[item.write.index] =
                            BuildBindingReadJson(item.write.bindingId, item.write.target, freshProp != null ? freshProp : item.prop, true);
                    }
                }
                catch (Exception ex)
                {
                    ok = false;
                    for (int i = 0; i < group.Count; i++)
                    {
                        ResolvedViewBindingWrite write = group[i];
                        if (resultItems[write.index] == null)
                            resultItems[write.index] =
                                BuildBindingErrorJson(write.bindingId, write.target, ex.Message);
                    }
                }
            }

            for (int i = 0; i < resultItems.Length; i++)
            {
                if (resultItems[i] == null)
                    resultItems[i] = BuildBindingErrorJson(null, null, "View binding write did not run");
            }

            string json = "{" +
                          "\"ok\":" + (ok ? "true" : "false") + "," +
                          "\"message\":\"" + JsonEscape(ok ? "Applied bindings." : "Some bindings failed.") + "\"," +
                          "\"results\":[" + string.Join(",", resultItems) + "]" +
                          "}";
            return json;
        }

        private static string BuildViewBindingObjectKey(ViewBindingTarget target)
        {
            return (target.kind ?? "").Trim().ToLowerInvariant() + "|" +
                   (target.path ?? "").Trim().Replace('\\', '/') + "|" +
                   (target.scenePath ?? "").Trim().Replace('\\', '/') + "|" +
                   (target.objectPath ?? "").Trim().Replace('\\', '/') + "|" +
                   (target.componentType ?? "").Trim() + "|" +
                   target.componentIndex.ToString(CultureInfo.InvariantCulture);
        }

        private static string ReadViewBinding(string bindingId, ViewBindingTarget target)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            var serialized = new SerializedObject(obj);
            serialized.Update();
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            return BuildBindingReadJson(bindingId, target, prop, false);
        }

        private static string WriteViewBinding(string bindingId, ViewBindingTarget target, string valueJson)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            var serialized = new SerializedObject(obj);
            serialized.Update();
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            SetSerializedPropertyValue(prop, valueJson);
            ApplyViewBindingSerializedChanges(serialized, obj);
            SerializedProperty updated = serialized.FindProperty(target.propertyPath);
            return BuildBindingReadJson(bindingId, target, updated != null ? updated : prop, true);
        }

        private static string DiscoverViewBindingProperties(ViewBindingDiscoverRequest request)
        {
            string query = NormalizeSearchText(request.query);
            string fieldName = (request.fieldName ?? "").Trim();
            string fieldType = (request.fieldType ?? "").Trim();
            if (string.IsNullOrEmpty(query) && string.IsNullOrEmpty(fieldName) && string.IsNullOrEmpty(fieldType))
                throw new Exception("View binding discover requires query, fieldName, or fieldType");

            int maxDepth = request.maxDepth > 0 ? Math.Min(request.maxDepth, 32) : 8;
            int maxResults = request.maxResults > 0 ? Math.Min(request.maxResults, 500) : 100;
            UnityEngine.Object obj = ResolveViewBindingObject(request.target);
            var serialized = new SerializedObject(obj);
            serialized.Update();

            var matches = new List<ViewBindingDiscoverMatch>();
            SerializedProperty cursor = serialized.GetIterator();
            bool enterChildren = true;
            while (cursor.NextVisible(enterChildren))
            {
                int depth = SerializedPropertyDepth(cursor.propertyPath);
                enterChildren = depth < maxDepth;
                if (depth > maxDepth)
                    continue;

                Type resolvedType = ResolveSerializedPropertyFieldType(cursor);
                if (!MatchesViewBindingDiscoveryName(cursor, fieldName))
                    continue;
                if (!MatchesViewBindingDiscoveryQuery(cursor, resolvedType, query))
                    continue;
                if (!string.IsNullOrEmpty(fieldType) && !TypeMatches(resolvedType, fieldType))
                    continue;

                matches.Add(BuildViewBindingDiscoverMatch(cursor, resolvedType, depth));
                if (matches.Count >= maxResults)
                    break;
            }

            return ToJsonValue(new ViewBindingDiscoverResponse
            {
                ok = true,
                bindingId = request.bindingId ?? "",
                message = matches.Count == 0 ? "No matching properties." : "ok",
                target = request.target,
                matches = matches.ToArray()
            }, 0);
        }

        private static UnityEngine.Object ResolveViewBindingObject(ViewBindingTarget target)
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
                    throw new Exception("Unsupported View binding target kind: " + target.kind);
            }
        }

        private static UnityEngine.Object ResolveAssetTarget(ViewBindingTarget target)
        {
            string path = target.path;
            UnityEngine.Object obj = !string.IsNullOrWhiteSpace(path)
                ? AssetDatabase.LoadMainAssetAtPath(path)
                : Selection.activeObject;
            if (obj == null)
                throw new Exception("Asset target not found: " + (path ?? "<selection>"));
            return obj;
        }

        private static GameObject ResolveGameObjectTarget(ViewBindingTarget target)
        {
            if (string.IsNullOrWhiteSpace(target.objectPath))
            {
                GameObject selected = Selection.activeGameObject;
                if (selected == null)
                    throw new Exception("GameObject target objectPath is required when no GameObject is selected");
                return selected;
            }

            Scene scene = ResolveScene(target.scenePath);
            string[] parts = target.objectPath.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length == 0)
                throw new Exception("GameObject target objectPath is empty");

            ObjectPathSegment rootSegment = ParseObjectPathSegment(parts[0]);
            GameObject current = scene.GetRootGameObjects()
                .Where(root => string.Equals(root.name, rootSegment.name, StringComparison.Ordinal))
                .Skip(rootSegment.index)
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
                        if (matchIndex == segment.index)
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

        private static Component ResolveComponentTarget(ViewBindingTarget target)
        {
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
            public int index;
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
                    if (index < 0)
                        throw new Exception("GameObject path index cannot be negative: " + segment);
                    return new ObjectPathSegment
                    {
                        name = source.Substring(0, ordinal),
                        index = index
                    };
                }
            }

            return new ObjectPathSegment
            {
                name = source,
                index = 0
            };
        }

        private static bool ApplyViewBindingSerializedChanges(SerializedObject serialized, UnityEngine.Object obj)
        {
            int undoGroup = Undo.GetCurrentGroup();
            Undo.SetCurrentGroupName("Locus View Binding");
            bool changed = serialized.ApplyModifiedProperties();
            if (changed)
            {
                RecordViewBindingPrefabModifications(obj);
                MarkViewBindingObjectDirty(obj);
                Undo.CollapseUndoOperations(undoGroup);
            }
            serialized.Update();
            return changed;
        }

        private static void RecordViewBindingPrefabModifications(UnityEngine.Object obj)
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

        private static void MarkViewBindingObjectDirty(UnityEngine.Object obj)
        {
            if (obj == null)
                return;

            EditorUtility.SetDirty(obj);
            Component component = obj as Component;
            GameObject go = obj as GameObject;
            if (component != null)
                EditorSceneManager.MarkSceneDirty(component.gameObject.scene);
            else if (go != null)
                EditorSceneManager.MarkSceneDirty(go.scene);
            else
                AssetDatabase.SaveAssetIfDirty(obj);
        }

        private static ViewBindingDiscoverMatch BuildViewBindingDiscoverMatch(
            SerializedProperty prop,
            Type resolvedType,
            int depth)
        {
            return new ViewBindingDiscoverMatch
            {
                propertyPath = prop.propertyPath,
                displayName = prop.displayName ?? "",
                name = prop.name ?? "",
                type = prop.propertyType.ToString(),
                valueType = prop.propertyType.ToString(),
                fieldTypeFullName = FieldTypeFullName(resolvedType),
                fieldTypeAssembly = FieldTypeAssembly(resolvedType),
                displayValue = SerializedPropertyDisplayValue(prop),
                editable = IsSerializedPropertyWritable(prop),
                hasChildren = prop.hasVisibleChildren,
                isArray = prop.isArray && prop.propertyType == SerializedPropertyType.Generic,
                isManagedReference = prop.propertyType == SerializedPropertyType.ManagedReference,
                depth = depth
            };
        }

        private static bool MatchesViewBindingDiscoveryName(SerializedProperty prop, string fieldName)
        {
            if (string.IsNullOrWhiteSpace(fieldName))
                return true;

            string expected = fieldName.Trim();
            return string.Equals(prop.name ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(prop.displayName ?? "", expected, StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(SerializedPropertyLeafName(prop.propertyPath), expected, StringComparison.OrdinalIgnoreCase) ||
                   (prop.propertyPath ?? "").EndsWith("." + expected, StringComparison.OrdinalIgnoreCase);
        }

        private static bool MatchesViewBindingDiscoveryQuery(SerializedProperty prop, Type resolvedType, string query)
        {
            if (string.IsNullOrEmpty(query))
                return true;

            return ContainsNormalized(prop.propertyPath, query) ||
                   ContainsNormalized(prop.displayName, query) ||
                   ContainsNormalized(prop.name, query) ||
                   ContainsNormalized(prop.propertyType.ToString(), query) ||
                   ContainsNormalized(FieldTypeFullName(resolvedType), query) ||
                   ContainsNormalized(FieldTypeAssembly(resolvedType), query);
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
            ViewBindingTarget target,
            SerializedProperty prop,
            bool saved)
        {
            SerializedPropertySnapshot snapshot = SnapshotSerializedProperty(prop);
            string snapshotFields = SerializedPropertySnapshotFieldsToJson(snapshot);
            return "{" +
                   "\"ok\":true," +
                   "\"bindingId\":" + NullableJsonString(bindingId) + "," +
                   "\"message\":\"ok\"," +
                   "\"target\":" + TargetToJson(target) + "," +
                   snapshotFields + "," +
                   "\"saved\":" + (saved ? "true" : "false") +
                   "}";
        }

        private static string BuildBindingErrorJson(string bindingId, ViewBindingTarget target, string message)
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

        private static string TargetToJson(ViewBindingTarget target)
        {
            if (target == null)
                return "null";
            return "{" +
                   "\"kind\":\"" + JsonEscape(target.kind) + "\"," +
                   "\"path\":" + NullableJsonString(target.path) + "," +
                   "\"scenePath\":" + NullableJsonString(target.scenePath) + "," +
                   "\"objectPath\":" + NullableJsonString(target.objectPath) + "," +
                   "\"componentType\":" + NullableJsonString(target.componentType) + "," +
                   "\"componentIndex\":" + target.componentIndex.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"propertyPath\":" + NullableJsonString(target.propertyPath) +
                   "}";
        }

        private static string NullableJsonString(string value)
        {
            return string.IsNullOrEmpty(value) ? "null" : "\"" + JsonEscape(value) + "\"";
        }
    }
}
