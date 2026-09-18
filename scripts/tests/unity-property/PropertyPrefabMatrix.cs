using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using Object = UnityEngine.Object;

namespace PropertyPrefabMatrix
{
    public static class PropertyPrefabMatrix
    {
        [Serializable] public class Target { public string kind = "asset", path, targetFileId, propertyPath; }
        [Serializable] public class Check { public Target target; public string expectedJson; }
        [Serializable] public class Write { public Target target; public string valueJson; }
        [Serializable] public class Case { public string name; public Check[] checks; public Write[] writes; }
        [Serializable] public class Manifest { public Case[] cases; }
        [Serializable] public class Report { public string version, error; public bool arrays, nestedArrays, topology, twins, managed, scene, references, emptyArrays, revertArray, applyArray, dormantArray; }
        static Target T(string path, Object obj, string property)
        {
            AssetDatabase.TryGetGUIDAndLocalFileIdentifier(obj, out string guid, out long id);
            if (path.EndsWith(".unity"))
            {
                var global = GlobalObjectId.GetGlobalObjectIdSlow(obj);
                id = (long)((global.targetObjectId ^ global.targetPrefabId) & long.MaxValue);
            }
            return new Target { path = path, targetFileId = id.ToString(), propertyPath = property };
        }
        static Check C(string path, Object obj, string property, string expected) => new Check { target = T(path, obj, property), expectedJson = expected };
        static Write W(string path, Object obj, string property, string value) => new Write { target = T(path, obj, property), valueJson = value };
        static GameObject Instance(string path) => (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(path));
        static GameObject Save(GameObject obj, string path) { var saved = PrefabUtility.SaveAsPrefabAsset(obj, path); Object.DestroyImmediate(obj); return saved; }
        static void Set(PrefabMatrixComponent obj, Action<SerializedObject> edit)
        {
            using (var so = new SerializedObject(obj)) { edit(so); so.ApplyModifiedPropertiesWithoutUndo(); }
            PrefabUtility.RecordPrefabInstancePropertyModifications(obj);
        }
        static void ArraySeed(PrefabMatrixComponent c)
        {
            Set(c, so => {
                var a = so.FindProperty("numbers"); a.arraySize = 3; a.GetArrayElementAtIndex(2).intValue = 30;
                var items = so.FindProperty("items"); items.arraySize = 2;
                items.GetArrayElementAtIndex(1).FindPropertyRelative("amount").intValue = 2;
                items.GetArrayElementAtIndex(1).FindPropertyRelative("label").stringValue = "007";
                var v = items.GetArrayElementAtIndex(1).FindPropertyRelative("values"); v.arraySize = 3; v.GetArrayElementAtIndex(2).intValue = 3;
            });
        }
        public static void Seed()
        {
            try
            {
                EditorSettings.serializationMode = SerializationMode.ForceText;
                var root = new GameObject("Base"); var c = root.AddComponent<PrefabMatrixComponent>();
                var keep = new GameObject("Keep"); keep.transform.SetParent(root.transform); keep.AddComponent<PrefabMatrixComponent>();
                var removed = new GameObject("Removed"); removed.transform.SetParent(root.transform); removed.AddComponent<PrefabMatrixComponent>();
                c.link = keep; c.node.next = c.node;
                Save(root, "Assets/Base.prefab");
                root = Instance("Assets/Base.prefab"); ArraySeed(root.GetComponent<PrefabMatrixComponent>());
                var array = Save(root, "Assets/Arrays.prefab").GetComponent<PrefabMatrixComponent>();
                // A second variant carries inherited, differently sized nested arrays.
                root = Instance("Assets/Arrays.prefab"); var nested = Save(root, "Assets/Deep.prefab").GetComponent<PrefabMatrixComponent>();
                root = Instance("Assets/Base.prefab");
                Object.DestroyImmediate(root.transform.Find("Removed").gameObject);
                Object.DestroyImmediate(root.transform.Find("Keep").GetComponent<PrefabMatrixComponent>());
                var added = root.transform.Find("Keep").gameObject.AddComponent<PrefabMatrixComponent>(); added.amount = 19;
                var child = new GameObject("Added"); child.transform.SetParent(root.transform); child.AddComponent<PrefabMatrixComponent>().amount = 23;
                var grandchild = new GameObject("Grandchild"); grandchild.transform.SetParent(child.transform);
                var topology = Save(root, "Assets/Topology.prefab");
                root = new GameObject("Outer"); var left = Instance("Assets/Topology.prefab"); left.name = "Left"; left.transform.SetParent(root.transform);
                var right = Instance("Assets/Topology.prefab"); right.name = "Right"; right.transform.SetParent(root.transform);
                var outer = Save(root, "Assets/Outer.prefab");
                root = Instance("Assets/Base.prefab"); Set(root.GetComponent<PrefabMatrixComponent>(), so => so.FindProperty("node.amount").intValue = 31);
                var managed = Save(root, "Assets/Managed.prefab").GetComponent<PrefabMatrixComponent>();
                root = Instance("Assets/Base.prefab");
                Set(root.GetComponent<PrefabMatrixComponent>(),so=>{
                    var a=so.FindProperty("empty");a.arraySize=2;
                    var item=a.GetArrayElementAtIndex(1);item.FindPropertyRelative("label").stringValue="001";
                    item.FindPropertyRelative("flag").boolValue=true;item.FindPropertyRelative("weight").floatValue=1;
                    var values=item.FindPropertyRelative("values");values.arraySize=1;values.GetArrayElementAtIndex(0).intValue=5;
                });
                var empty=Save(root,"Assets/Empty.prefab").GetComponent<PrefabMatrixComponent>();
                root=Instance("Assets/Base.prefab");ArraySeed(root.GetComponent<PrefabMatrixComponent>());
                var revert=Save(root,"Assets/Revert.prefab").GetComponent<PrefabMatrixComponent>();
                root=Instance("Assets/Base.prefab");PrefabUtility.UnpackPrefabInstance(root,PrefabUnpackMode.Completely,InteractionMode.AutomatedAction);
                Save(root,"Assets/ApplyBase.prefab");
                root=Instance("Assets/ApplyBase.prefab");ArraySeed(root.GetComponent<PrefabMatrixComponent>());Save(root,"Assets/ApplyMid.prefab");
                var apply=Save(Instance("Assets/ApplyMid.prefab"),"Assets/ApplyTop.prefab").GetComponent<PrefabMatrixComponent>();
                root=Instance("Assets/Base.prefab");PrefabUtility.UnpackPrefabInstance(root,PrefabUnpackMode.Completely,InteractionMode.AutomatedAction);
                root.GetComponent<PrefabMatrixComponent>().numbers.Add(30);Save(root,"Assets/DormantBase.prefab");
                root=Instance("Assets/DormantBase.prefab");Set(root.GetComponent<PrefabMatrixComponent>(),so=>so.FindProperty("numbers").GetArrayElementAtIndex(2).intValue=97);Save(root,"Assets/Dormant.prefab");
                root=PrefabUtility.LoadPrefabContents("Assets/DormantBase.prefab");root.GetComponent<PrefabMatrixComponent>().numbers.RemoveRange(1,2);
                PrefabUtility.SaveAsPrefabAsset(root,"Assets/DormantBase.prefab");PrefabUtility.UnloadPrefabContents(root);
                var dormant=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Dormant.prefab").GetComponent<PrefabMatrixComponent>();
                var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
                root = Instance("Assets/Deep.prefab");
                EditorSceneManager.SaveScene(scene, "Assets/Matrix.unity");
                var cases = new List<Case> {
                    new Case { name="inherited-array-commands", checks=new[]{C("Assets/Arrays.prefab",array,"numbers.Array.data[2]","30")}, writes=new[]{
                        W("Assets/Arrays.prefab",array,"numbers","{\"action\":\"insert\",\"index\":1,\"value\":99}"),
                        W("Assets/Arrays.prefab",array,"numbers","{\"action\":\"move\",\"index\":3,\"toIndex\":0}"),
                        W("Assets/Arrays.prefab",array,"numbers","{\"action\":\"delete\",\"index\":1}"),
                        W("Assets/Arrays.prefab",array,"numbers","{\"action\":\"resize\",\"size\":4,\"value\":40}"),
                        W("Assets/Arrays.prefab",array,"numbers.Array.data[3]","41") } },
                    new Case { name="multilevel-nested-array", checks=new[]{C("Assets/Deep.prefab",nested,"items.Array.data[1].label","\"007\"")}, writes=new[]{
                        W("Assets/Deep.prefab",nested,"items.Array.data[1].values","{\"action\":\"insert\",\"index\":1,\"value\":8}"),
                        W("Assets/Deep.prefab",nested,"items.Array.data[1].values.Array.data[3]","9") } },
                    new Case { name="added-and-removed-topology",checks=new[]{C("Assets/Topology.prefab",topology.transform.Find("Keep").GetComponent<PrefabMatrixComponent>(),"amount","19"),C("Assets/Topology.prefab",topology.transform.Find("Added").GetComponent<PrefabMatrixComponent>(),"amount","23")},writes=new[]{ W("Assets/Topology.prefab",topology.GetComponent<PrefabMatrixComponent>(),"amount","47") } },
                    new Case { name="nested-twin-added-component", checks=new[]{C("Assets/Outer.prefab",outer.transform.Find("Left/Keep").GetComponent<PrefabMatrixComponent>(),"amount","19"), C("Assets/Outer.prefab",outer.transform.Find("Right/Added").GetComponent<PrefabMatrixComponent>(),"amount","23")}, writes=new[]{W("Assets/Outer.prefab",outer.transform.Find("Left/Keep").GetComponent<PrefabMatrixComponent>(),"amount","59")} },
                    new Case { name="inherited-managed-leaf", checks=new[]{C("Assets/Managed.prefab",managed,"node.amount","31")},writes=new[]{W("Assets/Managed.prefab",managed,"node.next.amount","67")} },
                    new Case { name="scene-nested-array",checks=new[]{C("Assets/Matrix.unity",root.GetComponent<PrefabMatrixComponent>(),"items.Array.data[1].label","\"007\"")},writes=new[]{W("Assets/Matrix.unity",root.GetComponent<PrefabMatrixComponent>(),"items.Array.data[1].label","\"scene\"")} },
                    new Case { name="empty-custom-array-and-typed-leaves",checks=new[]{C("Assets/Empty.prefab",empty,"empty.Array.data[1].label","\"001\""),C("Assets/Empty.prefab",empty,"empty.Array.data[1].flag","true"),C("Assets/Empty.prefab",empty,"empty.Array.data[1].weight","1.0")},writes=new[]{
                        W("Assets/Empty.prefab",empty,"empty","{\"action\":\"insert\",\"index\":1,\"value\":{\"amount\":12,\"label\":\"009\",\"flag\":false,\"weight\":2.5,\"values\":[3,4]}}"),
                        W("Assets/Empty.prefab",empty,"empty.Array.data[1].values","{\"action\":\"move\",\"index\":0,\"toIndex\":1}") }
                    },
                    new Case{name="array-revert-subtree",checks=new[]{C("Assets/Revert.prefab",revert,"numbers.Array.data[2]","30")},writes=new[]{W("Assets/Revert.prefab",revert,"numbers","{\"action\":\"revert\"}")}},
                    new Case{name="array-apply-across-two-layers",checks=new[]{C("Assets/ApplyTop.prefab",apply,"numbers.Array.data[2]","30")},writes=new[]{
                        W("Assets/ApplyTop.prefab",apply,"numbers","{\"action\":\"insert\",\"index\":0,\"value\":88}"),
                        W("Assets/ApplyTop.prefab",apply,"numbers","{\"action\":\"applyToSource\",\"level\":1}"),
                        W("Assets/ApplyTop.prefab",apply,"numbers","{\"action\":\"applyToSource\",\"level\":2}")}},
                    new Case{name="shrunken-source-dormant-overrides",checks=new[]{C("Assets/Dormant.prefab",dormant,"numbers.Array.data[0]","10")},writes=new[]{W("Assets/Dormant.prefab",dormant,"amount","83")}}
                };
                File.WriteAllText("prefab-matrix.json",JsonUtility.ToJson(new Manifest { cases=cases.ToArray() },true));
                AssetDatabase.SaveAssets(); EditorApplication.Exit(0);
            }
            catch(Exception e) { Debug.LogException(e); EditorApplication.Exit(1); }
        }
        public static void Verify()
        {
            var r=new Report {version=Application.unityVersion};
            try
            {
                var arrays=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Arrays.prefab").GetComponent<PrefabMatrixComponent>();
                // The live side uses SerializedObject operations, including explicit fill semantics.
                var live=Instance("Assets/Base.prefab"); var lc=live.GetComponent<PrefabMatrixComponent>(); ArraySeed(lc);
                Set(lc,so=>{var a=so.FindProperty("numbers");a.InsertArrayElementAtIndex(1);a.GetArrayElementAtIndex(1).intValue=99;a.MoveArrayElement(3,0);a.DeleteArrayElementAtIndex(1);a.arraySize=4;a.GetArrayElementAtIndex(3).intValue=41;});
                r.arrays=arrays.numbers.SequenceEqual(lc.numbers); Object.DestroyImmediate(live);
                var deep=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Deep.prefab").GetComponent<PrefabMatrixComponent>();
                r.nestedArrays=deep.items[1].values.SequenceEqual(new[]{1,8,2,9}) && deep.items[1].label=="007";
                var t=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Topology.prefab");
                r.topology=t.transform.childCount==2 && t.transform.Find("Removed")==null && t.transform.Find("Keep").GetComponents<PrefabMatrixComponent>().Length==1 && t.transform.Find("Added/Grandchild")!=null && t.GetComponent<PrefabMatrixComponent>().amount==47;
                var outer=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Outer.prefab");
                r.twins=outer.transform.Find("Left/Keep").GetComponent<PrefabMatrixComponent>().amount==59 && outer.transform.Find("Right/Keep").GetComponent<PrefabMatrixComponent>().amount==19;
                var m=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Managed.prefab").GetComponent<PrefabMatrixComponent>();
                r.managed=m.node.amount==67 && ReferenceEquals(m.node,m.node.next);
                var scene=EditorSceneManager.OpenScene("Assets/Matrix.unity"); var sc=scene.GetRootGameObjects()[0].GetComponent<PrefabMatrixComponent>();
                r.scene=sc.items[1].label=="scene" && sc.items[1].values.SequenceEqual(new[]{1,8,2,9});
                r.references=t.GetComponent<PrefabMatrixComponent>().link==t.transform.Find("Keep").gameObject && outer.transform.Find("Left").GetComponent<PrefabMatrixComponent>().link==outer.transform.Find("Left/Keep").gameObject && outer.transform.Find("Right").GetComponent<PrefabMatrixComponent>().link==outer.transform.Find("Right/Keep").gameObject;
                var e=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Empty.prefab").GetComponent<PrefabMatrixComponent>().empty;
                r.emptyArrays=e.Count==3 && e[0].amount==0 && e[1].amount==12 && e[1].label=="009" && !e[1].flag && e[1].weight==2.5f && e[1].values.SequenceEqual(new[]{4,3}) && e[2].flag && e[2].weight==1 && e[2].values[0]==5;
                var reverted=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Revert.prefab").GetComponent<PrefabMatrixComponent>();
                r.revertArray=reverted.numbers.SequenceEqual(new[]{10,20})&&reverted.items.Count==2;
                r.applyArray=new[]{"Assets/ApplyBase.prefab","Assets/ApplyMid.prefab","Assets/ApplyTop.prefab"}.All(p=>AssetDatabase.LoadAssetAtPath<GameObject>(p).GetComponent<PrefabMatrixComponent>().numbers.SequenceEqual(new[]{88,10,20,30}));
                r.applyArray=r.applyArray&&!File.ReadAllText("Assets/ApplyTop.prefab").Contains("numbers.Array")&&!File.ReadAllText("Assets/ApplyMid.prefab").Contains("numbers.Array");
                var dormant=AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Dormant.prefab").GetComponent<PrefabMatrixComponent>();
                r.dormantArray=dormant.numbers.SequenceEqual(new[]{10})&&dormant.amount==83&&File.ReadAllText("Assets/Dormant.prefab").Contains("numbers.Array.data[2]");
            } catch(Exception e){r.error=e.ToString();}
            File.WriteAllText("prefab-matrix-results.json",JsonUtility.ToJson(r,true));
            EditorApplication.Exit(string.IsNullOrEmpty(r.error)&&r.arrays&&r.nestedArrays&&r.topology&&r.twins&&r.managed&&r.scene&&r.references&&r.emptyArrays&&r.revertArray&&r.applyArray&&r.dormantArray?0:1);
        }
    }
}
