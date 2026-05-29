using System;
using System.Collections.Generic;
using System.Globalization;
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
        private const double PostPerformProbeSeconds = 15d;
        private const int PostPerformProbeFrames = 900;
        private const int MaxPostPerformEventLogs = 12;
        private static readonly int[] PostPerformProbeFrameMarks =
            { 1, 2, 3, 5, 10, 30, 60, 120, 240, 480, 900 };
        private static readonly Action<EventType, KeyCode> BeforeEventProcessedHandler = HandleBeforeEventProcessed;
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
        private static double _armedAt;
        private static string _armedTraceId = "none";
        private static string _lastLoggedDragUpdatedTraceId = "";
        private static string _lastDragPerformTraceId = "";
        private static double _lastDragPerformAt;
        private static int _postPerformProbeFramesRemaining;
        private static int _hierarchyChangeLogCount;
        private static int _selectionChangeLogCount;
        private static int _delayCallProbeCount;
        private static EditorWindow _lastDropTargetWindow;
        private static Object[] _lastDragPerformObjectReferences = new Object[0];
        private static string[] _lastDragPerformPaths = new string[0];
        private static int _lastDragPerformInitialSceneObjectId;
        private static bool _instanceDetectedRepaintDone;
        private static bool _loggedHierarchyChangeAfterPerform;
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
            if (!TryBuildDragPayload(refs, "none", out objectReferences, out paths, out title, out error))
            {
                ClearArmedDrag();
                return;
            }

            SetArmedDrag(objectReferences, paths, title, false, "none");
        }

        internal static bool QueueAssetDrag(
            LocusEditorWindow.DroppedAssetRef[] refs,
            string traceId,
            out string message)
        {
            traceId = LocusEditorWindow.NormalizeAssetDragTraceId(traceId);
            Object[] objectReferences;
            string[] paths;
            string title;
            string error;
            if (!TryBuildDragPayload(refs, traceId, out objectReferences, out paths, out title, out error))
            {
                message = error;
                LocusEditorWindow.LogAssetDragTrace(traceId, "unity_queue_payload_failed", "error=" + (error ?? ""));
                return false;
            }

            SetArmedDrag(objectReferences, paths, title, true, traceId);
            message = "queued";
            return true;
        }

        internal static void CancelAssetDrag()
        {
            string traceId;
            lock (ArmedDragLock)
            {
                traceId = _armedTraceId;
            }
            if (string.Equals(traceId, "none", StringComparison.Ordinal)
                && HasRecentDragPerformTrace())
            {
                traceId = _lastDragPerformTraceId;
            }
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_cancel_asset_drag_start",
                BuildPostPerformSnapshot());
            ClearArmedDrag();
            DragAndDrop.PrepareStartDrag();
            DragAndDrop.objectReferences = new Object[0];
            DragAndDrop.paths = new string[0];
            DragAndDrop.visualMode = DragAndDropVisualMode.None;
            LocusEditorWindow.ClearPublishedUnityAssetDragState();
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_cancel_asset_drag_done",
                BuildPostPerformSnapshot());
        }

        private static void SetArmedDrag(
            Object[] objectReferences,
            string[] paths,
            string title,
            bool startQueued,
            string traceId)
        {
            traceId = LocusEditorWindow.NormalizeAssetDragTraceId(traceId);
            lock (ArmedDragLock)
            {
                _armedObjectReferences = objectReferences ?? new Object[0];
                _armedPaths = paths ?? new string[0];
                _armedTitle = string.IsNullOrEmpty(title) ? "Locus References" : title;
                _armedExpiresAt = EditorApplication.timeSinceStartup + ArmedDragSeconds;
                _armedAt = EditorApplication.timeSinceStartup;
                _armedTraceId = traceId;
                _lastLoggedDragUpdatedTraceId = "";
                _startQueued = startQueued;
                _dragStarted = false;
            }
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_drag_payload_armed",
                "objects=" + (objectReferences == null ? 0 : objectReferences.Length) +
                " paths=" + (paths == null ? 0 : paths.Length) +
                " startQueued=" + startQueued);
        }

        private static void Install()
        {
            InstallBeforeEventProcessedHandler();
            InstallGlobalEventHandler();
            EditorApplication.update -= GlobalUpdateHandler;
            EditorApplication.update += GlobalUpdateHandler;
            EditorApplication.hierarchyChanged -= HandleHierarchyChanged;
            EditorApplication.hierarchyChanged += HandleHierarchyChanged;
            Selection.selectionChanged -= HandleSelectionChanged;
            Selection.selectionChanged += HandleSelectionChanged;
        }

        private static void InstallBeforeEventProcessedHandler()
        {
            if (BeforeEventProcessedField == null)
                return;

            var current = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode>;
            current -= BeforeEventProcessedHandler;
            current = (Action<EventType, KeyCode>)Delegate.Combine(BeforeEventProcessedHandler, current);
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
                var beforeCurrent = BeforeEventProcessedField.GetValue(null) as Action<EventType, KeyCode>;
                beforeCurrent -= BeforeEventProcessedHandler;
                BeforeEventProcessedField.SetValue(null, beforeCurrent);
            }

            EditorApplication.update -= GlobalUpdateHandler;
            EditorApplication.hierarchyChanged -= HandleHierarchyChanged;
            Selection.selectionChanged -= HandleSelectionChanged;

            if (GlobalEventHandlerField == null)
                return;

            var current = GlobalEventHandlerField.GetValue(null) as EditorApplication.CallbackFunction;
            current -= GlobalEventHandler;
            GlobalEventHandlerField.SetValue(null, current);
        }

        private static void HandleBeforeEventProcessed(EventType eventType, KeyCode keyCode)
        {
            if (eventType == EventType.MouseDrag)
            {
                StartQueuedDrag("beforeEventProcessed");
                LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
                return;
            }

            if (eventType != EventType.DragUpdated && eventType != EventType.DragPerform)
                return;

            LocusEditorWindow.PublishCurrentUnityAssetDragState(eventType == EventType.DragPerform);
            string traceId;
            if (ApplyArmedDragPayload("beforeEventProcessed", eventType, out traceId)
                && eventType == EventType.DragPerform)
            {
                MarkDragPerform(traceId, "beforeEventProcessed");
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
                if (StartQueuedDrag("globalEvent"))
                    evt.Use();
                LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
                return;
            }

            if (evt.type != EventType.DragUpdated && evt.type != EventType.DragPerform)
                return;

            LocusEditorWindow.PublishCurrentUnityAssetDragState(evt.type == EventType.DragPerform);
            string traceId;
            if (ApplyArmedDragPayload("globalEvent", evt.type, out traceId)
                && evt.type == EventType.DragPerform)
            {
                MarkDragPerform(traceId, "globalEvent");
                ClearArmedDrag();
            }
        }

        private static void HandleGlobalUpdate()
        {
            ProbePostDragPerformIfNeeded();

            if (!ShouldPublishAssetDragStateOnGlobalUpdate())
            {
                LocusEditorWindow.ClearPublishedUnityAssetDragState();
                return;
            }

            LocusEditorWindow.PublishCurrentUnityAssetDragState(false);
        }

        private static bool ShouldPublishAssetDragStateOnGlobalUpdate()
        {
            return HasActiveArmedDrag()
                || LocusEditorWindow.HasCurrentUnityDragAndDropRefs();
        }

        private static string ElapsedMs(double startedAt)
        {
            if (startedAt <= 0d)
                return "0.0";

            return ((EditorApplication.timeSinceStartup - startedAt) * 1000d)
                .ToString("F1", CultureInfo.InvariantCulture);
        }

        private static bool LogDragEvent(
            string traceId,
            string source,
            EventType eventType,
            double armedAt)
        {
            if (eventType == EventType.DragUpdated)
            {
                if (string.Equals(_lastLoggedDragUpdatedTraceId, traceId, StringComparison.Ordinal))
                    return false;
                _lastLoggedDragUpdatedTraceId = traceId;
            }

            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_drag_event",
                "source=" + source +
                " event=" + eventType +
                " sinceArmedMs=" + ElapsedMs(armedAt));
            return true;
        }

        private static void MarkDragPerform(string traceId, string source)
        {
            traceId = LocusEditorWindow.NormalizeAssetDragTraceId(traceId);
            EditorWindow dropTargetWindow = EditorWindow.mouseOverWindow;
            _lastDragPerformTraceId = traceId;
            _lastDragPerformAt = EditorApplication.timeSinceStartup;
            _postPerformProbeFramesRemaining = PostPerformProbeFrames;
            _hierarchyChangeLogCount = 0;
            _selectionChangeLogCount = 0;
            _delayCallProbeCount = 0;
            _lastDropTargetWindow = dropTargetWindow;
            _lastDragPerformObjectReferences = SnapshotObjectArray(DragAndDrop.objectReferences);
            _lastDragPerformPaths = SnapshotStringArray(DragAndDrop.paths);
            _lastDragPerformInitialSceneObjectId = SceneInstanceObjectId(Selection.activeObject);
            _instanceDetectedRepaintDone = false;
            _loggedHierarchyChangeAfterPerform = false;
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_drag_perform",
                "source=" + source + " " + BuildPostPerformSnapshot());
            RepaintDropTargetWindowsAfterDragPerform(traceId, dropTargetWindow, 0);
            double performAt = _lastDragPerformAt;
            ScheduleDelayCallProbe(traceId, performAt);
        }

        private static void RepaintDropTargetWindowsAfterDragPerform(
            string traceId,
            EditorWindow dropTargetWindow,
            int delayIndex)
        {
            string targetKind = DropTargetWindowKind(dropTargetWindow);
            bool repaintSceneAndHierarchy = targetKind == "scene" || targetKind == "hierarchy";

            if (repaintSceneAndHierarchy)
            {
                SceneView.RepaintAll();
                EditorApplication.RepaintHierarchyWindow();
            }
            else
            {
                RepaintWindow(dropTargetWindow);
                if (dropTargetWindow != EditorWindow.focusedWindow)
                    RepaintWindow(EditorWindow.focusedWindow);
            }

            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_repaint_after_drag_perform",
                "delayIndex=" + delayIndex +
                " targetKind=" + targetKind +
                " target=" + DescribeEditorWindow(dropTargetWindow) +
                " sceneAndHierarchy=" + repaintSceneAndHierarchy +
                " " + BuildPostPerformSnapshot());

            if (delayIndex >= 2)
                return;

            EditorApplication.delayCall += delegate
            {
                RepaintDropTargetWindowsAfterDragPerform(traceId, dropTargetWindow, delayIndex + 1);
            };
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

        private static void ScheduleDelayCallProbe(string traceId, double performAt)
        {
            EditorApplication.delayCall += delegate
            {
                _delayCallProbeCount++;
                LocusEditorWindow.LogAssetDragTrace(
                    traceId,
                    "unity_delay_call_after_drag_perform",
                    "index=" + _delayCallProbeCount +
                    " sincePerformMs=" + ElapsedMs(performAt) +
                    " " + BuildPostPerformSnapshot());

                if (_delayCallProbeCount < 5 && HasRecentDragPerformTrace(traceId))
                    ScheduleDelayCallProbe(traceId, performAt);
            };
        }

        private static void ProbePostDragPerformIfNeeded()
        {
            if (_postPerformProbeFramesRemaining <= 0 || string.IsNullOrEmpty(_lastDragPerformTraceId))
                return;

            int frame = PostPerformProbeFrames + 1 - _postPerformProbeFramesRemaining;
            _postPerformProbeFramesRemaining--;
            RepaintAfterInstanceDetectedIfNeeded(frame);
            if (!ShouldLogPostPerformProbeFrame(frame))
                return;

            LocusEditorWindow.LogAssetDragTrace(
                _lastDragPerformTraceId,
                "unity_post_perform_update_probe",
                "frame=" + frame +
                " " + BuildPostPerformSnapshot());
        }

        private static void RepaintAfterInstanceDetectedIfNeeded(int frame)
        {
            if (_instanceDetectedRepaintDone || !HasRecentDragPerformTrace())
                return;

            if (!IsSceneInstanceObject(Selection.activeObject))
                return;

            _instanceDetectedRepaintDone = true;
            RepaintDropTargetWindowsAfterInstanceDetected(
                _lastDragPerformTraceId,
                _lastDropTargetWindow,
                frame);
        }

        private static void RepaintDropTargetWindowsAfterInstanceDetected(
            string traceId,
            EditorWindow dropTargetWindow,
            int frame)
        {
            string targetKind = DropTargetWindowKind(dropTargetWindow);
            bool repaintSceneAndHierarchy = targetKind == "scene" || targetKind == "hierarchy";

            if (repaintSceneAndHierarchy)
            {
                SceneView.RepaintAll();
                EditorApplication.RepaintHierarchyWindow();
            }
            else
            {
                RepaintWindow(dropTargetWindow);
                if (dropTargetWindow != EditorWindow.focusedWindow)
                    RepaintWindow(EditorWindow.focusedWindow);
            }

            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_repaint_after_instance_detected",
                "frame=" + frame +
                " targetKind=" + targetKind +
                " target=" + DescribeEditorWindow(dropTargetWindow) +
                " sceneAndHierarchy=" + repaintSceneAndHierarchy +
                " " + BuildPostPerformSnapshot());
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

        private static void HandleHierarchyChanged()
        {
            if (!HasRecentDragPerformTrace() || _hierarchyChangeLogCount >= MaxPostPerformEventLogs)
                return;

            _hierarchyChangeLogCount++;
            _loggedHierarchyChangeAfterPerform = true;
            LocusEditorWindow.LogAssetDragTrace(
                _lastDragPerformTraceId,
                "unity_hierarchy_changed_after_drag_perform",
                "index=" + _hierarchyChangeLogCount +
                " " + BuildPostPerformSnapshot());
        }

        private static void HandleSelectionChanged()
        {
            if (!HasRecentDragPerformTrace() || _selectionChangeLogCount >= MaxPostPerformEventLogs)
                return;

            _selectionChangeLogCount++;
            LocusEditorWindow.LogAssetDragTrace(
                _lastDragPerformTraceId,
                "unity_selection_changed_after_drag_perform",
                "index=" + _selectionChangeLogCount +
                " " + BuildPostPerformSnapshot());
        }

        private static bool ShouldLogPostPerformProbeFrame(int frame)
        {
            for (int i = 0; i < PostPerformProbeFrameMarks.Length; i++)
            {
                if (PostPerformProbeFrameMarks[i] == frame)
                    return true;
            }

            return false;
        }

        private static bool HasRecentDragPerformTrace(string traceId = null)
        {
            if (string.IsNullOrEmpty(_lastDragPerformTraceId) || _lastDragPerformAt <= 0d)
                return false;
            if (!string.IsNullOrEmpty(traceId)
                && !string.Equals(_lastDragPerformTraceId, traceId, StringComparison.Ordinal))
            {
                return false;
            }

            return EditorApplication.timeSinceStartup - _lastDragPerformAt <= PostPerformProbeSeconds;
        }

        private static string BuildPostPerformSnapshot()
        {
            return "sincePerformMs=" + ElapsedMs(_lastDragPerformAt) +
                " selectionCount=" + Selection.objects.Length +
                " active=" + DescribeUnityObject(Selection.activeObject) +
                " drag=" + DescribeDragAndDropState() +
                " scene=" + DescribeActiveSceneState() +
                " editor=" + DescribeEditorState();
        }

        private static string DescribeDragAndDropState()
        {
            Object[] objects = DragAndDrop.objectReferences;
            string[] paths = DragAndDrop.paths;
            Object firstObject = objects != null && objects.Length > 0 ? objects[0] : null;
            string firstPath = paths != null && paths.Length > 0 ? paths[0] : "";
            return "objects=" + (objects == null ? 0 : objects.Length) +
                ",paths=" + (paths == null ? 0 : paths.Length) +
                ",visualMode=" + DragAndDrop.visualMode +
                ",firstObject=" + DescribeUnityObject(firstObject) +
                ",firstPath=" + SanitizeLogValue(firstPath, 120);
        }

        private static string DescribeActiveSceneState()
        {
            try
            {
                UnityEngine.SceneManagement.Scene scene =
                    UnityEngine.SceneManagement.SceneManager.GetActiveScene();
                if (!scene.IsValid())
                    return "invalid";

                return "name=" + SanitizeLogValue(scene.name, 80) +
                    ",path=" + SanitizeLogValue(scene.path, 160) +
                    ",loaded=" + scene.isLoaded +
                    ",dirty=" + scene.isDirty +
                    ",rootCount=" + SafeSceneRootCount(scene) +
                    ",sceneCount=" + UnityEngine.SceneManagement.SceneManager.sceneCount;
            }
            catch (Exception ex)
            {
                return "error=" + SanitizeLogValue(ex.GetType().Name, 80);
            }
        }

        private static int SafeSceneRootCount(UnityEngine.SceneManagement.Scene scene)
        {
            try
            {
                return scene.GetRootGameObjects().Length;
            }
            catch
            {
                return -1;
            }
        }

        private static string DescribeEditorState()
        {
            return "isUpdating=" + EditorApplication.isUpdating +
                ",isCompiling=" + EditorApplication.isCompiling +
                ",focusedWindow=" + DescribeEditorWindow(EditorWindow.focusedWindow) +
                ",mouseOverWindow=" + DescribeEditorWindow(EditorWindow.mouseOverWindow);
        }

        private static string DescribeEditorWindow(EditorWindow window)
        {
            if (window == null)
                return "null";

            string title = window.titleContent != null ? window.titleContent.text : "";
            return SanitizeLogValue(window.GetType().Name + ":" + title, 100);
        }

        private static string DescribeUnityObject(Object obj)
        {
            if (obj == null)
                return "null";

            GameObject go = obj as GameObject;
            Component component = obj as Component;
            if (go == null && component != null)
                go = component.gameObject;

            string assetPath = "";
            try
            {
                assetPath = AssetDatabase.GetAssetPath(obj);
            }
            catch
            {
                assetPath = "";
            }

            bool persistent = false;
            try
            {
                persistent = EditorUtility.IsPersistent(obj);
            }
            catch
            {
            }

            string detail = SanitizeLogValue(obj.GetType().Name + ":" + obj.name, 120) +
                ",id=" + obj.GetInstanceID() +
                ",persistent=" + persistent +
                ",assetPath=" + SanitizeLogValue(assetPath, 160);

            if (go == null)
                return detail;

            detail += ",goPath=" + SanitizeLogValue(GameObjectPath(go), 180) +
                ",goScene=" + SanitizeLogValue(go.scene.path, 160) +
                ",goSceneLoaded=" + go.scene.isLoaded +
                ",goActive=" + go.activeInHierarchy;
            return detail;
        }

        private static string GameObjectPath(GameObject go)
        {
            if (go == null)
                return "";

            Stack<string> parts = new Stack<string>();
            Transform current = go.transform;
            while (current != null)
            {
                parts.Push(current.name);
                current = current.parent;
            }

            return string.Join("/", parts.ToArray());
        }

        private static string SanitizeLogValue(string value, int maxLength)
        {
            string sanitized = (value ?? "")
                .Replace('\r', ' ')
                .Replace('\n', ' ')
                .Replace('\t', ' ');
            if (sanitized.Length <= maxLength)
                return sanitized;
            return sanitized.Substring(0, maxLength) + "...";
        }

        private static bool HasActiveArmedDrag()
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();
                return HasArmedDragLocked();
            }
        }

        private static bool ApplyArmedDragPayload(string source, EventType eventType, out string traceId)
        {
            Object[] references;
            string[] paths;
            string title;
            double armedAt;
            if (!TryGetArmedDrag(out references, out paths, out title, out traceId, out armedAt))
                return false;

            bool loggedEvent = LogDragEvent(traceId, source, eventType, armedAt);
            DragAndDrop.objectReferences = references;
            DragAndDrop.paths = paths;
            DragAndDrop.visualMode = DragAndDropVisualMode.Copy;
            if (loggedEvent || eventType == EventType.DragPerform)
            {
                LocusEditorWindow.LogAssetDragTrace(
                    traceId,
                    "unity_apply_armed_payload",
                    "source=" + source +
                    " event=" + eventType +
                    " objects=" + references.Length +
                    " paths=" + paths.Length +
                    " sinceArmedMs=" + ElapsedMs(armedAt));
            }
            return true;
        }

        private static bool StartQueuedDrag(string source)
        {
            Object[] references;
            string[] paths;
            string title;
            string traceId;
            double armedAt;
            if (!TryConsumeStartQueuedDrag(out references, out paths, out title, out traceId, out armedAt))
                return false;

            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_start_queued_drag",
                "source=" + source +
                " objects=" + references.Length +
                " paths=" + paths.Length +
                " sinceArmedMs=" + ElapsedMs(armedAt));
            DragAndDrop.PrepareStartDrag();
            DragAndDrop.objectReferences = references;
            DragAndDrop.paths = paths;
            DragAndDrop.StartDrag(title);
            DragAndDrop.visualMode = DragAndDropVisualMode.Copy;
            LocusEditorWindow.LogAssetDragTrace(traceId, "unity_start_drag_called", "source=" + source);
            return true;
        }

        private static bool TryConsumeStartQueuedDrag(
            out Object[] objectReferences,
            out string[] paths,
            out string title,
            out string traceId,
            out double armedAt)
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();

                if (!_startQueued || _dragStarted || !HasArmedDragLocked())
                {
                    objectReferences = new Object[0];
                    paths = new string[0];
                    title = "Locus References";
                    traceId = "none";
                    armedAt = 0d;
                    return false;
                }

                _startQueued = false;
                _dragStarted = true;
                objectReferences = _armedObjectReferences;
                paths = _armedPaths;
                title = _armedTitle;
                traceId = _armedTraceId;
                armedAt = _armedAt;
                return true;
            }
        }

        private static bool TryGetArmedDrag(
            out Object[] objectReferences,
            out string[] paths,
            out string title,
            out string traceId,
            out double armedAt)
        {
            lock (ArmedDragLock)
            {
                ExpireArmedDragIfNeededLocked();

                objectReferences = _armedObjectReferences;
                paths = _armedPaths;
                title = _armedTitle;
                traceId = _armedTraceId;
                armedAt = _armedAt;
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
                _armedAt = 0d;
                _armedTraceId = "none";
                _lastLoggedDragUpdatedTraceId = "";
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
            _armedAt = 0d;
            _armedTraceId = "none";
            _lastLoggedDragUpdatedTraceId = "";
            _startQueued = false;
            _dragStarted = false;
        }

        private static bool HasArmedDragLocked()
        {
            return _armedObjectReferences.Length > 0 || _armedPaths.Length > 0;
        }

        private static bool TryBuildDragPayload(
            LocusEditorWindow.DroppedAssetRef[] refs,
            string traceId,
            out Object[] objectReferences,
            out string[] paths,
            out string title,
            out string error)
        {
            traceId = LocusEditorWindow.NormalizeAssetDragTraceId(traceId);
            double buildStartedAt = EditorApplication.timeSinceStartup;
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_build_drag_payload_start",
                "refs=" + (refs == null ? 0 : refs.Length));

            if (refs == null || refs.Length == 0)
            {
                objectReferences = new Object[0];
                paths = new string[0];
                title = "Locus Reference";
                error = "No supported Unity references were provided.";
                LocusEditorWindow.LogAssetDragTrace(traceId, "unity_build_drag_payload_empty");
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

                    double loadStartedAt = EditorApplication.timeSinceStartup;
                    Object reference = AssetDatabase.LoadMainAssetAtPath(assetPath);
                    LocusEditorWindow.LogAssetDragTrace(
                        traceId,
                        "unity_load_asset_for_drag",
                        "path=" + assetPath +
                        " found=" + (reference != null) +
                        " elapsedMs=" + ElapsedMs(loadStartedAt));
                    if (reference != null)
                        references.Add(reference);
                    pathRefs.Add(assetPath);
                    continue;
                }

                if (assetRef.kind == "sceneObject")
                {
                    Object reference;
                    string resolveError;
                    double resolveStartedAt = EditorApplication.timeSinceStartup;
                    if (!TryResolveSceneObject(assetRef.path, out reference, out resolveError))
                    {
                        LocusEditorWindow.LogAssetDragTrace(
                            traceId,
                            "unity_resolve_scene_object_for_drag_failed",
                            "path=" + (assetRef.path ?? "") +
                            " elapsedMs=" + ElapsedMs(resolveStartedAt) +
                            " error=" + (resolveError ?? ""));
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

                    LocusEditorWindow.LogAssetDragTrace(
                        traceId,
                        "unity_resolve_scene_object_for_drag",
                        "path=" + (assetRef.path ?? "") +
                        " found=" + (reference != null) +
                        " elapsedMs=" + ElapsedMs(resolveStartedAt));
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
                LocusEditorWindow.LogAssetDragTrace(
                    traceId,
                    "unity_build_drag_payload_no_objects",
                    "elapsedMs=" + ElapsedMs(buildStartedAt));
                return false;
            }

            error = "";
            LocusEditorWindow.LogAssetDragTrace(
                traceId,
                "unity_build_drag_payload_done",
                "objects=" + objectReferences.Length +
                " paths=" + paths.Length +
                " elapsedMs=" + ElapsedMs(buildStartedAt));
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
