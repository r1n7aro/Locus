using System;
using UnityEditor;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static string ReadPropertyTreeArrayPage(PropertyTreeReadRequest request, bool dynamicSchema)
        {
            var obj = ResolvePropertyTreeObject(request.target);
            var target = PropertyTreeTargetWithLocalFileIds(request.target, obj);
            using (var serialized = new SerializedObject(obj)) {
                serialized.Update(); var prop = serialized.FindProperty(target.propertyPath);
                if (prop == null || !prop.isArray || prop.propertyType != SerializedPropertyType.Generic)
                    throw new InvalidOperationException("Array pagination requires a serialized array property.");
                int start = Math.Min(Math.Max(0, request.arrayOffset), prop.arraySize);
                int count = Math.Min(prop.arraySize - start, request.maxArrayItems > 0 ? Math.Min(request.maxArrayItems, 1024) : 64);
                int depth = request.maxDepth > 0 ? Math.Min(request.maxDepth, 16) : 4;
                var snapshot = SnapshotSerializedProperty(prop, 0, 0, dynamicSchema, false);
                snapshot.children = new SerializedPropertySnapshot[count];
                for (int i = 0; i < count; i++) snapshot.children[i] = SnapshotSerializedProperty(prop.GetArrayElementAtIndex(start + i), depth - 1, 64, dynamicSchema, false);
                snapshot.visibleChildCount = count;
                snapshot.childrenTruncated = start + count < prop.arraySize;
                ApplyPropertyTreeTargetToSnapshotTree(snapshot, ToSerializedPropertyBindingTarget(target));
                return BuildBindingReadJson(request.bindingId, target, snapshot, false);
            }
        }
    }
}
