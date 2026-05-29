
using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.Text;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private class PipeEnvelope
        {
            public string id;
            public string reply_to;
            public string type;
            public bool ok;
            public string message;
            public string error;
        }

        [Serializable]
        private class SelectAssetRequest
        {
            public string assetPath;
            public bool focusProjectWindow = true;
        }

        [Serializable]
        private class SceneObjectRequest
        {
            public string scenePath;
            public string objectPath;
        }

        [Serializable]
        private class StartAssetDragRequest
        {
            public LocusEditorWindow.DroppedAssetRef[] refs;
            public string traceId;
        }

        [Serializable]
        private class CaptureViewportRequest
        {
            public string target;
            public string windowTitle;
        }

        [Serializable]
        private class CaptureViewportResponse
        {
            public string target;
            public string title;
            public string path;
            public int width;
            public int height;
            public int originalWidth;
            public int originalHeight;
            public string mimeType;
        }

        [Serializable]
        private class ExecuteCodeProgressSnapshot
        {
            public bool active;
            public string title;
            public string info;
            public float progress;
            public int revision;
            public string source;
        }

        public sealed class ScriptGlobals
        {
            private readonly StringBuilder _output = new StringBuilder(256);
            private readonly Action _touchActivity;

            public ScriptGlobals()
                : this(null)
            {
            }

            public ScriptGlobals(Action touchActivity)
            {
                _touchActivity = touchActivity;
            }

            private void TouchActivity()
            {
                try
                {
                    if (_touchActivity != null)
                        _touchActivity();
                }
                catch
                {
                }
            }

            /// <summary>
            /// Append obj.ToString() to the result buffer (one line per call).
            /// This is the primary way to return plain-text results to the agent.
            /// </summary>
            public void print(object obj)
            {
                TouchActivity();
                _output.AppendLine(obj != null ? obj.ToString() : "null");
            }

            /// <summary>
            /// Serialize a Unity object (or any object) to JSON and append it to the result buffer.
            /// Uses EditorJsonUtility for UnityEngine.Object types (preserves serialized fields,
            /// references, etc.), falls back to JsonUtility for plain C# objects.
            /// This is the preferred way to return structured data to the agent.
            /// </summary>
            public void printJson(object obj)
            {
                TouchActivity();
                if (obj == null)
                {
                    _output.AppendLine("null");
                    return;
                }

                try
                {
                    string json;
                    if (obj is UnityEngine.Object uObj)
                        json = EditorJsonUtility.ToJson(uObj, true);
                    else
                        json = JsonUtility.ToJson(obj, true);

                    _output.AppendLine(json);
                }
                catch (Exception ex)
                {
                    _output.Append("[printJson error: ").Append(ex.Message).Append("] ")
                           .AppendLine(obj.ToString());
                }
            }

            /// <summary>
            /// Clear the result buffer.
            /// </summary>
            public void clear()
            {
                _output.Length = 0;
            }

            public string GetOutput()
            {
                return _output.ToString();
            }
        }
    }

    internal static class LocusAssetInspectorUtility
    {
        public static void OpenLockedInspector(string assetPath)
        {
            string normalizedPath = NormalizeAssetPath(assetPath);
            if (string.IsNullOrEmpty(normalizedPath))
                throw new InvalidOperationException("Asset path is empty.");

            if (!IsProjectAssetPath(normalizedPath))
                throw new InvalidOperationException("Inspector asset path must start with Assets/ or Packages/.");

            if (AssetDatabase.IsValidFolder(normalizedPath))
                throw new InvalidOperationException("Folders cannot be opened in a locked Inspector.");

            UnityEngine.Object obj = AssetDatabase.LoadMainAssetAtPath(normalizedPath);
            if (obj == null)
                throw new InvalidOperationException("Asset was not found: " + normalizedPath);

            OpenLockedObjectInspector(obj);
            EditorGUIUtility.PingObject(obj);
        }

        public static void OpenLockedObjectInspector(UnityEngine.Object obj)
        {
            if (obj == null)
                throw new InvalidOperationException("Inspector target is unavailable.");

            if (!CanInspectObject(obj))
                throw new InvalidOperationException("Object cannot be inspected: " + obj.name);

            if (!TryOpenPropertyEditor(obj))
                LocusLockedAssetInspectorWindow.Open(obj);
        }

        private static string NormalizeAssetPath(string assetPath)
        {
            return (assetPath ?? "").Trim().Replace('\\', '/');
        }

        private static bool IsProjectAssetPath(string assetPath)
        {
            return assetPath.StartsWith("Assets/", StringComparison.Ordinal)
                || assetPath.StartsWith("Packages/", StringComparison.Ordinal);
        }

        private static bool CanInspectObject(UnityEngine.Object obj)
        {
            Editor editor = null;
            try
            {
                editor = Editor.CreateEditor(obj);
                return editor != null;
            }
            catch
            {
                return false;
            }
            finally
            {
                if (editor != null)
                    UnityEngine.Object.DestroyImmediate(editor);
            }
        }

        private static bool TryOpenPropertyEditor(UnityEngine.Object obj)
        {
            try
            {
                System.Reflection.MethodInfo method = typeof(EditorUtility).GetMethod(
                    "OpenPropertyEditor",
                    System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Static,
                    null,
                    new[] { typeof(UnityEngine.Object) },
                    null);
                if (method == null)
                    return false;

                method.Invoke(null, new object[] { obj });
                return true;
            }
            catch
            {
                return false;
            }
        }
    }

    internal static class LocusSceneObjectUtility
    {
        public static void SelectSceneObject(string scenePath, string objectPath)
        {
            GameObject target = ResolveSceneObject(scenePath, objectPath);
            Selection.activeGameObject = target;
            EditorGUIUtility.PingObject(target);
        }

        public static void OpenSceneObjectInspector(string scenePath, string objectPath)
        {
            GameObject target = ResolveSceneObject(scenePath, objectPath);
            Selection.activeGameObject = target;
            LocusAssetInspectorUtility.OpenLockedObjectInspector(target);
            EditorGUIUtility.PingObject(target);
        }

        public static GameObject ResolveSceneObject(string scenePath, string objectPath)
        {
            string normalizedScenePath = NormalizePath(scenePath);
            string normalizedObjectPath = NormalizeObjectPath(objectPath);

            if (string.IsNullOrEmpty(normalizedScenePath))
                throw new InvalidOperationException("Scene path is empty.");
            if (!normalizedScenePath.EndsWith(".unity", StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Scene object path must include a .unity scene asset.");
            if (string.IsNullOrEmpty(normalizedObjectPath))
                throw new InvalidOperationException("Scene object path is empty.");

            var scene = FindLoadedScene(normalizedScenePath);
            if (!scene.IsValid() || !scene.isLoaded)
                throw new InvalidOperationException("Scene is not loaded in the editor: " + normalizedScenePath);

            GameObject target = FindGameObjectByPath(scene.GetRootGameObjects(), normalizedObjectPath);
            if (target == null)
                throw new InvalidOperationException("GameObject was not found: " + normalizedScenePath + "/" + normalizedObjectPath);

            return target;
        }

        private static UnityEngine.SceneManagement.Scene FindLoadedScene(string scenePath)
        {
            for (int i = 0; i < EditorSceneManager.sceneCount; i++)
            {
                var scene = EditorSceneManager.GetSceneAt(i);
                if (scene.IsValid() && scene.path == scenePath)
                    return scene;
            }

            return default(UnityEngine.SceneManagement.Scene);
        }

        private static GameObject FindGameObjectByPath(GameObject[] roots, string objectPath)
        {
            string[] parts = objectPath.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length == 0)
                return null;

            GameObject current = null;
            foreach (GameObject root in roots)
            {
                if (root != null && root.name == parts[0])
                {
                    current = root;
                    break;
                }
            }

            if (current == null)
                return null;

            for (int i = 1; i < parts.Length; i++)
            {
                Transform child = current.transform.Find(parts[i]);
                if (child == null)
                    return null;
                current = child.gameObject;
            }

            return current;
        }

        private static string NormalizePath(string path)
        {
            return (path ?? "").Trim().Replace('\\', '/').Trim('/');
        }

        private static string NormalizeObjectPath(string path)
        {
            return (path ?? "").Trim().Replace('\\', '/').Trim('/');
        }
    }

    internal sealed class LocusLockedAssetInspectorWindow : EditorWindow
    {
        [NonSerialized] private UnityEngine.Object _targetObject;
        [NonSerialized] private Editor _editor;
        private Vector2 _scroll;

        public static void Open(UnityEngine.Object targetObject)
        {
            LocusLockedAssetInspectorWindow window = CreateInstance<LocusLockedAssetInspectorWindow>();
            window.SetTarget(targetObject);
            window.Show();
            window.Focus();
        }

        private void SetTarget(UnityEngine.Object targetObject)
        {
            _targetObject = targetObject;
            minSize = new Vector2(280f, 320f);
            titleContent = new GUIContent("Inspector", EditorGUIUtility.ObjectContent(targetObject, targetObject.GetType()).image);
        }

        private void OnDisable()
        {
            DestroyCachedEditor();
        }

        private void OnGUI()
        {
            if (_targetObject == null)
            {
                EditorGUILayout.HelpBox("The inspected asset is unavailable.", MessageType.Info);
                return;
            }

            if (_editor == null || _editor.target != _targetObject)
            {
                DestroyCachedEditor();
                Editor.CreateCachedEditor(_targetObject, null, ref _editor);
            }

            if (_editor == null)
            {
                EditorGUILayout.HelpBox("This asset cannot be inspected.", MessageType.Info);
                return;
            }

            _scroll = EditorGUILayout.BeginScrollView(_scroll);
            _editor.OnInspectorGUI();
            EditorGUILayout.EndScrollView();
        }

        private void DestroyCachedEditor()
        {
            if (_editor == null)
                return;

            DestroyImmediate(_editor);
            _editor = null;
        }
    }
}
