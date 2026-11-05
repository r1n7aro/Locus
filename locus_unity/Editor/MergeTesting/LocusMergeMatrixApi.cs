using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace Locus.MergeTesting
{
    // Unity-authored three-way histories for the CLI merge matrix. Each case saves one
    // base, then produces target and source as independent edits of that base: the
    // file is restored to the base bytes and reimported before the source edit, so
    // fileIDs, managed-reference ids and override lists are what two branches get.
    public static class LocusMergeMatrixApi
    {
        [Serializable]
        private sealed class CaseInfo
        {
            public string name;
            public string path;
            public string error;
        }

        [Serializable]
        private sealed class Manifest
        {
            public string unityVersion;
            public string folder;
            public string snapshots;
            public List<CaseInfo> cases = new List<CaseInfo>();
        }

        [Serializable]
        private sealed class Fact
        {
            public string key;
            public string value;
        }

        [Serializable]
        private sealed class Facts
        {
            public string name;
            public string error;
            public List<Fact> facts = new List<Fact>();
            public List<string> logs = new List<string>();

            public void Add(string key, object value)
            {
                facts.Add(new Fact { key = key, value = Format(value) });
            }
        }

        private sealed class Ctx
        {
            public string name;
            public string path;
            public string snapshots;

            public void Snap(string side)
            {
                File.Copy(path, Path.Combine(snapshots, name + "-" + side + ".yaml"), true);
            }

            public void Restore(string side)
            {
                UnloadMain(path);
                File.Copy(Path.Combine(snapshots, name + "-" + side + ".yaml"), path, true);
                AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
            }
        }

        private sealed class Case
        {
            public string file;
            public Action<Ctx> createBase;
            public Action<Ctx> target;
            public Action<Ctx> source;
            // A second source commit made on top of the first; both are merged in order.
            public Action<Ctx> source2;
            public Action<string, Facts> inspect;
            // Reads the copy the Editor already has open (scene or Prefab Stage), then closes it.
            public Action<string, Facts> inspectOpen;
        }

        private const string InnerFile = "Inner.prefab";
        private const string VariantBaseFile = "VariantBase.prefab";

        private static Dictionary<string, Case> Cases(string folder)
        {
            var inner = folder + "/" + InnerFile;
            var variantBase = folder + "/" + VariantBaseFile;
            return new Dictionary<string, Case>
            {
                // Both sides add a child GameObject; the source also moves an existing child.
                ["prefab_children"] = new Case
                {
                    file = "PrefabChildren.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root => Child(root, "A")),
                    target = c => EditPrefab(c.path, root => Child(root, "TargetChild")),
                    source = c => EditPrefab(c.path, root =>
                    {
                        Child(root, "SourceChild").localPosition = new Vector3(0, 2, 0);
                        root.transform.Find("A").localPosition = new Vector3(1, 0, 0);
                    }),
                    inspect = (path, f) => WithPrefab(path, root =>
                    {
                        f.Add("children", Sorted(ChildNames(root.transform)));
                        f.Add("a.x", root.transform.Find("A").localPosition.x);
                        f.Add("source.y", root.transform.Find("SourceChild").localPosition.y);
                    }),
                },
                // Both sides add a component to the same GameObject; one carries a SerializeReference.
                ["prefab_components"] = new Case
                {
                    file = "PrefabComponents.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root => { }),
                    target = c => EditPrefab(c.path, root => root.AddComponent<BoxCollider>().size = new Vector3(2, 2, 2)),
                    source = c => EditPrefab(c.path, root =>
                        root.AddComponent<LocusMergeFixtureComponent>().behavior = new MergeAction { label = "added", health = 3, speed = 1 }),
                    inspect = (path, f) => WithPrefab(path, root =>
                    {
                        f.Add("components", Sorted(ComponentNames(root)));
                        f.Add("box.size.x", root.GetComponent<BoxCollider>().size.x);
                        var component = root.GetComponent<LocusMergeFixtureComponent>();
                        f.Add("behavior.label", component.behavior.label);
                        f.Add("missingTypes", UnityEditor.SerializationUtility.HasManagedReferencesWithMissingTypes(component));
                    }),
                },
                // Different override elements of one nested instance, including SerializeReference fields.
                ["prefab_nested_overrides"] = new Case
                {
                    file = "NestedOverrides.prefab",
                    createBase = c => SaveOuter(c.path, inner),
                    target = c => EditPrefab(c.path, root =>
                    {
                        var instance = root.transform.Find("Inner");
                        SetFloat(instance, "m_LocalPosition.x", 3);
                        SetInt(instance.GetComponent<LocusMergeFixtureComponent>(), "behavior.health", 7);
                    }),
                    source = c => EditPrefab(c.path, root =>
                    {
                        var instance = root.transform.Find("Inner");
                        SetFloat(instance, "m_LocalPosition.y", 4);
                        SetFloat(instance.GetComponent<LocusMergeFixtureComponent>(), "behavior.speed", 9);
                    }),
                    inspect = (path, f) => WithPrefab(path, root => InstanceFacts(root.transform.Find("Inner"), f)),
                },
                // The same override changed differently: one element conflicts, the others merge.
                ["prefab_nested_conflict"] = new Case
                {
                    file = "NestedConflict.prefab",
                    createBase = c => SaveOuter(c.path, inner),
                    target = c => EditPrefab(c.path, root => SetFloat(root.transform.Find("Inner"), "m_LocalPosition.x", 3)),
                    source = c => EditPrefab(c.path, root =>
                    {
                        SetFloat(root.transform.Find("Inner"), "m_LocalPosition.x", 5);
                        SetFloat(root.transform.Find("Inner"), "m_LocalPosition.y", 4);
                    }),
                    inspect = (path, f) => WithPrefab(path, root => InstanceFacts(root.transform.Find("Inner"), f)),
                },
                // Both sides add components and child GameObjects to a nested prefab instance.
                ["prefab_nested_added"] = new Case
                {
                    file = "NestedAdded.prefab",
                    createBase = c => SaveOuter(c.path, inner),
                    target = c => EditPrefab(c.path, root =>
                    {
                        var instance = root.transform.Find("Inner").gameObject;
                        instance.AddComponent<BoxCollider>();
                        Child(instance, "TargetLeaf");
                    }),
                    source = c => EditPrefab(c.path, root =>
                    {
                        var instance = root.transform.Find("Inner").gameObject;
                        instance.AddComponent<AudioSource>();
                        Child(instance, "SourceLeaf");
                    }),
                    inspect = (path, f) => WithPrefab(path, root =>
                    {
                        var instance = root.transform.Find("Inner");
                        f.Add("components", Sorted(ComponentNames(instance.gameObject)));
                        f.Add("children", Sorted(ChildNames(instance)));
                        f.Add("instance", PrefabUtility.IsPartOfPrefabInstance(instance.gameObject));
                    }),
                },
                // Prefab variant: overrides, a removed component and added children on both sides.
                ["prefab_variant"] = new Case
                {
                    file = "Variant.prefab",
                    createBase = c => SaveVariant(c.path, variantBase),
                    target = c => EditPrefab(c.path, root =>
                    {
                        SetInt(root.GetComponent<LocusMergeFixtureComponent>(), "behavior.health", 7);
                        UnityEngine.Object.DestroyImmediate(root.GetComponent<BoxCollider>());
                        Child(root, "TargetChild");
                    }),
                    source = c => EditPrefab(c.path, root =>
                    {
                        SetFloat(root.transform.Find("Leaf"), "m_LocalPosition.z", 2);
                        Child(root, "SourceChild");
                    }),
                    inspect = (path, f) =>
                    {
                        f.Add("variant", PrefabUtility.GetPrefabAssetType(AssetDatabase.LoadMainAssetAtPath(path)) == PrefabAssetType.Variant);
                        WithPrefab(path, root =>
                        {
                            f.Add("health", ((MergeAction)root.GetComponent<LocusMergeFixtureComponent>().behavior).health);
                            f.Add("collider", root.GetComponent<BoxCollider>() != null);
                            f.Add("children", Sorted(ChildNames(root.transform)));
                            f.Add("leaf.z", root.transform.Find("Leaf").localPosition.z);
                        });
                    },
                },
                // The destination deletes a child the source moved.
                ["prefab_delete_modify"] = new Case
                {
                    file = "DeleteModify.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root => { Child(root, "A"); Child(root, "B"); }),
                    target = c => EditPrefab(c.path, root => UnityEngine.Object.DestroyImmediate(root.transform.Find("B").gameObject)),
                    source = c => EditPrefab(c.path, root => root.transform.Find("B").localPosition = new Vector3(0, 0, 5)),
                    inspect = (path, f) => WithPrefab(path, root => f.Add("children", Sorted(ChildNames(root.transform)))),
                },
                // Both sides append a new managed reference to the same SerializeReference list.
                ["sr_list_append"] = new Case
                {
                    file = "SrAppend.asset",
                    createBase = c => CreateAsset(c.path, asset =>
                    {
                        var group = new MergeGroup { label = "group" };
                        group.children.Add(new MergeAction { label = "n1", health = 1 });
                        group.children.Add(new MergeAction { label = "n2", health = 2 });
                        asset.root = group;
                    }),
                    target = c => EditAsset(c.path, asset => ((MergeGroup)asset.root).children.Add(new MergeAction { label = "t-new", health = 3 })),
                    source = c => EditAsset(c.path, asset => ((MergeGroup)asset.root).children.Add(new MergeWeightedAction { label = "s-new", weight = 0.5f })),
                    inspect = (path, f) => WithAsset(path, asset =>
                    {
                        var children = ((MergeGroup)asset.root).children;
                        f.Add("labels", String.Join("|", children.Select(node => node == null ? "null" : node.label)));
                        f.Add("types", String.Join("|", children.Select(node => node == null ? "null" : node.GetType().Name)));
                        f.Add("missingTypes", UnityEditor.SerializationUtility.HasManagedReferencesWithMissingTypes(asset));
                    }),
                },
                // The destination clears a managed reference the source edited.
                ["sr_delete_modify"] = new Case
                {
                    file = "SrDelete.asset",
                    createBase = c => CreateAsset(c.path, asset => asset.optional = new MergeAction { label = "opt", health = 5 }),
                    target = c => EditAsset(c.path, asset => asset.optional = null),
                    source = c => EditAsset(c.path, asset => ((MergeAction)asset.optional).health = 6),
                    inspect = (path, f) => WithAsset(path, asset =>
                    {
                        f.Add("optional", asset.optional == null ? "null" : asset.optional.GetType().Name);
                        f.Add("health", asset.optional is MergeAction action ? action.health : -1);
                        f.Add("missingTypes", UnityEditor.SerializationUtility.HasManagedReferencesWithMissingTypes(asset));
                    }),
                },
                // The source replaces a managed reference with another type; the destination edited the old object.
                ["sr_type_replace"] = new Case
                {
                    file = "SrReplace.asset",
                    createBase = c => CreateAsset(c.path, asset => asset.optional = new MergeAction { label = "opt", health = 5 }),
                    target = c => EditAsset(c.path, asset => ((MergeAction)asset.optional).health = 6),
                    source = c => EditAsset(c.path, asset => asset.optional = new MergeWeightedAction { label = "weighted", health = 5, weight = 0.5f }),
                    inspect = (path, f) => WithAsset(path, asset =>
                    {
                        f.Add("optional", asset.optional == null ? "null" : asset.optional.GetType().Name);
                        f.Add("label", asset.optional == null ? "" : asset.optional.label);
                        f.Add("missingTypes", UnityEditor.SerializationUtility.HasManagedReferencesWithMissingTypes(asset));
                    }),
                },
                // Independent fields of a SerializeReference on a prefab component.
                ["sr_prefab_fields"] = new Case
                {
                    file = "SrPrefab.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root =>
                        root.AddComponent<LocusMergeFixtureComponent>().behavior = new MergeAction { label = "b", health = 1, speed = 1 }),
                    target = c => EditPrefab(c.path, root => ((MergeAction)root.GetComponent<LocusMergeFixtureComponent>().behavior).health = 7),
                    source = c => EditPrefab(c.path, root =>
                    {
                        var behavior = (MergeAction)root.GetComponent<LocusMergeFixtureComponent>().behavior;
                        behavior.speed = 9;
                        behavior.label = "source";
                    }),
                    inspect = (path, f) => WithPrefab(path, root =>
                    {
                        var behavior = (MergeAction)root.GetComponent<LocusMergeFixtureComponent>().behavior;
                        f.Add("label", behavior.label);
                        f.Add("health", behavior.health);
                        f.Add("speed", behavior.speed);
                    }),
                },
                // The destination retargets a shared reference while the source edits both objects.
                ["sr_retarget"] = new Case
                {
                    file = "SrRetarget.asset",
                    createBase = c => CreateAsset(c.path, asset =>
                    {
                        var shared = new MergeAction { label = "shared", health = 1 };
                        var group = new MergeGroup { label = "group" };
                        group.children.Add(shared);
                        group.children.Add(new MergeAction { label = "other", health = 2 });
                        asset.root = group;
                        asset.alias = shared;
                    }),
                    target = c => EditAsset(c.path, asset => asset.alias = ((MergeGroup)asset.root).children[1]),
                    source = c => EditAsset(c.path, asset =>
                    {
                        var children = ((MergeGroup)asset.root).children;
                        ((MergeAction)children[0]).health = 9;
                        children[1].label = "other2";
                    }),
                    inspect = (path, f) => WithAsset(path, asset =>
                    {
                        var children = ((MergeGroup)asset.root).children;
                        f.Add("alias.label", asset.alias == null ? "null" : asset.alias.label);
                        f.Add("alias.isSecond", ReferenceEquals(asset.alias, children[1]));
                        f.Add("shared.health", ((MergeAction)children[0]).health);
                    }),
                },
                // A list without element identity: each side edits a different element.
                ["rows_elements"] = new Case
                {
                    file = "Rows.asset",
                    createBase = c => CreateAsset(c.path, asset =>
                    {
                        asset.rows.Add(new MergeRow { key = "alpha", amount = 1, position = new Vector3(1, 2, 3) });
                        asset.rows.Add(new MergeRow { key = "beta", amount = 2, position = new Vector3(4, 5, 6) });
                    }),
                    target = c => EditAsset(c.path, asset => { var row = asset.rows[0]; row.amount = 5; asset.rows[0] = row; }),
                    source = c => EditAsset(c.path, asset => { var row = asset.rows[1]; row.position = new Vector3(4, 9, 6); asset.rows[1] = row; }),
                    inspect = (path, f) => WithAsset(path, asset =>
                        f.Add("rows", String.Join("|", asset.rows.Select(row => row.key + ":" + row.amount + ":" + Format(row.position.x) + "," + Format(row.position.y) + "," + Format(row.position.z))))),
                },
                // Scene: both sides add root objects and override different values of a prefab instance.
                ["scene_roots"] = new Case
                {
                    file = "Roots.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        NewObject("A", scene);
                        PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(inner), scene);
                    }),
                    target = c => EditScene(c.path, scene =>
                    {
                        NewObject("TargetRoot", scene);
                        SetFloat(Root(scene, "Inner").transform, "m_LocalPosition.x", 3);
                    }),
                    source = c => EditScene(c.path, scene =>
                    {
                        // Within one Editor process, the first new object saved into the
                        // same base scene always receives the same fileID; separate
                        // sessions do not (checked with two batch-mode processes). Ids
                        // go to the newest object first, so a throwaway created after
                        // SourceRoot and saved with it takes that first id, leaving
                        // SourceRoot ids a second machine could produce.
                        NewObject("SourceRoot", scene);
                        var burner = NewObject("Burner", scene);
                        EditorSceneManager.MarkSceneDirty(scene);
                        EditorSceneManager.SaveScene(scene);
                        UnityEngine.Object.DestroyImmediate(burner);
                        Root(scene, "A").transform.localPosition = new Vector3(0, 2, 0);
                        SetFloat(Root(scene, "Inner").transform, "m_LocalPosition.y", 4);
                    }),
                    inspect = (path, f) => WithScene(path, scene =>
                    {
                        f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)));
                        f.Add("a.y", Root(scene, "A").transform.localPosition.y);
                        var instance = Root(scene, "Inner").transform;
                        f.Add("x", instance.localPosition.x);
                        f.Add("y", instance.localPosition.y);
                    }),
                },
                // Two different objects added in one Editor session receive one
                // fileID: the merge must ask, and the chosen side must load.
                ["scene_same_id"] = new Case
                {
                    file = "SameId.unity",
                    createBase = c => CreateScene(c.path, scene => NewObject("A", scene)),
                    target = c => EditScene(c.path, scene => NewObject("TargetRoot", scene)),
                    source = c => EditScene(c.path, scene => NewObject("SourceRoot", scene)),
                    inspect = (path, f) => WithScene(path, scene => f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)))),
                },
                // A merged scene the Editor has open must be reloaded from disk after
                // apply: written by Rust then reloaded, or written by the Editor's
                // own disk transaction.
                ["scene_open_reload"] = OpenSceneCase("Reload.unity"),
                ["scene_open_transaction"] = OpenSceneCase("ReloadTransaction.unity"),
                // Larger than the bridge's 16 MiB request: the Editor's transaction
                // must read the merged bytes from the staged file.
                ["scene_large_transaction"] = OpenSceneCase("LargeTransaction.unity", LargeSceneRoots),
                // Both sides add roots whose random fileIDs interleave across the
                // file and the source deletes one. The merged scene must be the file
                // Unity itself writes, so re-saving it changes no byte.
                ["scene_both_add"] = new Case
                {
                    file = "BothAdd.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        foreach (var name in new[] { "A", "B", "C" }) Child(NewObject(name, scene), name + "Leaf");
                    }),
                    target = c => EditScene(c.path, scene =>
                    {
                        Root(scene, "A").transform.localPosition = new Vector3(1, 0, 0);
                        AddRoots(scene, "T");
                    }),
                    source = c => EditScene(c.path, scene =>
                    {
                        AddRoots(scene, "S");
                        BurnTargetIds(scene);
                        Root(scene, "B").transform.localPosition = new Vector3(0, 2, 0);
                        UnityEngine.Object.DestroyImmediate(Root(scene, "C"));
                    }),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)));
                            f.Add("a.x", Root(scene, "A").transform.localPosition.x);
                            f.Add("b.y", Root(scene, "B").transform.localPosition.y);
                            f.Add("leaves", scene.GetRootGameObjects().Sum(root => root.transform.childCount));
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The destination never changed the scene: the result is the source
                // file itself (checked byte for byte by the driver) and stays stable.
                ["scene_untouched_destination"] = new Case
                {
                    file = "Untouched.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        foreach (var name in new[] { "A", "B" }) Child(NewObject(name, scene), name + "Leaf");
                    }),
                    target = c => { },
                    source = c => EditScene(c.path, scene =>
                    {
                        AddRoots(scene, "S");
                        Root(scene, "A").transform.localPosition = new Vector3(0, 2, 0);
                        UnityEngine.Object.DestroyImmediate(Root(scene, "B"));
                    }),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)));
                            f.Add("a.y", Root(scene, "A").transform.localPosition.y);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // Hierarchy edits on both sides: the source reparents a child and
                // adds a component, the destination renames another child; both
                // edit different axes of one position.
                ["scene_hierarchy_edits"] = new Case
                {
                    file = "Hierarchy.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        var a = NewObject("A", scene);
                        Child(a, "A1");
                        Child(a, "A2");
                        Child(NewObject("B", scene), "B1");
                    }),
                    target = c => EditScene(c.path, scene =>
                    {
                        Root(scene, "A").transform.Find("A2").name = "A2T";
                        Root(scene, "B").transform.Find("B1").localPosition = new Vector3(1, 0, 0);
                    }),
                    source = c => EditScene(c.path, scene =>
                    {
                        var a = Root(scene, "A");
                        a.transform.Find("A1").SetParent(Root(scene, "B").transform, false);
                        a.AddComponent<BoxCollider>();
                        Root(scene, "B").transform.Find("B1").localPosition = new Vector3(0, 3, 0);
                    }),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            var a = Root(scene, "A");
                            var b = Root(scene, "B");
                            f.Add("a.children", Sorted(ChildNames(a.transform)));
                            f.Add("b.children", Sorted(ChildNames(b.transform)));
                            f.Add("a.components", Sorted(ComponentNames(a)));
                            var b1 = b.transform.Find("B1");
                            f.Add("b1.x", b1.localPosition.x);
                            f.Add("b1.y", b1.localPosition.y);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The source removes a component the destination edited; keeping
                // the destination's component must leave it attached.
                ["scene_component_delete_modify"] = new Case
                {
                    file = "ComponentDeleteModify.unity",
                    createBase = c => CreateScene(c.path, scene => NewObject("A", scene).AddComponent<BoxCollider>()),
                    target = c => EditScene(c.path, scene => SetFloat(Root(scene, "A").GetComponent<BoxCollider>(), "m_Size.x", 2)),
                    source = c => EditScene(c.path, scene => UnityEngine.Object.DestroyImmediate(Root(scene, "A").GetComponent<BoxCollider>())),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            var a = Root(scene, "A");
                            f.Add("a.components", Sorted(ComponentNames(a)));
                            var box = a.GetComponent<BoxCollider>();
                            f.Add("box.x", box == null ? (object)"none" : box.size.x);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The destination removed a component the source edited; restoring
                // the source's component must attach it to its GameObject again.
                ["scene_component_restore"] = new Case
                {
                    file = "ComponentRestore.unity",
                    createBase = c => CreateScene(c.path, scene => NewObject("A", scene).AddComponent<BoxCollider>()),
                    target = c => EditScene(c.path, scene => UnityEngine.Object.DestroyImmediate(Root(scene, "A").GetComponent<BoxCollider>())),
                    source = c => EditScene(c.path, scene => SetFloat(Root(scene, "A").GetComponent<BoxCollider>(), "m_Size.x", 3)),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            var a = Root(scene, "A");
                            f.Add("a.components", Sorted(ComponentNames(a)));
                            var box = a.GetComponent<BoxCollider>();
                            f.Add("box.x", box == null ? (object)"none" : box.size.x);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The destination deleted a root with its children while the
                // source edited a collider two levels down: one restore brings
                // the whole root back under the scene roots.
                ["scene_subtree_restore"] = new Case
                {
                    file = "SubtreeRestore.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        NewObject("A", scene);
                        var r = NewObject("R", scene);
                        var child = Child(r, "C");
                        Child(child.gameObject, "D").gameObject.AddComponent<BoxCollider>();
                    }),
                    target = c => EditScene(c.path, scene => UnityEngine.Object.DestroyImmediate(Root(scene, "R"))),
                    source = c => EditScene(c.path, scene =>
                        SetFloat(Root(scene, "R").transform.Find("C/D").GetComponent<BoxCollider>(), "m_Size.x", 3)),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)));
                            var d = Root(scene, "R").transform.Find("C/D");
                            f.Add("d", d == null ? "missing" : "present");
                            f.Add("d.box.x", d == null ? (object)"none" : d.GetComponent<BoxCollider>().size.x);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The destination deleted a child of a kept root, leaving its
                // `m_Children: []`; the restore adds it back there.
                ["scene_child_restore"] = new Case
                {
                    file = "ChildRestore.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        var r = NewObject("R", scene);
                        Child(Child(r, "C").gameObject, "D").gameObject.AddComponent<BoxCollider>();
                    }),
                    target = c => EditScene(c.path, scene => UnityEngine.Object.DestroyImmediate(Root(scene, "R").transform.Find("C").gameObject)),
                    source = c => EditScene(c.path, scene =>
                        SetFloat(Root(scene, "R").transform.Find("C/D").GetComponent<BoxCollider>(), "m_Size.x", 3)),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            var r = Root(scene, "R").transform;
                            f.Add("r.children", Sorted(ChildNames(r)));
                            var d = r.Find("C/D");
                            f.Add("d.box.x", d == null ? (object)"none" : d.GetComponent<BoxCollider>().size.x);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The destination deleted a Prefab child the source moved; restoring
                // keeps the move.
                ["prefab_delete_modify_restore"] = new Case
                {
                    file = "DeleteModifyRestore.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root => { Child(root, "A"); Child(root, "B"); }),
                    target = c => EditPrefab(c.path, root => UnityEngine.Object.DestroyImmediate(root.transform.Find("B").gameObject)),
                    source = c => EditPrefab(c.path, root => root.transform.Find("B").localPosition = new Vector3(0, 0, 5)),
                    inspect = (path, f) => WithPrefab(path, root =>
                    {
                        f.Add("children", Sorted(ChildNames(root.transform)));
                        var b = root.transform.Find("B");
                        f.Add("b.z", b == null ? (object)"none" : b.localPosition.z);
                    }),
                },
                // Two consecutive source commits: the second edits the root the
                // first added and a value the first changed.
                ["scene_commit_sequence"] = new Case
                {
                    file = "Sequence.unity",
                    createBase = c => CreateScene(c.path, scene =>
                    {
                        NewObject("A", scene);
                        Child(NewObject("B", scene), "BLeaf");
                    }),
                    target = c => EditScene(c.path, scene => Root(scene, "A").transform.localPosition = new Vector3(1, 0, 0)),
                    source = c => EditScene(c.path, scene =>
                    {
                        Child(NewObject("S", scene), "SLeaf");
                        Root(scene, "B").transform.localPosition = new Vector3(0, 1, 0);
                    }),
                    source2 = c => EditScene(c.path, scene =>
                    {
                        Root(scene, "S").transform.localPosition = new Vector3(0, 2, 0);
                        Root(scene, "B").transform.localPosition = new Vector3(0, 5, 0);
                    }),
                    inspect = (path, f) =>
                    {
                        WithScene(path, scene =>
                        {
                            f.Add("roots", Sorted(scene.GetRootGameObjects().Select(root => root.name)));
                            f.Add("a.x", Root(scene, "A").transform.localPosition.x);
                            f.Add("b.y", Root(scene, "B").transform.localPosition.y);
                            f.Add("s.y", Root(scene, "S").transform.localPosition.y);
                        });
                        ResaveFacts(path, f);
                    },
                },
                // The same for a prefab open in Prefab Mode.
                ["prefab_stage_reload"] = new Case
                {
                    file = "StageReload.prefab",
                    createBase = c => SavePrefab(c.path, "Root", root => Child(root, "A")),
                    target = c => EditPrefab(c.path, root => root.transform.Find("A").localPosition = new Vector3(1, 0, 0)),
                    source = c => EditPrefab(c.path, root => root.transform.Find("A").localPosition = new Vector3(0, 2, 0)),
                    inspect = (path, f) => WithPrefab(path, root => PositionFacts(root.transform.Find("A"), f)),
                    inspectOpen = (path, f) =>
                    {
                        var stage = PrefabStageUtility.GetCurrentPrefabStage();
                        if (stage == null || stage.assetPath != path) throw new InvalidOperationException("The merged prefab is no longer open in Prefab Mode: " + path);
                        try
                        {
                            f.Add("dirty", stage.scene.isDirty);
                            PositionFacts(stage.prefabContentsRoot.transform.Find("A"), f);
                        }
                        finally { StageUtility.GoToMainStage(); }
                    },
                },
            };
        }

        private const int AddedRoots = 4;
        // About 2.9 KB of YAML per root: roughly 18 MB in total.
        private const int LargeSceneRoots = 6500;

        private static void AddRoots(Scene scene, string prefix)
        {
            for (var i = 0; i < AddedRoots; i++) Child(NewObject(prefix + i, scene), prefix + "Leaf" + i);
        }

        // One Editor session gives the same fileIDs to the same sequence of new
        // objects, newest first. Throwaways shaped like the target's additions,
        // created after the source's own and saved once, take those ids so the
        // source's objects keep ids a second machine could produce.
        private static void BurnTargetIds(Scene scene)
        {
            var burners = new List<GameObject>();
            for (var i = 0; i < AddedRoots; i++)
            {
                var burner = NewObject("Burner" + i, scene);
                Child(burner, "BurnerLeaf" + i);
                burners.Add(burner);
            }
            EditorSceneManager.MarkSceneDirty(scene);
            if (!EditorSceneManager.SaveScene(scene)) throw new InvalidOperationException("Unable to save the id burners");
            foreach (var burner in burners) UnityEngine.Object.DestroyImmediate(burner);
        }

        // Unity writes a scene in its own document order; a merge that produced
        // that order saves back to identical bytes.
        private static void ResaveFacts(string path, Facts f)
        {
            var before = File.ReadAllBytes(path);
            var scene = EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
            try
            {
                EditorSceneManager.MarkSceneDirty(scene);
                if (!EditorSceneManager.SaveScene(scene)) throw new InvalidOperationException("Unable to re-save the merged scene: " + path);
            }
            finally { EditorSceneManager.CloseScene(scene, true); }
            var after = File.ReadAllBytes(path);
            var stable = before.SequenceEqual(after);
            f.Add("resave_stable", stable);
            if (!stable)
            {
                var a = System.Text.Encoding.UTF8.GetString(before).Split('\n');
                var b = System.Text.Encoding.UTF8.GetString(after).Split('\n');
                var line = Enumerable.Range(0, Math.Min(a.Length, b.Length)).FirstOrDefault(i => a[i] != b[i]);
                f.Add("resave_first_difference", "line " + (line + 1) + ": merged '" + (line < a.Length ? a[line] : "") + "' saved '" + (line < b.Length ? b[line] : "") + "'");
            }
        }

        private static Case OpenSceneCase(string file, int bulk = 0)
        {
            return new Case
            {
                file = file,
                createBase = c => CreateScene(c.path, scene =>
                {
                    NewObject("A", scene);
                    var padding = new string('x', 2400);
                    for (var i = 0; i < bulk; i++) NewObject("Bulk" + i.ToString("D5", CultureInfo.InvariantCulture) + padding, scene);
                }),
                target = c => EditScene(c.path, scene => Root(scene, "A").transform.localPosition = new Vector3(1, 0, 0)),
                source = c => EditScene(c.path, scene => Root(scene, "A").transform.localPosition = new Vector3(0, 2, 0)),
                inspect = (path, f) => WithScene(path, scene => PositionFacts(Root(scene, "A").transform, f)),
                inspectOpen = (path, f) =>
                {
                    var scene = SceneManager.GetSceneByPath(path);
                    if (!scene.isLoaded) throw new InvalidOperationException("The merged scene is no longer open: " + path);
                    try
                    {
                        f.Add("dirty", scene.isDirty);
                        PositionFacts(Root(scene, "A").transform, f);
                    }
                    finally { EditorSceneManager.CloseScene(scene, true); }
                },
            };
        }

        // Opens a case asset the way a user has it open while merging: scenes
        // additively, prefabs in Prefab Mode. Reads the bytes now on disk.
        public static string Open(string folder, string name)
        {
            ValidateFolder(folder);
            if (!Cases(folder).TryGetValue(name, out var item)) throw new ArgumentException("Unknown merge matrix case: " + name);
            var path = folder + "/" + item.file;
            AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
            if (path.EndsWith(".unity", StringComparison.Ordinal)) EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
            else if (PrefabStageUtility.OpenPrefab(path) == null) throw new InvalidOperationException("Prefab Mode did not open " + path);
            return "LOCUS_MERGE_MATRIX_OPENED";
        }

        // Leaves no case asset open after a failed run.
        public static string Close(string folder, string name)
        {
            ValidateFolder(folder);
            if (!Cases(folder).TryGetValue(name, out var item)) throw new ArgumentException("Unknown merge matrix case: " + name);
            var path = folder + "/" + item.file;
            var scene = SceneManager.GetSceneByPath(path);
            if (scene.isLoaded) EditorSceneManager.CloseScene(scene, true);
            var stage = PrefabStageUtility.GetCurrentPrefabStage();
            if (stage != null && stage.assetPath == path) StageUtility.GoToMainStage();
            return "LOCUS_MERGE_MATRIX_CLOSED";
        }

        public static string InspectOpen(string folder, string name)
        {
            return Collect(folder, name, (item, path, result) =>
            {
                if (item.inspectOpen == null) throw new ArgumentException("Case has no open-asset inspection: " + name);
                item.inspectOpen(path, result);
            });
        }

        public static string Generate(string folder)
        {
            ValidateFolder(folder);
            if (Directory.Exists(folder)) throw new InvalidOperationException("Fixture folder already exists: " + folder);
            AssetDatabase.CreateFolder("Assets", folder.Substring("Assets/".Length));
            var snapshots = Path.GetFullPath("Library/LocusMergeDriver/" + Path.GetFileName(folder));
            Directory.CreateDirectory(snapshots);
            var manifest = new Manifest { unityVersion = Application.unityVersion, folder = folder, snapshots = snapshots };
            SaveInner(folder + "/" + InnerFile);
            SaveVariantBase(folder + "/" + VariantBaseFile);
            foreach (var pair in Cases(folder))
            {
                var info = new CaseInfo { name = pair.Key, path = folder + "/" + pair.Value.file };
                var ctx = new Ctx { name = pair.Key, path = info.path, snapshots = snapshots };
                try
                {
                    pair.Value.createBase(ctx);
                    ctx.Snap("base");
                    pair.Value.target(ctx);
                    ctx.Snap("target");
                    ctx.Restore("base");
                    pair.Value.source(ctx);
                    ctx.Snap("source");
                    if (pair.Value.source2 != null)
                    {
                        pair.Value.source2(ctx);
                        ctx.Snap("source2");
                    }
                    ctx.Restore("base");
                }
                catch (Exception error)
                {
                    info.error = error.GetType().Name + ": " + error.Message;
                }
                manifest.cases.Add(info);
            }
            return JsonUtility.ToJson(manifest);
        }

        public static string Inspect(string folder, string name)
        {
            return Collect(folder, name, (item, path, result) =>
            {
                UnloadMain(path);
                AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
                item.inspect(path, result);
            });
        }

        // Runs one read with import errors captured as facts.
        private static string Collect(string folder, string name, Action<Case, string, Facts> read)
        {
            ValidateFolder(folder);
            var result = new Facts { name = name };
            Application.LogCallback hook = (condition, stack, type) =>
            {
                if (type == LogType.Error || type == LogType.Exception || type == LogType.Assert)
                    result.logs.Add(type + ": " + condition);
                else if (type == LogType.Warning && condition.Contains(folder))
                    result.logs.Add(type + ": " + condition);
            };
            Application.logMessageReceived += hook;
            try
            {
                if (!Cases(folder).TryGetValue(name, out var item)) throw new ArgumentException("Unknown merge matrix case: " + name);
                read(item, folder + "/" + item.file, result);
            }
            catch (Exception error)
            {
                result.error = error.GetType().Name + ": " + error.Message;
            }
            finally
            {
                Application.logMessageReceived -= hook;
            }
            return JsonUtility.ToJson(result);
        }

        public static string Remove(string folder)
        {
            ValidateFolder(folder);
            if (!AssetDatabase.DeleteAsset(folder)) throw new InvalidOperationException("Matrix fixture cleanup failed: " + folder);
            return "LOCUS_MERGE_MATRIX_REMOVED";
        }

        private static void SaveInner(string path)
        {
            SavePrefab(path, "Inner", root =>
            {
                root.AddComponent<LocusMergeFixtureComponent>().behavior = new MergeAction { label = "inner", health = 1, speed = 1 };
                Child(root, "Leaf");
            });
        }

        private static void SaveVariantBase(string path)
        {
            SavePrefab(path, "VariantRoot", root =>
            {
                root.AddComponent<LocusMergeFixtureComponent>().behavior = new MergeAction { label = "variant", health = 1, speed = 1 };
                root.AddComponent<BoxCollider>();
                Child(root, "Leaf");
            });
        }

        private static void SaveOuter(string path, string inner)
        {
            SavePrefab(path, "Outer", root =>
            {
                var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(inner), root.scene);
                instance.transform.SetParent(root.transform, false);
            });
        }

        private static void SaveVariant(string path, string basePath)
        {
            var scene = EditorSceneManager.NewPreviewScene();
            try
            {
                var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(basePath), scene);
                try { PrefabUtility.SaveAsPrefabAsset(instance, path); }
                finally { UnityEngine.Object.DestroyImmediate(instance); }
            }
            finally { EditorSceneManager.ClosePreviewScene(scene); }
        }

        private static void SavePrefab(string path, string name, Action<GameObject> build)
        {
            var scene = EditorSceneManager.NewPreviewScene();
            try
            {
                var root = NewObject(name, scene);
                try
                {
                    build(root);
                    PrefabUtility.SaveAsPrefabAsset(root, path);
                }
                finally { UnityEngine.Object.DestroyImmediate(root); }
            }
            finally { EditorSceneManager.ClosePreviewScene(scene); }
        }

        private static void EditPrefab(string path, Action<GameObject> edit)
        {
            var root = PrefabUtility.LoadPrefabContents(path);
            try
            {
                edit(root);
                PrefabUtility.SaveAsPrefabAsset(root, path);
            }
            finally { PrefabUtility.UnloadPrefabContents(root); }
        }

        private static void WithPrefab(string path, Action<GameObject> read)
        {
            var root = PrefabUtility.LoadPrefabContents(path);
            try { read(root); }
            finally { PrefabUtility.UnloadPrefabContents(root); }
        }

        private static void CreateAsset(string path, Action<LocusMergeFixtureAsset> build)
        {
            var asset = ScriptableObject.CreateInstance<LocusMergeFixtureAsset>();
            build(asset);
            AssetDatabase.CreateAsset(asset, path);
            AssetDatabase.SaveAssetIfDirty(asset);
            Resources.UnloadAsset(asset);
        }

        private static void EditAsset(string path, Action<LocusMergeFixtureAsset> edit)
        {
            var asset = AssetDatabase.LoadAssetAtPath<LocusMergeFixtureAsset>(path);
            if (asset == null) throw new InvalidOperationException("Fixture asset cannot be loaded: " + path);
            edit(asset);
            EditorUtility.SetDirty(asset);
            AssetDatabase.SaveAssetIfDirty(asset);
        }

        private static void WithAsset(string path, Action<LocusMergeFixtureAsset> read)
        {
            var asset = AssetDatabase.LoadAssetAtPath<LocusMergeFixtureAsset>(path);
            if (asset == null) throw new InvalidOperationException("Merged asset cannot be loaded: " + path);
            read(asset);
        }

        // A saved copy of the current scene gives a real Unity scene file without
        // replacing the Editor's open scenes; its roots are cleared before the base is built.
        private static void CreateScene(string path, Action<Scene> build)
        {
            if (!EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), path, true))
                throw new InvalidOperationException("Unable to create the fixture scene: " + path);
            AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceSynchronousImport);
            EditScene(path, scene =>
            {
                foreach (var root in scene.GetRootGameObjects()) UnityEngine.Object.DestroyImmediate(root);
                build(scene);
            });
        }

        private static void EditScene(string path, Action<Scene> edit)
        {
            var scene = EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
            try
            {
                edit(scene);
                EditorSceneManager.MarkSceneDirty(scene);
                if (!EditorSceneManager.SaveScene(scene)) throw new InvalidOperationException("Unable to save fixture scene: " + path);
            }
            finally { EditorSceneManager.CloseScene(scene, true); }
        }

        private static void WithScene(string path, Action<Scene> read)
        {
            var scene = EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
            try { read(scene); }
            finally { EditorSceneManager.CloseScene(scene, true); }
        }

        private static GameObject Root(Scene scene, string name)
        {
            var root = scene.GetRootGameObjects().FirstOrDefault(item => item.name == name);
            if (root == null) throw new InvalidOperationException("Missing scene root: " + name);
            return root;
        }

        private static void InstanceFacts(Transform instance, Facts f)
        {
            var behavior = (MergeAction)instance.GetComponent<LocusMergeFixtureComponent>().behavior;
            f.Add("x", instance.localPosition.x);
            f.Add("y", instance.localPosition.y);
            f.Add("health", behavior.health);
            f.Add("speed", behavior.speed);
            f.Add("instance", PrefabUtility.IsPartOfPrefabInstance(instance.gameObject));
        }

        private static void PositionFacts(Transform value, Facts f)
        {
            f.Add("x", value.localPosition.x);
            f.Add("y", value.localPosition.y);
        }

        private static GameObject NewObject(string name, Scene scene)
        {
            var value = EditorUtility.CreateGameObjectWithHideFlags(name, HideFlags.HideAndDontSave);
            SceneManager.MoveGameObjectToScene(value, scene);
            value.hideFlags = HideFlags.None;
            foreach (var component in value.GetComponents<Component>()) component.hideFlags = HideFlags.None;
            return value;
        }

        private static Transform Child(GameObject parent, string name)
        {
            var child = NewObject(name, parent.scene);
            child.transform.SetParent(parent.transform, false);
            return child.transform;
        }

        private static void SetFloat(UnityEngine.Object target, string property, float value)
        {
            var serialized = new SerializedObject(target);
            var field = serialized.FindProperty(property) ?? throw new InvalidOperationException("Missing property " + property);
            field.floatValue = value;
            serialized.ApplyModifiedPropertiesWithoutUndo();
        }

        private static void SetInt(UnityEngine.Object target, string property, int value)
        {
            var serialized = new SerializedObject(target);
            var field = serialized.FindProperty(property) ?? throw new InvalidOperationException("Missing property " + property);
            field.intValue = value;
            serialized.ApplyModifiedPropertiesWithoutUndo();
        }

        private static IEnumerable<string> ChildNames(Transform parent)
        {
            for (var i = 0; i < parent.childCount; i++) yield return parent.GetChild(i).name;
        }

        private static IEnumerable<string> ComponentNames(GameObject value)
        {
            return value.GetComponents<Component>().Select(component => component == null ? "Missing" : component.GetType().Name);
        }

        private static string Sorted(IEnumerable<string> values)
        {
            return String.Join("|", values.OrderBy(value => value, StringComparer.Ordinal));
        }

        private static void UnloadMain(string path)
        {
            if (!path.EndsWith(".asset", StringComparison.Ordinal)) return;
            var loaded = AssetDatabase.LoadMainAssetAtPath(path);
            if (loaded is ScriptableObject) Resources.UnloadAsset(loaded);
        }

        private static string Format(object value)
        {
            switch (value)
            {
                case null: return "null";
                case bool flag: return flag ? "true" : "false";
                case float number: return number.ToString("R", CultureInfo.InvariantCulture);
                case IFormattable formattable: return formattable.ToString(null, CultureInfo.InvariantCulture);
                default: return value.ToString();
            }
        }

        private static void ValidateFolder(string folder)
        {
            if (String.IsNullOrEmpty(folder) || !folder.StartsWith("Assets/LocusMergeMatrix-", StringComparison.Ordinal) || folder.Contains("..") || folder.Contains("\\") || folder.Substring(7).Contains("/"))
                throw new ArgumentException("Expected a dedicated Assets/LocusMergeMatrix-* fixture folder");
        }
    }
}
