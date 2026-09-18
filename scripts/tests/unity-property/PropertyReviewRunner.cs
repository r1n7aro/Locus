using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using Locus;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;
using Object = UnityEngine.Object;

namespace PropertyReview
{
    // Runs inside an owned, isolated project. Calls the repository's real bridge,
    // avoiding transport mocks for Unity serialization / prefab / undo behavior.
    public static partial class PropertyReviewRunner
    {
        [Serializable] public sealed class CaseResult
        {
            public string id, expectation, actual, error;
            public bool passed;
            public double elapsedMs;
        }
        [Serializable] public sealed class Report
        {
            public string unityVersion, project, utc;
            public CaseResult[] cases;
        }
        private static readonly List<CaseResult> Cases = new List<CaseResult>();
        private static readonly Type Bridge = typeof(LocusBridge);
        private const string Folder = "Assets/PropertyReview/Fixtures";
        private const string DataPath = Folder + "/Data.asset";
        private const string ScenePath = Folder + "/Review.unity";
        private static ReviewData Data;
        private static string observed;

        public static void Run()
        {
            try
            {
                Directory.CreateDirectory(Folder);
                AssetDatabase.Refresh();
                EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
                Data = ScriptableObject.CreateInstance<ReviewData>();
                AssetDatabase.CreateAsset(Data, DataPath);
                EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);

                Check("MR01-null-types", "Null managed reference exposes type choices; setType creates a concrete instance", () => {
                    Data.node = null;
                    var before = Snap(Data, "node");
                    Write(Data, "node", "{\"action\":\"setType\",\"typeName\":\"PropertyReview.Runtime PropertyReview.ReviewLeaf\"}");
                    observed = "isManaged=" + before.isManagedReference + "; choices=" + before.managedReferenceTypes.Length + "; concrete=" + Data.node?.GetType().Name;
                    return before.isManagedReference && before.managedReferenceTypes.Any(t => t.fullName == typeof(ReviewLeaf).FullName) && Data.node is ReviewLeaf;
                });
                Check("MR02-visible-roundtrip", "Switch concrete type and restore preserves visible scalar data", () => {
                    Data.node = new ReviewLeaf { amount = 23, label = "before" };
                    var before = Snap(Data, "node");
                    Write(Data, "node", "{\"action\":\"setType\",\"typeName\":\"PropertyReview.Runtime PropertyReview.ReviewOther\"}");
                    Restore(Data, "node", before);
                    long restoredId = Snap(Data, "node").managedReferenceId;
                    observed = Data.node?.GetType().Name + "; amount=" + Data.node?.amount + "; label=" + (Data.node as ReviewLeaf)?.label + "; sameId=" + (restoredId == before.managedReferenceId);
                    return Data.node is ReviewLeaf leaf && leaf.amount == 23 && leaf.label == "before" && restoredId == before.managedReferenceId;
                });
                Check("MR03-shared-identity", "Restoring one of two shared managed references preserves alias identity", () => {
                    Data.node = Data.alias = new ReviewLeaf { amount = 23 };
                    var before = Snap(Data, "node");
                    Write(Data, "node", "null");
                    Restore(Data, "node", before);
                    observed = "same=" + ReferenceEquals(Data.node, Data.alias) + "; amounts=" + Data.node?.amount + "," + Data.alias?.amount;
                    return ReferenceEquals(Data.node, Data.alias);
                });
                Check("MR04-cycle", "A self-cycle remains a self-cycle after snapshot restore", () => {
                    Data.node = new ReviewLeaf { amount = 31 }; Data.node.next = Data.node;
                    var before = Snap(Data, "node");
                    observed = "snapshotBytes=" + LocusBridge.SerializedPropertySnapshotToJson(before).Length;
                    Write(Data, "node", "null"); Restore(Data, "node", before);
                    observed += "; same=" + ReferenceEquals(Data.node, Data.node?.next) + "; chain=" + ChainLength(Data.node);
                    return ReferenceEquals(Data.node, Data.node?.next);
                });
                Check("MR05-hidden-field", "Changing managed reference type and undo preserves hidden serialized fields", () => {
                    Data.node = new ReviewLeaf { amount = 23, hidden = 99 };
                    var before = Snap(Data, "node");
                    Write(Data, "node", "null"); Restore(Data, "node", before);
                    observed = "hidden=" + Data.node.hidden + "; visibleChildren=" + string.Join(",", before.children.Select(c => c.name));
                    return Data.node.hidden == 99;
                });
                Check("MR06-deep-chain", "A six-node managed chain survives a default-depth restore", () => {
                    Data.node = null; for (int i = 5; i >= 0; i--) Data.node = new ReviewLeaf { amount = 100 + i, next = Data.node };
                    var before = Snap(Data, "node");
                    Write(Data, "node", "null"); Restore(Data, "node", before);
                    observed = "length=" + ChainLength(Data.node);
                    return ChainLength(Data.node) == 6;
                });
                Check("MR07-list-alias", "Reordering and restoring a SerializeReference list preserves shared entries", () => {
                    var shared = new ReviewLeaf { amount = 17 }; Data.nodes = new List<ReviewNode> { shared, new ReviewOther { amount = 18 }, shared };
                    var before = Snap(Data, "nodes"); Write(Data, "nodes", "{\"action\":\"move\",\"index\":0,\"toIndex\":1}"); Restore(Data, "nodes", before);
                    observed = "count=" + Data.nodes.Count + "; first=" + Data.nodes[0].GetType().Name + "; same=" + ReferenceEquals(Data.nodes[0], Data.nodes[2]);
                    return Data.nodes.Count == 3 && Data.nodes[0] is ReviewLeaf && ReferenceEquals(Data.nodes[0], Data.nodes[2]);
                });
                Check("MR08-native-graph-undo", "Native Unity undo preserves a shared, cyclic managed graph including hidden serialized fields", () => {
                    Data.node = Data.alias = new ReviewLeaf { amount = 51, hidden = 99 }; Data.node.next = Data.node;
                    EditorUtility.SetDirty(Data); AssetDatabase.SaveAssets(); Undo.ClearAll(); Undo.IncrementCurrentGroup();
                    Write(Data, "node", "null"); Undo.FlushUndoRecordObjects(); Undo.PerformUndo();
                    observed = "shared=" + ReferenceEquals(Data.node, Data.alias) + "; cycle=" + ReferenceEquals(Data.node, Data.node?.next) + "; hidden=" + Data.node?.hidden;
                    return ReferenceEquals(Data.node, Data.alias) && ReferenceEquals(Data.node, Data.node?.next) && Data.node?.hidden == 99;
                });
                Check("AR01-truncated-undo", "A 100-element array survives delete then default snapshot undo", () => {
                    Data.numbers = Enumerable.Range(0, 100).ToList(); var before = Snap(Data, "numbers");
                    Write(Data, "numbers", "{\"action\":\"delete\",\"index\":0}"); Restore(Data, "numbers", before);
                    observed = "snapshotChildren=" + before.children.Length + "; truncated=" + before.childrenTruncated + "; count=" + Data.numbers.Count + "; index64=" + Data.numbers[64];
                    return Data.numbers.SequenceEqual(Enumerable.Range(0, 100));
                });
                Check("AR02-page-after-1024", "Paged reads return the final elements of an array larger than one request budget", () => {
                    Data.numbers = Enumerable.Range(0, 1100).ToList();
                    Type requestType = Bridge.GetNestedType("PropertyTreeReadRequest", BindingFlags.NonPublic);
                    object request = Activator.CreateInstance(requestType, true);
                    requestType.GetField("target").SetValue(request, TargetFor(Data, "numbers"));
                    requestType.GetField("arrayOffset").SetValue(request, 1024);
                    requestType.GetField("maxArrayItems").SetValue(request, 128);
                    string json = (string)Call("ReadPropertyTreeArrayPage", request, false);
                    var decode = Bridge.GetMethod("DeserializeJson", BindingFlags.Static | BindingFlags.NonPublic).MakeGenericMethod(typeof(LocusBridge.SerializedPropertySnapshot));
                    var page = (LocusBridge.SerializedPropertySnapshot)decode.Invoke(null, new object[] { json });
                    observed = "size=" + page.arraySize + "; count=" + page.children.Length + "; first=" + page.children[0].propertyPath + "; last=" + page.children[page.children.Length - 1].propertyPath;
                    return page.arraySize == 1100 && page.children.Length == 76 && page.children[0].propertyPath.EndsWith("[1024]") && page.children[75].propertyPath.EndsWith("[1099]") && !page.childrenTruncated;
                });
                Check("NUM01-wide", "long and double are read without narrowing", () => {
                    var wide = Snap(Data, "wide"); var precise = Snap(Data, "precise");
                    observed = "long=" + wide.displayValue + "; double=" + precise.displayValue + "; raw=" + Data.wide + "," + Data.precise.ToString("R", CultureInfo.InvariantCulture);
                    return Convert.ToInt64(wide.value) == Data.wide && Convert.ToDouble(precise.value) == Data.precise;
                });
                Check("NUM02-exact-limits", "Signed and unsigned 64-bit limits remain exact across JSON writes and restoration", () => {
                    Data.wide = long.MaxValue; var before = Snap(Data, "wide");
                    Write(Data, "wide", "\"9223372036854775806\""); bool wrote = Data.wide == long.MaxValue - 1;
                    Restore(Data, "wide", before); var unsigned = Snap(Data, "unsignedWide");
                    Write(Data, "unsignedWide", "\"18446744073709551614\""); bool unsignedWrote = Data.unsignedWide == ulong.MaxValue - 1;
                    Restore(Data, "unsignedWide", unsigned);
                    observed = "signedType=" + before.valueType + "; signed=" + Data.wide + "; unsigned=" + Data.unsignedWide;
                    return wrote && unsignedWrote && before.value is string && unsigned.value is string && Data.wide == long.MaxValue && Data.unsignedWide == ulong.MaxValue;
                });
                Check("NUM03-int-range", "An out-of-range Int32 write is rejected without changing the object", () => {
                    bool rejected = false; try { Write(Data, "regularInt", "5000000000"); } catch { rejected = true; }
                    observed = "rejected=" + rejected + "; value=" + Data.regularInt;
                    return rejected && Data.regularInt == 17;
                });
                foreach (string path in new[] { "intVector", "hdr", "curve", "gradient" }) {
                    string field = path;
                    Check("VALUE-" + field, "Structured value snapshot can be restored using the View undo protocol", () => {
                        var before = Snap(Data, field); observed = "display=" + before.displayValue;
                        string expected = (string)Call("SerializedPropertySnapshotValueJson", before);
                        string edit = field == "intVector" ? "{\"x\":1,\"y\":2,\"z\":3}"
                            : field == "hdr" ? "{\"r\":0.1,\"g\":0.2,\"b\":0.3,\"a\":0.4}"
                            : field == "curve" ? "{\"keys\":[{\"time\":0,\"value\":42}]}"
                            : "{\"colorKeys\":[{\"time\":0,\"color\":\"#123456\"}],\"alphaKeys\":[{\"time\":0,\"alpha\":0.5}],\"mode\":\"fixed\"}";
                        Write(Data, field, edit);
                        Restore(Data, field, before);
                        string actual = (string)Call("SerializedPropertySnapshotValueJson", Snap(Data, field));
                        observed += "; exact=" + (actual == expected);
                        return actual == expected;
                    });
                }
                foreach (string path in new[] { "hash", "exposed" }) {
                    string field = path;
                    Check("READ-" + field, "Special serialized type has a non-null value payload", () => {
                        var value = Snap(Data, field); observed = "type=" + value.type + "; value=" + value.value + "; display=" + value.displayValue + "; editable=" + value.editable;
                        return value.value != null;
                    });
                }

                PrefabCases();
                Check("REF01-scene", "Scene object reference survives snapshot restoration", () => {
                    var holder = new GameObject("RefHolder").AddComponent<ReviewComponent>();
                    holder.sceneReference = new GameObject("Referenced"); EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                    var before = Snap(holder, "sceneReference"); Restore(holder, "sceneReference", before);
                    observed = "snapshotValue=" + before.value + "; display=" + before.displayValue + "; restored=" + (holder.sceneReference != null);
                    return holder.sceneReference != null;
                });
                Check("BOOL01-synthetic-undo", "Active synthetic boolean supports restoreSnapshot", () => {
                    var go = new GameObject("ToggleTarget"); EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                    var before = Snap(go, "m_IsActive"); Write(go, "m_IsActive", "false"); Restore(go, "m_IsActive", before);
                    observed = "active=" + go.activeSelf; return go.activeSelf;
                });
                Check("BOOL02-static-flags", "Undo of the Static toggle preserves the original subset of static flags", () => {
                    var go = new GameObject("StaticTarget"); GameObjectUtility.SetStaticEditorFlags(go, StaticEditorFlags.OccluderStatic);
                    EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                    var before = (LocusBridge.SerializedPropertySnapshot)Call("BuildPropertyTreeSyntheticHeaderPropertySnapshot", go,
                        new LocusBridge.SerializedPropertyBindingTarget { propertyPath = "__locus_static" });
                    Write(go, "__locus_static", "false"); Restore(go, "__locus_static", before);
                    var flags = GameObjectUtility.GetStaticEditorFlags(go); observed = "flags=" + flags;
                    return flags == StaticEditorFlags.OccluderStatic;
                });
                Check("SELECT01-stable-target", "Recorded selection target keeps addressing original object after selection changes", () => {
                    var first = new GameObject("First"); var second = new GameObject("Second"); Selection.activeObject = first;
                    object t = Target("selection", "", "", "m_Name", "");
                    Call("WritePropertyTree", "review", t, "\"EditedFirst\"", "commit", false);
                    // The frontend stores the resolved identity returned by the bridge.
                    t = Call("PropertyTreeTargetWithLocalFileIds", t, first);
                    Selection.activeObject = second;
                    Call("WritePropertyTree", "review", t, "\"First\"", "commit", false);
                    observed = "first=" + first.name + "; second=" + second.name;
                    return first.name == "First" && second.name == "Second";
                });
                Check("UNDO01-preview-native", "A preview then identical commit keeps the pre-drag value in native Undo", () => {
                    Data.node = new ReviewLeaf { amount = 1 }; EditorUtility.SetDirty(Data); AssetDatabase.SaveAssets(); Undo.ClearAll();
                    object target = TargetFor(Data, "node.amount"); Undo.IncrementCurrentGroup();
                    Call("WritePropertyTree", "review", target, "2", "preview", false);
                    Call("WritePropertyTree", "review", target, "2", "commit", false); Undo.FlushUndoRecordObjects();
                    Undo.PerformUndo(); observed = "before=1; preview=2; commit=2; undo=" + Data.node.amount;
                    return Data.node.amount == 1;
                });
                Check("UNDO02-independent-commits", "Separate bridge commits remain separate native Undo steps", () => {
                    Data.regularInt = 17; EditorUtility.SetDirty(Data); AssetDatabase.SaveAssets(); Undo.ClearAll();
                    Write(Data, "regularInt", "18"); Undo.FlushUndoRecordObjects();
                    Write(Data, "regularInt", "19"); Undo.FlushUndoRecordObjects(); Undo.PerformUndo();
                    int reverted = Data.regularInt; Undo.PerformRedo();
                    observed = "undo=" + reverted + "; redo=" + Data.regularInt;
                    return reverted == 18 && Data.regularInt == 19;
                });
                Check("PERF01-1000-list", "Record snapshot costs for 64 / 1024 item budgets (informational)", () => {
                    Data.nodes.Clear(); for (int i = 0; i < 1000; i++) Data.nodes.Add(new ReviewLeaf { amount = i });
                    var timings = new List<string>();
                    foreach (int limit in new[] { 64, 1024 }) {
                        var watch = Stopwatch.StartNew(); var snap = Snap(Data, "nodes", 4, limit); var readMs = watch.Elapsed.TotalMilliseconds;
                        string json = LocusBridge.SerializedPropertySnapshotToJson(snap); var totalMs = watch.Elapsed.TotalMilliseconds;
                        timings.Add("limit=" + limit + ": children=" + snap.children.Length + ", readMs=" + readMs.ToString("F2") + ", readAndJsonMs=" + totalMs.ToString("F2") + ", chars=" + json.Length);
                    }
                    observed = string.Join("; ", timings); return true;
                });
                AssetDatabase.SaveAssets();
                YamlParityCases();
            }
            catch (Exception ex) { Cases.Add(new CaseResult { id = "HARNESS", error = ex.ToString() }); }
            finally
            {
                var report = new Report { unityVersion = Application.unityVersion, project = Directory.GetCurrentDirectory(), utc = DateTime.UtcNow.ToString("O"), cases = Cases.ToArray() };
                File.WriteAllText(Path.Combine(Directory.GetCurrentDirectory(), "property-review-results.json"), JsonUtility.ToJson(report, true));
                UnityEngine.Debug.Log("PROPERTY_REVIEW_FINISHED cases=" + Cases.Count + " passed=" + Cases.Count(c => c.passed));
                EditorApplication.Exit(Cases.Any(c => c.id == "HARNESS") ? 2 : Cases.Any(c => !c.passed) ? 1 : 0);
            }
        }

        private static void PrefabCases()
        {
            string innerPath = Folder + "/Inner.prefab", outerPath = Folder + "/Outer.prefab", variantPath = Folder + "/Variant.prefab";
            var innerSource = new GameObject("Nested"); innerSource.AddComponent<ReviewComponent>();
            var inner = PrefabUtility.SaveAsPrefabAsset(innerSource, innerPath); Object.DestroyImmediate(innerSource);
            var outerSource = new GameObject("Root"); var nested = (GameObject)PrefabUtility.InstantiatePrefab(inner); nested.transform.SetParent(outerSource.transform);
            var outer = PrefabUtility.SaveAsPrefabAsset(outerSource, outerPath); Object.DestroyImmediate(outerSource);
            var variantSource = (GameObject)PrefabUtility.InstantiatePrefab(outer);
            var variant = PrefabUtility.SaveAsPrefabAsset(variantSource, variantPath); Object.DestroyImmediate(variantSource);
            var instance = (GameObject)PrefabUtility.InstantiatePrefab(outer); instance.name = "SceneInstance";
            EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
            var component = instance.GetComponentInChildren<ReviewComponent>();
            Check("PF01-nested-read", "Nested prefab hierarchy includes child component values and prefab origin", () => {
                string raw = (string)Call("ReadPropertyTree", "review", Target("asset", outerPath, "", "", ""), 8, 64, 0, null, false);
                File.WriteAllText("prefab-hierarchy-snapshot.json", raw);
                observed = "chars=" + raw.Length + "; amount=" + raw.Contains("\"propertyPath\":\"amount\"") + "; innerSource=" + raw.Contains(innerPath);
                return raw.Contains("\"propertyPath\":\"amount\"") && raw.Contains(innerPath);
            });
            Check("PF02-instance-override-native-undo", "Nested scene-instance property write records override and supports native Undo/Redo", () => {
                Undo.IncrementCurrentGroup(); Write(component, "amount", "21"); Undo.FlushUndoRecordObjects();
                bool overridden = Snap(component, "amount").prefabOverride; int changed = component.amount;
                Undo.PerformUndo(); int undone = component.amount; Undo.PerformRedo(); int redone = component.amount;
                observed = "changed=" + changed + "; override=" + overridden + "; undo=" + undone + "; redo=" + redone;
                return changed == 21 && overridden && undone == 10 && redone == 21;
            });
            Check("PF03-variant-isolation", "Writing nested component in a variant leaves outer and inner prefab values intact", () => {
                var vc = variant.GetComponentInChildren<ReviewComponent>();
                object t = Target("component", variantPath, Hierarchy(vc.gameObject), "amount", typeof(ReviewComponent).FullName);
                Call("WritePropertyTree", "review", t, "33", "commit", false);
                PrefabUtility.SavePrefabAsset(variant);
                observed = "variant=" + vc.amount + "; outer=" + outer.GetComponentInChildren<ReviewComponent>().amount + "; inner=" + inner.GetComponent<ReviewComponent>().amount + "; override=" + Snap(vc, "amount").prefabOverride;
                return vc.amount == 33 && outer.GetComponentInChildren<ReviewComponent>().amount == 10 && inner.GetComponent<ReviewComponent>().amount == 10;
            });
            Check("PF04-managed-override-restore", "Managed reference on nested instance restores visible data after type removal", () => {
                var before = Snap(component, "behavior"); Write(component, "behavior", "null"); Restore(component, "behavior", before);
                observed = "type=" + component.behavior?.GetType().Name + "; amount=" + component.behavior?.amount + "; override=" + Snap(component, "behavior").prefabOverride;
                return component.behavior is ReviewLeaf && component.behavior.amount == 7;
            });
            Check("REF02-prefab-asset", "Prefab asset object reference survives snapshot restoration", () => {
                Data.reference = inner; var before = Snap(Data, "reference"); Restore(Data, "reference", before);
                observed = "value=" + before.value + "; display=" + before.displayValue + "; restored=" + (Data.reference != null);
                return Data.reference == inner;
            });
            Check("PF05-duplicate-nested-identity", "A captured prefab target with file IDs survives reordering same-name nested instances", () => {
                string path = Folder + "/Duplicate.prefab";
                var source = new GameObject("Duplicate");
                foreach (int amount in new[] { 11, 22 }) {
                    var child = (GameObject)PrefabUtility.InstantiatePrefab(inner); child.transform.SetParent(source.transform);
                    var c = child.GetComponent<ReviewComponent>(); c.amount = amount; PrefabUtility.RecordPrefabInstancePropertyModifications(c);
                }
                var asset = PrefabUtility.SaveAsPrefabAsset(source, path); Object.DestroyImmediate(source);
                var original = asset.transform.GetChild(0).GetComponent<ReviewComponent>();
                object target = Target("component", path, original.name + "[1]", "amount", typeof(ReviewComponent).FullName);
                target = Call("PropertyTreeTargetWithLocalFileIds", target, original);
                var contents = PrefabUtility.LoadPrefabContents(path); contents.transform.GetChild(0).SetSiblingIndex(1);
                PrefabUtility.SaveAsPrefabAsset(contents, path); PrefabUtility.UnloadPrefabContents(contents);
                Call("WritePropertyTree", "review", target, "99", "commit", false);
                asset = AssetDatabase.LoadAssetAtPath<GameObject>(path);
                int first = asset.transform.GetChild(0).GetComponent<ReviewComponent>().amount;
                int second = asset.transform.GetChild(1).GetComponent<ReviewComponent>().amount;
                observed = "expected reordered original at index1=99; index0=" + first + "; index1=" + second;
                return first == 22 && second == 99;
            });
            Check("PF06-view-undo-override", "Snapshot undo restores both the nested instance value and its original non-override state", () => {
                var fresh = (GameObject)PrefabUtility.InstantiatePrefab(outer); fresh.name = "FreshInstance";
                EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                var c = fresh.GetComponentInChildren<ReviewComponent>(); var before = Snap(c, "amount");
                Write(c, "amount", "44"); Restore(c, "amount", before); var after = Snap(c, "amount");
                observed = "value=" + c.amount + "; beforeOverride=" + before.prefabOverride + "; afterOverride=" + after.prefabOverride;
                return c.amount == 10 && before.prefabOverride == after.prefabOverride;
            });
            Check("PF07-second-scene-instance-id", "A resolved target with file IDs addresses the second scene instance of the same nested prefab", () => {
                var second = (GameObject)PrefabUtility.InstantiatePrefab(outer); second.name = "SecondSceneInstance";
                EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                var c = second.GetComponentInChildren<ReviewComponent>();
                int originalAmount = component.amount;
                object target = Call("PropertyTreeTargetWithLocalFileIds", TargetFor(c, "amount"), c);
                var firstId = GlobalObjectId.GetGlobalObjectIdSlow(component); var secondId = GlobalObjectId.GetGlobalObjectIdSlow(c);
                Call("WritePropertyTree", "review", target, "88", "commit", false);
                observed = "first=" + component.amount + "; second=" + c.amount + "; firstGlobal=" + firstId + "; secondGlobal=" + secondId;
                return component.amount == originalAmount && c.amount == 88;
            });
            Check("PF08-batch-instance-identity", "Batch writes distinguish complete identities even when fallback paths and local IDs collide", () => {
                var sibling = (GameObject)PrefabUtility.InstantiatePrefab(outer); sibling.name = "BatchSibling";
                EditorSceneManager.SaveScene(SceneManager.GetActiveScene(), ScenePath);
                var other = sibling.GetComponentInChildren<ReviewComponent>();
                object first = Call("PropertyTreeTargetWithLocalFileIds", TargetFor(component, "amount"), component);
                object second = Call("PropertyTreeTargetWithLocalFileIds", TargetFor(other, "amount"), other);
                // Paths may be stale after rename/reorder. The complete identity remains authoritative.
                foreach (var field in first.GetType().GetFields())
                    if (field.Name != "globalObjectId") field.SetValue(second, field.GetValue(first));
                Type writeType = Bridge.GetNestedType("PropertyTreeWriteRequest", BindingFlags.NonPublic);
                Array writes = Array.CreateInstance(writeType, 2);
                for (int i = 0; i < 2; i++) {
                    object write = Activator.CreateInstance(writeType, true);
                    writeType.GetField("target").SetValue(write, i == 0 ? first : second);
                    writeType.GetField("valueJson").SetValue(write, i == 0 ? "101" : "202");
                    writeType.GetField("mode").SetValue(write, "commit");
                    writes.SetValue(write, i);
                }
                Type applyType = Bridge.GetNestedType("PropertyTreeApplyRequest", BindingFlags.NonPublic);
                object request = Activator.CreateInstance(applyType, true);
                applyType.GetField("writes").SetValue(request, writes);
                string result = (string)Call("ApplyPropertyTrees", request);
                observed = "first=" + component.amount + "; second=" + other.amount + "; ok=" + result.StartsWith("{\"ok\":true");
                return component.amount == 101 && other.amount == 202;
            });
            Check("REF03-subasset-same-name", "An object reference to the second same-name subasset restores the same file ID", () => {
                var a = ScriptableObject.CreateInstance<ReviewData>(); a.name = "Duplicate";
                var b = ScriptableObject.CreateInstance<ReviewData>(); b.name = "Duplicate";
                AssetDatabase.AddObjectToAsset(a, Data); AssetDatabase.AddObjectToAsset(b, Data); AssetDatabase.SaveAssets();
                var ordered = AssetDatabase.LoadAllAssetsAtPath(DataPath).Where(o => o != null && o.name == "Duplicate").ToArray();
                var wanted = ordered[ordered.Length - 1]; Data.reference = wanted; var before = Snap(Data, "reference"); Restore(Data, "reference", before);
                observed = "value=" + before.value + "; restoredOriginal=" + (Data.reference == wanted) + "; selectedFirst=" + (Data.reference == ordered[0]);
                return Data.reference == wanted;
            });
            Check("REF04-subasset-escaped-name", "A subasset name containing a slash roundtrips through the serialized path", () => {
                var named = ScriptableObject.CreateInstance<ReviewData>(); named.name = "A/B";
                AssetDatabase.AddObjectToAsset(named, Data); AssetDatabase.SaveAssets(); Data.reference = named;
                var before = Snap(Data, "reference"); Restore(Data, "reference", before);
                observed = "value=" + before.value + "; restoredOriginal=" + (Data.reference == named) + "; null=" + (Data.reference == null);
                return Data.reference == named;
            });
        }

        private static void Check(string id, string expectation, Func<bool> action)
        {
            observed = ""; var watch = Stopwatch.StartNew(); var result = new CaseResult { id = id, expectation = expectation };
            try { result.passed = action(); } catch (Exception ex) { result.error = Unwrap(ex).ToString(); }
            result.actual = observed; result.elapsedMs = watch.Elapsed.TotalMilliseconds; Cases.Add(result);
            UnityEngine.Debug.Log("PROPERTY_REVIEW_CASE " + id + " pass=" + result.passed + " " + observed + " " + result.error);
        }
        private static Exception Unwrap(Exception ex) { while (ex is TargetInvocationException && ex.InnerException != null) ex = ex.InnerException; return ex; }
        private static object Call(string name, params object[] args)
        { return Bridge.GetMethods(BindingFlags.Static | BindingFlags.NonPublic).Single(m => m.Name == name && m.GetParameters().Length == args.Length).Invoke(null, args); }
        private static object Target(string kind, string path, string objectPath, string propertyPath, string componentType)
        {
            var type = Bridge.GetNestedType("PropertyTreeTarget", BindingFlags.NonPublic); object target = Activator.CreateInstance(type, true);
            foreach (var item in new Dictionary<string, object> { { "kind", kind }, { "path", path }, { "objectPath", objectPath }, { "propertyPath", propertyPath }, { "componentType", componentType }, { "scenePath", ScenePath } }) type.GetField(item.Key).SetValue(target, item.Value);
            return target;
        }
        private static string Hierarchy(GameObject go) { return go.transform.parent == null ? go.name : Hierarchy(go.transform.parent.gameObject) + "/" + go.name; }
        private static object TargetFor(Object obj, string path)
        {
            var component = obj as Component; var go = obj as GameObject;
            return component != null ? Target("component", "", Hierarchy(component.gameObject), path, component.GetType().FullName)
                : go != null && !EditorUtility.IsPersistent(go) ? Target("gameobject", "", Hierarchy(go), path, "")
                : Target("asset", AssetDatabase.GetAssetPath(obj), "", path, "");
        }
        private static void Write(Object obj, string path, string value) { Call("WritePropertyTree", "review", TargetFor(obj, path), value, "commit", false); }
        private static void Restore(Object obj, string path, LocusBridge.SerializedPropertySnapshot snapshot)
        { Write(obj, path, "{\"action\":\"restoreSnapshot\",\"snapshot\":" + LocusBridge.SerializedPropertySnapshotToJson(snapshot) + "}"); }
        private static LocusBridge.SerializedPropertySnapshot Snap(Object obj, string path, int depth = 4, int limit = 64)
        { using (var serialized = new SerializedObject(obj)) { serialized.Update(); return LocusBridge.SnapshotSerializedProperty(serialized.FindProperty(path), depth, limit); } }
        private static int ChainLength(ReviewNode node) { var seen = new HashSet<ReviewNode>(); while (node != null && seen.Add(node)) node = node.next; return seen.Count; }
    }
}

