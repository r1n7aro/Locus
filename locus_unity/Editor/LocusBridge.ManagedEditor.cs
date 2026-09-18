using System;
using System.Threading.Tasks;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static Task<PipeEnvelope> HandleManagedEditorClose(string reqId)
        {
            var completion = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    if (!Application.isBatchMode)
                        throw new InvalidOperationException("Automatic shutdown only applies to headless Editors.");
                    if (EditorApplication.isCompiling || EditorApplication.isUpdating || EditorApplication.isPlayingOrWillChangePlaymode)
                        throw new InvalidOperationException("Unity is still compiling, importing or playing.");
                    for (int i = 0; i < EditorSceneManager.sceneCount; i++)
                    {
                        var scene = EditorSceneManager.GetSceneAt(i);
                        if (scene.isDirty)
                            throw new InvalidOperationException("Unsaved scene: " + scene.name);
                    }
                    var stage = PrefabStageUtility.GetCurrentPrefabStage();
                    if (stage != null && stage.scene.isDirty)
                        throw new InvalidOperationException("Unsaved Prefab: " + stage.assetPath);
                    foreach (var asset in Resources.FindObjectsOfTypeAll<UnityEngine.Object>())
                    {
                        if (asset == null || !EditorUtility.IsPersistent(asset) || !EditorUtility.IsDirty(asset)) continue;
                        string path = AssetDatabase.GetAssetPath(asset);
                        if (path.StartsWith("Assets/", StringComparison.Ordinal) || path.StartsWith("Packages/", StringComparison.Ordinal))
                            throw new InvalidOperationException("Unsaved asset: " + path);
                    }
                    completion.SetResult(OkResponse(reqId, "idle_shutdown_requested"));
                    EditorApplication.delayCall += delegate { EditorApplication.Exit(0); };
                }
                catch (Exception error) { completion.SetResult(ErrorResponse(reqId, error.Message)); }
            });
            return completion.Task;
        }
    }
}
