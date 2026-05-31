using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using UnityEditor;
using UnityEngine;
using Object = UnityEngine.Object;

namespace Locus
{
    [InitializeOnLoad]
    internal static class LocusExternalAssetDragBridge
    {
        private const double ArmedDragSeconds = 8d;
        private const int PostPerformRepaintFrames = 120;
#if UNITY_6000_0_OR_NEWER
        private static readonly Action<EventType, KeyCode, EventModifiers> BeforeEventProcessedHandler = HandleBeforeEventProcessed;
#else
        private static readonly Action<EventType, KeyCode> BeforeEventProcessedHandler = HandleBeforeEventProcessed;
#endif
        private static readonly EditorApplication.CallbackFunction GlobalEventHandler = HandleGlobalEvent;
        private static readonly EditorApplication.CallbackFunction GlobalUpdateHandler = HandleGlobalUpdate;
        private static readonly FieldInfo BeforeEventProcessedField =
            typeof(GUIUtility).GetField("beforeEventProcessed", BindingFlags.Static | BindingFlags.NonPublic);
        private static readonly FieldInfo GlobalEventHandlerField =
            typeof(EditorApplication).GetField("globalEventHandler", BindingFlags.Static | BindingFlags.NonPublic);
        private static readonly object ArmedDragLock = new object();
        private static Object[] _armedObjectReferences = new Object[0];
        private static string[] _armedPaths = new string[0];
        private static string _armedTitle = "Locus References";
        private static double _armedExpiresAt;
        private static EditorWindow _lastDropTargetWindow;
        private static Object[] _lastDragPerformObjectReferences = new Object[0];
        private static string[] _lastDragPerformPaths = new string[0];
        private static int _lastDragPerformInitialSceneObjectId;
        private static int _postPerformRepaintFramesRemaining;
        private static bool _instanceDetectedRepaintDone;
        private static bool _startQueued;
        private static bool _dragStarted;

        static LocusExternalAssetDragBridge()
        {
            Install();
            AssemblyReloadEvents.beforeAssemblyReload += Uninstall;
        }

        internal static void ArmAssetDrag(LocusEditorWindow.DroppedAssetRef[] refs)
        {
            Object[] objectReferences;
            string[] paths;
            string title;
            string error;
            if (!TryBuildDragPayload(refs, out objectReferences, out paths, out title, out error))
            {
                ClearArmedDrag();
                return;
            }

            SetArmedDrag(objectReferences, paths, title, false);
        }

        internal static bool QueueAssetDrag(LocusEditorWindow.DroppedAssetRef[] refs, out string message)
        {
            Object[] objectReferences;
            string[] paths;
            string title;
            string error;
            if (!TryBuildDragPayload(refs, out objectReferences, out paths, out title, out error))
            {
                message = error;
                return false;
            }

            SetArmedDrag(objectReferences, paths, title, true);
            message = "queued";
            return true;
        }

        internal static void CancelAssetDrag()
        {
            ClearArmedDrag();
            DragAndDrop.PrepareStartDrag();
            DragAndDrop.objectReferences = new Object[0];
            DragAndDrop.paths = new string[0];
            DragAndDrop.visualMode = DragAndDropVisualMode.None;
            LocusEditorWindow.ClearPublishedUnityAssetDragState();
        }

        private static void SetArmedDrag(
            Object[] objectReferences,
            string[] paths,
            string title,
            bool startQueued)
        {
            lock (ArmedDragLock)
            {
                _armedObjectReferences = objectReferences ?? new Object[0];
                _armedPaths = paths ?? new string[0];
                _armedTitle = string.IsNullOrEmpty(title) ? "Locus References" : title;
                _armedExpiresAt = EditorApplication.timeSinceStartup + ArmedDragSeconds;
                _startQueued = startQueued;
                _dragStarted = false;
            }
        }

        private static void Install()
        {
            InstallBeforeEventProcessedHandler();
            InstallGlobalEventHandler();
            EditorApplication.update -= GlobalUpdateHandler;
            EditorApplication.update += GlobalUpdateHandler;
        }

        private static void InstallBeforeEventProcessedHandler()
        {
            if (BeforeEventProcessedField == null)
                return;

#if UNITY_6000_0_OR_NEWER
            var current = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode, EventModifiers>;
            current -= BeforeEventProcessedHandler;
            current = (Action<EventType, KeyCode, EventModifiers>)Delegate.Combine(BeforeEventProcessedHandler, current);
#else
            var current = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode>;
            current -= BeforeEventProcessedHandler;
            current = (Action<EventType, KeyCode>)Delegate.Combine(BeforeEventProcessedHandler, current);
#endif
            BeforeEventProcessedField.SetValue(null, current);
        }

        private static void InstallGlobalEventHandler()
        {
            if (GlobalEventHandlerField == null)
                return;

            var current = GlobalEventHandlerField.GetValue(null) as EditorApplication.CallbackFunction;
            current -= GlobalEventHandler;
            current = (EditorApplication.CallbackFunction)Delegate.Combine(GlobalEventHandler, current);
            GlobalEventHandlerField.SetValue(null, current);
        }

        private static void Uninstall()
        {
            if (BeforeEventProcessedField != null)
            {
#if UNITY_6000_0_OR_NEWER
                var beforeCurrent = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode, EventModifiers>;
                beforeCurrent -= BeforeEventProcessedHandler;
                BeforeEventProcessedField.SetValue(null, beforeCurrent);
#else
                var beforeCurrent = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode>;
                beforeCurrent -= BeforeEventProcessedHandler;
                BeforeEventProcessedField.SetValue(null, beforeCurrent);
#endif
            }

            EditorApplication.update -= GlobalUpdateHandler;

            if (GlobalEventHandlerField == null)
                return;

            var current = GlobalEventHandlerField.GetValue(null) as EditorApplication.CallbackFunction;
            current -= GlobalEventHandler;
            GlobalEventHandlerField.SetValue(null, current);
        }

#if UNITY_6000_0_OR_NEWER
        private static void HandleBeforeEventProcessed(EventType eventType, KeyCode keyCode, EventModifiers modifiers)
#else
        private static void HandleBeforeEventProcessed(EventType eventType, KeyCode keyCode)
#endif
        {
            if (eventType == EventType.MouseDrag)
            {
                StartQueuedDrag();
                LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
                return;
            }

            if (eventType != EventType.DragUpdated && eventType != EventType.DragPerform)
                return;

            LocusEditorWindow.PublishCurrentUnityAssetDragState(eventType == EventType.DragPerform);
            if (ApplyArmedDragPayload() && eventType == EventType.DragPerform)
            {
                MarkDragPerform();
                ClearArmedDrag();
            }
        }

        private static void HandleGlobalEvent()
        {
            Event evt = Event.current;
            if (evt == null)
                return;

            if (evt.type == EventType.MouseDrag)
            {
                if (StartQueuedDrag())
                    evt.Use();
                LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
                return;
            }

            if (evt.type != EventType.DragUpdated && evt.type != EventType.DragPerform)
                return;

            LocusEditorWindow.PublishCurrentUnityAssetDragState(evt.type == EventType.DragPerform);
            if (ApplyArmedDragPayload() && evt.type == EventType.DragPerform)
            {
                MarkDragPerform();
                ClearArmedDrag();
            }
        }

        private static void HandleGlobalUpdate()
        {
            RepaintAfterDragPerformIfNeeded();

            if (!ShouldPublishAssetDragStateOnGlobalUpdate())
            {
                LocusEditorWindow.ClearPublishedUnityAssetDragState();
                return;
            }

            LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
        }

        private static void MarkDragPerform()
        {
            EditorWindow dropTargetWindow = EditorWindow.mouseOverWindow;
            _lastDropTargetWindow = dropTargetWindow;
            _lastDragPerformObjectReferences = SnapshotObjectArray(DragAndDrop.objectReferences);
            _lastDragPerformPaths = SnapshotStringArray(DragAndDrop.paths);
            _lastDragPerformInitialSceneObjectId = SceneInstanceObjectId(Selection.activeObject);
            _postPerformRepaintFramesRemaining = PostPerformRepaintFrames;
            _instanceDetectedRepaintDone = false;
            RepaintDropTargetWindowsAfterDragPerform(dropTargetWindow, 0);
        }

        private static void RepaintDropTargetWindowsAfterDragPerform(
            EditorWindow dropTargetWindow,
            int delayIndex)
        {
            RepaintDropTargetWindows(dropTargetWindow);

            if (delayIndex >= 2)
                return;

            EditorApplication.delayCall += delegate
            {
                RepaintDropTargetWindowsAfterDragPerform(dropTargetWindow, delayIndex + 1);
            };
        }

        private static void RepaintAfterDragPerformIfNeeded()
        {
            if (_postPerformRepaintFramesRemaining <= 0)
                return;

            _postPerformRepaintFramesRemaining--;
            RepaintAfterInstanceDetectedIfNeeded();
        }

        private static void RepaintAfterInstanceDetectedIfNeeded()
        {
            if (_instanceDetectedRepaintDone)
                return;

            if (!IsSceneInstanceObject(Selection.activeObject))
                return;

            _instanceDetectedRepaintDone = true;
            _postPerformRepaintFramesRemaining = 0;
            RepaintDropTargetWindows(_lastDropTargetWindow);
        }

        private static void RepaintDropTargetWindows(EditorWindow dropTargetWindow)
        {
            string targetKind = DropTargetWindowKind(dropTargetWindow);
            bool repaintSceneAndHierarchy = targetKind == "scene" || targetKind == "hierarchy";

            if (repaintSceneAndHierarchy)
            {
                SceneView.RepaintAll();
                EditorApplication.RepaintHierarchyWindow();
                return;
            }

            RepaintWindow(dropTargetWindow);
            if (dropTargetWindow != EditorWindow.focusedWindow)
                RepaintWindow(EditorWindow.focusedWindow);
        }

        private static void RepaintWindow(EditorWindow window)
        {
            if (window == null)
                return;

            try
            {
                window.Repaint();
            }
            catch
            {
            }
        }

        private static string DropTargetWindowKind(EditorWindow window)
        {
            if (window == null)
                return "none";
            if (window is SceneView)
                return "scene";

            string typeName = window.GetType().Name;
            string title = window.titleContent != null ? window.titleContent.text : "";
            if (string.Equals(typeName, "SceneHierarchyWindow", StringComparison.Ordinal)
                || string.Equals(title, "Hierarchy", StringComparison.OrdinalIgnoreCase)
                || typeName.IndexOf("Hierarchy", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                return "hierarchy";
            }

            return "window";
        }

        private static bool IsSceneInstanceObject(Object obj)
        {
            GameObject go = SceneInstanceGameObject(obj);
            if (go == null)
                return false;

            if (go.GetInstanceID() == _lastDragPerformInitialSceneObjectId)
                return false;

            if (_lastDragPerformObjectReferences.Length == 0 && _lastDragPerformPaths.Length == 0)
                return true;

            Object source = null;
            try
            {
                source = PrefabUtility.GetCorrespondingObjectFromSource(go);
            }
            catch
            {
                source = null;
            }

            return MatchesLastDragPerformSource(source);
        }

        private static int SceneInstanceObjectId(Object obj)
        {
            GameObject go = SceneInstanceGameObject(obj);
            return go != null ? go.GetInstanceID() : 0;
        }

        private static GameObject SceneInstanceGameObject(Object obj)
        {
            if (obj == null)
                return null;

            GameObject go = obj as GameObject;
            Component component = obj as Component;
            if (go == null && component != null)
                go = component.gameObject;
            if (go == null)
                return null;

            try
            {
                if (EditorUtility.IsPersistent(obj))
                    return null;
            }
            catch
            {
                return null;
            }

            return go.scene.IsValid() && go.scene.isLoaded ? go : null;
        }

        private static bool MatchesLastDragPerformSource(Object source)
        {
            if (source == null)
                return false;

            string sourcePath = SafeAssetPath(source);
            for (int i = 0; i < _lastDragPerformObjectReferences.Length; i++)
            {
                Object reference = _lastDragPerformObjectReferences[i];
                if (reference == null)
                    continue;
                if (reference == source)
                    return true;

                string referencePath = SafeAssetPath(reference);
                if (!string.IsNullOrEmpty(sourcePath)
                    && string.Equals(sourcePath, referencePath, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }

            if (!string.IsNullOrEmpty(sourcePath))
            {
                for (int i = 0; i < _lastDragPerformPaths.Length; i++)
                {
                    if (string.Equals(sourcePath, _lastDragPerformPaths[i], StringComparison.OrdinalIgnoreCase))
                        return true;
                }
            }

            return false;
        }

        private static string SafeAssetPath(Object obj)
        {
            if (obj == null)
                return "";

            try
            {
                return AssetDatabase.GetAssetPath(obj) ?? "";
            }
            catch
            {
                return "";
            }
        }

        private static Object[] SnapshotObjectArray(Object[] values)
        {
            if (values == null || values.Length == 0)
                return new Object[0];

            Object[] snapshot = new Object[values.Length];
            Array.Copy(values, snapshot, values.Length);
            return snapshot;
        }

        private static string[] SnapshotStringArray(string[] values)
        {
            if (values == null || values.Length == 0)
                return new string[0];

            string[] snapshot = new string[values.Length];
            Array.Copy(values, snapshot, values.Length);
            return snapshot;
        }

        private static bool ShouldPublishAssetDragStateOnGlobalUpdate()
        {
            return HasActiveArmedDrag()
                || LocusEditorWindow.HasCurrentUnityDragAndDropRefs();
        }

        private static bool HasActiveArmedDrag()
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();
                return HasArmedDragLocked();
            }
        }

        private static bool ApplyArmedDragPayload()
        {
            Object[] references;
            string[] paths;
            string title;
            if (!TryGetArmedDrag(out references, out paths, out title))
                return false;

            DragAndDrop.objectReferences = references;
            DragAndDrop.paths = paths;
            DragAndDrop.visualMode = DragAndDropVisualMode.Copy;
            return true;
        }

        private static bool StartQueuedDrag()
        {
            Object[] references;
            string[] paths;
            string title;
            if (!TryConsumeStartQueuedDrag(out references, out paths, out title))
                return false;

            DragAndDrop.PrepareStartDrag();
            DragAndDrop.objectReferences = references;
            DragAndDrop.paths = paths;
            DragAndDrop.StartDrag(title);
            DragAndDrop.visualMode = DragAndDropVisualMode.Copy;
            return true;
        }

        private static bool TryConsumeStartQueuedDrag(
            out Object[] objectReferences,
            out string[] paths,
            out string title)
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();

                if (!_startQueued || _dragStarted || !HasArmedDragLocked())
                {
                    objectReferences = new Object[0];
                    paths = new string[0];
                    title = "Locus References";
                    return false;
                }

                _startQueued = false;
                _dragStarted = true;
                objectReferences = _armedObjectReferences;
                paths = _armedPaths;
                title = _armedTitle;
                return true;
            }
        }

        private static bool TryGetArmedDrag(out Object[] objectReferences, out string[] paths, out string title)
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();

                objectReferences = _armedObjectReferences;
                paths = _armedPaths;
                title = _armedTitle;
                return HasArmedDragLocked();
            }
        }

        private static void ClearArmedDrag()
        {
            lock (ArmedDragLock)
            {
                _armedObjectReferences = new Object[0];
                _armedPaths = new string[0];
                _armedTitle = "Locus References";
                _armedExpiresAt = 0d;
                _startQueued = false;
                _dragStarted = false;
            }
        }

        private static void ExpireArmedDragIfNeededLocked()
        {
            if (EditorApplication.timeSinceStartup <= _armedExpiresAt)
                return;

            _armedObjectReferences = new Object[0];
            _armedPaths = new string[0];
            _armedTitle = "Locus References";
            _armedExpiresAt = 0d;
            _startQueued = false;
            _dragStarted = false;
        }

        private static bool HasArmedDragLocked()
        {
            return _armedObjectReferences.Length > 0 || _armedPaths.Length > 0;
        }

        private static bool TryBuildDragPayload(
            LocusEditorWindow.DroppedAssetRef[] refs,
            out Object[] objectReferences,
            out string[] paths,
            out string title,
            out string error)
        {
            if (refs == null || refs.Length == 0)
            {
                objectReferences = new Object[0];
                paths = new string[0];
                title = "Locus Reference";
                error = "No supported Unity references were provided.";
                return false;
            }

            List<Object> references = new List<Object>();
            List<string> pathRefs = new List<string>();
            HashSet<string> seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            string firstName = "";
            foreach (LocusEditorWindow.DroppedAssetRef assetRef in refs)
            {
                if (assetRef == null)
                    continue;

                if (string.IsNullOrEmpty(firstName))
                    firstName = !string.IsNullOrEmpty(assetRef.name)
                        ? assetRef.name
                        : Path.GetFileNameWithoutExtension(assetRef.path);

                if (assetRef.kind == "asset")
                {
                    string assetPath = ToProjectAssetPath(assetRef.path);
                    if (string.IsNullOrEmpty(assetPath) || !seen.Add("asset\n" + assetPath))
                        continue;

                    Object reference = AssetDatabase.LoadMainAssetAtPath(assetPath);
                    if (reference != null)
                        references.Add(reference);
                    pathRefs.Add(assetPath);
                    continue;
                }

                if (assetRef.kind == "sceneObject")
                {
                    Object reference;
                    string resolveError;
                    if (!TryResolveSceneObject(assetRef.path, out reference, out resolveError))
                    {
                        if (!string.IsNullOrEmpty(resolveError))
                        {
                            objectReferences = new Object[0];
                            paths = new string[0];
                            title = "Locus Reference";
                            error = resolveError;
                            return false;
                        }
                        continue;
                    }

                    if (reference != null && seen.Add("sceneObject\n" + assetRef.path))
                        references.Add(reference);
                }
            }

            objectReferences = references.ToArray();
            paths = pathRefs.ToArray();
            title = refs.Length == 1 && !string.IsNullOrEmpty(firstName)
                ? firstName
                : "Locus References";
            if (objectReferences.Length == 0 && paths.Length == 0)
            {
                error = "No Unity objects could be resolved for drag.";
                return false;
            }

            error = "";
            return true;
        }

        private static bool TryResolveSceneObject(string path, out Object reference, out string error)
        {
            reference = null;
            error = "";
            string scenePath;
            string objectPath;
            if (!TrySplitSceneObjectRefPath(path, out scenePath, out objectPath))
                return false;

            try
            {
                reference = LocusSceneObjectUtility.ResolveSceneObject(scenePath, objectPath);
                return reference != null;
            }
            catch (Exception ex)
            {
                error = ex.Message;
                return false;
            }
        }

        private static bool TrySplitSceneObjectRefPath(string path, out string scenePath, out string objectPath)
        {
            scenePath = "";
            objectPath = "";

            string normalized = (path ?? "").Trim().Replace('\\', '/');
            int marker = normalized.IndexOf(".unity/", StringComparison.OrdinalIgnoreCase);
            if (marker >= 0)
            {
                int split = marker + ".unity".Length;
                scenePath = normalized.Substring(0, split);
                objectPath = normalized.Substring(split + 1).Trim('/');
                return !string.IsNullOrEmpty(scenePath) && !string.IsNullOrEmpty(objectPath);
            }

            marker = normalized.IndexOf("::", StringComparison.Ordinal);
            if (marker <= 0 || marker + 2 >= normalized.Length)
                return false;

            scenePath = normalized.Substring(0, marker);
            objectPath = normalized.Substring(marker + 2).Trim('/');
            return !string.IsNullOrEmpty(scenePath) && !string.IsNullOrEmpty(objectPath);
        }

        private static string ToProjectAssetPath(string path)
        {
            if (string.IsNullOrWhiteSpace(path))
                return null;

            string normalized = path.Trim().Replace('\\', '/');
            if (IsProjectRelativeAssetPath(normalized))
                return normalized;

            string projectRoot = Path.GetDirectoryName(Application.dataPath);
            if (string.IsNullOrEmpty(projectRoot))
                return null;

            projectRoot = projectRoot.Replace('\\', '/').TrimEnd('/');
            if (!normalized.StartsWith(projectRoot + "/", StringComparison.OrdinalIgnoreCase))
                return null;

            string relative = normalized.Substring(projectRoot.Length + 1);
            return IsProjectRelativeAssetPath(relative) ? relative : null;
        }

        private static bool IsProjectRelativeAssetPath(string path)
        {
            return path.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                || path.StartsWith("Packages/", StringComparison.OrdinalIgnoreCase)
                || path.StartsWith("ProjectSettings/", StringComparison.OrdinalIgnoreCase);
        }
    }
}
