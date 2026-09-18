using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEngine;

namespace Locus
{
    public static partial class LocusBridge
    {
        // Separate from display snapshots: no depth/item truncation, display strings,
        // schema metadata or loss of managed-reference edges. Kept as JSON text over IPC
        // so 64-bit reference IDs never pass through JavaScript numbers.
        [Serializable]
        public sealed class PropertyRestoreNode
        {
            public string name, type, valueJson, managedType;
            public long managedId;
            public bool isArray, prefabOverride;
            public int arraySize;
            public PropertyRestoreNode[] children;
        }

        private sealed class PropertyObjectReferenceValue { public string globalObjectId; }

        private static void SetPropertyIntegerValue(SerializedProperty prop, string json)
        {
            if (prop.type == "ulong") { prop.ulongValue = ulong.Parse(TrimJsonString(json), System.Globalization.CultureInfo.InvariantCulture); return; }
            long value = long.Parse(TrimJsonString(json), System.Globalization.CultureInfo.InvariantCulture);
            if (prop.type == "long") { prop.longValue = value; return; }
            Type type = ResolveSerializedPropertyFieldType(prop);
            long min = int.MinValue, max = int.MaxValue;
            if (type == typeof(uint) || prop.type == "uint" || prop.type == "unsigned int") { min = 0; max = uint.MaxValue; }
            else if (type == typeof(short)) { min = short.MinValue; max = short.MaxValue; }
            else if (type == typeof(ushort)) { min = 0; max = ushort.MaxValue; }
            else if (type == typeof(byte)) { min = 0; max = byte.MaxValue; }
            else if (type == typeof(sbyte)) { min = sbyte.MinValue; max = sbyte.MaxValue; }
            if (prop.propertyType == SerializedPropertyType.ArraySize) min = 0;
            if (value < min || value > max) throw new ArgumentOutOfRangeException(prop.propertyPath, "Integer is out of range.");
            prop.longValue = value;
        }

        private static string SyntheticSerializedPath(string path) { return path == PropertyTreeGameObjectStaticPropertyPath ? "m_StaticEditorFlags" : path; }

        private static SerializedPropertySnapshot WithSyntheticPropertyRestoreState(UnityEngine.Object obj, SerializedPropertySnapshot snapshot)
        {
            using (var serialized = new SerializedObject(obj)) {
                serialized.Update(); var prop = serialized.FindProperty(SyntheticSerializedPath(snapshot.propertyPath));
                if (prop != null) snapshot.restoreState = Locus.Json.LocusJson.SerializeData(CapturePropertyRestoreState(prop));
            }
            return snapshot;
        }

        private static readonly Dictionary<string, SerializedPropertySnapshot> PropertyPreviewStarts = new Dictionary<string, SerializedPropertySnapshot>();

        private static string PropertyPreviewKey(UnityEngine.Object obj, string path) { return PropertyTreeIdentity(obj) + "|" + path; }

        private static void BeginPropertyPreview(UnityEngine.Object obj, SerializedProperty prop)
        {
            string key = PropertyPreviewKey(obj, prop.propertyPath);
            if (PropertyPreviewStarts.ContainsKey(key)) return;
            PropertyPreviewStarts[key] = SnapshotSerializedProperty(prop);
            Undo.IncrementCurrentGroup();
            Undo.RegisterCompleteObjectUndo(obj, "Locus Property Tree");
        }

        private static SerializedPropertySnapshot TakePropertyBeforeSnapshot(UnityEngine.Object obj, SerializedProperty prop)
        {
            string key = PropertyPreviewKey(obj, prop.propertyPath);
            SerializedPropertySnapshot before;
            if (PropertyPreviewStarts.TryGetValue(key, out before)) { PropertyPreviewStarts.Remove(key); return before; }
            return SnapshotSerializedProperty(prop);
        }

        private static string WithPropertyBeforeSnapshot(string response, SerializedPropertySnapshot before)
        {
            return response.Substring(0, response.Length - 1) + ",\"beforeSnapshot\":" + SerializedPropertySnapshotToJson(before) + "}";
        }

        private static UnityEngine.Object ResolvePropertyReferenceValue(SerializedProperty prop, string json)
        {
            if (json.TrimStart().StartsWith("{", StringComparison.Ordinal)) {
                var reference = DeserializeJson<PropertyObjectReferenceValue>(json);
                if (string.IsNullOrEmpty(reference.globalObjectId)) return null;
                var obj = ResolvePropertyTreeIdentity(reference.globalObjectId);
                if (!IsSerializedObjectReferenceCompatible(obj, ResolveSerializedPropertyFieldType(prop)))
                    throw new InvalidOperationException("Object reference type does not match " + prop.propertyPath);
                return obj;
            }
            return ResolveSerializedObjectReference(prop, ParseStringJson(json));
        }

        private static PropertyRestoreNode CapturePropertyRestoreState(SerializedProperty prop, HashSet<long> visited = null)
        {
            if (visited == null) visited = new HashSet<long>();
            var node = new PropertyRestoreNode {
                name = prop.name, type = prop.propertyType.ToString(), prefabOverride = prop.prefabOverride,
                isArray = prop.isArray && prop.propertyType == SerializedPropertyType.Generic,
                children = new PropertyRestoreNode[0]
            };
            if (prop.propertyType == SerializedPropertyType.ManagedReference)
            {
                node.managedId = prop.managedReferenceId;
                node.managedType = prop.managedReferenceFullTypename;
                if (string.IsNullOrEmpty(node.managedType) || !visited.Add(node.managedId)) return node;
            }
            if (node.isArray)
            {
                node.arraySize = prop.arraySize;
                var children = new PropertyRestoreNode[node.arraySize];
                for (int i = 0; i < children.Length; i++) children[i] = CapturePropertyRestoreState(prop.GetArrayElementAtIndex(i), visited);
                node.children = children;
            }
            else if (prop.propertyType == SerializedPropertyType.Generic || prop.propertyType == SerializedPropertyType.ManagedReference)
            {
                var children = new List<PropertyRestoreNode>();
                SerializedProperty cursor = prop.Copy(), end = cursor.GetEndProperty();
                bool enter = true;
                while (cursor.Next(enter) && !SerializedProperty.EqualContents(cursor, end)) {
                    children.Add(CapturePropertyRestoreState(cursor, visited)); enter = false;
                }
                node.children = children.ToArray();
            }
            else if (prop.propertyType == SerializedPropertyType.ObjectReference || prop.propertyType == SerializedPropertyType.ExposedReference)
            {
                var reference = prop.propertyType == SerializedPropertyType.ObjectReference ? prop.objectReferenceValue : prop.exposedReferenceValue;
                node.valueJson = Locus.Json.LocusJson.SerializeData(new Dictionary<string, object> { { "globalObjectId", PropertyTreeIdentity(reference) } });
            }
            else node.valueJson = Locus.Json.LocusJson.SerializeData(SerializedPropertyValue(prop));
            return node;
        }

        private static Dictionary<long, object> ExistingManagedPropertyReferences(SerializedObject serialized)
        {
            var values = new Dictionary<long, object>();
            SerializedProperty cursor = serialized.GetIterator();
            bool enter = true;
            while (cursor.Next(enter)) {
                enter = true;
                if (cursor.propertyType != SerializedPropertyType.ManagedReference || cursor.managedReferenceValue == null) continue;
                long id = cursor.managedReferenceId;
                if (values.ContainsKey(id)) enter = false;
                else values[id] = cursor.managedReferenceValue;
            }
            return values;
        }

        private static void RestorePropertyState(SerializedProperty prop, PropertyRestoreNode node,
            Dictionary<long, object> references, HashSet<long> restored)
        {
            if (node.type != prop.propertyType.ToString()) throw new InvalidOperationException("Serialized property type changed: " + prop.propertyPath);
            if (node.isArray) {
                prop.arraySize = node.arraySize;
                for (int i = 0; i < node.children.Length; i++) RestorePropertyState(prop.GetArrayElementAtIndex(i), node.children[i], references, restored);
                return;
            }
            if (prop.propertyType == SerializedPropertyType.ManagedReference) {
                if (string.IsNullOrEmpty(node.managedType)) { prop.managedReferenceValue = null; return; }
                object value;
                Type type = ResolveManagedReferenceTypeName(node.managedType);
                if (type == null) throw new InvalidOperationException("Managed reference type no longer exists: " + node.managedType);
                if (!references.TryGetValue(node.managedId, out value) || value.GetType() != type) {
                    value = CreateManagedReferenceInstance(type); references[node.managedId] = value;
                    if (node.managedId >= 0) UnityEngine.Serialization.ManagedReferenceUtility.SetManagedReferenceIdForObject(prop.serializedObject.targetObject, value, node.managedId);
                }
                prop.managedReferenceValue = value;
                if (!restored.Add(node.managedId)) return;
            }
            if (prop.propertyType == SerializedPropertyType.Generic || prop.propertyType == SerializedPropertyType.ManagedReference) {
                foreach (var child in node.children ?? new PropertyRestoreNode[0]) {
                    var property = prop.FindPropertyRelative(child.name);
                    if (property == null) throw new InvalidOperationException("Serialized field no longer exists: " + prop.propertyPath + "." + child.name);
                    RestorePropertyState(property, child, references, restored);
                }
                return;
            }
            SetSerializedPropertyValue(prop, node.valueJson);
        }

        private static void RestorePropertyPrefabState(SerializedProperty prop, PropertyRestoreNode node)
        {
            if (!PrefabUtility.IsPartOfPrefabInstance(prop.serializedObject.targetObject)) return;
            if (!node.prefabOverride && prop.prefabOverride) {
                PrefabUtility.RevertPropertyOverride(prop, InteractionMode.AutomatedAction);
                return;
            }
            if (node.isArray) {
                for (int i = 0; i < node.children.Length && i < prop.arraySize; i++) RestorePropertyPrefabState(prop.GetArrayElementAtIndex(i), node.children[i]);
            } else {
                foreach (var child in node.children ?? new PropertyRestoreNode[0]) {
                    var property = prop.FindPropertyRelative(child.name);
                    if (property != null) RestorePropertyPrefabState(property, child);
                }
            }
        }

        private static void CompletePropertySnapshotRestore(SerializedObject serialized, string propertyPath, string valueJson)
        {
            SerializedPropertySnapshot snapshot;
            if (!TryParseRestoreSnapshotCommand(valueJson, out snapshot) || string.IsNullOrEmpty(snapshot.restoreState)) return;
            var state = DeserializeJson<PropertyRestoreNode>(snapshot.restoreState);
            var prop = serialized.FindProperty(propertyPath);
            if (prop != null) { RestorePropertyPrefabState(prop, state); serialized.Update(); }
        }
    }
}
