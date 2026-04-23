
using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.Text;
using System.Threading.Tasks;
using System.Collections.Generic;

namespace Locus
{
    public static partial class LocusBridge
    {
        // ═══════════════════════════════════════════════════════════
        // ═══════════════════════════════════════════════════════════

        [Serializable]
        private class SetSerializedDataRequest
        {
            public string scenePath;
            public string gameObjectPath;
            public string componentType;
            public string propertyPath;
            public string valueType;
        }

        private static async Task<PipeEnvelope> HandleSetSerializedData(string reqId, string messageJson)
        {
            var tcs = new TaskCompletionSource<PipeEnvelope>();

            PostToMainThread(delegate
            {
                try
                {
                    var jsonObj = MiniJson.Deserialize(messageJson) as Dictionary<string, object>;
                    if (jsonObj == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Invalid JSON"));
                        return;
                    }

                    string goPath = jsonObj.ContainsKey("gameObjectPath") ? jsonObj["gameObjectPath"] as string : null;
                    string compType = jsonObj.ContainsKey("componentType") ? jsonObj["componentType"] as string : null;
                    string propPath = jsonObj.ContainsKey("propertyPath") ? jsonObj["propertyPath"] as string : null;
                    string valueType = jsonObj.ContainsKey("valueType") ? jsonObj["valueType"] as string : null;

                    if (string.IsNullOrEmpty(goPath) || string.IsNullOrEmpty(compType) ||
                        string.IsNullOrEmpty(propPath) || string.IsNullOrEmpty(valueType))
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Missing required fields: gameObjectPath, componentType, propertyPath, valueType"));
                        return;
                    }

                    var go = FindGameObjectByPath(goPath);
                    if (go == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "GameObject not found: " + goPath));
                        return;
                    }

                    var component = FindComponentByType(go, compType);
                    if (component == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Component not found: " + compType + " on " + goPath));
                        return;
                    }

                    Undo.RecordObject(component, "Canvas: " + propPath);

                    var so = new SerializedObject(component);
                    var prop = so.FindProperty(propPath);
                    if (prop == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Property not found: " + propPath));
                        return;
                    }

                    object valueObj = jsonObj.ContainsKey("value") ? jsonObj["value"] : null;
                    SetPropertyValue(prop, valueObj, valueType);
                    so.ApplyModifiedProperties();

                    if (IsProjectPrefabPath(goPath))
                    {
                        EditorUtility.SetDirty(component);
                        AssetDatabase.SaveAssetIfDirty(component);
                    }

                    tcs.SetResult(OkResponse(reqId, "ok"));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                }
            });

            return await tcs.Task;
        }

        // ═══════════════════════════════════════════════════════════
        // ═══════════════════════════════════════════════════════════

        private static async Task<PipeEnvelope> HandleGetSerializedData(string reqId, string messageJson)
        {
            var tcs = new TaskCompletionSource<PipeEnvelope>();

            PostToMainThread(delegate
            {
                try
                {
                    var jsonObj = MiniJson.Deserialize(messageJson) as Dictionary<string, object>;
                    if (jsonObj == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Invalid JSON"));
                        return;
                    }

                    var queries = jsonObj.ContainsKey("queries") ? jsonObj["queries"] as List<object> : null;
                    if (queries == null)
                    {
                        tcs.SetResult(ErrorResponse(reqId, "Missing queries array"));
                        return;
                    }

                    var results = new List<object>();

                    foreach (var queryObj in queries)
                    {
                        var query = queryObj as Dictionary<string, object>;
                        if (query == null) continue;

                        string id = query.ContainsKey("id") ? query["id"] as string : "";
                        string goPath = query.ContainsKey("gameObjectPath") ? query["gameObjectPath"] as string : "";
                        string compType = query.ContainsKey("componentType") ? query["componentType"] as string : "";
                        string propPath = query.ContainsKey("propertyPath") ? query["propertyPath"] as string : "";

                        try
                        {
                            var go = FindGameObjectByPath(goPath);
                            if (go == null)
                            {
                                results.Add(MakeErrorResult(id, "GameObject not found: " + goPath));
                                continue;
                            }

                            var component = FindComponentByType(go, compType);
                            if (component == null)
                            {
                                results.Add(MakeErrorResult(id, "Component not found: " + compType + " on " + goPath));
                                continue;
                            }

                            var so = new SerializedObject(component);
                            var prop = so.FindProperty(propPath);
                            if (prop == null)
                            {
                                results.Add(MakeErrorResult(id, "Property not found: " + propPath));
                                continue;
                            }

                            var (value, valueType) = ReadPropertyValue(prop);
                            var result = new Dictionary<string, object>
                            {
                                { "id", id },
                                { "exists", true },
                                { "value", value },
                                { "valueType", valueType }
                            };
                            results.Add(result);
                        }
                        catch (Exception ex)
                        {
                            results.Add(MakeErrorResult(id, ex.Message));
                        }
                    }

                    var response = new Dictionary<string, object> { { "results", results } };
                    tcs.SetResult(OkResponse(reqId, MiniJson.Serialize(response)));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                }
            });

            return await tcs.Task;
        }

        // ═══════════════════════════════════════════════════════════
        // ═══════════════════════════════════════════════════════════

        /// <summary>
        /// </summary>
        private static GameObject FindGameObjectByPath(string path)
        {
            if (string.IsNullOrEmpty(path)) return null;

            if (IsProjectPrefabPath(path))
            {
                return AssetDatabase.LoadAssetAtPath<GameObject>(path);
            }

            string trimmed = path.TrimStart('/');
            string[] parts = trimmed.Split('/');

            var roots = UnityEngine.SceneManagement.SceneManager.GetActiveScene().GetRootGameObjects();
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
                var child = current.transform.Find(parts[i]);
                if (child == null) return null;
                current = child.gameObject;
            }
            return current;
        }

        /// <summary>
        /// </summary>
        private static Component FindComponentByType(GameObject go, string typeName)
        {
            var components = go.GetComponents<Component>();
            foreach (var comp in components)
            {
                if (comp == null) continue;
                var type = comp.GetType();
                if (type.Name == typeName || type.FullName == typeName)
                    return comp;
            }
            return null;
        }

        /// <summary>
        /// </summary>
        private static void SetPropertyValue(SerializedProperty prop, object value, string valueType)
        {
            switch (valueType)
            {
                case "int":
                    prop.intValue = Convert.ToInt32(value);
                    break;
                case "float":
                    prop.floatValue = Convert.ToSingle(value);
                    break;
                case "bool":
                    prop.boolValue = value is bool b ? b : Convert.ToBoolean(value);
                    break;
                case "string":
                    prop.stringValue = value as string ?? "";
                    break;
                case "enum":
                    prop.enumValueIndex = Convert.ToInt32(value);
                    break;
                case "vector2":
                {
                    var dict = value as Dictionary<string, object>;
                    if (dict != null)
                        prop.vector2Value = new Vector2(
                            Convert.ToSingle(dict.ContainsKey("x") ? dict["x"] : 0),
                            Convert.ToSingle(dict.ContainsKey("y") ? dict["y"] : 0));
                    break;
                }
                case "vector3":
                {
                    var dict = value as Dictionary<string, object>;
                    if (dict != null)
                        prop.vector3Value = new Vector3(
                            Convert.ToSingle(dict.ContainsKey("x") ? dict["x"] : 0),
                            Convert.ToSingle(dict.ContainsKey("y") ? dict["y"] : 0),
                            Convert.ToSingle(dict.ContainsKey("z") ? dict["z"] : 0));
                    break;
                }
                case "vector4":
                {
                    var dict = value as Dictionary<string, object>;
                    if (dict != null)
                        prop.vector4Value = new Vector4(
                            Convert.ToSingle(dict.ContainsKey("x") ? dict["x"] : 0),
                            Convert.ToSingle(dict.ContainsKey("y") ? dict["y"] : 0),
                            Convert.ToSingle(dict.ContainsKey("z") ? dict["z"] : 0),
                            Convert.ToSingle(dict.ContainsKey("w") ? dict["w"] : 0));
                    break;
                }
                case "color":
                {
                    var dict = value as Dictionary<string, object>;
                    if (dict != null)
                        prop.colorValue = new Color(
                            Convert.ToSingle(dict.ContainsKey("r") ? dict["r"] : 0),
                            Convert.ToSingle(dict.ContainsKey("g") ? dict["g"] : 0),
                            Convert.ToSingle(dict.ContainsKey("b") ? dict["b"] : 0),
                            Convert.ToSingle(dict.ContainsKey("a") ? dict["a"] : 1));
                    break;
                }
                case "object_ref":
                {
                    string refPath = value as string;
                    if (string.IsNullOrEmpty(refPath) || refPath == "null")
                        prop.objectReferenceValue = null;
                    else
                        prop.objectReferenceValue = FindGameObjectByPath(refPath);
                    break;
                }
                case "asset_ref":
                {
                    string assetPath = value as string;
                    if (string.IsNullOrEmpty(assetPath) || assetPath == "null")
                        prop.objectReferenceValue = null;
                    else
                        prop.objectReferenceValue = AssetDatabase.LoadMainAssetAtPath(assetPath);
                    break;
                }
                default:
                    throw new ArgumentException("Unknown valueType: " + valueType);
            }
        }

        /// <summary>
        /// </summary>
        private static (object value, string valueType) ReadPropertyValue(SerializedProperty prop)
        {
            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                    return (prop.intValue, "int");
                case SerializedPropertyType.Float:
                    return ((double)prop.floatValue, "float");
                case SerializedPropertyType.Boolean:
                    return (prop.boolValue, "bool");
                case SerializedPropertyType.String:
                    return (prop.stringValue, "string");
                case SerializedPropertyType.Enum:
                    return (prop.enumValueIndex, "enum");
                case SerializedPropertyType.Color:
                {
                    var c = prop.colorValue;
                    return (new Dictionary<string, object>
                    {
                        { "r", (double)c.r }, { "g", (double)c.g },
                        { "b", (double)c.b }, { "a", (double)c.a }
                    }, "color");
                }
                case SerializedPropertyType.Vector2:
                {
                    var v = prop.vector2Value;
                    return (new Dictionary<string, object>
                    {
                        { "x", (double)v.x }, { "y", (double)v.y }
                    }, "vector2");
                }
                case SerializedPropertyType.Vector3:
                {
                    var v = prop.vector3Value;
                    return (new Dictionary<string, object>
                    {
                        { "x", (double)v.x }, { "y", (double)v.y }, { "z", (double)v.z }
                    }, "vector3");
                }
                case SerializedPropertyType.Vector4:
                {
                    var v = prop.vector4Value;
                    return (new Dictionary<string, object>
                    {
                        { "x", (double)v.x }, { "y", (double)v.y },
                        { "z", (double)v.z }, { "w", (double)v.w }
                    }, "vector4");
                }
                case SerializedPropertyType.ObjectReference:
                {
                    if (prop.objectReferenceValue == null)
                        return (null, "object_ref");
                    string path = GetObjectPath(prop.objectReferenceValue);
                    string type = IsProjectAssetPath(path) ? "asset_ref" : "object_ref";
                    return (path, type);
                }
                default:
                    return (prop.ToString(), "string");
            }
        }

        /// <summary>
        /// </summary>
        private static string GetObjectPath(UnityEngine.Object obj)
        {
            if (obj is GameObject go)
                return "/" + GetHierarchyPath(go.transform);
            if (obj is Component comp)
                return "/" + GetHierarchyPath(comp.transform);

            string assetPath = AssetDatabase.GetAssetPath(obj);
            if (!string.IsNullOrEmpty(assetPath))
                return assetPath;

            return obj.name;
        }

        private static string GetHierarchyPath(Transform t)
        {
            if (t.parent == null) return t.name;
            return GetHierarchyPath(t.parent) + "/" + t.name;
        }

        /// <summary>
        /// </summary>
        private static Dictionary<string, object> MakeErrorResult(string id, string error)
        {
            return new Dictionary<string, object>
            {
                { "id", id },
                { "exists", false },
                { "error", error }
            };
        }

        // ═══════════════════════════════════════════════════════════
        // ═══════════════════════════════════════════════════════════

        private static class MiniJson
        {
            public static object Deserialize(string json)
            {
                if (string.IsNullOrEmpty(json)) return null;
                return JsonUtility_Parse(json);
            }

            public static string Serialize(object obj)
            {
                var sb = new StringBuilder(256);
                SerializeValue(obj, sb);
                return sb.ToString();
            }

            private static object JsonUtility_Parse(string json)
            {
                int idx = 0;
                return ParseValue(json, ref idx);
            }

            private static void SkipWhitespace(string json, ref int idx)
            {
                while (idx < json.Length && char.IsWhiteSpace(json[idx])) idx++;
            }

            private static object ParseValue(string json, ref int idx)
            {
                SkipWhitespace(json, ref idx);
                if (idx >= json.Length) return null;

                char c = json[idx];
                if (c == '{') return ParseObject(json, ref idx);
                if (c == '[') return ParseArray(json, ref idx);
                if (c == '"') return ParseString(json, ref idx);
                if (c == 't' || c == 'f') return ParseBool(json, ref idx);
                if (c == 'n') { idx += 4; return null; } // null
                return ParseNumber(json, ref idx);
            }

            private static Dictionary<string, object> ParseObject(string json, ref int idx)
            {
                var dict = new Dictionary<string, object>();
                idx++; // skip {
                SkipWhitespace(json, ref idx);
                if (idx < json.Length && json[idx] == '}') { idx++; return dict; }

                while (idx < json.Length)
                {
                    SkipWhitespace(json, ref idx);
                    string key = ParseString(json, ref idx);
                    SkipWhitespace(json, ref idx);
                    idx++; // skip :
                    dict[key] = ParseValue(json, ref idx);
                    SkipWhitespace(json, ref idx);
                    if (idx < json.Length && json[idx] == ',') idx++;
                    else break;
                }
                if (idx < json.Length && json[idx] == '}') idx++;
                return dict;
            }

            private static List<object> ParseArray(string json, ref int idx)
            {
                var list = new List<object>();
                idx++; // skip [
                SkipWhitespace(json, ref idx);
                if (idx < json.Length && json[idx] == ']') { idx++; return list; }

                while (idx < json.Length)
                {
                    list.Add(ParseValue(json, ref idx));
                    SkipWhitespace(json, ref idx);
                    if (idx < json.Length && json[idx] == ',') idx++;
                    else break;
                }
                if (idx < json.Length && json[idx] == ']') idx++;
                return list;
            }

            private static string ParseString(string json, ref int idx)
            {
                idx++; // skip opening "
                var sb = new StringBuilder();
                while (idx < json.Length)
                {
                    char c = json[idx++];
                    if (c == '"') return sb.ToString();
                    if (c == '\\' && idx < json.Length)
                    {
                        char next = json[idx++];
                        switch (next)
                        {
                            case '"': sb.Append('"'); break;
                            case '\\': sb.Append('\\'); break;
                            case '/': sb.Append('/'); break;
                            case 'n': sb.Append('\n'); break;
                            case 'r': sb.Append('\r'); break;
                            case 't': sb.Append('\t'); break;
                            case 'u':
                                if (idx + 4 <= json.Length)
                                {
                                    string hex = json.Substring(idx, 4);
                                    sb.Append((char)Convert.ToInt32(hex, 16));
                                    idx += 4;
                                }
                                break;
                            default: sb.Append(next); break;
                        }
                    }
                    else
                    {
                        sb.Append(c);
                    }
                }
                return sb.ToString();
            }

            private static bool ParseBool(string json, ref int idx)
            {
                if (json[idx] == 't') { idx += 4; return true; }
                idx += 5; return false;
            }

            private static object ParseNumber(string json, ref int idx)
            {
                int start = idx;
                if (idx < json.Length && json[idx] == '-') idx++;
                while (idx < json.Length && (char.IsDigit(json[idx]) || json[idx] == '.' || json[idx] == 'e' || json[idx] == 'E' || json[idx] == '+' || json[idx] == '-'))
                    idx++;
                string numStr = json.Substring(start, idx - start);
                if (numStr.Contains(".") || numStr.Contains("e") || numStr.Contains("E"))
                    return double.Parse(numStr, System.Globalization.CultureInfo.InvariantCulture);
                long l = long.Parse(numStr);
                if (l >= int.MinValue && l <= int.MaxValue) return (double)l;
                return (double)l;
            }

            private static void SerializeValue(object value, StringBuilder sb)
            {
                if (value == null) { sb.Append("null"); return; }
                if (value is string s) { SerializeString(s, sb); return; }
                if (value is bool b) { sb.Append(b ? "true" : "false"); return; }
                if (value is int i) { sb.Append(i); return; }
                if (value is long l) { sb.Append(l); return; }
                if (value is float f) { sb.Append(f.ToString("R", System.Globalization.CultureInfo.InvariantCulture)); return; }
                if (value is double d) { sb.Append(d.ToString("R", System.Globalization.CultureInfo.InvariantCulture)); return; }
                if (value is Dictionary<string, object> dict)
                {
                    sb.Append('{');
                    bool first = true;
                    foreach (var kv in dict)
                    {
                        if (!first) sb.Append(',');
                        first = false;
                        SerializeString(kv.Key, sb);
                        sb.Append(':');
                        SerializeValue(kv.Value, sb);
                    }
                    sb.Append('}');
                    return;
                }
                if (value is List<object> list)
                {
                    sb.Append('[');
                    for (int idx = 0; idx < list.Count; idx++)
                    {
                        if (idx > 0) sb.Append(',');
                        SerializeValue(list[idx], sb);
                    }
                    sb.Append(']');
                    return;
                }
                SerializeString(value.ToString(), sb);
            }

            private static void SerializeString(string s, StringBuilder sb)
            {
                sb.Append('"');
                foreach (char c in s)
                {
                    switch (c)
                    {
                        case '"': sb.Append("\\\""); break;
                        case '\\': sb.Append("\\\\"); break;
                        case '\n': sb.Append("\\n"); break;
                        case '\r': sb.Append("\\r"); break;
                        case '\t': sb.Append("\\t"); break;
                        default: sb.Append(c); break;
                    }
                }
                sb.Append('"');
            }
        }
    }
}
