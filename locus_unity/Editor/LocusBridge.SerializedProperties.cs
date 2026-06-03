using UnityEditor;
using UnityEngine;

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Reflection;
using System.Runtime.Serialization;

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed class SerializedPropertySnapshot
        {
            public string propertyPath;
            public SerializedPropertyBindingTarget bindingTarget;
            public string displayName;
            public string name;
            public string type;
            public string valueType;
            public string fieldTypeFullName;
            public string fieldTypeAssembly;
            public object value;
            public string displayValue;
            public bool editable;
            public bool hasChildren;
            public bool isArray;
            public int arraySize;
            public bool isFlagsEnum;
            public int enumValueIndex;
            public long enumValueFlag;
            public SerializedEnumOption[] enumOptions;
            public SerializedPropertySnapshot[] children;
            public bool isManagedReference;
            public string managedReferenceFullTypename;
            public string managedReferenceFieldTypename;
            public string managedReferenceDisplayName;
            public SerializedManagedReferenceTypeOption[] managedReferenceTypes;
            public string tooltip;
            public string header;
            public bool hasRange;
            public float rangeMin;
            public float rangeMax;
            public float numberStep;
            public bool multiline;
            public int minLines;
            public int maxLines;
            public string referenceTypeFullName;
            public string referenceTypeAssembly;
            public SerializedPropertyAttributeInfo[] attributes;
        }

        public sealed class SerializedPropertyBindingTarget
        {
            public string kind;
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

        public sealed class SerializedEnumOption
        {
            public string label;
            public string value;
            public string name;
            public int index;
            public long numericValue;
        }

        public sealed class SerializedManagedReferenceTypeOption
        {
            public string label;
            public string value;
            public string fullName;
            public string assembly;
        }

        public sealed class SerializedPropertyAttributeInfo
        {
            public string type;
            public string displayName;
            public string value;
        }

        private sealed class Vector2Json
        {
            public float x;
            public float y;
        }

        private sealed class Vector3Json
        {
            public float x;
            public float y;
            public float z;
        }

        private sealed class Vector4Json
        {
            public float x;
            public float y;
            public float z;
            public float w;
        }

        private sealed class QuaternionJson
        {
            public string action;
            public float x;
            public float y;
            public float z;
            public float w = 1f;
        }

        private sealed class ColorJson
        {
            public float r;
            public float g;
            public float b;
            public float a = 1f;
        }

        private sealed class SerializedArrayWriteCommand
        {
            public string action;
            public int index;
            public int toIndex;
            public int size;
        }

        private sealed class SerializedManagedReferenceWriteCommand
        {
            public string action;
            public string typeName;
            public string type;
            public string fullName;
            public string assembly;
        }

        private sealed class SerializedEnumWriteCommand
        {
            public string action;
            public string name;
            public string label;
            public string value;
            public int index = -1;
            public long numericValue;
            public long flagValue;
        }

        private sealed class SerializedPropertyRestoreCommand
        {
            public string action;
            public SerializedPropertySnapshot snapshot;
        }

        public static SerializedPropertySnapshot SnapshotSerializedProperty(
            SerializedProperty prop,
            int maxDepth = 4,
            int maxArrayItems = 64)
        {
            if (prop == null)
                return null;

            maxDepth = Math.Max(0, maxDepth);
            maxArrayItems = Math.Max(0, maxArrayItems);
            return SnapshotSerializedProperty(prop.Copy(), 0, maxDepth, maxArrayItems);
        }

        public static SerializedPropertySnapshot SnapshotSerializedObject(
            UnityEngine.Object obj,
            int maxDepth = 4,
            int maxArrayItems = 64)
        {
            if (obj == null)
                return null;

            maxDepth = Math.Max(0, maxDepth);
            maxArrayItems = Math.Max(0, maxArrayItems);

            var serialized = new SerializedObject(obj);
            serialized.Update();

            var children = new List<SerializedPropertySnapshot>();
            SerializedProperty cursor = serialized.GetIterator();
            bool enterChildren = true;
            while (cursor.NextVisible(enterChildren))
            {
                children.Add(SnapshotSerializedProperty(cursor, 1, maxDepth, maxArrayItems));
                enterChildren = false;
            }

            Type type = obj.GetType();
            string name = obj.name ?? "";
            return new SerializedPropertySnapshot
            {
                propertyPath = "",
                displayName = string.IsNullOrEmpty(name) ? type.Name : name,
                name = name,
                type = type.Name,
                valueType = "Object",
                fieldTypeFullName = FieldTypeFullName(type),
                fieldTypeAssembly = FieldTypeAssembly(type),
                value = name,
                displayValue = string.IsNullOrEmpty(name) ? type.Name : name,
                editable = false,
                hasChildren = children.Count > 0,
                isArray = false,
                arraySize = -1,
                isFlagsEnum = false,
                enumValueIndex = -1,
                enumValueFlag = 0,
                enumOptions = new SerializedEnumOption[0],
                children = children.ToArray(),
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

        public static string SerializedPropertySnapshotToJson(SerializedPropertySnapshot snapshot)
        {
            return ToJsonValue(snapshot, 0);
        }

        public static bool IsSerializedPropertyWritable(SerializedProperty prop)
        {
            if (prop == null || prop.propertyPath == "m_Script" || !prop.editable)
                return false;

            if (prop.isArray && prop.propertyType == SerializedPropertyType.Generic)
                return true;

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.Boolean:
                case SerializedPropertyType.Float:
                case SerializedPropertyType.String:
                case SerializedPropertyType.Enum:
                case SerializedPropertyType.ObjectReference:
                case SerializedPropertyType.LayerMask:
                case SerializedPropertyType.ArraySize:
                case SerializedPropertyType.Vector2:
                case SerializedPropertyType.Vector3:
                case SerializedPropertyType.Vector4:
                case SerializedPropertyType.Quaternion:
                case SerializedPropertyType.Color:
                case SerializedPropertyType.Rect:
                case SerializedPropertyType.ManagedReference:
                    return true;
                default:
                    return false;
            }
        }

        public static object SerializedPropertyValue(SerializedProperty prop)
        {
            if (prop == null)
                return null;

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                case SerializedPropertyType.LayerMask:
                    return prop.intValue;
                case SerializedPropertyType.Boolean:
                    return prop.boolValue;
                case SerializedPropertyType.Float:
                    return prop.floatValue;
                case SerializedPropertyType.String:
                    return prop.stringValue;
                case SerializedPropertyType.Enum:
                    return new Dictionary<string, object>
                    {
                        { "index", prop.enumValueIndex },
                        { "name", SerializedEnumDisplayName(prop) },
                        { "numericValue", SerializedEnumNumericValue(prop) },
                        { "isFlags", IsFlagsEnum(prop) }
                    };
                case SerializedPropertyType.ObjectReference:
                    return prop.objectReferenceValue != null
                        ? SerializedObjectReferencePath(prop.objectReferenceValue)
                        : "";
                case SerializedPropertyType.Vector2:
                    return VectorValue(prop.vector2Value);
                case SerializedPropertyType.Vector3:
                    return VectorValue(prop.vector3Value);
                case SerializedPropertyType.Vector4:
                    return VectorValue(prop.vector4Value);
                case SerializedPropertyType.Quaternion:
                    return QuaternionValue(prop.quaternionValue);
                case SerializedPropertyType.Color:
                    return "#" + ColorUtility.ToHtmlStringRGBA(prop.colorValue);
                case SerializedPropertyType.Rect:
                    Rect rect = prop.rectValue;
                    return new Dictionary<string, object>
                    {
                        { "x", rect.x },
                        { "y", rect.y },
                        { "width", rect.width },
                        { "height", rect.height }
                    };
                case SerializedPropertyType.ManagedReference:
                    return new Dictionary<string, object>
                    {
                        { "typeName", prop.managedReferenceFullTypename ?? "" },
                        { "fieldTypeName", prop.managedReferenceFieldTypename ?? "" },
                        { "displayName", ManagedReferenceDisplayName(prop.managedReferenceFullTypename) }
                    };
                default:
                    return null;
            }
        }

        public static string SerializedPropertyDisplayValue(SerializedProperty prop)
        {
            if (prop == null)
                return "";

            if (prop.isArray && prop.propertyType == SerializedPropertyType.Generic)
            {
                int count;
                return TryGetArraySize(prop, out count) ? "Array (" + count.ToString(CultureInfo.InvariantCulture) + ")" : "Array";
            }

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                case SerializedPropertyType.LayerMask:
                    return prop.intValue.ToString(CultureInfo.InvariantCulture);
                case SerializedPropertyType.Boolean:
                    return prop.boolValue ? "true" : "false";
                case SerializedPropertyType.Float:
                    return prop.floatValue.ToString(CultureInfo.InvariantCulture);
                case SerializedPropertyType.String:
                    return prop.stringValue ?? "";
                case SerializedPropertyType.Enum:
                    return SerializedEnumDisplayName(prop);
                case SerializedPropertyType.ObjectReference:
                    if (prop.objectReferenceValue == null)
                        return "";
                    return SerializedObjectReferencePath(prop.objectReferenceValue);
                case SerializedPropertyType.Vector2:
                    return FormatVector(prop.vector2Value);
                case SerializedPropertyType.Vector3:
                    return FormatVector(prop.vector3Value);
                case SerializedPropertyType.Vector4:
                    return FormatVector(prop.vector4Value);
                case SerializedPropertyType.Quaternion:
                    return FormatVector(prop.quaternionValue.eulerAngles);
                case SerializedPropertyType.Color:
                    return "#" + ColorUtility.ToHtmlStringRGBA(prop.colorValue);
                case SerializedPropertyType.Rect:
                    Rect rect = prop.rectValue;
                    return rect.x.ToString(CultureInfo.InvariantCulture) + ", " +
                           rect.y.ToString(CultureInfo.InvariantCulture) + ", " +
                           rect.width.ToString(CultureInfo.InvariantCulture) + ", " +
                           rect.height.ToString(CultureInfo.InvariantCulture);
                case SerializedPropertyType.ManagedReference:
                    return ManagedReferenceDisplayName(prop.managedReferenceFullTypename);
                default:
                    return prop.hasVisibleChildren ? "Object" : "";
            }
        }

        public static void SetSerializedPropertyValue(SerializedProperty prop, string valueJson)
        {
            if (prop == null)
                throw new Exception("SerializedProperty is required");
            string json = string.IsNullOrWhiteSpace(valueJson) ? "null" : valueJson.Trim();
            SerializedPropertySnapshot restoreSnapshot;
            if (TryParseRestoreSnapshotCommand(json, out restoreSnapshot))
            {
                RestoreSerializedPropertySnapshot(prop, restoreSnapshot);
                return;
            }
            if (!IsSerializedPropertyWritable(prop))
                throw new Exception("SerializedProperty is read only: " + prop.propertyPath);

            if (prop.isArray && prop.propertyType == SerializedPropertyType.Generic)
            {
                ApplySerializedArrayCommand(prop, json);
                return;
            }

            switch (prop.propertyType)
            {
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.ArraySize:
                case SerializedPropertyType.LayerMask:
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
                    prop.objectReferenceValue = ResolveSerializedObjectReference(prop, assetPath);
                    break;
                case SerializedPropertyType.Vector2:
                    Vector2Json v2 = DeserializeJson<Vector2Json>(json);
                    prop.vector2Value = new Vector2(v2.x, v2.y);
                    break;
                case SerializedPropertyType.Vector3:
                    Vector3Json v3 = DeserializeJson<Vector3Json>(json);
                    prop.vector3Value = new Vector3(v3.x, v3.y, v3.z);
                    break;
                case SerializedPropertyType.Vector4:
                    Vector4Json v4 = DeserializeJson<Vector4Json>(json);
                    prop.vector4Value = new Vector4(v4.x, v4.y, v4.z, v4.w);
                    break;
                case SerializedPropertyType.Quaternion:
                    prop.quaternionValue = ParseQuaternionJson(json);
                    break;
                case SerializedPropertyType.Color:
                    prop.colorValue = ParseColorJson(json);
                    break;
                case SerializedPropertyType.Rect:
                    RectJson rect = DeserializeJson<RectJson>(json);
                    prop.rectValue = new Rect(rect.x, rect.y, rect.width, rect.height);
                    break;
                case SerializedPropertyType.ManagedReference:
                    SetManagedReferenceValue(prop, json);
                    break;
                default:
                    throw new Exception("SerializedProperty type is not writable: " + prop.propertyType);
            }
        }

        private static string SerializedObjectReferencePath(UnityEngine.Object obj)
        {
            if (obj == null)
                return "";
            string path = AssetDatabase.GetAssetPath(obj);
            if (string.IsNullOrEmpty(path))
                return obj.name ?? "";
            if (AssetDatabase.IsSubAsset(obj))
            {
                string name = obj.name ?? "";
                if (!string.IsNullOrWhiteSpace(name))
                    return path.TrimEnd('/') + "/" + name.Trim();
            }
            return path;
        }

        private static UnityEngine.Object ResolveSerializedObjectReference(
            SerializedProperty prop,
            string value)
        {
            string reference = (value ?? "").Trim().Replace('\\', '/');
            if (string.IsNullOrWhiteSpace(reference))
                return null;

            Type targetType = ResolveSerializedPropertyFieldType(prop);
            UnityEngine.Object exact = LoadAssetAtPathForSerializedReference(reference, targetType);
            if (exact != null)
                return exact;

            UnityEngine.Object subAsset = ResolveSubAssetReference(reference, targetType);
            if (subAsset != null)
                return subAsset;

            UnityEngine.Object fallback = AssetDatabase.LoadAssetAtPath<UnityEngine.Object>(reference);
            return IsSerializedObjectReferenceCompatible(fallback, targetType) ? fallback : null;
        }

        private static UnityEngine.Object LoadAssetAtPathForSerializedReference(
            string assetPath,
            Type targetType)
        {
            if (targetType != null && typeof(UnityEngine.Object).IsAssignableFrom(targetType))
            {
                UnityEngine.Object typed = AssetDatabase.LoadAssetAtPath(assetPath, targetType) as UnityEngine.Object;
                if (typed != null)
                    return typed;
            }
            UnityEngine.Object fallback = AssetDatabase.LoadAssetAtPath<UnityEngine.Object>(assetPath);
            return IsSerializedObjectReferenceCompatible(fallback, targetType) ? fallback : null;
        }

        private static UnityEngine.Object ResolveSubAssetReference(string reference, Type targetType)
        {
            int slash = reference.LastIndexOf('/');
            while (slash > 0)
            {
                string assetPath = reference.Substring(0, slash);
                string subAssetName = reference.Substring(slash + 1).Trim();
                if (!string.IsNullOrWhiteSpace(subAssetName)
                    && AssetDatabase.LoadMainAssetAtPath(assetPath) != null)
                {
                    if (!IsSubAssetReferenceAllowed(assetPath))
                        return null;
                    UnityEngine.Object resolved = FindSubAssetByName(assetPath, subAssetName, targetType);
                    if (resolved != null)
                        return resolved;
                }
                slash = reference.LastIndexOf('/', slash - 1);
            }
            return null;
        }

        private static bool IsSubAssetReferenceAllowed(string assetPath)
        {
            string ext = AssetPathExtension(assetPath);
            return !string.Equals(ext, "playable", StringComparison.OrdinalIgnoreCase);
        }

        private static string AssetPathExtension(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath))
                return "";
            int slash = assetPath.LastIndexOf('/');
            int dot = assetPath.LastIndexOf('.');
            if (dot < 0 || dot < slash)
                return "";
            return assetPath.Substring(dot + 1).Trim();
        }

        private static UnityEngine.Object FindSubAssetByName(
            string assetPath,
            string subAssetName,
            Type targetType)
        {
            UnityEngine.Object[] objects = AssetDatabase.LoadAllAssetsAtPath(assetPath);
            UnityEngine.Object fallback = null;
            UnityEngine.Object caseInsensitiveFallback = null;
            foreach (UnityEngine.Object obj in objects)
            {
                if (obj == null || !AssetDatabase.IsSubAsset(obj))
                    continue;
                string name = obj.name ?? "";
                if (name == subAssetName)
                {
                    if (fallback == null)
                        fallback = obj;
                    if (IsSerializedObjectReferenceCompatible(obj, targetType))
                        return obj;
                }
                else if (caseInsensitiveFallback == null
                    && string.Equals(name, subAssetName, StringComparison.OrdinalIgnoreCase))
                {
                    caseInsensitiveFallback = obj;
                }
            }
            if (fallback != null && IsSerializedObjectReferenceCompatible(fallback, targetType))
                return fallback;
            if (caseInsensitiveFallback != null
                && IsSerializedObjectReferenceCompatible(caseInsensitiveFallback, targetType))
                return caseInsensitiveFallback;
            return null;
        }

        private static bool IsSerializedObjectReferenceCompatible(UnityEngine.Object obj, Type targetType)
        {
            if (obj == null)
                return false;
            if (targetType == null || !typeof(UnityEngine.Object).IsAssignableFrom(targetType))
                return true;
            return targetType.IsAssignableFrom(obj.GetType());
        }

        private sealed class RectJson
        {
            public float x;
            public float y;
            public float width;
            public float height;
        }

        private static bool TryParseRestoreSnapshotCommand(
            string json,
            out SerializedPropertySnapshot snapshot)
        {
            snapshot = null;
            if (string.IsNullOrWhiteSpace(json) || !json.TrimStart().StartsWith("{", StringComparison.Ordinal))
                return false;

            SerializedPropertyRestoreCommand command;
            try
            {
                command = DeserializeJson<SerializedPropertyRestoreCommand>(json);
            }
            catch
            {
                return false;
            }

            string action = (command != null ? command.action : null) ?? "";
            if (!string.Equals(action.Trim(), "restoreSnapshot", StringComparison.OrdinalIgnoreCase))
                return false;
            if (command.snapshot == null)
                throw new Exception("restoreSnapshot requires a snapshot");

            snapshot = command.snapshot;
            return true;
        }

        private static void RestoreSerializedPropertySnapshot(
            SerializedProperty prop,
            SerializedPropertySnapshot snapshot)
        {
            if (prop == null)
                throw new Exception("SerializedProperty is required");
            if (snapshot == null)
                throw new Exception("SerializedProperty snapshot is required");

            if (snapshot.isArray && prop.isArray && prop.propertyType == SerializedPropertyType.Generic)
            {
                int targetSize = Math.Max(0, snapshot.arraySize);
                prop.arraySize = targetSize;
                SerializedPropertySnapshot[] children = snapshot.children ?? new SerializedPropertySnapshot[0];
                int count = Math.Min(targetSize, children.Length);
                for (int i = 0; i < count; i++)
                {
                    SerializedProperty element = prop.GetArrayElementAtIndex(i);
                    if (element != null)
                        RestoreSerializedPropertySnapshot(element, children[i]);
                }
                return;
            }

            if (prop.propertyType == SerializedPropertyType.ManagedReference)
            {
                if (string.IsNullOrWhiteSpace(snapshot.managedReferenceFullTypename))
                    prop.managedReferenceValue = null;
                else
                    SetManagedReferenceValue(prop, ToJsonValue(new Dictionary<string, object>
                    {
                        { "action", "setType" },
                        { "typeName", snapshot.managedReferenceFullTypename }
                    }, 0));
                RestoreSerializedPropertyChildren(prop, snapshot);
                return;
            }

            if (prop.propertyType == SerializedPropertyType.Generic && snapshot.children != null && snapshot.children.Length > 0)
            {
                RestoreSerializedPropertyChildren(prop, snapshot);
                return;
            }

            if (!IsSerializedPropertyWritable(prop))
                throw new Exception("SerializedProperty is read only: " + prop.propertyPath);
            SetSerializedPropertyValue(prop, SerializedPropertySnapshotValueJson(snapshot));
        }

        private static void RestoreSerializedPropertyChildren(
            SerializedProperty prop,
            SerializedPropertySnapshot snapshot)
        {
            SerializedPropertySnapshot[] children = snapshot.children ?? new SerializedPropertySnapshot[0];
            for (int i = 0; i < children.Length; i++)
            {
                SerializedPropertySnapshot childSnapshot = children[i];
                if (childSnapshot == null || string.IsNullOrWhiteSpace(childSnapshot.propertyPath))
                    continue;
                SerializedProperty child = prop.serializedObject.FindProperty(childSnapshot.propertyPath);
                if (child != null)
                    RestoreSerializedPropertySnapshot(child, childSnapshot);
            }
        }

        private static string SerializedPropertySnapshotValueJson(SerializedPropertySnapshot snapshot)
        {
            string valueType = FirstNonEmpty(snapshot.valueType, snapshot.type);
            switch (valueType)
            {
                case "Integer":
                case "ArraySize":
                case "LayerMask":
                case "Float":
                    return snapshot.displayValue ?? "0";
                case "Boolean":
                    return string.Equals(snapshot.displayValue, "true", StringComparison.OrdinalIgnoreCase)
                        ? "true"
                        : "false";
                case "String":
                case "ObjectReference":
                case "Color":
                    return ToJsonValue(snapshot.displayValue ?? "", 0);
                case "Enum":
                    return ToJsonValue(new Dictionary<string, object>
                    {
                        { "action", snapshot.isFlagsEnum ? "setFlags" : "setIndex" },
                        { "index", snapshot.enumValueIndex },
                        { "numericValue", snapshot.enumValueFlag },
                        { "flagValue", snapshot.enumValueFlag },
                        { "label", snapshot.displayValue ?? "" },
                        { "name", snapshot.displayValue ?? "" }
                    }, 0);
                case "Vector2":
                    return SnapshotVectorJson(snapshot.displayValue, new[] { "x", "y" });
                case "Vector3":
                    return SnapshotVectorJson(snapshot.displayValue, new[] { "x", "y", "z" });
                case "Vector4":
                    return SnapshotVectorJson(snapshot.displayValue, new[] { "x", "y", "z", "w" });
                case "Quaternion":
                    return SnapshotQuaternionEulerJson(snapshot.displayValue);
                case "Rect":
                    return SnapshotVectorJson(snapshot.displayValue, new[] { "x", "y", "width", "height" });
                default:
                    return ToJsonValue(snapshot.displayValue ?? "", 0);
            }
        }

        private static string SnapshotVectorJson(string displayValue, string[] keys)
        {
            string[] parts = (displayValue ?? "")
                .Split(new[] { ',', ' ', '\t', '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries);
            var values = new Dictionary<string, object>();
            for (int i = 0; i < keys.Length; i++)
            {
                float parsed = 0f;
                if (i < parts.Length)
                    float.TryParse(parts[i], NumberStyles.Float, CultureInfo.InvariantCulture, out parsed);
                values[keys[i]] = parsed;
            }
            return ToJsonValue(values, 0);
        }

        private static string SnapshotQuaternionEulerJson(string displayValue)
        {
            string vectorJson = SnapshotVectorJson(displayValue, new[] { "x", "y", "z" });
            return "{\"action\":\"setEuler\"," + vectorJson.TrimStart('{');
        }

        private static SerializedPropertySnapshot SnapshotSerializedProperty(
            SerializedProperty prop,
            int depth,
            int maxDepth,
            int maxArrayItems)
        {
            bool isArray = prop.isArray && prop.propertyType == SerializedPropertyType.Generic;
            int arraySize = -1;
            if (isArray)
                TryGetArraySize(prop, out arraySize);

            Type fieldType = ResolveSerializedPropertyFieldType(prop);
            FieldInfo fieldInfo = ResolveSerializedPropertyFieldInfo(prop);
            var snapshot = new SerializedPropertySnapshot
            {
                propertyPath = prop.propertyPath,
                displayName = prop.displayName,
                name = prop.name,
                type = prop.propertyType.ToString(),
                valueType = prop.propertyType.ToString(),
                fieldTypeFullName = FieldTypeFullName(fieldType),
                fieldTypeAssembly = FieldTypeAssembly(fieldType),
                value = SerializedPropertyValue(prop),
                displayValue = SerializedPropertyDisplayValue(prop),
                editable = IsSerializedPropertyWritable(prop),
                isArray = isArray,
                arraySize = arraySize,
                isFlagsEnum = prop.propertyType == SerializedPropertyType.Enum && IsFlagsEnum(prop),
                enumValueIndex = prop.propertyType == SerializedPropertyType.Enum ? prop.enumValueIndex : -1,
                enumValueFlag = prop.propertyType == SerializedPropertyType.Enum ? SerializedEnumNumericValue(prop) : 0,
                enumOptions = prop.propertyType == SerializedPropertyType.Enum
                    ? SerializedEnumOptions(prop)
                    : new SerializedEnumOption[0],
                children = new SerializedPropertySnapshot[0],
                isManagedReference = prop.propertyType == SerializedPropertyType.ManagedReference,
                managedReferenceFullTypename = prop.propertyType == SerializedPropertyType.ManagedReference ? prop.managedReferenceFullTypename ?? "" : "",
                managedReferenceFieldTypename = prop.propertyType == SerializedPropertyType.ManagedReference ? prop.managedReferenceFieldTypename ?? "" : "",
                managedReferenceDisplayName = prop.propertyType == SerializedPropertyType.ManagedReference ? ManagedReferenceDisplayName(prop.managedReferenceFullTypename) : "",
                managedReferenceTypes = prop.propertyType == SerializedPropertyType.ManagedReference
                    ? ManagedReferenceTypeOptions(prop)
                    : new SerializedManagedReferenceTypeOption[0],
                tooltip = SerializedFieldTooltip(fieldInfo),
                header = SerializedFieldHeader(fieldInfo),
                hasRange = HasSerializedFieldRange(fieldInfo),
                rangeMin = SerializedFieldRangeMin(fieldInfo),
                rangeMax = SerializedFieldRangeMax(fieldInfo),
                numberStep = SerializedFieldNumberStep(fieldInfo, prop),
                multiline = IsSerializedFieldMultiline(fieldInfo),
                minLines = SerializedFieldMinLines(fieldInfo),
                maxLines = SerializedFieldMaxLines(fieldInfo),
                referenceTypeFullName = prop.propertyType == SerializedPropertyType.ObjectReference
                    ? FieldTypeFullName(fieldType)
                    : "",
                referenceTypeAssembly = prop.propertyType == SerializedPropertyType.ObjectReference
                    ? FieldTypeAssembly(fieldType)
                    : "",
                attributes = SerializedFieldAttributes(fieldInfo)
            };

            if (depth < maxDepth)
                snapshot.children = SnapshotChildren(prop, depth, maxDepth, maxArrayItems, isArray, arraySize);

            snapshot.hasChildren = snapshot.children != null && snapshot.children.Length > 0;
            return snapshot;
        }

        private static SerializedPropertySnapshot[] SnapshotChildren(
            SerializedProperty prop,
            int depth,
            int maxDepth,
            int maxArrayItems,
            bool isArray,
            int arraySize)
        {
            if (isArray)
            {
                if (arraySize < 0)
                    return new SerializedPropertySnapshot[0];

                int shown = Math.Min(arraySize, maxArrayItems);
                var items = new List<SerializedPropertySnapshot>(shown);
                for (int i = 0; i < shown; i++)
                {
                    try
                    {
                        SerializedProperty element = prop.GetArrayElementAtIndex(i);
                        if (element != null)
                            items.Add(SnapshotSerializedProperty(element, depth + 1, maxDepth, maxArrayItems));
                    }
                    catch
                    {
                    }
                }
                return items.ToArray();
            }

            if (!prop.hasVisibleChildren)
                return new SerializedPropertySnapshot[0];

            var children = new List<SerializedPropertySnapshot>();
            SerializedProperty cursor = prop.Copy();
            SerializedProperty end = cursor.GetEndProperty();
            bool enterChildren = true;
            while (cursor.NextVisible(enterChildren) && (end == null || !SerializedProperty.EqualContents(cursor, end)))
            {
                children.Add(SnapshotSerializedProperty(cursor, depth + 1, maxDepth, maxArrayItems));
                enterChildren = false;
            }
            return children.ToArray();
        }

        private static bool TryGetArraySize(SerializedProperty prop, out int arraySize)
        {
            try
            {
                arraySize = prop.arraySize;
                return true;
            }
            catch
            {
                arraySize = -1;
                return false;
            }
        }

        private static void ApplySerializedArrayCommand(SerializedProperty prop, string json)
        {
            if (!json.StartsWith("{", StringComparison.Ordinal))
            {
                prop.arraySize = Math.Max(0, ParseIntJson(json));
                return;
            }

            SerializedArrayWriteCommand command = DeserializeJson<SerializedArrayWriteCommand>(json);
            string action = (command != null ? command.action : null) ?? "";
            action = action.Trim().ToLowerInvariant();

            if (string.IsNullOrEmpty(action) || action == "resize")
            {
                int size = Math.Max(0, command != null ? command.size : 0);
                prop.arraySize = size;
                return;
            }

            if (action == "insert" || action == "add")
            {
                int index = ClampArrayIndex(command.index, prop.arraySize, true);
                prop.InsertArrayElementAtIndex(index);
                return;
            }

            if (action == "delete" || action == "remove")
            {
                int index = ClampArrayIndex(command.index, prop.arraySize, false);
                int before = prop.arraySize;
                prop.DeleteArrayElementAtIndex(index);
                if (prop.arraySize == before && index >= 0 && index < prop.arraySize)
                    prop.DeleteArrayElementAtIndex(index);
                return;
            }

            if (action == "move")
            {
                int from = ClampArrayIndex(command.index, prop.arraySize, false);
                int to = ClampArrayIndex(command.toIndex, prop.arraySize, false);
                prop.MoveArrayElement(from, to);
                return;
            }

            if (action == "clear")
            {
                prop.ClearArray();
                return;
            }

            throw new Exception("Unsupported array action: " + action);
        }

        private static int ClampArrayIndex(int index, int arraySize, bool allowEnd)
        {
            int max = allowEnd ? arraySize : arraySize - 1;
            if (max < 0)
                throw new Exception("Array is empty");
            if (index < 0 || index > max)
                throw new Exception("Array index out of range: " + index.ToString(CultureInfo.InvariantCulture));
            return index;
        }

        private static void SetManagedReferenceValue(SerializedProperty prop, string json)
        {
            string text = TrimJsonString(json);
            if (string.Equals(json, "null", StringComparison.OrdinalIgnoreCase) || string.IsNullOrWhiteSpace(text))
            {
                prop.managedReferenceValue = null;
                return;
            }

            SerializedManagedReferenceWriteCommand command = null;
            if (json.StartsWith("{", StringComparison.Ordinal))
                command = DeserializeJson<SerializedManagedReferenceWriteCommand>(json);

            string action = (command != null ? command.action : null) ?? "";
            action = action.Trim().ToLowerInvariant();
            if (action == "clear" || action == "null")
            {
                prop.managedReferenceValue = null;
                return;
            }

            string typeName = command != null
                ? FirstNonEmpty(command.typeName, command.type, CombineManagedReferenceTypeName(command.assembly, command.fullName))
                : text;
            if (string.IsNullOrWhiteSpace(typeName))
                throw new Exception("Managed reference type is required");

            Type fieldType = ResolveManagedReferenceTypeName(prop.managedReferenceFieldTypename);
            Type concreteType = ResolveManagedReferenceTypeName(typeName);
            if (concreteType == null)
                throw new Exception("Managed reference type not found: " + typeName);
            if (fieldType != null && !fieldType.IsAssignableFrom(concreteType))
                throw new Exception("Managed reference type is not assignable to " + fieldType.FullName + ": " + concreteType.FullName);
            if (!IsManagedReferenceConcreteType(concreteType))
                throw new Exception("Managed reference type is not serializable: " + concreteType.FullName);

            prop.managedReferenceValue = CreateManagedReferenceInstance(concreteType);
        }

        private static object CreateManagedReferenceInstance(Type type)
        {
            try
            {
                return Activator.CreateInstance(type);
            }
            catch
            {
                return FormatterServices.GetUninitializedObject(type);
            }
        }

        private static SerializedManagedReferenceTypeOption[] ManagedReferenceTypeOptions(SerializedProperty prop)
        {
            Type fieldType = ResolveManagedReferenceTypeName(prop.managedReferenceFieldTypename);
            if (fieldType == null)
                return new SerializedManagedReferenceTypeOption[0];

            var types = new List<Type>();
            if (IsManagedReferenceConcreteType(fieldType))
                types.Add(fieldType);
            types.AddRange(TypeCache.GetTypesDerivedFrom(fieldType));

            Type currentType = ResolveManagedReferenceTypeName(prop.managedReferenceFullTypename);
            var result = types
                .Where(IsManagedReferenceConcreteType)
                .Distinct()
                .OrderBy(type => type.FullName, StringComparer.Ordinal)
                .Take(200)
                .ToList();

            if (currentType != null &&
                IsManagedReferenceConcreteType(currentType) &&
                fieldType.IsAssignableFrom(currentType) &&
                !result.Contains(currentType))
            {
                result.Insert(0, currentType);
            }

            return result
                .Select(ManagedReferenceTypeOption)
                .ToArray();
        }

        private static SerializedManagedReferenceTypeOption ManagedReferenceTypeOption(Type type)
        {
            return new SerializedManagedReferenceTypeOption
            {
                label = type.FullName ?? type.Name,
                value = ManagedReferenceTypeName(type),
                fullName = type.FullName ?? type.Name,
                assembly = type.Assembly.GetName().Name
            };
        }

        private static bool IsManagedReferenceConcreteType(Type type)
        {
            return type != null &&
                   !type.IsAbstract &&
                   !type.IsInterface &&
                   !type.IsGenericTypeDefinition &&
                   !type.ContainsGenericParameters &&
                   !typeof(UnityEngine.Object).IsAssignableFrom(type) &&
                   (type.IsSerializable || Attribute.IsDefined(type, typeof(SerializableAttribute), false));
        }

        public static bool TypeMatches(Type type, string expected)
        {
            if (type == null)
                return false;
            if (string.IsNullOrWhiteSpace(expected))
                return true;

            string target = expected.Trim();
            for (Type current = type; current != null; current = current.BaseType)
            {
                if (string.Equals(current.Name, target, StringComparison.Ordinal) ||
                    string.Equals(current.FullName, target, StringComparison.Ordinal))
                    return true;
            }

            Type[] interfaces = type.GetInterfaces();
            for (int i = 0; i < interfaces.Length; i++)
            {
                Type interfaceType = interfaces[i];
                if (string.Equals(interfaceType.Name, target, StringComparison.Ordinal) ||
                    string.Equals(interfaceType.FullName, target, StringComparison.Ordinal))
                    return true;
            }
            return false;
        }

        private static Type ResolveManagedReferenceTypeName(string typeName)
        {
            typeName = (typeName ?? "").Trim();
            if (string.IsNullOrEmpty(typeName))
                return null;

            Type direct = Type.GetType(typeName);
            if (direct != null)
                return direct;

            string assemblyName;
            string fullName;
            SplitManagedReferenceTypeName(typeName, out assemblyName, out fullName);

            foreach (var assembly in AppDomain.CurrentDomain.GetAssemblies())
            {
                if (assembly == null || assembly.IsDynamic)
                    continue;
                if (!string.IsNullOrEmpty(assemblyName) &&
                    !string.Equals(assembly.GetName().Name, assemblyName, StringComparison.Ordinal))
                    continue;

                Type type = assembly.GetType(fullName);
                if (type != null)
                    return type;
            }

            return null;
        }

        private static string ManagedReferenceTypeName(Type type)
        {
            return type.Assembly.GetName().Name + " " + (type.FullName ?? type.Name);
        }

        private static string ManagedReferenceDisplayName(string typeName)
        {
            string assemblyName;
            string fullName;
            SplitManagedReferenceTypeName(typeName, out assemblyName, out fullName);
            if (string.IsNullOrEmpty(fullName))
                return "";
            int dot = fullName.LastIndexOf('.');
            return dot >= 0 ? fullName.Substring(dot + 1) : fullName;
        }

        private static string CombineManagedReferenceTypeName(string assemblyName, string fullName)
        {
            assemblyName = (assemblyName ?? "").Trim();
            fullName = (fullName ?? "").Trim();
            if (string.IsNullOrEmpty(fullName))
                return "";
            return string.IsNullOrEmpty(assemblyName) ? fullName : assemblyName + " " + fullName;
        }

        private static void SplitManagedReferenceTypeName(string typeName, out string assemblyName, out string fullName)
        {
            typeName = (typeName ?? "").Trim();
            int space = typeName.IndexOf(' ');
            if (space > 0)
            {
                assemblyName = typeName.Substring(0, space).Trim();
                fullName = typeName.Substring(space + 1).Trim();
                return;
            }

            int comma = typeName.IndexOf(',');
            if (comma > 0)
            {
                fullName = typeName.Substring(0, comma).Trim();
                assemblyName = typeName.Substring(comma + 1).Trim().Split(',')[0].Trim();
                return;
            }

            assemblyName = "";
            fullName = typeName;
        }

        private static string FirstNonEmpty(params string[] values)
        {
            for (int i = 0; i < values.Length; i++)
            {
                string value = values[i];
                if (!string.IsNullOrWhiteSpace(value))
                    return value.Trim();
            }
            return "";
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
                throw new Exception("Expected number value");
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

        private static Quaternion ParseQuaternionJson(string json)
        {
            QuaternionJson value = DeserializeJson<QuaternionJson>(json);
            if (value == null)
                return Quaternion.identity;
            string action = (value != null ? value.action : null) ?? "";
            action = action.Trim().ToLowerInvariant();
            if (action == "seteuler" || action == "euler" || !JsonObjectHasKey(json, "w"))
                return Quaternion.Euler(value.x, value.y, value.z);
            return new Quaternion(value.x, value.y, value.z, value.w);
        }

        private static Color ParseColorJson(string json)
        {
            string text = TrimJsonString(json);
            Color color;
            if (!string.IsNullOrWhiteSpace(text) && ColorUtility.TryParseHtmlString(text, out color))
                return color;

            ColorJson value = DeserializeJson<ColorJson>(json);
            return new Color(value.r, value.g, value.b, value.a);
        }

        private static void SetEnumValue(SerializedProperty prop, string json)
        {
            string text = TrimJsonString(json);
            bool isFlags = IsFlagsEnum(prop);

            if (json.StartsWith("{", StringComparison.Ordinal))
            {
                SerializedEnumWriteCommand command = DeserializeJson<SerializedEnumWriteCommand>(json);
                string action = (command != null ? command.action : null) ?? "";
                action = action.Trim().ToLowerInvariant();
                if (isFlags || action == "setflags" || action == "flags" || action == "setflag")
                {
                    prop.enumValueFlag = (int)FirstNonZero(command.flagValue, command.numericValue, ParseLongOrDefault(command.value));
                    return;
                }
                if (command.index >= 0)
                {
                    prop.enumValueIndex = command.index;
                    return;
                }
                text = FirstNonEmpty(command.name, command.label, command.value);
            }

            int index;
            if (int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out index))
            {
                if (isFlags)
                    prop.enumValueFlag = index;
                else
                    prop.enumValueIndex = index;
                return;
            }

            string[] displayNames = prop.enumDisplayNames;
            if (displayNames != null)
            {
                for (int i = 0; i < displayNames.Length; i++)
                {
                    if (string.Equals(displayNames[i], text, StringComparison.OrdinalIgnoreCase))
                    {
                        prop.enumValueIndex = i;
                        return;
                    }
                }
            }

            string[] names = prop.enumNames;
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

        private static string SerializedEnumDisplayName(SerializedProperty prop)
        {
            string[] names = prop.enumDisplayNames;
            int index = prop.enumValueIndex;
            if (names != null && index >= 0 && index < names.Length)
                return names[index];
            return index.ToString(CultureInfo.InvariantCulture);
        }

        private static long SerializedEnumNumericValue(SerializedProperty prop)
        {
            try
            {
                if (IsFlagsEnum(prop))
                    return prop.enumValueFlag;
            }
            catch
            {
            }

            try
            {
                return prop.intValue;
            }
            catch
            {
                return prop.enumValueIndex;
            }
        }

        private static SerializedEnumOption[] SerializedEnumOptions(SerializedProperty prop)
        {
            string[] displayNames = prop.enumDisplayNames ?? new string[0];
            string[] names = prop.enumNames ?? new string[0];
            Type enumType = ResolveSerializedPropertyFieldType(prop);
            Array enumValues = enumType != null && enumType.IsEnum ? Enum.GetValues(enumType) : null;
            string[] enumNames = enumType != null && enumType.IsEnum ? Enum.GetNames(enumType) : null;
            int count = Math.Max(displayNames.Length, names.Length);
            if (enumValues != null)
                count = Math.Max(count, enumValues.Length);

            var options = new List<SerializedEnumOption>(count);
            for (int i = 0; i < count; i++)
            {
                string name = FirstNonEmpty(
                    enumNames != null && i < enumNames.Length ? enumNames[i] : "",
                    i < names.Length ? names[i] : "",
                    i < displayNames.Length ? displayNames[i] : "",
                    i.ToString(CultureInfo.InvariantCulture));
                string label = FirstNonEmpty(
                    i < displayNames.Length ? displayNames[i] : "",
                    name);
                long numericValue = i;
                if (enumValues != null && i < enumValues.Length)
                {
                    try
                    {
                        numericValue = Convert.ToInt64(enumValues.GetValue(i), CultureInfo.InvariantCulture);
                    }
                    catch
                    {
                        numericValue = i;
                    }
                }

                options.Add(new SerializedEnumOption
                {
                    label = label,
                    value = i.ToString(CultureInfo.InvariantCulture),
                    name = name,
                    index = i,
                    numericValue = numericValue
                });
            }
            return options.ToArray();
        }

        private static bool IsFlagsEnum(SerializedProperty prop)
        {
            Type type = ResolveSerializedPropertyFieldType(prop);
            return type != null && type.IsEnum && Attribute.IsDefined(type, typeof(FlagsAttribute), false);
        }

        private static string SerializedFieldTooltip(FieldInfo field)
        {
            TooltipAttribute attr = field != null
                ? field.GetCustomAttribute<TooltipAttribute>(true)
                : null;
            return attr != null ? attr.tooltip ?? "" : "";
        }

        private static string SerializedFieldHeader(FieldInfo field)
        {
            HeaderAttribute attr = field != null
                ? field.GetCustomAttribute<HeaderAttribute>(true)
                : null;
            return attr != null ? attr.header ?? "" : "";
        }

        private static bool HasSerializedFieldRange(FieldInfo field)
        {
            return field != null && field.GetCustomAttribute<RangeAttribute>(true) != null;
        }

        private static float SerializedFieldRangeMin(FieldInfo field)
        {
            RangeAttribute attr = field != null
                ? field.GetCustomAttribute<RangeAttribute>(true)
                : null;
            return attr != null ? attr.min : 0f;
        }

        private static float SerializedFieldRangeMax(FieldInfo field)
        {
            RangeAttribute attr = field != null
                ? field.GetCustomAttribute<RangeAttribute>(true)
                : null;
            return attr != null ? attr.max : 0f;
        }

        private static float SerializedFieldNumberStep(FieldInfo field, SerializedProperty prop)
        {
            if (prop != null && prop.propertyType == SerializedPropertyType.Integer)
                return 1f;
            return 0f;
        }

        private static bool IsSerializedFieldMultiline(FieldInfo field)
        {
            return field != null &&
                   (field.GetCustomAttribute<TextAreaAttribute>(true) != null ||
                    field.GetCustomAttribute<MultilineAttribute>(true) != null);
        }

        private static int SerializedFieldMinLines(FieldInfo field)
        {
            if (field == null)
                return 0;
            TextAreaAttribute textArea = field.GetCustomAttribute<TextAreaAttribute>(true);
            if (textArea != null)
                return textArea.minLines;
            MultilineAttribute multiline = field.GetCustomAttribute<MultilineAttribute>(true);
            return multiline != null ? multiline.lines : 0;
        }

        private static int SerializedFieldMaxLines(FieldInfo field)
        {
            if (field == null)
                return 0;
            TextAreaAttribute textArea = field.GetCustomAttribute<TextAreaAttribute>(true);
            return textArea != null ? textArea.maxLines : 0;
        }

        private static SerializedPropertyAttributeInfo[] SerializedFieldAttributes(FieldInfo field)
        {
            if (field == null)
                return new SerializedPropertyAttributeInfo[0];

            return field
                .GetCustomAttributes(true)
                .OfType<Attribute>()
                .Take(32)
                .Select(SerializedFieldAttributeInfo)
                .ToArray();
        }

        private static SerializedPropertyAttributeInfo SerializedFieldAttributeInfo(Attribute attr)
        {
            string value = "";
            RangeAttribute range = attr as RangeAttribute;
            TooltipAttribute tooltip = attr as TooltipAttribute;
            HeaderAttribute header = attr as HeaderAttribute;
            TextAreaAttribute textArea = attr as TextAreaAttribute;
            MultilineAttribute multiline = attr as MultilineAttribute;
            MinAttribute min = attr as MinAttribute;
            if (range != null)
                value = range.min.ToString(CultureInfo.InvariantCulture) + ".." + range.max.ToString(CultureInfo.InvariantCulture);
            else if (tooltip != null)
                value = tooltip.tooltip ?? "";
            else if (header != null)
                value = header.header ?? "";
            else if (textArea != null)
                value = textArea.minLines.ToString(CultureInfo.InvariantCulture) + ".." + textArea.maxLines.ToString(CultureInfo.InvariantCulture);
            else if (multiline != null)
                value = multiline.lines.ToString(CultureInfo.InvariantCulture);
            else if (min != null)
                value = min.min.ToString(CultureInfo.InvariantCulture);

            Type type = attr.GetType();
            return new SerializedPropertyAttributeInfo
            {
                type = type.FullName ?? type.Name,
                displayName = type.Name,
                value = value
            };
        }

        private static Type ResolveSerializedPropertyFieldType(SerializedProperty prop)
        {
            if (prop == null || prop.serializedObject == null || prop.serializedObject.targetObject == null)
                return null;

            Type builtInType = ResolveBuiltInSerializedPropertyFieldType(prop);
            if (builtInType != null)
                return builtInType;

            Type current = prop.serializedObject.targetObject.GetType();
            string[] parts = (prop.propertyPath ?? "").Replace(".Array.data[", "[").Split('.');
            for (int i = 0; i < parts.Length; i++)
            {
                string part = parts[i];
                if (string.IsNullOrEmpty(part))
                    continue;

                int bracket = part.IndexOf('[');
                string memberName = bracket >= 0 ? part.Substring(0, bracket) : part;
                if (!string.IsNullOrEmpty(memberName))
                    current = SerializedMemberType(current, memberName);
                if (current == null)
                    return null;

                while (bracket >= 0)
                {
                    current = SerializedElementType(current);
                    if (current == null)
                        return null;
                    bracket = part.IndexOf('[', bracket + 1);
                }
            }
            return current;
        }

        private static FieldInfo ResolveSerializedPropertyFieldInfo(SerializedProperty prop)
        {
            if (prop == null || prop.serializedObject == null || prop.serializedObject.targetObject == null)
                return null;

            Type current = prop.serializedObject.targetObject.GetType();
            FieldInfo field = null;
            string[] parts = (prop.propertyPath ?? "").Replace(".Array.data[", "[").Split('.');
            for (int i = 0; i < parts.Length; i++)
            {
                string part = parts[i];
                if (string.IsNullOrEmpty(part))
                    continue;

                int bracket = part.IndexOf('[');
                string memberName = bracket >= 0 ? part.Substring(0, bracket) : part;
                if (!string.IsNullOrEmpty(memberName))
                {
                    field = SerializedMemberField(current, memberName);
                    current = field != null ? field.FieldType : SerializedMemberType(current, memberName);
                }
                if (current == null)
                    return field;

                while (bracket >= 0)
                {
                    current = SerializedElementType(current);
                    if (current == null)
                        return field;
                    bracket = part.IndexOf('[', bracket + 1);
                }
            }
            return field;
        }

        private static string FieldTypeFullName(Type type)
        {
            return type != null ? type.FullName ?? type.Name ?? "" : "";
        }

        private static string FieldTypeAssembly(Type type)
        {
            try
            {
                return type != null && type.Assembly != null ? type.Assembly.GetName().Name ?? "" : "";
            }
            catch
            {
                return "";
            }
        }

        private static Type SerializedMemberType(Type ownerType, string memberName)
        {
            for (Type current = ownerType; current != null; current = current.BaseType)
            {
                var field = SerializedMemberField(current, memberName);
                if (field != null)
                    return field.FieldType;

                var property = current.GetProperty(
                    memberName,
                    BindingFlags.Instance |
                    BindingFlags.Public |
                    BindingFlags.NonPublic);
                if (property != null)
                    return property.PropertyType;
            }
            return null;
        }

        private static FieldInfo SerializedMemberField(Type ownerType, string memberName)
        {
            for (Type current = ownerType; current != null; current = current.BaseType)
            {
                var field = current.GetField(
                    memberName,
                    BindingFlags.Instance |
                    BindingFlags.Public |
                    BindingFlags.NonPublic);
                if (field != null)
                    return field;
            }
            return null;
        }

        private static Type SerializedElementType(Type type)
        {
            if (type == null)
                return null;
            if (type.IsArray)
                return type.GetElementType();
            if (type.IsGenericType)
                return type.GetGenericArguments().FirstOrDefault();
            return null;
        }

        private static long ParseLongOrDefault(string value)
        {
            long parsed;
            return long.TryParse((value ?? "").Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out parsed)
                ? parsed
                : 0;
        }

        private static long FirstNonZero(params long[] values)
        {
            for (int i = 0; i < values.Length; i++)
            {
                if (values[i] != 0)
                    return values[i];
            }
            return 0;
        }

        private static string TrimJsonString(string json)
        {
            if (string.IsNullOrWhiteSpace(json) || string.Equals(json, "null", StringComparison.OrdinalIgnoreCase))
                return "";

            json = json.Trim();
            if (json.Length >= 2 && json[0] == '"' && json[json.Length - 1] == '"')
            {
                try
                {
                    return Locus.Json.LocusJson.Deserialize<string>(json) ?? "";
                }
                catch
                {
                    return UnescapeJsonString(json.Substring(1, json.Length - 2));
                }
            }
            return json;
        }

        private static T DeserializeJson<T>(string json)
        {
            return Locus.Json.LocusJson.Deserialize<T>(string.IsNullOrWhiteSpace(json) ? "{}" : json);
        }

        private static bool JsonObjectHasKey(string json, string key)
        {
            string text = json ?? "";
            string quotedKey = "\"" + key + "\"";
            return text.IndexOf(quotedKey, StringComparison.Ordinal) >= 0;
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

        private static Dictionary<string, object> VectorValue(Vector2 value)
        {
            return new Dictionary<string, object>
            {
                { "x", value.x },
                { "y", value.y }
            };
        }

        private static Dictionary<string, object> VectorValue(Vector3 value)
        {
            return new Dictionary<string, object>
            {
                { "x", value.x },
                { "y", value.y },
                { "z", value.z }
            };
        }

        private static Dictionary<string, object> VectorValue(Vector4 value)
        {
            return new Dictionary<string, object>
            {
                { "x", value.x },
                { "y", value.y },
                { "z", value.z },
                { "w", value.w }
            };
        }

        private static Dictionary<string, object> QuaternionValue(Quaternion value)
        {
            return new Dictionary<string, object>
            {
                { "x", value.x },
                { "y", value.y },
                { "z", value.z },
                { "w", value.w }
            };
        }

        private static string FormatVector(Vector2 value)
        {
            return value.x.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.y.ToString(CultureInfo.InvariantCulture);
        }

        private static string FormatVector(Vector3 value)
        {
            return value.x.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.y.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.z.ToString(CultureInfo.InvariantCulture);
        }

        private static string FormatVector(Vector4 value)
        {
            return value.x.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.y.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.z.ToString(CultureInfo.InvariantCulture) + ", " +
                   value.w.ToString(CultureInfo.InvariantCulture);
        }
    }
}
