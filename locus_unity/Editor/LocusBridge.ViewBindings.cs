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
        [Serializable]
        private sealed class ViewBindingTarget
        {
            public string kind;
            public string path;
            public string scenePath;
            public string objectPath;
            public long objectFileId;
            public long targetFileId;
            public string componentType;
            public int componentIndex;
            public string propertyPath;
        }

        [Serializable]
        private sealed class ViewBindingReadRequest
        {
            public string bindingId;
            public ViewBindingTarget target;
            public int maxDepth;
            public int maxArrayItems;
        }

        [Serializable]
        private sealed class ViewBindingWriteRequest
        {
            public string bindingId;
            public ViewBindingTarget target;
            public string valueJson;
            public string mode;
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
                ValidateViewBindingObjectTarget(request != null ? request.target : null);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await RunViewBindingOnMainThread(
                requestId,
                "view_binding_read",
                delegate { return ReadViewBinding(request.bindingId, request.target, request.maxDepth, request.maxArrayItems); });
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
                delegate { return WriteViewBinding(request.bindingId, request.target, request.valueJson, request.mode); });
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
            public string mode;
            public UnityEngine.Object obj;
        }

        private sealed class AppliedViewBindingWrite
        {
            public ResolvedViewBindingWrite write;
            public SerializedProperty prop;
        }

        private const string ViewBindingComponentEnabledPropertyPath = "m_Enabled";
        private const string ViewBindingGameObjectActivePropertyPath = "m_IsActive";

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
                        mode = write.mode,
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
                            if (IsViewBindingSyntheticHeaderProperty(obj, write.target))
                            {
                                resultItems[write.index] = WriteViewBindingSyntheticHeaderProperty(
                                    write.bindingId,
                                    write.target,
                                    obj,
                                    write.valueJson);
                                continue;
                            }

                            SerializedProperty prop = serialized.FindProperty(write.target.propertyPath);
                            if (prop == null)
                                throw new Exception("SerializedProperty not found: " + write.target.propertyPath);

                            if (IsViewBindingPreviewMode(write.mode))
                            {
                                prop = ApplyViewBindingPreviewValue(obj, serialized, prop, write);
                                resultItems[write.index] =
                                    BuildBindingReadJson(write.bindingId, write.target, prop, false);
                            }
                            else
                            {
                                SetSerializedPropertyValue(prop, write.valueJson);
                                applied.Add(new AppliedViewBindingWrite
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
                   target.objectFileId.ToString(CultureInfo.InvariantCulture) + "|" +
                   target.targetFileId.ToString(CultureInfo.InvariantCulture) + "|" +
                   (target.componentType ?? "").Trim() + "|" +
                   target.componentIndex.ToString(CultureInfo.InvariantCulture);
        }

        private static string ReadViewBinding(string bindingId, ViewBindingTarget target, int maxDepth = 0, int maxArrayItems = 0)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            var serialized = new SerializedObject(obj);
            serialized.Update();
            if (string.IsNullOrWhiteSpace(target.propertyPath))
            {
                int depthLimit = maxDepth > 0 ? Math.Min(maxDepth, 16) : 4;
                int arrayLimit = maxArrayItems > 0 ? Math.Min(maxArrayItems, 512) : 64;
                SerializedPropertySnapshot[] properties = SnapshotViewBindingObjectProperties(
                    target,
                    obj,
                    depthLimit,
                    arrayLimit);
                SerializedPropertySnapshot snapshot = properties.Length == 1
                    ? properties[0]
                    : BuildViewBindingAggregateSnapshot(target, obj, properties);
                return BuildBindingReadJson(bindingId, target, snapshot, false, properties.Length > 1 ? properties : null);
            }
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            int propertyDepthLimit = maxDepth > 0 ? Math.Min(maxDepth, 16) : 4;
            int propertyArrayLimit = maxArrayItems > 0 ? Math.Min(maxArrayItems, 512) : 64;
            return BuildBindingReadJson(
                bindingId,
                target,
                SnapshotSerializedProperty(prop, propertyDepthLimit, propertyArrayLimit),
                false);
        }

        private static SerializedPropertySnapshot[] SnapshotViewBindingObjectProperties(
            ViewBindingTarget target,
            UnityEngine.Object obj,
            int maxDepth,
            int maxArrayItems)
        {
            GameObject go = obj as GameObject;
            if (go == null)
                return new[] { SnapshotViewBindingObject(target, obj, maxDepth, maxArrayItems) };

            var properties = new List<SerializedPropertySnapshot>();
            properties.Add(SnapshotViewBindingObject(
                ViewBindingGameObjectTarget(target),
                go,
                maxDepth,
                maxArrayItems));

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

                properties.Add(SnapshotViewBindingObject(
                    ViewBindingComponentTarget(target, componentType, componentIndex),
                    component,
                    maxDepth,
                    maxArrayItems));
            }

            return properties.ToArray();
        }

        private static SerializedPropertySnapshot SnapshotViewBindingObject(
            ViewBindingTarget target,
            UnityEngine.Object obj,
            int maxDepth,
            int maxArrayItems)
        {
            target = ViewBindingTargetWithLocalFileIds(target, obj);
            SerializedPropertySnapshot snapshot = SnapshotSerializedObject(obj, maxDepth, maxArrayItems);
            if (snapshot == null)
                return null;

            SerializedPropertyBindingTarget bindingTarget = ToSerializedPropertyBindingTarget(target);
            ApplyViewBindingTargetToSnapshotTree(snapshot, bindingTarget);
            snapshot.displayName = ViewBindingObjectDisplayName(obj);
            snapshot.name = snapshot.displayName;
            snapshot.children = WithViewBindingSyntheticHeaderProperties(
                obj,
                bindingTarget,
                snapshot.children);
            snapshot.hasChildren = snapshot.children != null && snapshot.children.Length > 0;
            return snapshot;
        }

        private static void ApplyViewBindingTargetToSnapshotTree(
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
                ApplyViewBindingTargetToSnapshotTree(children[i], bindingTarget);
        }

        private static SerializedPropertySnapshot[] WithViewBindingSyntheticHeaderProperties(
            UnityEngine.Object obj,
            SerializedPropertyBindingTarget bindingTarget,
            SerializedPropertySnapshot[] children)
        {
            SerializedPropertySnapshot synthetic = BuildViewBindingSyntheticHeaderPropertySnapshot(obj, bindingTarget);
            if (synthetic == null)
                return children ?? new SerializedPropertySnapshot[0];

            SerializedPropertySnapshot[] existing = children ?? new SerializedPropertySnapshot[0];
            for (int i = 0; i < existing.Length; i++)
            {
                if (existing[i] != null
                    && string.Equals(existing[i].propertyPath, synthetic.propertyPath, StringComparison.Ordinal))
                    return existing;
            }

            var merged = new SerializedPropertySnapshot[existing.Length + 1];
            merged[0] = synthetic;
            Array.Copy(existing, 0, merged, 1, existing.Length);
            return merged;
        }

        private static SerializedPropertySnapshot BuildViewBindingSyntheticHeaderPropertySnapshot(
            UnityEngine.Object obj,
            SerializedPropertyBindingTarget bindingTarget)
        {
            GameObject go = obj as GameObject;
            if (go != null)
            {
                return BuildViewBindingSyntheticBooleanPropertySnapshot(
                    bindingTarget,
                    ViewBindingGameObjectActivePropertyPath,
                    "Active",
                    go.activeSelf,
                    true);
            }

            Component component = obj as Component;
            bool enabled;
            if (component != null && TryGetViewBindingComponentEnabledState(component, out enabled))
            {
                return BuildViewBindingSyntheticBooleanPropertySnapshot(
                    bindingTarget,
                    ViewBindingComponentEnabledPropertyPath,
                    "Enabled",
                    enabled,
                    CanSetViewBindingComponentEnabledState(component));
            }

            return null;
        }

        private static SerializedPropertySnapshot BuildViewBindingSyntheticBooleanPropertySnapshot(
            SerializedPropertyBindingTarget bindingTarget,
            string propertyPath,
            string displayName,
            bool value,
            bool editable)
        {
            SerializedPropertyBindingTarget propertyTarget = CloneSerializedPropertyBindingTarget(bindingTarget);
            if (propertyTarget != null)
                propertyTarget.propertyPath = propertyPath;

            return new SerializedPropertySnapshot
            {
                propertyPath = propertyPath,
                bindingTarget = propertyTarget,
                displayName = displayName,
                name = propertyPath,
                type = "Boolean",
                valueType = "Boolean",
                fieldTypeFullName = typeof(bool).FullName,
                fieldTypeAssembly = typeof(bool).Assembly.GetName().Name,
                value = value,
                displayValue = value ? "true" : "false",
                editable = editable,
                hasChildren = false,
                isArray = false,
                arraySize = -1,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = new SerializedPropertySnapshot[0],
                isManagedReference = false,
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
                path = source.path ?? "",
                scenePath = source.scenePath ?? "",
                objectPath = source.objectPath ?? "",
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType ?? "",
                componentIndex = source.componentIndex,
                propertyPath = source.propertyPath ?? ""
            };
        }

        private static SerializedPropertySnapshot BuildViewBindingAggregateSnapshot(
            ViewBindingTarget target,
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
                bindingTarget = ToSerializedPropertyBindingTarget(target),
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
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = properties ?? new SerializedPropertySnapshot[0],
                isManagedReference = false,
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

        private static ViewBindingTarget ViewBindingTargetWithLocalFileIds(
            ViewBindingTarget source,
            UnityEngine.Object obj)
        {
            if (source == null)
                return null;

            var target = new ViewBindingTarget
            {
                kind = source.kind,
                path = source.path,
                scenePath = source.scenePath,
                objectPath = source.objectPath,
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType,
                componentIndex = source.componentIndex,
                propertyPath = source.propertyPath
            };

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

            return target;
        }

        private static ViewBindingTarget ViewBindingGameObjectTarget(ViewBindingTarget source)
        {
            if (source == null)
                return null;
            return new ViewBindingTarget
            {
                kind = source.kind,
                path = source.path,
                scenePath = source.scenePath,
                objectPath = source.objectPath,
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = "",
                componentIndex = 0,
                propertyPath = ""
            };
        }

        private static ViewBindingTarget ViewBindingComponentTarget(
            ViewBindingTarget source,
            string componentType,
            int componentIndex)
        {
            return new ViewBindingTarget
            {
                kind = "component",
                path = source != null ? source.path : "",
                scenePath = source != null ? source.scenePath : "",
                objectPath = source != null ? source.objectPath : "",
                objectFileId = source != null ? source.objectFileId : 0,
                targetFileId = 0,
                componentType = componentType,
                componentIndex = componentIndex,
                propertyPath = ""
            };
        }

        private static SerializedPropertyBindingTarget ToSerializedPropertyBindingTarget(ViewBindingTarget source)
        {
            if (source == null)
                return null;
            return new SerializedPropertyBindingTarget
            {
                kind = source.kind ?? "",
                path = source.path ?? "",
                scenePath = source.scenePath ?? "",
                objectPath = source.objectPath ?? "",
                objectFileId = source.objectFileId,
                targetFileId = source.targetFileId,
                componentType = source.componentType ?? "",
                componentIndex = source.componentIndex,
                propertyPath = source.propertyPath ?? ""
            };
        }

        private static string ComponentBindingTypeName(Component component)
        {
            Type type = component != null ? component.GetType() : null;
            return type != null ? type.FullName ?? type.Name ?? "" : "";
        }

        private static string ViewBindingObjectDisplayName(UnityEngine.Object obj)
        {
            if (obj is GameObject)
                return "GameObject";

            Component component = obj as Component;
            if (component != null)
            {
                Type type = component.GetType();
                string label = ObjectNames.NicifyVariableName(type.Name);
                if (component is MonoBehaviour)
                    label += " (Script)";
                return label;
            }

            if (obj == null)
                return "Unity Object";

            Type objectType = obj.GetType();
            return ObjectNames.NicifyVariableName(objectType.Name);
        }

        private static bool IsViewBindingPreviewMode(string mode)
        {
            return string.Equals((mode ?? "").Trim(), "preview", StringComparison.OrdinalIgnoreCase);
        }

        private static SerializedProperty ApplyViewBindingPreviewValue(
            UnityEngine.Object obj,
            SerializedObject serialized,
            SerializedProperty prop,
            ResolvedViewBindingWrite write)
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

        private static string WriteViewBinding(string bindingId, ViewBindingTarget target, string valueJson, string mode = null)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            if (IsViewBindingSyntheticHeaderProperty(obj, target))
                return WriteViewBindingSyntheticHeaderProperty(bindingId, target, obj, valueJson);

            var serialized = new SerializedObject(obj);
            serialized.Update();
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);

            if (IsViewBindingPreviewMode(mode))
            {
                var write = new ResolvedViewBindingWrite
                {
                    index = 0,
                    bindingId = bindingId,
                    target = target,
                    valueJson = valueJson,
                    mode = mode,
                    obj = obj
                };
                prop = ApplyViewBindingPreviewValue(obj, serialized, prop, write);
                return BuildBindingReadJson(bindingId, target, prop, false);
            }

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
            if (IsPrefabAssetPath(target.path))
                return ResolvePrefabAssetGameObjectTarget(target);

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

        private static GameObject ResolvePrefabAssetGameObjectTarget(ViewBindingTarget target)
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

        private static Component ResolveComponentTarget(ViewBindingTarget target)
        {
            if (target.targetFileId != 0 && !IsPrefabAssetPath(target.path))
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

        private static bool IsViewBindingSyntheticHeaderProperty(
            UnityEngine.Object obj,
            ViewBindingTarget target)
        {
            string propertyPath = (target != null ? target.propertyPath : "") ?? "";
            propertyPath = propertyPath.Trim();

            if (string.Equals(propertyPath, ViewBindingGameObjectActivePropertyPath, StringComparison.Ordinal))
                return obj is GameObject;

            if (string.Equals(propertyPath, ViewBindingComponentEnabledPropertyPath, StringComparison.Ordinal))
                return obj is Component && HasViewBindingComponentEnabledState((Component)obj);

            return false;
        }

        private static string WriteViewBindingSyntheticHeaderProperty(
            string bindingId,
            ViewBindingTarget target,
            UnityEngine.Object obj,
            string valueJson)
        {
            bool value = ParseBoolJson(string.IsNullOrWhiteSpace(valueJson) ? "false" : valueJson);
            int undoGroup = Undo.GetCurrentGroup();
            Undo.SetCurrentGroupName("Locus View Binding");
            Undo.RecordObject(obj, "Locus View Binding");

            string propertyPath = (target != null ? target.propertyPath : "") ?? "";
            propertyPath = propertyPath.Trim();

            GameObject go = obj as GameObject;
            if (go != null && string.Equals(propertyPath, ViewBindingGameObjectActivePropertyPath, StringComparison.Ordinal))
            {
                go.SetActive(value);
            }
            else
            {
                Component component = obj as Component;
                if (component == null
                    || !string.Equals(propertyPath, ViewBindingComponentEnabledPropertyPath, StringComparison.Ordinal)
                    || !TrySetViewBindingComponentEnabledState(component, value))
                {
                    throw new Exception("Synthetic View binding property is not writable: " + propertyPath);
                }
            }

            RecordViewBindingPrefabModifications(obj);
            MarkViewBindingObjectDirty(obj);
            Undo.CollapseUndoOperations(undoGroup);

            SerializedPropertySnapshot snapshot = BuildViewBindingSyntheticHeaderPropertySnapshot(
                obj,
                ToSerializedPropertyBindingTarget(target));
            return BuildBindingReadJson(bindingId, target, snapshot, true);
        }

        private static bool HasViewBindingComponentEnabledState(Component component)
        {
            bool enabled;
            return TryGetViewBindingComponentEnabledState(component, out enabled);
        }

        private static bool TryGetViewBindingComponentEnabledState(Component component, out bool enabled)
        {
            enabled = false;
            PropertyInfo property = ViewBindingComponentEnabledProperty(component);
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

        private static bool CanSetViewBindingComponentEnabledState(Component component)
        {
            PropertyInfo property = ViewBindingComponentEnabledProperty(component);
            return property != null && property.CanWrite;
        }

        private static bool TrySetViewBindingComponentEnabledState(Component component, bool enabled)
        {
            PropertyInfo property = ViewBindingComponentEnabledProperty(component);
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

        private static PropertyInfo ViewBindingComponentEnabledProperty(Component component)
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
            if (IsViewBindingPrefabAssetObject(obj))
            {
                GameObject prefabRoot = ViewBindingPrefabAssetRoot(obj);
                if (prefabRoot != null)
                    EditorUtility.SetDirty(prefabRoot);
            }
            else if (component != null)
                EditorSceneManager.MarkSceneDirty(component.gameObject.scene);
            else if (go != null)
                EditorSceneManager.MarkSceneDirty(go.scene);
        }

        private static bool IsViewBindingPrefabAssetObject(UnityEngine.Object obj)
        {
            return ViewBindingPrefabAssetRoot(obj) != null;
        }

        private static GameObject ViewBindingPrefabAssetRoot(UnityEngine.Object obj)
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
            return BuildBindingReadJson(bindingId, target, snapshot, saved);
        }

        private static string BuildBindingReadJson(
            string bindingId,
            ViewBindingTarget target,
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
                   (properties != null ? "\"properties\":" + ToJsonValue(properties, 0) + "," : "") +
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
                   "\"objectFileId\":" + NullableJsonLong(target.objectFileId) + "," +
                   "\"targetFileId\":" + NullableJsonLong(target.targetFileId) + "," +
                   "\"componentType\":" + NullableJsonString(target.componentType) + "," +
                   "\"componentIndex\":" + target.componentIndex.ToString(CultureInfo.InvariantCulture) + "," +
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
