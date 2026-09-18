using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using UnityEditor;
using UnityEngine;
using Object = UnityEngine.Object;

namespace PropertyReview
{
    public static partial class PropertyReviewRunner
    {
        [Serializable] private sealed class AuthoringCandidate { public string source; public string text; }
        [Serializable] private sealed class AuthoringOutput { public AuthoringCandidate[] candidates; public string text; public string value_json; }
        private static string ObjectId(Object target)
        {
            AssetDatabase.TryGetGUIDAndLocalFileIdentifier(target, out string guid, out long id);
            return id.ToString(CultureInfo.InvariantCulture);
        }
        private static AuthoringOutput Author(string path, string id, string property, object command, params string[] sources)
        {
            string[] args = Environment.GetCommandLineArgs();
            string driver = args[Array.IndexOf(args, "-locusPropertyYamlDriver") + 1];
            var files = sources.ToDictionary(p => AssetDatabase.AssetPathToGUID(p), p => Path.GetFullPath(p));
            var request = new { source = Path.GetFullPath(path), object_id = id, propertyPath = property, authoring = command, prefabFiles = files };
            var start = new ProcessStartInfo(driver) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true, RedirectStandardOutput = true, RedirectStandardError = true, StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8 };
            using (var process = Process.Start(start))
            {
                var output = process.StandardOutput.ReadToEndAsync(); var errors = process.StandardError.ReadToEndAsync();
                byte[] bytes = Encoding.UTF8.GetBytes(Json(request)); process.StandardInput.BaseStream.Write(bytes, 0, bytes.Length); process.StandardInput.Close();
                if (!process.WaitForExit(60000)) { process.Kill(); throw new Exception("Owned authoring driver timed out"); }
                if (process.ExitCode != 0) throw new Exception(errors.Result);
                return JsonUtility.FromJson<AuthoringOutput>(output.Result);
            }
        }
        private static void ApplyAuthor(AuthoringOutput output)
        {
            var entries = new List<Dictionary<string, object>>();
            foreach (var candidate in output.candidates)
            {
                string path = candidate.source.Replace('\\', '/'); string root = Path.GetFullPath(".").Replace('\\', '/') + "/";
                if (!path.StartsWith(root, StringComparison.OrdinalIgnoreCase)) throw new Exception("Candidate escaped fixture project");
                path = path.Substring(root.Length); string sha;
                using (var hash = SHA256.Create()) sha = BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(path))).Replace("-", "").ToLowerInvariant();
                entries.Add(new Dictionary<string, object> { { "path", path }, { "expected_sha256", sha }, { "bytes_base64", Convert.ToBase64String(Encoding.UTF8.GetBytes(candidate.text)) } });
            }
            ApplyYaml(entries.ToArray());
        }
        private static object ManagedTemplate(bool cycle)
        {
            return new { rootRid = "1", entries = new object[] { new { rid = "1", type = new { @class = "ReviewOther", ns = "PropertyReview", asm = "PropertyReview.Runtime" }, data = new { amount = 73, hidden = 29, enabled = false, next = new { rid = cycle ? "1" : "-2" } } } } };
        }
        private static void AuthoringParityCases()
        {
            Check("YAML14-bulk-performance-sample", "One thousand accumulated scalar requests produce the final value with one Unity import", () => {
                var data = ParityAsset("Bulk"); string path = AssetDatabase.GetAssetPath(data);
                object[] writes = Enumerable.Range(0, 1000).Select(i => (object)new { propertyPath = "regularInt", value = i }).ToArray();
                var watch = Stopwatch.StartNew(); var candidate = PrepareYaml(data, writes, true); double prepareMs = watch.Elapsed.TotalMilliseconds;
                PropertyYamlImportCounter.Counts.Clear(); watch.Restart(); ApplyYaml(candidate); double importMs = watch.Elapsed.TotalMilliseconds;
                int imports = PropertyYamlImportCounter.Counts.TryGetValue(path, out int count) ? count : 0;
                observed = "requests=1000; prepareProcessMs=" + prepareMs.ToString("F2", CultureInfo.InvariantCulture) + "; commitImportMs=" + importMs.ToString("F2", CultureInfo.InvariantCulture) + "; imports=" + imports
                    + "; writeMs=" + lastDiskProfile.writeMs.ToString("F2", CultureInfo.InvariantCulture) + "; importMs=" + lastDiskProfile.importMs.ToString("F2", CultureInfo.InvariantCulture);
                return data.regularInt == 999 && imports == 1 && UntouchedGraph(data) && lastDiskProfile.changedFiles == 1 && lastDiskProfile.preflightMs >= 0 && lastDiskProfile.writeMs >= 0 && lastDiskProfile.importMs >= 0;
            });
            Check("YAML12-complex-value-parity", "Weighted curves and quantized gradients match the live PropertyTree payload after reload", () => {
                var yaml = ParityAsset("Complex"); var live = ParityAsset("ComplexLive");
                var curve = new { preWrapMode = "Loop", postWrapMode = "PingPong", keys = new[] {
                    new { time = 0f, value = 3f, inTangent = 0f, outTangent = 2f, inWeight = 0.2f, outWeight = 0.4f, weightedMode = "Both" },
                    new { time = 2f, value = 7f, inTangent = 2f, outTangent = 0f, inWeight = 0.3f, outWeight = 0.6f, weightedMode = "In" }
                } };
                var gradient = new { mode = "Fixed", colorKeys = new[] { new { time = 0f, color = "#FF0000" }, new { time = 0.35f, color = "#00FF00" }, new { time = 1f, color = "#0000FF" } }, alphaKeys = new[] { new { time = 0f, alpha = 0.2f }, new { time = 1f, alpha = 0.8f } } };
                var candidate = PrepareYaml(yaml, new object[] { new { propertyPath = "curve", value = (object)curve }, new { propertyPath = "gradient", value = (object)gradient } }, true);
                LiveWrite(live, "curve", Json(curve)); LiveWrite(live, "gradient", Json(gradient)); AssetDatabase.SaveAssets(); ApplyYaml(candidate);
                observed = "keys=" + yaml.curve.length + "; gradient=" + yaml.gradient.colorKeys.Length;
                return yaml.curve.keys.SequenceEqual(live.curve.keys) && yaml.curve.preWrapMode == live.curve.preWrapMode && yaml.curve.postWrapMode == live.curve.postWrapMode
                    && yaml.gradient.mode == live.gradient.mode && yaml.gradient.colorKeys.SequenceEqual(live.gradient.colorKeys) && yaml.gradient.alphaKeys.SequenceEqual(live.gradient.alphaKeys);
            });
            Check("YAML13-component-topology", "Adding/removing a materialized component updates GameObject ownership in the same candidate", () => {
                string path = Folder + "/ParityStructure.prefab"; var source = new GameObject("Structure"); source.AddComponent<ReviewComponent>();
                var prefab = PrefabUtility.SaveAsPrefabAsset(source, path); Object.DestroyImmediate(source);
                var component = prefab.GetComponent<ReviewComponent>(); string host = ObjectId(prefab), transform = ObjectId(prefab.transform), existing = ObjectId(component), created = "700000000000000001";
                string scriptGuid = AssetDatabase.AssetPathToGUID(AssetDatabase.GetAssetPath(MonoScript.FromMonoBehaviour(component)));
                var data = new Dictionary<string, object> { { "m_ObjectHideFlags", 0 }, { "m_CorrespondingSourceObject", new { fileID = "0" } }, { "m_PrefabInstance", new { fileID = "0" } }, { "m_PrefabAsset", new { fileID = "0" } },
                    { "m_GameObject", new { fileID = host } }, { "m_Enabled", 1 }, { "m_EditorHideFlags", 0 }, { "m_Script", new { fileID = "11500000", guid = scriptGuid, type = 3 } }, { "m_Name", "" }, { "m_EditorClassIdentifier", "" },
                    { "amount", 71 }, { "label", "base" }, { "wide", 7 }, { "behavior", new { rid = "-2" } }, { "sceneReference", new { fileID = "0" } } };
                object[] components = { new { component = new { fileID = transform } }, new { component = new { fileID = existing } }, new { component = new { fileID = created } } };
                ApplyAuthor(Author(path, existing, "", new { action = "editObjects", add = new[] { new { id = created, classId = "114", rootType = "MonoBehaviour", data } }, remove = new string[0], updates = new[] { new { objectId = host, propertyPath = "m_Component", value = components } } }));
                prefab = AssetDatabase.LoadAssetAtPath<GameObject>(path); bool added = prefab.GetComponents<ReviewComponent>().Length == 2 && prefab.GetComponents<ReviewComponent>()[1].amount == 71;
                ApplyAuthor(Author(path, existing, "", new { action = "editObjects", add = new object[0], remove = new[] { created }, updates = new[] { new { objectId = host, propertyPath = "m_Component", value = components.Take(2).ToArray() } } }));
                prefab = AssetDatabase.LoadAssetAtPath<GameObject>(path); observed = "added=" + added + "; remaining=" + prefab.GetComponents<ReviewComponent>().Length;
                return added && prefab.GetComponents<ReviewComponent>().Length == 1 && prefab.GetComponent<ReviewComponent>().amount == 10;
            });
            Check("YAML08-create-managed-template", "Template creation remaps colliding rid labels and preserves existing aliases after reload", () => {
                var data = ParityAsset("Create"); string path = AssetDatabase.GetAssetPath(data); long old = Snap(data, "alias").managedReferenceId;
                data.node = null; EditorUtility.SetDirty(data); AssetDatabase.SaveAssets();
                ApplyAuthor(Author(path, Id(data), "node", new { action = "createManaged", template = ManagedTemplate(true) }));
                observed = "type=" + data.node?.GetType().Name + "; old=" + old + "; node=" + Snap(data, "node").managedReferenceId;
                return data.node is ReviewOther && data.node.amount == 73 && !((ReviewOther)data.node).enabled && ReferenceEquals(data.node, data.node.next)
                    && data.alias.amount == 23 && ReferenceEquals(data.alias, data.alias.next) && old == Snap(data, "alias").managedReferenceId;
            });
            Check("YAML09-replace-managed-parity", "Explicit template fields agree with live type creation plus initialization", () => {
                var data = ParityAsset("Replace"); var live = ParityAsset("ReplaceLive");
                ApplyAuthor(Author(AssetDatabase.GetAssetPath(data), Id(data), "node", new { action = "createManaged", template = ManagedTemplate(false) }));
                LiveWrite(live, "node", "{\"action\":\"setType\",\"typeName\":\"PropertyReview.ReviewOther\",\"assemblyName\":\"PropertyReview.Runtime\"}");
                LiveWrite(live, "node.amount", "73"); LiveWrite(live, "node.hidden", "29"); LiveWrite(live, "node.enabled", "false"); AssetDatabase.SaveAssets();
                observed = "yaml=" + data.node?.GetType().Name + "; live=" + live.node?.GetType().Name;
                return data.node.GetType() == live.node.GetType() && data.node.amount == live.node.amount && data.node.hidden == live.node.hidden && data.node.next == null && !((ReviewOther)data.node).enabled && data.alias.amount == 23;
            });
            Check("YAML10-prefab-override-revert-apply", "Nested Variant override, Revert and cross-file Apply preserve source ownership", () => {
                string basePath = Folder + "/ParityBase.prefab", outerPath = Folder + "/ParityOuter.prefab", variantPath = Folder + "/ParityVariant.prefab";
                var go = new GameObject("Base"); go.AddComponent<ReviewComponent>(); var baseAsset = PrefabUtility.SaveAsPrefabAsset(go, basePath); Object.DestroyImmediate(go);
                var outer = new GameObject("Outer"); var nested = (GameObject)PrefabUtility.InstantiatePrefab(baseAsset); nested.transform.SetParent(outer.transform);
                var outerAsset = PrefabUtility.SaveAsPrefabAsset(outer, outerPath); Object.DestroyImmediate(outer);
                var variant = (GameObject)PrefabUtility.InstantiatePrefab(outerAsset); var variantAsset = PrefabUtility.SaveAsPrefabAsset(variant, variantPath); Object.DestroyImmediate(variant);
                string id = ObjectId(variantAsset.GetComponentInChildren<ReviewComponent>());
                ApplyAuthor(Author(variantPath, id, "amount", new { action = "override", value = 93 }, basePath, outerPath));
                bool changed = AssetDatabase.LoadAssetAtPath<GameObject>(variantPath).GetComponentInChildren<ReviewComponent>().amount == 93 && baseAsset.GetComponent<ReviewComponent>().amount == 10;
                ApplyAuthor(Author(variantPath, id, "amount", new { action = "revert" }, basePath, outerPath));
                bool reverted = AssetDatabase.LoadAssetAtPath<GameObject>(variantPath).GetComponentInChildren<ReviewComponent>().amount == 10;
                ApplyAuthor(Author(variantPath, id, "amount", new { action = "override", value = 57 }, basePath, outerPath));
                var applied = Author(variantPath, id, "amount", new { action = "applyToSource", level = 2 }, basePath, outerPath);
                int files = applied.candidates.Length; ApplyAuthor(applied);
                int actual = AssetDatabase.LoadAssetAtPath<GameObject>(basePath).GetComponent<ReviewComponent>().amount;
                observed = "changed=" + changed + "; reverted=" + reverted + "; source=" + actual + "; files=" + files;
                return changed && reverted && actual == 57 && files == 2 && AssetDatabase.LoadAssetAtPath<GameObject>(variantPath).GetComponentInChildren<ReviewComponent>().amount == 57;
            });
            Check("YAML11-nested-instance-isolation", "Two instances of one prefab resolve distinct virtual IDs and only the selected instance changes", () => {
                string sourcePath = Folder + "/ParityBase.prefab", path = Folder + "/ParityTwins.prefab";
                var root = new GameObject("Twins"); var source = AssetDatabase.LoadAssetAtPath<GameObject>(sourcePath);
                for (int i = 0; i < 2; i++) { var child = (GameObject)PrefabUtility.InstantiatePrefab(source); child.transform.SetParent(root.transform); }
                var asset = PrefabUtility.SaveAsPrefabAsset(root, path); Object.DestroyImmediate(root);
                string id = ObjectId(asset.transform.GetChild(1).GetComponent<ReviewComponent>());
                ApplyAuthor(Author(path, id, "amount", new { action = "override", value = 129 }, sourcePath));
                asset = AssetDatabase.LoadAssetAtPath<GameObject>(path);
                observed = asset.transform.GetChild(0).GetComponent<ReviewComponent>().amount + "," + asset.transform.GetChild(1).GetComponent<ReviewComponent>().amount;
                return asset.transform.GetChild(0).GetComponent<ReviewComponent>().amount == 57 && asset.transform.GetChild(1).GetComponent<ReviewComponent>().amount == 129;
            });
            Check("YAML15-source-dependency-guard", "Dirty or stale read-only source dependencies reject the whole disk batch before writes", () => {
                var source = ParityAsset("Dependency"); var target = ParityAsset("DependencyTarget"); string sourcePath = AssetDatabase.GetAssetPath(source), targetPath = AssetDatabase.GetAssetPath(target);
                var entry = PrepareYaml(target, new object[] { Op(Id(target), "regularInt", 700) }); string hash;
                using (var sha = SHA256.Create()) hash = BitConverter.ToString(sha.ComputeHash(File.ReadAllBytes(sourcePath))).Replace("-", "").ToLowerInvariant();
                string request = Json(new { action = "disk_apply", transaction_id = Guid.NewGuid().ToString(), entries = new[] { entry }, dependencies = new[] { new { path = sourcePath, expected_sha256 = hash } } });
                source.regularInt = 101; EditorUtility.SetDirty(source); bool dirty = false, stale = false;
                try { Call("ExecuteAssetApiRequest", request); } catch (Exception e) { dirty = e.ToString().Contains("dirty_asset"); }
                AssetDatabase.SaveAssets(); byte[] before = File.ReadAllBytes(targetPath);
                try { Call("ExecuteAssetApiRequest", request); } catch (Exception e) { stale = e.ToString().Contains("revision_conflict"); }
                observed = "dirty=" + dirty + "; stale=" + stale; return dirty && stale && before.SequenceEqual(File.ReadAllBytes(targetPath)) && target.regularInt == 17;
            });
            Check("YAML17-unity-override-string-parity", "Unity-authored numeric-looking string overrides keep their exact text in the shared graph", () => {
                string basePath = Folder + "/ParityBase.prefab", path = Folder + "/ParityLexical.prefab";
                var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(basePath));
                var prefab = PrefabUtility.SaveAsPrefabAsset(instance, path); Object.DestroyImmediate(instance);
                string id = ObjectId(prefab.GetComponent<ReviewComponent>());
                foreach (string value in new[] { "007", "1e3", "null", "true", "00123456789012345678901234567890", "" }) {
                    var contents = PrefabUtility.LoadPrefabContents(path); var component = contents.GetComponent<ReviewComponent>();
                    component.label = value; PrefabUtility.RecordPrefabInstancePropertyModifications(component);
                    PrefabUtility.SaveAsPrefabAsset(contents, path); PrefabUtility.UnloadPrefabContents(contents);
                    string actual = Author(path, id, "label", new { action = "read" }, basePath).value_json;
                    if (actual != Json(value)) { observed = "expected=" + Json(value) + "; actual=" + actual; return false; }
                }
                observed = "six lexical string forms preserved"; return true;
            });
            Check("YAML18-small-to-int64-apply", "An Int64 override of a small source value stays exact through cross-layer Apply and reimport", () => {
                string basePath = Folder + "/ParityBase.prefab", path = Folder + "/ParityWide.prefab";
                var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(basePath));
                var prefab = PrefabUtility.SaveAsPrefabAsset(instance, path); Object.DestroyImmediate(instance); string id = ObjectId(prefab.GetComponent<ReviewComponent>());
                ApplyAuthor(Author(path, id, "wide", new { action = "override", value = new { kind = "int64", value = "9223372036854775807" } }, basePath));
                bool exact = AssetDatabase.LoadAssetAtPath<GameObject>(path).GetComponent<ReviewComponent>().wide == long.MaxValue;
                ApplyAuthor(Author(path, id, "wide", new { action = "applyToSource", level = 1 }, basePath));
                observed = "overrideExact=" + exact + "; source=" + AssetDatabase.LoadAssetAtPath<GameObject>(basePath).GetComponent<ReviewComponent>().wide;
                return exact && AssetDatabase.LoadAssetAtPath<GameObject>(basePath).GetComponent<ReviewComponent>().wide == long.MaxValue;
            });
            Check("YAML16-variant-scene-propagation", "Scene instance override/Revert uses the transitive Variant graph and reloads the scene", () => {
                string basePath = Folder + "/ParityBase.prefab", outerPath = Folder + "/ParityOuter.prefab", variantPath = Folder + "/ParityVariant.prefab", scenePath = Folder + "/ParityScene.unity";
                var previous = UnityEngine.SceneManagement.SceneManager.GetActiveScene();
                var scene = UnityEditor.SceneManagement.EditorSceneManager.NewScene(UnityEditor.SceneManagement.NewSceneSetup.EmptyScene, UnityEditor.SceneManagement.NewSceneMode.Additive);
                UnityEngine.SceneManagement.SceneManager.SetActiveScene(scene);
                try {
                    var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>(variantPath));
                    UnityEditor.SceneManagement.EditorSceneManager.SaveScene(scene, scenePath);
                    var gid = GlobalObjectId.GetGlobalObjectIdSlow(instance.GetComponentInChildren<ReviewComponent>());
                    string id = ((gid.targetObjectId ^ gid.targetPrefabId) & 0x7fffffffffffffffUL).ToString(CultureInfo.InvariantCulture);
                    ApplyAuthor(Author(scenePath, id, "amount", new { action = "override", value = 173 }, basePath, outerPath, variantPath));
                    scene = UnityEngine.SceneManagement.SceneManager.GetSceneByPath(scenePath);
                    bool changed = scene.GetRootGameObjects().SelectMany(g => g.GetComponentsInChildren<ReviewComponent>()).Single().amount == 173;
                    ApplyAuthor(Author(scenePath, id, "amount", new { action = "revert" }, basePath, outerPath, variantPath));
                    scene = UnityEngine.SceneManagement.SceneManager.GetSceneByPath(scenePath); int value = scene.GetRootGameObjects().SelectMany(g => g.GetComponentsInChildren<ReviewComponent>()).Single().amount;
                    observed = "changed=" + changed + "; inherited=" + value; return changed && value == 57;
                } finally {
                    if (previous.IsValid() && previous.isLoaded) UnityEngine.SceneManagement.SceneManager.SetActiveScene(previous);
                    scene = UnityEngine.SceneManagement.SceneManager.GetSceneByPath(scenePath);
                    if (scene.IsValid() && scene.isLoaded) UnityEditor.SceneManagement.EditorSceneManager.CloseScene(scene, true);
                }
            });
        }
    }
}
