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
        private sealed class Vector2Json
        {
            public float x;
            public float y;
        }

        [Serializable]
        private sealed class Vector3Json
        {
            public float x;
            public float y;
            public float z;
        }

        [Serializable]
        private sealed class Vector4Json
        {
            public float x;
            public float y;
            public float z;
            public float w;
        }

        [Serializable]
        private sealed class ColorJson
        {
            public float r;
            public float g;
            public float b;
            public float a = 1f;
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
            if (target == null)
                throw new Exception("View binding target is required");
            if (string.IsNullOrWhiteSpace(target.kind))
                throw new Exception("View binding target kind is required");
            if (string.IsNullOrWhiteSpace(target.propertyPath))
                throw new Exception("View binding target propertyPath is required");
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
                    {
                        serialized.ApplyModifiedProperties();
                        MarkViewBindingObjectDirty(obj);
                    }

                    for (int i = 0; i < applied.Count; i++)
                    {
                        AppliedViewBindingWrite item = applied[i];
                        resultItems[item.write.index] =
                            BuildBindingReadJson(item.write.bindingId, item.write.target, item.prop, true);
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
                   (target.componentType ?? "").Trim();
        }

        private static string ReadViewBinding(string bindingId, ViewBindingTarget target)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            var serialized = new SerializedObject(obj);
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            return BuildBindingReadJson(bindingId, target, prop, false);
        }

        private static string WriteViewBinding(string bindingId, ViewBindingTarget target, string valueJson)
        {
            UnityEngine.Object obj = ResolveViewBindingObject(target);
            var serialized = new SerializedObject(obj);
            SerializedProperty prop = serialized.FindProperty(target.propertyPath);
            if (prop == null)
                throw new Exception("SerializedProperty not found: " + target.propertyPath);
            serialized.Update();
            SetSerializedPropertyValue(prop, valueJson);
            serialized.ApplyModifiedProperties();
            MarkViewBindingObjectDirty(obj);
            return BuildBindingReadJson(bindingId, target, prop, true);
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

            GameObject current = scene.GetRootGameObjects()
                .FirstOrDefault(root => string.Equals(NormalizeObjectPathSegment(root.name), NormalizeObjectPathSegment(parts[0]), StringComparison.Ordinal));
            if (current == null)
                throw new Exception("Root GameObject not found: " + parts[0]);

            for (int i = 1; i < parts.Length; i++)
            {
                string name = NormalizeObjectPathSegment(parts[i]);
                Transform child = null;
                for (int j = 0; j < current.transform.childCount; j++)
                {
                    Transform candidate = current.transform.GetChild(j);
                    if (string.Equals(candidate.name, name, StringComparison.Ordinal))
                    {
                        child = candidate;
                        break;
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

            Component component = go.GetComponents<Component>()
                .FirstOrDefault(candidate =>
                    candidate != null &&
                    (string.Equals(candidate.GetType().FullName, typeName, StringComparison.Ordinal) ||
                     string.Equals(candidate.GetType().Name, typeName, StringComparison.Ordinal)));
            if (component == null)
                throw new Exception("Component not found: " + typeName);
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

        private static string NormalizeObjectPathSegment(string segment)
        {
            if (string.IsNullOrEmpty(segment))
                return "";
            int ordinal = segment.LastIndexOf('[');
            return ordinal > 0 && segment.EndsWith("]", StringComparison.Ordinal)
                ? segment.Substring(0, ordinal)
                : segment;
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

        private static string BuildBindingReadJson(
            string bindingId,
            ViewBindingTarget target,
            SerializedProperty prop,
            bool saved)
        {
            return "{" +
                   "\"ok\":true," +
                   "\"bindingId\":" + NullableJsonString(bindingId) + "," +
                   "\"message\":\"ok\"," +
                   "\"target\":" + TargetToJson(target) + "," +
                   "\"propertyPath\":\"" + JsonEscape(prop.propertyPath) + "\"," +
                   "\"displayName\":\"" + JsonEscape(prop.displayName) + "\"," +
                   "\"valueType\":\"" + JsonEscape(prop.propertyType.ToString()) + "\"," +
                   "\"value\":" + SerializedPropertyValueToJson(prop) + "," +
                   "\"editable\":" + (IsSerializedPropertyEditable(prop) ? "true" : "false") + "," +
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
                   "\"valueType\":\"Error\"," +
                   "\"value\":null," +
                   "\"editable\":false," +
                   "\"saved\":false" +
                   "}";
        }

        private static bool IsSerializedPropertyEditable(SerializedProperty prop)
        {
            return prop.propertyType != SerializedPropertyType.Generic;
        }

        private static string SerializedPropertyValueToJson(SerializedProperty prop)
        {
            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                    return prop.intValue.ToString(CultureInfo.InvariantCulture);
                case SerializedPropertyType.Boolean:
                    return prop.boolValue ? "true" : "false";
                case SerializedPropertyType.Float:
                    return prop.floatValue.ToString(CultureInfo.InvariantCulture);
                case SerializedPropertyType.String:
                    return "\"" + JsonEscape(prop.stringValue) + "\"";
                case SerializedPropertyType.Enum:
                    return "{" +
                           "\"index\":" + prop.enumValueIndex.ToString(CultureInfo.InvariantCulture) + "," +
                           "\"name\":\"" + JsonEscape(prop.enumDisplayNames != null && prop.enumValueIndex >= 0 && prop.enumValueIndex < prop.enumDisplayNames.Length ? prop.enumDisplayNames[prop.enumValueIndex] : "") + "\"" +
                           "}";
                case SerializedPropertyType.ObjectReference:
                    return "\"" + JsonEscape(prop.objectReferenceValue != null ? AssetDatabase.GetAssetPath(prop.objectReferenceValue) : "") + "\"";
                case SerializedPropertyType.Vector2:
                    return VectorToJson(prop.vector2Value);
                case SerializedPropertyType.Vector3:
                    return VectorToJson(prop.vector3Value);
                case SerializedPropertyType.Vector4:
                    return VectorToJson(prop.vector4Value);
                case SerializedPropertyType.Color:
                    return "\"" + JsonEscape("#" + ColorUtility.ToHtmlStringRGBA(prop.colorValue)) + "\"";
                case SerializedPropertyType.Rect:
                    Rect rect = prop.rectValue;
                    return "{" +
                           "\"x\":" + rect.x.ToString(CultureInfo.InvariantCulture) + "," +
                           "\"y\":" + rect.y.ToString(CultureInfo.InvariantCulture) + "," +
                           "\"width\":" + rect.width.ToString(CultureInfo.InvariantCulture) + "," +
                           "\"height\":" + rect.height.ToString(CultureInfo.InvariantCulture) +
                           "}";
                default:
                    return "null";
            }
        }

        private static void SetSerializedPropertyValue(SerializedProperty prop, string valueJson)
        {
            string json = string.IsNullOrWhiteSpace(valueJson) ? "null" : valueJson.Trim();
            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                    prop.intValue = ParseIntJson(json);
                    break;
                case SerializedPropertyType.Boolean:
                    prop.boolValue = ParseBoolJson(json);
                    break;
                case SerializedPropertyType.Float:
                    prop.floatValue = ParseFloatJson(json);
                    break;
                case SerializedPropertyType.String:
                    prop.stringValue = ParseStringJson(json);
                    break;
                case SerializedPropertyType.Enum:
                    SetEnumValue(prop, json);
                    break;
                case SerializedPropertyType.ObjectReference:
                    string assetPath = ParseStringJson(json);
                    prop.objectReferenceValue = string.IsNullOrWhiteSpace(assetPath)
                        ? null
                        : AssetDatabase.LoadAssetAtPath<UnityEngine.Object>(assetPath);
                    break;
                case SerializedPropertyType.Vector2:
                    Vector2Json v2 = JsonUtility.FromJson<Vector2Json>(json);
                    prop.vector2Value = new Vector2(v2.x, v2.y);
                    break;
                case SerializedPropertyType.Vector3:
                    Vector3Json v3 = JsonUtility.FromJson<Vector3Json>(json);
                    prop.vector3Value = new Vector3(v3.x, v3.y, v3.z);
                    break;
                case SerializedPropertyType.Vector4:
                    Vector4Json v4 = JsonUtility.FromJson<Vector4Json>(json);
                    prop.vector4Value = new Vector4(v4.x, v4.y, v4.z, v4.w);
                    break;
                case SerializedPropertyType.Color:
                    prop.colorValue = ParseColorJson(json);
                    break;
                default:
                    throw new Exception("SerializedProperty type is not writable: " + prop.propertyType);
            }
        }

        private static int ParseIntJson(string json)
        {
            int value;
            if (!int.TryParse(TrimJsonString(json), NumberStyles.Integer, CultureInfo.InvariantCulture, out value))
                throw new Exception("Expected integer value");
            return value;
        }

        private static float ParseFloatJson(string json)
        {
            float value;
            if (!float.TryParse(TrimJsonString(json), NumberStyles.Float, CultureInfo.InvariantCulture, out value))
                throw new Exception("Expected float value");
            return value;
        }

        private static bool ParseBoolJson(string json)
        {
            bool value;
            if (!bool.TryParse(TrimJsonString(json), out value))
                throw new Exception("Expected boolean value");
            return value;
        }

        private static string ParseStringJson(string json)
        {
            return TrimJsonString(json);
        }

        private static Color ParseColorJson(string json)
        {
            string text = TrimJsonString(json);
            Color color;
            if (!string.IsNullOrWhiteSpace(text) && ColorUtility.TryParseHtmlString(text, out color))
                return color;

            ColorJson value = JsonUtility.FromJson<ColorJson>(json);
            return new Color(value.r, value.g, value.b, value.a);
        }

        private static void SetEnumValue(SerializedProperty prop, string json)
        {
            string text = TrimJsonString(json);
            int index;
            if (int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out index))
            {
                prop.enumValueIndex = index;
                return;
            }

            string[] names = prop.enumDisplayNames;
            if (names != null)
            {
                for (int i = 0; i < names.Length; i++)
                {
                    if (string.Equals(names[i], text, StringComparison.OrdinalIgnoreCase))
                    {
                        prop.enumValueIndex = i;
                        return;
                    }
                }
            }
            throw new Exception("Enum value not found: " + text);
        }

        private static string TrimJsonString(string json)
        {
            if (string.IsNullOrWhiteSpace(json) || string.Equals(json, "null", StringComparison.OrdinalIgnoreCase))
                return "";

            json = json.Trim();
            if (json.Length >= 2 && json[0] == '"' && json[json.Length - 1] == '"')
                return UnescapeJsonString(json.Substring(1, json.Length - 2));
            return json;
        }

        private static string UnescapeJsonString(string value)
        {
            return value
                .Replace("\\\"", "\"")
                .Replace("\\\\", "\\")
                .Replace("\\n", "\n")
                .Replace("\\r", "\r")
                .Replace("\\t", "\t");
        }

        private static string VectorToJson(Vector2 value)
        {
            return "{" +
                   "\"x\":" + value.x.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"y\":" + value.y.ToString(CultureInfo.InvariantCulture) +
                   "}";
        }

        private static string VectorToJson(Vector3 value)
        {
            return "{" +
                   "\"x\":" + value.x.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"y\":" + value.y.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"z\":" + value.z.ToString(CultureInfo.InvariantCulture) +
                   "}";
        }

        private static string VectorToJson(Vector4 value)
        {
            return "{" +
                   "\"x\":" + value.x.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"y\":" + value.y.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"z\":" + value.z.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"w\":" + value.w.ToString(CultureInfo.InvariantCulture) +
                   "}";
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
                   "\"propertyPath\":" + NullableJsonString(target.propertyPath) +
                   "}";
        }

        private static string NullableJsonString(string value)
        {
            return string.IsNullOrEmpty(value) ? "null" : "\"" + JsonEscape(value) + "\"";
        }
    }
}
