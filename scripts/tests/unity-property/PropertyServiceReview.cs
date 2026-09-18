using System;
using System.IO;
using UnityEditor;
using UnityEngine;

namespace PropertyServiceReview
{
    public static class PropertyServiceReview
    {
        [Serializable] public class Report { public string version, error; public bool managedType, managedCycle, nestedValues, liveParity, rejectedUnverified, nativeTopology; }
        public static void Seed()
        {
            EditorSettings.serializationMode = SerializationMode.ForceText;
            var data = ScriptableObject.CreateInstance<ServiceData>();
            var group = new ServiceData.Group();
            group.items.Add(new ServiceData.Item { flag = true, weight = 1, label = "first" });
            data.groups.Add(group);
            AssetDatabase.CreateAsset(data, "Assets/Data.asset");
            AssetDatabase.CreateAsset(ScriptableObject.CreateInstance<ServiceUnverified>(), "Assets/Unverified.asset");
            var gameObject = new GameObject("Structural");
            PrefabUtility.SaveAsPrefabAsset(gameObject, "Assets/Structure.prefab");
            UnityEngine.Object.DestroyImmediate(gameObject);
            AssetDatabase.SaveAssets();
            EditorApplication.Exit(0);
        }
        public static void Verify()
        {
            var result = new Report { version = Application.unityVersion };
            try
            {
                var data = AssetDatabase.LoadAssetAtPath<ServiceData>("Assets/Data.asset");
                result.managedType = data.node is ServiceData.Node && data.node.amount == 73;
                result.managedCycle = ReferenceEquals(data.node, data.node.next);
                var item = data.groups[0].items[1];
                result.nestedValues = !item.flag && item.weight == 2.5f && item.label == "007" && data.amount == 499;
                var live = ScriptableObject.CreateInstance<ServiceData>();
                live.groups.Add(new ServiceData.Group());
                live.groups[0].items.Add(new ServiceData.Item { flag = true, weight = 1, label = "first" });
                var serialized = new SerializedObject(live);
                var array = serialized.FindProperty("groups.Array.data[0].items");
                array.InsertArrayElementAtIndex(1);
                var entry = array.GetArrayElementAtIndex(1);
                entry.FindPropertyRelative("flag").boolValue = false;
                entry.FindPropertyRelative("weight").floatValue = 2.5f;
                entry.FindPropertyRelative("label").stringValue = "007";
                serialized.FindProperty("amount").intValue = 499;
                serialized.FindProperty("node").managedReferenceValue = new ServiceData.Node { amount = 73 };
                serialized.ApplyModifiedPropertiesWithoutUndo();
                live.node.next = live.node;
                result.liveParity = JsonUtility.ToJson(data.groups[0]) == JsonUtility.ToJson(live.groups[0])
                    && live.amount == data.amount && live.node.GetType() == data.node.GetType()
                    && live.node.amount == data.node.amount && ReferenceEquals(live.node, live.node.next) == result.managedCycle;
                result.rejectedUnverified = AssetDatabase.LoadAssetAtPath<ServiceUnverified>("Assets/Unverified.asset").amount == 7;
                var prefab=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Structure.prefab");
                result.nativeTopology=prefab.transform.childCount==1 && prefab.transform.GetChild(0).name=="AddedByYaml" && prefab.transform.GetChild(0).parent==prefab.transform;
            }
            catch (Exception error) { result.error = error.ToString(); }
            File.WriteAllText("service-review-results.json", JsonUtility.ToJson(result, true));
            EditorApplication.Exit(string.IsNullOrEmpty(result.error) && result.managedType && result.managedCycle && result.nestedValues && result.liveParity && result.rejectedUnverified && result.nativeTopology ? 0 : 1);
        }
    }
}
