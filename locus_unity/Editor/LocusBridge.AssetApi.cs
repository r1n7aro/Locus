using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace Locus
{
    public static partial class LocusBridge
    {
        // The public SDK sends JSON values. The Rust transport encodes each value as
        // value_json so the bridge does not bind to Unity's Newtonsoft assembly.
        [Serializable]
        private sealed class AssetApiRequest
        {
            public string action;
            public string path;
            public string expected_revision;
            public AssetApiOperation[] operations;
            public string persist = "disk";
            public AssetApiRequest[] entries;
            public AssetApiRequest[] dependencies;
            public string transaction_id;
            public string expected_sha256;
            public string bytes_base64;
            public string[] paths;
        }

        [Serializable]
        private sealed class AssetApiOperation
        {
            public string op;
            public string object_id;
            public string property_path;
            public string value_json;
            public int index = -1;
            public int to_index = -1;
            public int size = -1;
        }

        private sealed class AssetApiObject : IDisposable
        {
            public UnityEngine.Object target;
            public SerializedObject serialized;
            public string id;
            public string classId;
            public string rootType;
            public bool dirty;
            public void Dispose() { if (serialized != null) serialized.Dispose(); }
        }

        private sealed class AssetApiScope : IDisposable
        {
            public string path;
            public string absolutePath;
            public List<AssetApiObject> objects = new List<AssetApiObject>();
            public List<object> diagnostics = new List<object>();
            public bool containsPrefabInheritance;
            public Scene openedScene;
            public bool sceneWasDirty;
            public void Dispose()
            {
                foreach (AssetApiObject item in objects) item.Dispose();
                // An apply(persist:false) intentionally leaves newly opened scenes
                // open: closing one would discard the promised live edit.
                if (openedScene.IsValid() && openedScene.isLoaded && !openedScene.isDirty)
                    EditorSceneManager.CloseScene(openedScene, true);
            }
        }

        private static Task<PipeEnvelope> HandleAssetApi(string requestId, string message)
        {
            return RunPropertyTreeOnMainThread(requestId, "asset_api", delegate
            {
                using (LocusAssetApiSaveGuard.Protect())
                    return ExecuteAssetApiRequest(message);
            });
        }

        private static string ExecuteAssetApiRequest(string message)
        {
                AssetApiRequest request = DeserializeJson<AssetApiRequest>(message);
                if (request == null) throw new Exception("invalid_request: empty asset request");
                if (EditorApplication.isPlayingOrWillChangePlaymode || EditorApplication.isCompiling || EditorApplication.isUpdating)
                    throw new Exception("editor_busy: live asset access requires an idle editor in edit mode");
                if (request.action == "disk_preflight")
                {
                    var paths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                    foreach (string path in request.paths ?? new string[0])
                    {
                        if (string.IsNullOrEmpty(path) || (!path.StartsWith("Assets/", StringComparison.Ordinal) && !path.StartsWith("Packages/", StringComparison.Ordinal))
                            || path.Contains('\\') || path.Contains(':') || path.Split('/').Any(part => part.Length == 0 || part == "." || part == ".."))
                            throw new Exception("invalid_path: disk preflight requires project-relative asset paths");
                        paths.Add(path.EndsWith(".meta", StringComparison.Ordinal) ? path.Substring(0, path.Length - 5) : path);
                    }
                    AssetApiPreflightDiskPaths(paths);
                    return AssetApiJson(new Dictionary<string, object> { { "ready", true }, { "checked_paths", paths.ToArray() } });
                }
                if (request.action == "disk_apply") return ExecuteAssetApiDiskEdits(request);
                if (request.action != "read") return ExecuteAssetApiEdits(request);
                using (AssetApiScope scope = OpenAssetApiScope(request.path))
                {
                    return AssetApiJson(ReadAssetApiSnapshot(scope));
                }
        }

        private sealed class AssetApiDiskEntry
        {
            public string path;
            public string absolutePath;
            public byte[] before;
            public byte[] after;
        }

        private sealed class AssetApiReloadContext
        {
            public List<string> scenePaths = new List<string>();
            public string activeScenePath;
            public string prefabStagePath;
        }

        private static string ExecuteAssetApiDiskEdits(AssetApiRequest request)
        {
            var phaseTimer = System.Diagnostics.Stopwatch.StartNew();
            Guid transaction;
            if (!Guid.TryParse(request.transaction_id, out transaction)) throw new Exception("invalid_request: disk transaction ID must be a UUID");
            if (request.entries == null || request.entries.Length == 0 || request.entries.Length > 256)
                throw new Exception("invalid_request: disk transaction requires 1 to 256 entries");
            var entries = new List<AssetApiDiskEntry>();
            var paths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (AssetApiRequest entry in request.entries)
            {
                if (entry == null) throw new Exception("invalid_request: null disk entry");
                string absolutePath = ValidateAssetApiPath(entry.path);
                if (!paths.Add(entry.path)) throw new Exception("invalid_request: duplicate disk asset path");
                if (string.IsNullOrEmpty(entry.expected_sha256) || !Regex.IsMatch(entry.expected_sha256, "^[a-fA-F0-9]{64}$"))
                    throw new Exception("invalid_request: expected_sha256 is required");
                byte[] before = File.ReadAllBytes(absolutePath);
                if (!string.Equals(AssetApiSha256(before), entry.expected_sha256, StringComparison.OrdinalIgnoreCase))
                    throw new Exception("revision_conflict: asset changed before disk apply: " + entry.path);
                byte[] after;
                try { after = Convert.FromBase64String(entry.bytes_base64 ?? ""); }
                catch (FormatException) { throw new Exception("invalid_request: asset bytes are not base64"); }
                if (after.Length == 0 || after.Length > 128 * 1024 * 1024)
                    throw new Exception("limit_exceeded: disk asset must contain 1 to 134217728 bytes");
                entries.Add(new AssetApiDiskEntry { path = entry.path, absolutePath = absolutePath, before = before, after = after });
            }
            var dependencies = new List<AssetApiDiskEntry>();
            foreach (AssetApiRequest dependency in request.dependencies ?? new AssetApiRequest[0])
            {
                if (dependency == null || string.IsNullOrEmpty(dependency.path) || paths.Contains(dependency.path))
                    throw new Exception("invalid_request: dependency must be a distinct read-only path");
                string dependencyPath = dependency.path;
                string assetPath = dependencyPath.EndsWith(".meta", StringComparison.Ordinal) ? dependencyPath.Substring(0, dependencyPath.Length - 5) : dependencyPath;
                string absolute = ValidateAssetApiPath(assetPath, true) + (dependencyPath.EndsWith(".meta", StringComparison.Ordinal) ? ".meta" : "");
                byte[] bytes = File.ReadAllBytes(absolute);
                if (!string.Equals(AssetApiSha256(bytes), dependency.expected_sha256, StringComparison.OrdinalIgnoreCase))
                    throw new Exception("revision_conflict: source dependency changed: " + dependencyPath);
                dependencies.Add(new AssetApiDiskEntry { path = dependencyPath, absolutePath = absolute, before = bytes });
            }
            var preflightPaths = new HashSet<string>(paths, StringComparer.OrdinalIgnoreCase);
            foreach (AssetApiDiskEntry dependency in dependencies)
                preflightPaths.Add(dependency.path.EndsWith(".meta", StringComparison.Ordinal) ? dependency.path.Substring(0, dependency.path.Length - 5) : dependency.path);
            AssetApiReloadContext reload = AssetApiPreflightDiskPaths(preflightPaths);
            Action checkDependencies = () => {
                foreach (AssetApiDiskEntry dependency in dependencies)
                    if (!File.ReadAllBytes(dependency.absolutePath).SequenceEqual(dependency.before))
                        throw new Exception("revision_conflict: source dependency changed: " + dependency.path);
            };
            checkDependencies();
            // A preflight rejection must never enter restoration: it has not
            // acquired ownership of any changed file or editor state.
            foreach (AssetApiDiskEntry entry in entries)
                if (!File.ReadAllBytes(entry.absolutePath).SequenceEqual(entry.before))
                    throw new Exception("revision_conflict: asset changed during disk preflight: " + entry.path);

            var attempted = new List<AssetApiDiskEntry>();
            double preflightMs = phaseTimer.Elapsed.TotalMilliseconds;
            try
            {
                phaseTimer.Restart();
                AssetDatabase.StartAssetEditing();
                try
                {
                    AssetDatabase.ReleaseCachedFileHandles();
                    foreach (AssetApiDiskEntry entry in entries)
                    {
                        if (!File.ReadAllBytes(entry.absolutePath).SequenceEqual(entry.before))
                            throw new Exception("revision_conflict: asset changed during disk apply: " + entry.path);
                        if (entry.before.SequenceEqual(entry.after)) continue;
                        attempted.Add(entry);
                        AssetApiReplaceFile(entry.absolutePath, entry.after, entry.before);
                    }
                    checkDependencies();
                }
                finally { AssetDatabase.StopAssetEditing(); }
                double writeMs = phaseTimer.Elapsed.TotalMilliseconds;
                phaseTimer.Restart();
                AssetApiReloadDiskPaths(reload, attempted.Select(entry => entry.path));
                checkDependencies();
                return AssetApiJson(new Dictionary<string, object> {
                    { "applied", true }, { "persisted", true }, { "transaction_id", request.transaction_id },
                    { "timings", new Dictionary<string, object> { { "preflightMs", preflightMs }, { "writeMs", writeMs }, { "importMs", phaseTimer.Elapsed.TotalMilliseconds }, { "changedFiles", attempted.Count } } }
                });
            }
            catch (Exception failure)
            {
                var recoveryErrors = new List<string>();
                var restored = new List<string>();
                AssetDatabase.StartAssetEditing();
                try
                {
                    AssetDatabase.ReleaseCachedFileHandles();
                    foreach (AssetApiDiskEntry entry in attempted)
                    {
                        try
                        {
                            byte[] current = File.ReadAllBytes(entry.absolutePath);
                            if (current.SequenceEqual(entry.before)) continue;
                            if (!current.SequenceEqual(entry.after))
                            {
                                recoveryErrors.Add("external change preserved: " + entry.path);
                                continue;
                            }
                            AssetApiReplaceFile(entry.absolutePath, entry.before, entry.after);
                            restored.Add(entry.path);
                        }
                        catch (Exception restoreFailure) { recoveryErrors.Add(entry.path + ": " + restoreFailure.Message); }
                    }
                }
                finally { AssetDatabase.StopAssetEditing(); }
                try { AssetApiReloadDiskPaths(reload, restored); }
                catch (Exception reloadFailure) { recoveryErrors.Add("editor reload: " + reloadFailure.Message); }
                throw new Exception((recoveryErrors.Count == 0 ? "disk_apply_failed: " : "recovery_required: ") + failure.Message
                    + "; transaction_id=" + request.transaction_id + (recoveryErrors.Count == 0 ? "; rolled_back" : "; " + string.Join("; ", recoveryErrors)));
            }
        }

        private static AssetApiReloadContext AssetApiPreflightDiskPaths(HashSet<string> paths)
        {
            var reload = new AssetApiReloadContext { activeScenePath = SceneManager.GetActiveScene().path };
            for (int i = 0; i < SceneManager.sceneCount; i++)
            {
                Scene scene = SceneManager.GetSceneAt(i);
                if (!scene.isLoaded || !paths.Contains(scene.path)) continue;
                if (scene.isDirty) throw new Exception("dirty_asset: save or revert the target scene before YAML editing: " + scene.path);
                reload.scenePaths.Add(scene.path);
            }
            var stage = PrefabStageUtility.GetCurrentPrefabStage();
            if (stage != null && paths.Contains(stage.assetPath))
            {
                if (stage.scene.isDirty) throw new Exception("dirty_asset: save or revert the target Prefab Stage before YAML editing: " + stage.assetPath);
                reload.prefabStagePath = stage.assetPath;
            }
            // One loaded-object pass for the entire batch; no LoadAllAssetsAtPath
            // per file, so unopened bulk assets remain unopened during preflight.
            foreach (UnityEngine.Object target in Resources.FindObjectsOfTypeAll<UnityEngine.Object>())
            {
                if (target == null || !EditorUtility.IsPersistent(target) || !EditorUtility.IsDirty(target)) continue;
                string path = AssetDatabase.GetAssetPath(target);
                if (paths.Contains(path)) throw new Exception("dirty_asset: save or revert the target asset before YAML editing: " + path);
            }
            return reload;
        }

        private static void AssetApiReloadDiskPaths(AssetApiReloadContext context, IEnumerable<string> changedPaths)
        {
            string[] paths = changedPaths.Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            if (paths.Length == 0) return;
            var affected = new HashSet<string>(paths, StringComparer.OrdinalIgnoreCase);
            string[] scenes = context.scenePaths.Where(affected.Contains).ToArray();
            bool reloadStage = !string.IsNullOrEmpty(context.prefabStagePath) && affected.Contains(context.prefabStagePath);
            Scene scratch = default(Scene);
            var closed = new List<string>();
            try
            {
                if (reloadStage)
                {
                    var stage = PrefabStageUtility.GetCurrentPrefabStage();
                    if (stage != null && stage.assetPath == context.prefabStagePath && stage.scene.isDirty)
                        throw new Exception("dirty_asset: target Prefab Stage became dirty during import: " + context.prefabStagePath);
                    StageUtility.GoToMainStage();
                }
                if (scenes.Length > 0)
                {
                    // Unity cannot close its last scene; an empty temporary scene
                    // lets us reload targets without closing unrelated scenes.
                    scratch = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Additive);
                    foreach (string path in scenes)
                    {
                        Scene scene = SceneManager.GetSceneByPath(path);
                        if (!scene.IsValid() || !scene.isLoaded) { closed.Add(path); continue; }
                        if (scene.isDirty) throw new Exception("dirty_asset: target scene became dirty during import: " + path);
                        if (!EditorSceneManager.CloseScene(scene, true)) throw new Exception("reload_failed: could not close target scene " + path);
                        closed.Add(path);
                    }
                }
                foreach (string path in paths)
                    AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate | ImportAssetOptions.ForceSynchronousImport);
            }
            finally
            {
                foreach (string path in closed)
                    if (!SceneManager.GetSceneByPath(path).IsValid()) EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
                Scene active = SceneManager.GetSceneByPath(context.activeScenePath ?? "");
                if (active.IsValid() && active.isLoaded) SceneManager.SetActiveScene(active);
                if (scratch.IsValid() && scratch.isLoaded && !scratch.isDirty) EditorSceneManager.CloseScene(scratch, true);
                if (reloadStage) PrefabStageUtility.OpenPrefab(context.prefabStagePath);
            }
        }

        private static void AssetApiReplaceFile(string path, byte[] bytes, byte[] expected)
        {
            LocusAssetApiAtomicFile.Replace(path, bytes, expected);
        }

        private static string AssetApiSha256(byte[] bytes)
        {
            using (SHA256 hash = SHA256.Create())
                return BitConverter.ToString(hash.ComputeHash(bytes)).Replace("-", "").ToLowerInvariant();
        }

        private static string ValidateAssetApiPath(string requestedPath, bool readDependency = false)
        {
            string path = requestedPath ?? "";
            string project = Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
            string absolute = Path.GetFullPath(Path.Combine(project, path));
            bool package = readDependency && path.StartsWith("Packages/", StringComparison.Ordinal);
            string assetsRoot = Path.GetFullPath(package ? Path.Combine(project, "Packages") : Application.dataPath) + Path.DirectorySeparatorChar;
            if ((!path.StartsWith("Assets/", StringComparison.Ordinal) && !package) || path.Contains('\\') || path.Contains(':')
                || !absolute.StartsWith(assetsRoot, StringComparison.OrdinalIgnoreCase)
                || path.Split('/').Any(part => part.Length == 0 || part == ".." || part == "."))
                throw new Exception("invalid_path: asset path must be inside Assets");
            string current = project;
            foreach (string segment in path.Split('/'))
            {
                current = Path.Combine(current, segment);
                if ((File.Exists(current) || Directory.Exists(current)) && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                    throw new Exception("invalid_path: reparse points are not writable asset paths");
            }
            if (!File.Exists(absolute)) throw new Exception("asset_not_found: " + path);
            string[] extensions = { ".asset", ".mat", ".prefab", ".unity", ".controller", ".anim", ".overrideController", ".playable", ".mask" };
            if (!extensions.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase)
                && !(readDependency && new[] { ".cs", ".asmdef" }.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase)))
                throw new Exception("unsupported_capability: asset serialization format " + Path.GetExtension(path));
            return absolute;
        }

        private static string ExecuteAssetApiEdits(AssetApiRequest request)
        {
            bool batch = request.action == "preview_batch" || request.action == "apply_batch";
            bool previewOnly = request.action == "preview" || request.action == "preview_batch";
            if (!previewOnly && request.action != "apply" && request.action != "apply_batch")
                throw new Exception("unsupported_capability: unknown asset action " + request.action);
            if (!string.IsNullOrEmpty(request.persist) && request.persist != "disk")
                throw new Exception("unsupported_capability: asset API persist must be disk");
            AssetApiRequest[] entries = batch ? request.entries : new[] { request };
            if (entries == null || entries.Length == 0) throw new Exception("invalid_request: batch requires entries");
            var scopes = new List<AssetApiScope>();
            var changes = new List<HashSet<AssetApiObject>>();
            var revisions = new List<string>();
            var results = new List<object>();
            var originals = new List<byte[]>();
            var uniquePaths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                // Stage every file before applying any SerializedObject. Dispose
                // discards all pending state if any later operation fails.
                for (int n = 0; n < entries.Length; n++)
                {
                    AssetApiRequest entry = entries[n];
                    if (entry == null) throw new Exception("invalid_request: null batch entry");
                    AssetApiScope scope = OpenAssetApiScope(entry.path);
                    scopes.Add(scope);
                    if (!uniquePaths.Add(scope.absolutePath)) throw new Exception("invalid_request: duplicate batch path " + entry.path);
                    originals.Add(File.ReadAllBytes(scope.absolutePath));
                    string revision = (string)ReadAssetApiSnapshot(scope)["revision"];
                    revisions.Add(revision);
                    if (!previewOnly && string.IsNullOrEmpty(entry.expected_revision))
                        throw new Exception("revision_required: read the asset before editing");
                    if (!string.IsNullOrEmpty(entry.expected_revision) && !string.Equals(revision, entry.expected_revision, StringComparison.Ordinal))
                        throw new Exception("revision_conflict: asset changed since the supplied snapshot");
                    var changed = new HashSet<AssetApiObject>();
                    changes.Add(changed);
                    AssetApiOperation[] operations = entry.operations ?? new AssetApiOperation[0];
                    for (int i = 0; i < operations.Length; i++)
                    {
                        AssetApiOperation operation = operations[i];
                        if (operation == null) throw new Exception("invalid_operation: null operation");
                        AssetApiObject item = scope.objects.Find(candidate => candidate.id == operation.object_id);
                        if (item == null)
                            throw new Exception(scope.containsPrefabInheritance
                                ? "unsupported_capability: inherited Prefab fields require an effective-value adapter"
                                : "object_not_found: " + operation.object_id);
                        SerializedProperty property = FindAssetApiProperty(item, operation.property_path);
                        if (property == null) throw new Exception("property_not_found: " + operation.property_path);
                        ApplyAssetApiOperation(scope, item, property, operation);
                        changed.Add(item);
                    }
                    results.Add(AssetApiEditResult(scope, revision, operations.Length, false));
                }
                if (previewOnly) return AssetApiBatchResult(batch, false, results);
                for (int n = 0; n < scopes.Count; n++)
                    if (!File.ReadAllBytes(scopes[n].absolutePath).SequenceEqual(originals[n]))
                        throw new Exception("revision_conflict: asset file changed while staging edits");
                Undo.IncrementCurrentGroup();
                int undoGroup = Undo.GetCurrentGroup();
                Undo.SetCurrentGroupName("Locus Asset API");
                var persistedBytes = new Dictionary<AssetApiScope, byte[]>();
                var savingAttempted = new HashSet<AssetApiScope>();
                var undoTargets = new HashSet<AssetApiObject>();
                try
                {
                    for (int n = 0; n < scopes.Count; n++)
                    {
                        if (changes[n].Count == 0) continue;
                        if (!File.ReadAllBytes(scopes[n].absolutePath).SequenceEqual(originals[n]))
                            throw new Exception("revision_conflict: asset file changed before save: " + scopes[n].path);
                        // Validation for the whole batch is already complete.
                        // Publish and save one file at a time: saving a scene or
                        // prefab may flush other dirty assets. Later entries must
                        // remain private SerializedObject buffers until their turn.
                        // Complete-object Undo can itself mark objects dirty, so
                        // its registration belongs inside the same per-file step.
                        foreach (AssetApiObject item in changes[n])
                        {
                            Undo.RegisterCompleteObjectUndo(item.target, "Locus Asset API");
                            undoTargets.Add(item);
                        }
                        foreach (AssetApiObject item in changes[n])
                        {
                            item.serialized.ApplyModifiedPropertiesWithoutUndo();
                            if (PrefabUtility.IsPartOfPrefabInstance(item.target))
                                PrefabUtility.RecordPrefabInstancePropertyModifications(item.target);
                            MarkPropertyTreeObjectDirty(item.target);
                        }
                        savingAttempted.Add(scopes[n]);
                        SaveAssetApiScope(scopes[n], changes[n]);
                        persistedBytes[scopes[n]] = File.ReadAllBytes(scopes[n].absolutePath);
                    }
                    results.Clear();
                    for (int n = 0; n < scopes.Count; n++)
                    {
                        foreach (AssetApiObject item in scopes[n].objects) item.serialized.Update();
                        results.Add(AssetApiEditResult(scopes[n], revisions[n], (entries[n].operations ?? new AssetApiOperation[0]).Length, true));
                    }
                    Undo.CollapseUndoOperations(undoGroup);
                    return AssetApiBatchResult(batch, true, results);
                }
                catch (Exception failure)
                {
                    var recoveryErrors = new List<string>();
                    bool undoSucceeded = false;
                    try { Undo.RevertAllDownToGroup(undoGroup); undoSucceeded = true; }
                    catch (Exception undoFailure) { recoveryErrors.Add("undo: " + undoFailure.Message); }
                    try { AssetDatabase.ReleaseCachedFileHandles(); }
                    catch (Exception releaseFailure) { recoveryErrors.Add("file handles: " + releaseFailure.Message); }
                    for (int n = 0; n < scopes.Count; n++)
                    {
                        AssetApiScope scope = scopes[n];
                        try
                        {
                            byte[] current = File.ReadAllBytes(scope.absolutePath);
                            byte[] ownBytes;
                            if (!current.SequenceEqual(originals[n]))
                            {
                                if (persistedBytes.TryGetValue(scope, out ownBytes) && current.SequenceEqual(ownBytes))
                                    AssetApiReplaceFile(scope.absolutePath, originals[n], ownBytes);
                                else recoveryErrors.Add((savingAttempted.Contains(scope) ? "unknown or externally changed persisted bytes: " : "external change preserved: ") + scope.path);
                            }
                        }
                        catch (Exception restoreFailure) { recoveryErrors.Add(scope.path + ": " + restoreFailure.Message); }
                        foreach (AssetApiObject item in changes[n])
                            if (undoSucceeded && undoTargets.Contains(item) && !item.dirty) EditorUtility.ClearDirty(item.target);
                    }
                    if (recoveryErrors.Count != 0) throw new Exception("recovery_required: " + failure.Message + "; " + string.Join("; ", recoveryErrors));
                    throw;
                }
            }
            finally { foreach (AssetApiScope scope in scopes) scope.Dispose(); }
        }

        private static Dictionary<string, object> AssetApiEditResult(AssetApiScope scope, string revision, int count, bool applied)
        {
            return new Dictionary<string, object> { { "path", scope.path }, { "applied", applied }, { "persisted", applied },
                { "snapshot", ReadAssetApiSnapshot(scope) }, { "previous_revision", revision },
                { "operations_count", count }, { "diagnostics", new object[0] } };
        }

        private static string AssetApiBatchResult(bool batch, bool applied, List<object> results)
        {
            return AssetApiJson(batch ? (object)new Dictionary<string, object> {
                { "applied", applied }, { "persisted", applied }, { "results", results }
            } : results[0]);
        }

        private static AssetApiScope OpenAssetApiScope(string requestedPath)
        {
            string path = (requestedPath ?? "").Replace('\\', '/');
            string project = Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
            string absolute = Path.GetFullPath(Path.Combine(project, path));
            string assetsRoot = Path.GetFullPath(Application.dataPath) + Path.DirectorySeparatorChar;
            if (!path.StartsWith("Assets/", StringComparison.Ordinal)
                || !absolute.StartsWith(assetsRoot, StringComparison.OrdinalIgnoreCase)
                || path.Split('/').Any(part => part == ".." || part == "."))
                throw new Exception("invalid_path: asset path must be inside Assets");
            if (!File.Exists(absolute)) throw new Exception("asset_not_found: " + path);
            string[] extensions = { ".asset", ".mat", ".prefab", ".unity", ".controller", ".anim", ".overrideController", ".playable", ".mask" };
            if (!extensions.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase))
                throw new Exception("unsupported_capability: asset serialization format " + Path.GetExtension(path));

            var metadata = new Dictionary<string, string[]>();
            // Header metadata is only used for persistent identities and class roots.
            // Every value below is read from SerializedObject, including dirty values.
            string text = File.ReadAllText(absolute);
            foreach (Match match in Regex.Matches(text, @"(?m)^--- !u!(-?\d+) &(-?\d+)[^\r\n]*\r?\n([^:\r\n]+):"))
                metadata[match.Groups[2].Value] = new[] { match.Groups[1].Value, match.Groups[3].Value,
                    match.Value.Substring(0, match.Value.IndexOf('\n')).Contains(" stripped") ? "stripped" : "materialized" };
            if (metadata.Count == 0) throw new Exception("unsupported_capability: asset is not Unity text YAML");

            var scope = new AssetApiScope { path = path, absolutePath = absolute,
                containsPrefabInheritance = metadata.Values.Any(info => info[0] == "1001" || info[2] == "stripped") };
            try
            {
                var targets = new List<UnityEngine.Object>();
                var stage = PrefabStageUtility.GetCurrentPrefabStage();
                if (stage != null && stage.assetPath == path)
                {
                    AssetApiCollectHierarchy(stage.prefabContentsRoot, targets);
                    scope.sceneWasDirty = stage.scene.isDirty;
                }
                else if (path.EndsWith(".unity", StringComparison.OrdinalIgnoreCase))
                {
                    Scene scene = SceneManager.GetSceneByPath(path);
                    if (!scene.IsValid() || !scene.isLoaded)
                    {
                        scene = EditorSceneManager.OpenScene(path, OpenSceneMode.Additive);
                        scope.openedScene = scene;
                    }
                    scope.sceneWasDirty = scene.isDirty;
                    foreach (GameObject root in scene.GetRootGameObjects()) AssetApiCollectHierarchy(root, targets);
                }
                else
                {
                    targets.AddRange(AssetDatabase.LoadAllAssetsAtPath(path));
                    GameObject prefab = AssetDatabase.LoadAssetAtPath<GameObject>(path);
                    if (prefab != null) AssetApiCollectHierarchy(prefab, targets);
                }
#if UNITY_6000_5_OR_NEWER
                var seen = new HashSet<EntityId>();
#else
                var seen = new HashSet<int>();
#endif
                foreach (UnityEngine.Object target in targets)
                {
#if UNITY_6000_5_OR_NEWER
                    if (target == null || !seen.Add(target.GetEntityId())) continue;
#else
                    if (target == null || !seen.Add(target.GetInstanceID())) continue;
#endif
                    long id;
                    if (!TryGetLocalFileId(target, out id) || id == 0) continue;
                    string[] info;
                    if (!metadata.TryGetValue(id.ToString(CultureInfo.InvariantCulture), out info)) continue;
                    if (info[0] == "1001" || info[2] == "stripped") continue;
                    var item = new AssetApiObject { target = target, id = id.ToString(CultureInfo.InvariantCulture),
                        classId = info[0], rootType = info[1], dirty = EditorUtility.IsDirty(target), serialized = new SerializedObject(target) };
                    item.serialized.Update();
                    scope.objects.Add(item);
                }
                scope.objects.Sort((a, b) => string.CompareOrdinal(a.id, b.id));
                if (scope.objects.Count == 0) throw new Exception(scope.containsPrefabInheritance
                    ? "unsupported_capability: inherited Prefab fields require an effective-value adapter"
                    : "unsupported_capability: no loaded serialized objects at " + path);
                foreach (string id in metadata.Keys.Where(id => !scope.objects.Any(item => item.id == id)))
                    scope.diagnostics.Add(new Dictionary<string, object> {
                        { "code", metadata[id][0] == "1001" || metadata[id][2] == "stripped" ? "unsupported_prefab_inheritance" : "unavailable_live_object" }, { "severity", "warning" },
                        { "message", metadata[id][0] == "1001" || metadata[id][2] == "stripped"
                            ? "Inherited Prefab fields require an effective-value adapter; this object is omitted."
                            : "This YAML object is not exposed as a loaded Unity serialized object." },
                        { "object_id", id }, { "property_path", null },
                        { "span", new Dictionary<string, object> { { "start", 0 }, { "end", 0 } } }
                    });
                return scope;
            }
            catch { scope.Dispose(); throw; }
        }

        private static void AssetApiCollectHierarchy(GameObject root, List<UnityEngine.Object> output)
        {
            foreach (Transform transform in root.GetComponentsInChildren<Transform>(true))
            {
                output.Add(transform.gameObject);
                output.AddRange(transform.GetComponents<Component>());
            }
        }

        private static void SaveAssetApiScope(AssetApiScope scope, HashSet<AssetApiObject> changed)
        {
            using (LocusAssetApiSaveGuard.AllowOnly(scope.path))
                SaveAssetApiScopeAllowed(scope, changed);
        }

        private static void SaveAssetApiScopeAllowed(AssetApiScope scope, HashSet<AssetApiObject> changed)
        {
            var stage = PrefabStageUtility.GetCurrentPrefabStage();
            if (stage != null && stage.assetPath == scope.path)
            {
                bool success;
                PrefabUtility.SaveAsPrefabAsset(stage.prefabContentsRoot, scope.path, out success);
                if (!success) throw new Exception("persist_failed: could not save prefab stage");
            }
            else if (scope.path.EndsWith(".unity", StringComparison.OrdinalIgnoreCase))
            {
                if (!EditorSceneManager.SaveScene(SceneManager.GetSceneByPath(scope.path)))
                    throw new Exception("persist_failed: could not save scene");
            }
            else if (scope.path.EndsWith(".prefab", StringComparison.OrdinalIgnoreCase))
            {
                bool success;
                PrefabUtility.SavePrefabAsset(AssetDatabase.LoadAssetAtPath<GameObject>(scope.path), out success);
                if (!success) throw new Exception("persist_failed: could not save prefab");
            }
            else
            {
                foreach (AssetApiObject item in changed) AssetDatabase.SaveAssetIfDirty(item.target);
                if (changed.Any(item => EditorUtility.IsDirty(item.target)))
                    throw new Exception("persist_failed: asset remains dirty after save");
            }
        }

        private static Dictionary<string, object> ReadAssetApiSnapshot(AssetApiScope scope)
        {
            var objects = new List<object>();
            foreach (AssetApiObject item in scope.objects)
            {
                var fields = new List<object>();
                var managed = new Dictionary<long, SerializedProperty>();
                SerializedProperty iterator = item.serialized.GetIterator();
                if (iterator.Next(true)) do
                {
                    string pointer = "/" + AssetApiEscape(item.rootType) + "/" + AssetApiEscape(iterator.name);
                    ReadAssetApiProperty(iterator, pointer, scope, fields, managed, 0);
                } while (iterator.Next(false));
                // Registry IDs are identity selectors, not array offsets. Multiple
                // fields can refer to the same managed instance without recursion.
                var emitted = new HashSet<long>();
                var registry = new List<object>();
                while (managed.Keys.Any(id => !emitted.Contains(id)))
                {
                    long id = managed.Keys.Where(key => !emitted.Contains(key)).Min();
                    emitted.Add(id);
                    SerializedProperty reference = managed[id];
                    string prefix = "/" + AssetApiEscape(item.rootType) + "/references/RefIds/@rid=" + id.ToString(CultureInfo.InvariantCulture);
                    AddAssetApiField(fields, prefix + "/rid", "integer", AssetApiInteger(id));
                    var data = new Dictionary<string, object>();
                    foreach (SerializedProperty child in AssetApiChildren(reference))
                        data[child.name] = ReadAssetApiProperty(child, prefix + "/data/" + AssetApiEscape(child.name), scope, fields, managed, 0);
                    AddAssetApiField(fields, prefix + "/data", "object", data);
                    string fullType = reference.managedReferenceFullTypename ?? "";
                    int separator = fullType.IndexOf(' ');
                    string assembly = separator < 0 ? "" : fullType.Substring(0, separator);
                    string typeName = separator < 0 ? fullType : fullType.Substring(separator + 1);
                    int namespaceSeparator = typeName.LastIndexOf('.');
                    var type = new Dictionary<string, object> {
                        { "class", namespaceSeparator < 0 ? typeName : typeName.Substring(namespaceSeparator + 1) },
                        { "ns", namespaceSeparator < 0 ? "" : typeName.Substring(0, namespaceSeparator) }, { "asm", assembly }
                    };
                    AddAssetApiField(fields, prefix + "/type", "object", type);
                    foreach (KeyValuePair<string, object> member in type) AddAssetApiField(fields, prefix + "/type/" + member.Key, "string", member.Value);
                    var entry = new Dictionary<string, object> { { "rid", AssetApiInteger(id) }, { "type", type }, { "data", data } };
                    registry.Add(entry);
                    AddAssetApiField(fields, prefix, "object", entry);
                }
                if (registry.Count > 0)
                {
                    string prefix = "/" + AssetApiEscape(item.rootType) + "/references";
                    AddAssetApiField(fields, prefix + "/version", "integer", 2);
                    AddAssetApiField(fields, prefix + "/RefIds", "array", registry);
                    AddAssetApiField(fields, prefix, "object", new Dictionary<string, object> { { "version", 2 }, { "RefIds", registry } });
                }
                fields.Sort((a, b) => string.CompareOrdinal((string)((Dictionary<string, object>)a)["property_path"], (string)((Dictionary<string, object>)b)["property_path"]));
                objects.Add(new Dictionary<string, object> { { "object_id", item.id }, { "class_id", item.classId }, { "root_type", item.rootType }, { "fields", fields } });
            }
            string content = AssetApiJson(objects);
            string revision;
            using (SHA256 hash = SHA256.Create())
            {
                string diskRevision = BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(scope.absolutePath)));
                revision = "live:" + BitConverter.ToString(hash.ComputeHash(Encoding.UTF8.GetBytes(content + diskRevision))).Replace("-", "").ToLowerInvariant();
            }
            return new Dictionary<string, object> { { "revision", revision }, { "objects", objects }, { "diagnostics", scope.diagnostics },
                { "capabilities", new Dictionary<string, object> { { "representation", "serialized" },
                    { "prefab_inherited_fields", false }, { "managed_type_creation", false },
                    { "contains_prefab_inheritance", scope.containsPrefabInheritance } } } };
        }

        private static object ReadAssetApiProperty(SerializedProperty property, string pointer, AssetApiScope scope,
            List<object> fields, Dictionary<long, SerializedProperty> managed, int depth)
        {
            if (depth > 96) throw new Exception("limit_exceeded: serialized nesting exceeds 96");
            object value;
            string kind;
            if (property.isArray && property.propertyType != SerializedPropertyType.String)
            {
                var array = new List<object>();
                string[] keys = AssetApiStableArrayKeys(property);
                for (int i = 0; i < property.arraySize; i++)
                    array.Add(ReadAssetApiProperty(property.GetArrayElementAtIndex(i), pointer + "/" + (keys == null ? i.ToString(CultureInfo.InvariantCulture) : keys[i]), scope, fields, managed, depth + 1));
                value = array; kind = "array";
            }
            else switch (property.propertyType)
            {
                case SerializedPropertyType.Boolean: value = property.boolValue ? 1 : 0; kind = "integer"; break;
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.LayerMask:
                case SerializedPropertyType.Character:
                case SerializedPropertyType.Enum:
                    value = property.type == "ulong" ? AssetApiUnsignedInteger(property.ulongValue) : AssetApiInteger(property.longValue); kind = "integer"; break;
                case SerializedPropertyType.Float:
                    double number = property.doubleValue;
                    value = double.IsNaN(number) || double.IsInfinity(number)
                        ? (object)new Dictionary<string, object> { { "kind", "float64" }, { "value", double.IsNaN(number) ? ".nan" : number < 0 ? "-.inf" : ".inf" } }
                        : property.type == "double" ? (object)number : property.floatValue;
                    kind = "number"; break;
                case SerializedPropertyType.String: value = property.stringValue; kind = "string"; break;
                case SerializedPropertyType.ObjectReference:
                    value = AssetApiObjectReference(property.objectReferenceValue, scope.path); kind = "object_reference"; break;
                case SerializedPropertyType.ManagedReference:
                    long rid = property.managedReferenceId;
                    value = new Dictionary<string, object> { { "rid", rid.ToString(CultureInfo.InvariantCulture) } };
                    kind = "managed_reference";
                    if (rid >= 0 && !managed.ContainsKey(rid)) managed[rid] = property.Copy();
                    break;
                default:
                    var mapping = new Dictionary<string, object>();
                    foreach (SerializedProperty child in AssetApiChildren(property))
                        mapping[child.name] = ReadAssetApiProperty(child, pointer + "/" + AssetApiEscape(child.name), scope, fields, managed, depth + 1);
                    value = mapping; kind = "object";
                    break;
            }
            AddAssetApiField(fields, pointer, kind, value);
            return value;
        }

        private static IEnumerable<SerializedProperty> AssetApiChildren(SerializedProperty parent)
        {
            SerializedProperty cursor = parent.Copy();
            SerializedProperty end = parent.GetEndProperty();
            if (!cursor.Next(true)) yield break;
            while (!SerializedProperty.EqualContents(cursor, end) && cursor.depth > parent.depth)
            {
                yield return cursor.Copy();
                if (!cursor.Next(false)) yield break;
            }
        }

        private static string[] AssetApiStableArrayKeys(SerializedProperty property)
        {
            string prefix = property.name == "m_Children" ? "@child=" : property.name == "m_Component" ? "@component=" : null;
            if (prefix == null) return null;
            var keys = new List<string>();
            var unique = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < property.arraySize; i++)
            {
                SerializedProperty element = property.GetArrayElementAtIndex(i);
                SerializedProperty reference = property.name == "m_Component" ? element.FindPropertyRelative("component") ?? element : element;
                long id;
                if (reference.propertyType != SerializedPropertyType.ObjectReference || reference.objectReferenceValue == null
                    || !TryGetLocalFileId(reference.objectReferenceValue, out id)) return null;
                string key = prefix + id.ToString(CultureInfo.InvariantCulture);
                if (!unique.Add(key)) return null;
                keys.Add(key);
            }
            return keys.ToArray();
        }

        private static void AddAssetApiField(List<object> fields, string pointer, string kind, object value)
        {
            fields.Add(new Dictionary<string, object> { { "property_path", pointer }, { "kind", kind }, { "value", value } });
        }

        private static object AssetApiInteger(long value)
        {
            if (value >= -9007199254740991L && value <= 9007199254740991L) return value;
            return new Dictionary<string, object> { { "kind", "int64" }, { "value", value.ToString(CultureInfo.InvariantCulture) } };
        }

        private static object AssetApiUnsignedInteger(ulong value)
        {
            if (value <= 9007199254740991UL) return value;
            return new Dictionary<string, object> { { "kind", value > long.MaxValue ? "uint64" : "int64" }, { "value", value.ToString(CultureInfo.InvariantCulture) } };
        }

        private static object AssetApiObjectReference(UnityEngine.Object target, string ownerPath)
        {
            if (target == null) return new Dictionary<string, object> { { "fileID", "0" } };
            long id;
            if (!TryGetLocalFileId(target, out id)) throw new Exception("unsupported_capability: reference has no persistent file ID");
            string path = AssetDatabase.GetAssetPath(target);
            if (string.IsNullOrEmpty(path))
            {
                Component component = target as Component;
                GameObject gameObject = target as GameObject;
                path = component != null ? component.gameObject.scene.path : gameObject != null ? gameObject.scene.path : "";
            }
            var result = new Dictionary<string, object> { { "fileID", id.ToString(CultureInfo.InvariantCulture) } };
            if (path != ownerPath)
            {
                result["guid"] = AssetDatabase.AssetPathToGUID(path);
                // Unity's text-serialized native assets use reference type 2;
                // imported assets and MonoScripts use type 3.
                result["type"] = target is MonoScript || !new[] { ".asset", ".mat", ".controller", ".anim", ".overrideController", ".playable", ".mask" }.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase) ? 3 : 2;
            }
            return result;
        }

        private static SerializedProperty FindAssetApiProperty(AssetApiObject item, string pointer)
        {
            if (string.IsNullOrEmpty(pointer) || pointer[0] != '/') throw new Exception("invalid_path: expected RFC6901 property pointer");
            string[] segments = pointer.Substring(1).Split('/').Select(AssetApiUnescape).ToArray();
            if (segments.Length < 2 || segments[0] != item.rootType) throw new Exception("property_not_found: wrong serialized root in " + pointer);
            int start = 1;
            SerializedProperty property = null;
            if (segments.Length >= 6 && segments[1] == "references" && segments[2] == "RefIds" && segments[3].StartsWith("@rid=", StringComparison.Ordinal) && segments[4] == "data")
            {
                long rid;
                if (!long.TryParse(segments[3].Substring(5), NumberStyles.Integer, CultureInfo.InvariantCulture, out rid)) throw new Exception("invalid_id: managed reference ID");
                property = FindAssetApiManagedReference(item.serialized, rid);
                if (property == null) return null;
                start = 5;
            }
            for (int i = start; i < segments.Length; i++)
            {
                if (property == null) property = item.serialized.FindProperty(segments[i]);
                else if (property.isArray && property.propertyType != SerializedPropertyType.String)
                {
                    int index;
                    if (segments[i].StartsWith("@", StringComparison.Ordinal))
                    {
                        string[] keys = AssetApiStableArrayKeys(property);
                        index = keys == null ? -1 : Array.IndexOf(keys, segments[i]);
                    }
                    else if (!int.TryParse(segments[i], out index)) return null;
                    if (index < 0 || index >= property.arraySize) return null;
                    property = property.GetArrayElementAtIndex(index);
                }
                else property = property.FindPropertyRelative(segments[i]);
                if (property == null) return null;
            }
            return property;
        }

        private static SerializedProperty FindAssetApiManagedReference(SerializedObject serialized, long rid)
        {
            SerializedProperty cursor = serialized.GetIterator();
            var visitedReferences = new HashSet<long>();
            bool enterChildren = true;
            while (cursor.Next(enterChildren))
            {
                enterChildren = true;
                if (cursor.propertyType != SerializedPropertyType.ManagedReference) continue;
                if (cursor.managedReferenceId == rid) return cursor.Copy();
                // SerializeReference graphs can contain aliases and cycles.
                enterChildren = cursor.managedReferenceId >= 0 && visitedReferences.Add(cursor.managedReferenceId);
            }
            return null;
        }

        private static void ApplyAssetApiOperation(AssetApiScope scope, AssetApiObject item, SerializedProperty property, AssetApiOperation operation)
        {
            if (!property.editable) throw new Exception("unsupported_capability: property is read only " + operation.property_path);
            if (operation.op == "set")
            {
                if (operation.value_json == null) throw new Exception("invalid_value: set requires value");
                SetAssetApiValue(scope, item, property, operation.value_json, 0);
                return;
            }
            if (!property.isArray || property.propertyType == SerializedPropertyType.String) throw new Exception("type_mismatch: operation requires array");
            int length = property.arraySize;
            switch (operation.op)
            {
                case "array_insert":
                    if (operation.index < 0 || operation.index > length || operation.value_json == null) throw new Exception("invalid_operation: array insert requires valid index and value");
                    property.InsertArrayElementAtIndex(operation.index);
                    SetAssetApiValue(scope, item, property.GetArrayElementAtIndex(operation.index), operation.value_json, 0);
                    break;
                case "array_remove":
                    if (operation.index < 0 || operation.index >= length) throw new Exception("invalid_operation: array index out of bounds");
                    property.DeleteArrayElementAtIndex(operation.index);
                    if (property.arraySize == length) property.DeleteArrayElementAtIndex(operation.index);
                    break;
                case "array_move":
                    if (operation.index < 0 || operation.index >= length || operation.to_index < 0 || operation.to_index >= length) throw new Exception("invalid_operation: array move index out of bounds");
                    property.MoveArrayElement(operation.index, operation.to_index);
                    break;
                case "array_resize":
                    if (operation.size < 0 || operation.size > 1000000) throw new Exception("invalid_operation: array size out of bounds");
                    if (operation.size > length && operation.value_json == null) throw new Exception("invalid_value: growing an array requires an explicit fill value");
                    property.arraySize = operation.size;
                    for (int i = length; i < operation.size; i++) SetAssetApiValue(scope, item, property.GetArrayElementAtIndex(i), operation.value_json, 0);
                    break;
                default: throw new Exception("unsupported_capability: operation " + operation.op);
            }
        }

        private static void SetAssetApiValue(AssetApiScope scope, AssetApiObject item, SerializedProperty property, string json, int depth)
        {
            if (depth > 96) throw new Exception("limit_exceeded: serialized nesting exceeds 96");
            json = json.Trim();
            if (property.isArray && property.propertyType != SerializedPropertyType.String)
            {
                if (!json.StartsWith("[", StringComparison.Ordinal)) throw new Exception("type_mismatch: expected JSON array");
                var values = DeserializeJson<List<object>>(json);
                if (values.Count > 1000000) throw new Exception("limit_exceeded: array size");
                property.arraySize = values.Count;
                for (int i = 0; i < values.Count; i++) SetAssetApiValue(scope, item, property.GetArrayElementAtIndex(i), AssetApiRawJson(values[i]), depth + 1);
                return;
            }
            switch (property.propertyType)
            {
                case SerializedPropertyType.Boolean:
                    if (json != "0" && json != "1" && json != "false" && json != "true") throw new Exception("type_mismatch: expected boolean or 0/1");
                    property.boolValue = json == "1" || json == "true"; return;
                case SerializedPropertyType.Integer:
                case SerializedPropertyType.LayerMask:
                case SerializedPropertyType.Character:
                case SerializedPropertyType.Enum:
                    Type integerType = ResolveSerializedPropertyFieldType(property);
                    if (integerType != null && integerType.IsEnum) integerType = Enum.GetUnderlyingType(integerType);
                    if (integerType == typeof(ulong) || property.type == "ulong")
                    {
                        ulong unsigned;
                        if (!ulong.TryParse(AssetApiIntegerText(json), NumberStyles.None, CultureInfo.InvariantCulture, out unsigned)) throw new Exception("type_mismatch: expected unsigned integer");
                        property.ulongValue = unsigned;
                        return;
                    }
                    long integer = AssetApiParseInteger(json);
                    long minimum = int.MinValue;
                    long maximum = int.MaxValue;
                    if (integerType == typeof(long) || property.type == "long") { minimum = long.MinValue; maximum = long.MaxValue; }
                    else if (integerType == typeof(uint) || property.type == "uint") { minimum = 0; maximum = uint.MaxValue; }
                    else if (integerType == typeof(short)) { minimum = short.MinValue; maximum = short.MaxValue; }
                    else if (integerType == typeof(ushort) || integerType == typeof(char)) { minimum = 0; maximum = ushort.MaxValue; }
                    else if (integerType == typeof(byte)) { minimum = 0; maximum = byte.MaxValue; }
                    else if (integerType == typeof(sbyte)) { minimum = sbyte.MinValue; maximum = sbyte.MaxValue; }
                    if (integer < minimum || integer > maximum) throw new Exception("type_mismatch: integer outside field range");
                    property.longValue = integer; return;
                case SerializedPropertyType.Float:
                    double number;
                    if (!double.TryParse(json, NumberStyles.Float, CultureInfo.InvariantCulture, out number) || double.IsNaN(number) || double.IsInfinity(number)) throw new Exception("type_mismatch: expected finite number");
                    if (property.type != "double" && (number < -float.MaxValue || number > float.MaxValue)) throw new Exception("type_mismatch: number outside single precision field range");
                    property.doubleValue = number; return;
                case SerializedPropertyType.String:
                    if (!json.StartsWith("\"", StringComparison.Ordinal)) throw new Exception("type_mismatch: expected string");
                    property.stringValue = DeserializeJson<string>(json); return;
                case SerializedPropertyType.ObjectReference:
                    var reference = DeserializeJson<Dictionary<string, object>>(json);
                    object idValue;
                    if (reference == null || !reference.TryGetValue("fileID", out idValue) || !(idValue is string)) throw new Exception("invalid_id: object reference fileID must be a decimal string");
                    if (reference.Keys.Any(key => key != "fileID" && key != "guid" && key != "type")) throw new Exception("invalid_value: unknown object reference member");
                    long id;
                    if (!long.TryParse((string)idValue, NumberStyles.Integer, CultureInfo.InvariantCulture, out id) || id.ToString(CultureInfo.InvariantCulture) != (string)idValue) throw new Exception("invalid_id: object reference fileID");
                    if (id == 0)
                    {
                        if (reference.Count != 1) throw new Exception("invalid_value: null object references contain only fileID");
                        property.objectReferenceValue = null; return;
                    }
                    object guid;
                    bool external = reference.TryGetValue("guid", out guid);
                    if (external && (!(guid is string) || !Regex.IsMatch((string)guid, "^[0-9a-fA-F]{32}$"))) throw new Exception("invalid_id: reference GUID must contain 32 hexadecimal digits");
                    string targetPath = external ? AssetDatabase.GUIDToAssetPath((string)guid) : scope.path;
                    if (string.IsNullOrEmpty(targetPath)) throw new Exception("object_not_found: reference GUID " + guid);
                    UnityEngine.Object target;
                    if (targetPath == scope.path) target = scope.objects.Where(candidate => candidate.id == (string)idValue).Select(candidate => candidate.target).FirstOrDefault();
                    else
                    {
                        var candidates = new List<UnityEngine.Object>(AssetDatabase.LoadAllAssetsAtPath(targetPath));
                        GameObject root = AssetDatabase.LoadAssetAtPath<GameObject>(targetPath);
                        if (root != null) AssetApiCollectHierarchy(root, candidates);
                        target = candidates.Find(candidate => { long candidateId; return candidate != null && TryGetLocalFileId(candidate, out candidateId) && candidateId == id; });
                    }
                    if (target == null) throw new Exception("object_not_found: reference " + targetPath + "#" + id);
                    var canonicalReference = (Dictionary<string, object>)AssetApiObjectReference(target, scope.path);
                    object referenceType;
                    if (reference.TryGetValue("type", out referenceType))
                    {
                        object canonicalType;
                        if (!canonicalReference.TryGetValue("type", out canonicalType) || AssetApiRawJson(referenceType) != AssetApiRawJson(canonicalType))
                            throw new Exception("type_mismatch: object reference serialization type does not match target");
                    }
                    Type expected = ResolveSerializedPropertyFieldType(property);
                    if (expected != null && !expected.IsInstanceOfType(target)) throw new Exception("type_mismatch: incompatible object reference");
                    property.objectReferenceValue = target; return;
                case SerializedPropertyType.ManagedReference:
                    var managedPointer = DeserializeJson<Dictionary<string, object>>(json);
                    object rawRid;
                    long rid;
                    if (managedPointer == null || managedPointer.Count != 1 || !managedPointer.TryGetValue("rid", out rawRid)
                        || !(rawRid is string) || !long.TryParse((string)rawRid, NumberStyles.Integer, CultureInfo.InvariantCulture, out rid)
                        || rid.ToString(CultureInfo.InvariantCulture) != (string)rawRid)
                        throw new Exception("invalid_id: managed reference requires a decimal rid string");
                    string ridText = (string)rawRid;
                    if (rid == -2) { property.managedReferenceValue = null; return; }
                    if (rid < 0) throw new Exception("unsupported_capability: only -2 denotes a writable null managed reference");
                    SerializedProperty managedTarget = FindAssetApiManagedReference(item.serialized, rid);
                    if (managedTarget == null) throw new Exception("object_not_found: managed reference " + ridText);
                    object managedValue = managedTarget.managedReferenceValue;
                    Type managedType = ResolveSerializedPropertyFieldType(property);
                    if (managedValue == null || (managedType != null && !managedType.IsInstanceOfType(managedValue))) throw new Exception("type_mismatch: incompatible managed reference");
                    property.managedReferenceValue = managedValue;
                    return;
                default:
                    if (!json.StartsWith("{", StringComparison.Ordinal)) throw new Exception("type_mismatch: expected object");
                    var members = DeserializeJson<Dictionary<string, object>>(json);
                    var children = AssetApiChildren(property).ToList();
                    if (members.Count != children.Count || children.Any(child => !members.ContainsKey(child.name)))
                        throw new Exception("type_mismatch: set object requires exactly the serialized members");
                    foreach (SerializedProperty child in children) SetAssetApiValue(scope, item, child, AssetApiRawJson(members[child.name]), depth + 1);
                    return;
            }
        }

        private static long AssetApiParseInteger(string json)
        {
            json = AssetApiIntegerText(json);
            long integer;
            if (!long.TryParse(json, NumberStyles.Integer, CultureInfo.InvariantCulture, out integer)) throw new Exception("type_mismatch: expected signed integer");
            return integer;
        }

        private static string AssetApiIntegerText(string json)
        {
            if (json.StartsWith("{", StringComparison.Ordinal))
            {
                var value = DeserializeJson<Dictionary<string, object>>(json);
                object rawKind, rawValue;
                if (value.Count != 2 || !value.TryGetValue("kind", out rawKind) || !(rawKind is string)
                    || !value.TryGetValue("value", out rawValue) || !(rawValue is string)) throw new Exception("type_mismatch: invalid 64-bit integer envelope");
                string kind = (string)rawKind;
                json = (string)rawValue;
                if (kind == "uint64")
                {
                    ulong unsigned;
                    if (!Regex.IsMatch(json, "^(0|[1-9][0-9]*)$") || !ulong.TryParse(json, NumberStyles.None, CultureInfo.InvariantCulture, out unsigned))
                        throw new Exception("type_mismatch: uint64 value outside canonical unsigned range");
                }
                else if (kind == "int64")
                {
                    long signed;
                    if (json == "-0" || !Regex.IsMatch(json, "^-?(0|[1-9][0-9]*)$") || !long.TryParse(json, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out signed))
                        throw new Exception("type_mismatch: int64 value outside canonical signed range");
                }
                else throw new Exception("type_mismatch: expected int64 or uint64 envelope");
            }
            return json;
        }

        private static string AssetApiRawJson(object value)
        {
            // Nested values are private, vendored JTokens; ToString is their JSON
            // representation. Primitive strings must go through the JSON writer.
            if (value != null && value.GetType().Namespace != null && value.GetType().Namespace.EndsWith("Json.Linq", StringComparison.Ordinal)) return value.ToString();
            return AssetApiJson(value);
        }

        private static string AssetApiJson(object value) { return ToJsonValue(value, 0, 256, false); }
        private static string AssetApiEscape(string value) { return value.Replace("~", "~0").Replace("/", "~1"); }
        private static string AssetApiUnescape(string value)
        {
            if (Regex.IsMatch(value, "~(?![01])")) throw new Exception("invalid_path: malformed RFC6901 escape");
            return value.Replace("~1", "/").Replace("~0", "~");
        }
    }
}
