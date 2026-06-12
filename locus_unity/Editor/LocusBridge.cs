// 2026-03-28 - Introduce Unity edit sessions via Auto Refresh suppression and persist recompile results across domain reloads

using UnityEngine;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEditor.SceneManagement;

using System;
using System.Globalization;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Collections.Generic;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Locus
{
    [InitializeOnLoad]
    public static partial class LocusBridge
    {
        // ───────────────── Connection state ─────────────────

        private static string _pipeName;
        private static string PipeName
        {
            get
            {
                if (_pipeName == null)
                    _pipeName = GeneratePipeName();
                return _pipeName;
            }
        }

        private static CancellationTokenSource _cts;
        private static Task _serverTask;

        private static readonly object _connectionLock = new object();
        private static readonly SemaphoreSlim _writeLock = new SemaphoreSlim(1, 1);
        private static readonly SemaphoreSlim _executeCodeLock = new SemaphoreSlim(1, 1);
        private static readonly SemaphoreSlim _runStatesLock = new SemaphoreSlim(1, 1);

        private static NamedPipeServerStream _currentServer;
        private static StreamWriter _currentWriter;
        private static volatile bool _desktopPipeConnected;
        private static readonly int _editorProcessId = ResolveCurrentProcessId();
        private static readonly string _editorProcessPath = ResolveCurrentProcessPath();

        private static readonly UTF8Encoding Utf8NoBom = new UTF8Encoding(false);

        // ───────────────── Constants ─────────────────

        private const int ExecuteTimeoutMs = 30000;
        private const int PipeBufferSize = 64 * 1024;
        private const int TextReaderWriterBufferSize = 16 * 1024;
        private const int MaxMainThreadActionsPerUpdate = 32;
#if UNITY_2020
        private const PipeOptions ServerPipeOptions = PipeOptions.None;
#else
        private const PipeOptions ServerPipeOptions = PipeOptions.Asynchronous;
#endif

        // ───────────────── Main-thread dispatcher ─────────────────

        private static readonly object _mainThreadQueueLock = new object();
        private static readonly Queue<Action> _mainThreadQueue = new Queue<Action>(64);

        // ───────────────── Cached editor state (updated on main thread) ─────────────────

        private static volatile bool _isPlaying;
        private static volatile bool _isPaused;
        private static volatile string _activeScenePath = "";
        private static int _editorUpdateEventSequence;
        private static double _lastEditorUpdateEventAt = -1.0;
        private static string _lastEditorUpdateSelectionKey = "";
        private const double EditorUpdateEventIntervalSeconds = 0.25;

        // ───────────────── Runtime compilation cache ─────────────────

        private static readonly object _compileCacheLock = new object();

        private static List<MetadataReference> _cachedMetadataReferences;
        private static bool _metadataReferencesReady;
        private static List<string> _cachedCompileReferencePaths;
        private static bool _compileReferencePathsReady;
        private static int _snippetAssemblyCounter;

        // ───────────────── Agent-controlled recompile ─────────────────

        private const string SessionKey_RecompileInProgress = "Locus_RecompileInProgress";
        private const string SessionKey_RecompileResult = "Locus_RecompileResult";

        private static volatile bool _recompileRequested;
        private static volatile string _lastCompileResult;
        private static readonly HashSet<string> _activeEditSessionOwners =
            new HashSet<string>(StringComparer.Ordinal);
        private static readonly HashSet<string> _pendingChangedAssetPaths =
            new HashSet<string>(StringComparer.Ordinal);
        private static int _autoRefreshSuppressionCount;

        /// <summary>
        /// Frame counter for detecting "no compilation started" after request_recompile.
        /// -1 = inactive; 0+ = counting frames since recompile was requested.
        /// </summary>
        private static int _recompileCheckFrames = -1;
        private const int RecompileCheckDelayFrames = 5;

        /// <summary>
        /// Frame counter for detecting "domain reload not triggered" after compilation succeeded.
        /// -1 = inactive; 0+ = counting frames since compilation succeeded and we're waiting for domain reload.
        /// If domain reload happens, static fields are reset (new AppDomain) so this counter disappears.
        /// If we're still counting past the threshold, domain reload didn't happen.
        /// </summary>
        private static int _domainReloadCheckFrames = -1;
        private const int DomainReloadCheckDelayFrames = 100;

        private static readonly List<string> _recompileErrors = new List<string>();
        private static readonly object _recompileErrorsLock = new object();

        private static readonly string[] SnippetPreprocessorSymbols = BuildSnippetPreprocessorSymbols();

        private static readonly CSharpParseOptions SnippetParseOptions =
            new CSharpParseOptions(
                kind: SourceCodeKind.Regular,
                documentationMode: DocumentationMode.None,
                languageVersion: LanguageVersion.CSharp9,
                preprocessorSymbols: SnippetPreprocessorSymbols
            );

        private static readonly CSharpCompilationOptions SnippetCompilationOptions =
            new CSharpCompilationOptions(
                outputKind: OutputKind.DynamicallyLinkedLibrary,
                optimizationLevel: OptimizationLevel.Release,
                allowUnsafe: false,
                assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default
            );

        private static string[] BuildSnippetPreprocessorSymbols()
        {
            var symbols = new HashSet<string>(StringComparer.Ordinal)
            {
                "UNITY_EDITOR"
            };

#if UNITY_EDITOR_WIN
            symbols.Add("UNITY_EDITOR_WIN");
            symbols.Add("UNITY_STANDALONE_WIN");
#endif
#if UNITY_EDITOR_OSX
            symbols.Add("UNITY_EDITOR_OSX");
            symbols.Add("UNITY_STANDALONE_OSX");
#endif
#if UNITY_EDITOR_LINUX
            symbols.Add("UNITY_EDITOR_LINUX");
            symbols.Add("UNITY_STANDALONE_LINUX");
#endif
            AddUnityVersionPreprocessorSymbols(symbols);

#if UNITY_2020
            symbols.Add("UNITY_2020");
#endif
#if UNITY_2021
            symbols.Add("UNITY_2021");
#endif
#if UNITY_2022
            symbols.Add("UNITY_2022");
#endif
#if UNITY_2023
            symbols.Add("UNITY_2023");
#endif
#if UNITY_6000_0_OR_NEWER
            symbols.Add("UNITY_6000_0_OR_NEWER");
#endif
#if UNITY_2020_3_OR_NEWER
            symbols.Add("UNITY_2020_3_OR_NEWER");
#endif
#if UNITY_2021_3_OR_NEWER
            symbols.Add("UNITY_2021_3_OR_NEWER");
#endif
#if UNITY_2022_3_OR_NEWER
            symbols.Add("UNITY_2022_3_OR_NEWER");
#endif
#if UNITY_2023_1_OR_NEWER
            symbols.Add("UNITY_2023_1_OR_NEWER");
#endif
#if ENABLE_INPUT_SYSTEM
            symbols.Add("ENABLE_INPUT_SYSTEM");
#endif
#if ENABLE_LEGACY_INPUT_MANAGER
            symbols.Add("ENABLE_LEGACY_INPUT_MANAGER");
#endif

            try
            {
                var group = EditorUserBuildSettings.selectedBuildTargetGroup;
                string raw = PlayerSettings.GetScriptingDefineSymbolsForGroup(group);
                if (!string.IsNullOrEmpty(raw))
                {
                    string[] customSymbols = raw.Split(';');
                    for (int i = 0; i < customSymbols.Length; i++)
                    {
                        string symbol = customSymbols[i].Trim();
                        if (!string.IsNullOrEmpty(symbol))
                            symbols.Add(symbol);
                    }
                }
            }
            catch
            {
            }

            var list = new List<string>(symbols);
            list.Sort(StringComparer.Ordinal);
            return list.ToArray();
        }

        private static void AddUnityVersionPreprocessorSymbols(HashSet<string> symbols)
        {
            string version = Application.unityVersion ?? "";
            string[] parts = version.Split('.');
            int major = parts.Length > 0 ? ReadLeadingInt(parts[0]) : -1;
            int minor = parts.Length > 1 ? ReadLeadingInt(parts[1]) : -1;
            int patch = parts.Length > 2 ? ReadLeadingInt(parts[2]) : -1;
            if (major <= 0)
                return;

            symbols.Add("UNITY_" + major.ToString(CultureInfo.InvariantCulture));
            if (minor >= 0)
            {
                string majorMinor = "UNITY_" +
                    major.ToString(CultureInfo.InvariantCulture) +
                    "_" +
                    minor.ToString(CultureInfo.InvariantCulture);
                symbols.Add(majorMinor);
                symbols.Add(majorMinor + "_OR_NEWER");

                int firstMinor = major >= 6000 ? 0 : 1;
                for (int currentMinor = firstMinor; currentMinor <= minor; currentMinor++)
                {
                    symbols.Add(
                        "UNITY_" +
                        major.ToString(CultureInfo.InvariantCulture) +
                        "_" +
                        currentMinor.ToString(CultureInfo.InvariantCulture) +
                        "_OR_NEWER");
                }
            }

            if (minor >= 0 && patch >= 0)
            {
                symbols.Add(
                    "UNITY_" +
                    major.ToString(CultureInfo.InvariantCulture) +
                    "_" +
                    minor.ToString(CultureInfo.InvariantCulture) +
                    "_" +
                    patch.ToString(CultureInfo.InvariantCulture));
            }
        }

        private static int ReadLeadingInt(string value)
        {
            if (string.IsNullOrEmpty(value))
                return -1;

            int end = 0;
            while (end < value.Length && char.IsDigit(value[end]))
                end++;
            if (end == 0)
                return -1;

            int result;
            return int.TryParse(value.Substring(0, end), NumberStyles.None, CultureInfo.InvariantCulture, out result)
                ? result
                : -1;
        }

        [Serializable]
        private sealed class EditorUpdatePayload
        {
            public int sequence;
            public double timeSinceStartup;
            public bool isPlaying;
            public bool isPaused;
            public string activeScenePath;
            public EditorSelectionSnapshot selection;
        }

        [Serializable]
        private sealed class EditorSelectionSnapshot
        {
            public string kind;
            public string name;
            public string type;
            public string path;
            public int instanceId;
        }

        // ───────────────── Lifecycle ─────────────────

        static LocusBridge()
        {
            // Keep the bridge alive across edit sessions. Auto Refresh is only suppressed while a session is active.
            EditorApplication.update += PumpMainThreadQueue;
            EditorApplication.delayCall += Start;
            EditorApplication.quitting += OnQuitting;
            AssemblyReloadEvents.beforeAssemblyReload += Stop;
            AssemblyReloadEvents.afterAssemblyReload += OnAfterAssemblyReload;
            CompilationPipeline.compilationFinished += OnCompilationFinished;
            CompilationPipeline.assemblyCompilationFinished += OnAssemblyCompilationFinished;
        }

        private static void OnQuitting()
        {
            ReleaseAllEditSessions();
            Stop();
        }

        private static string GeneratePipeName()
        {
            string projectPath = Directory.GetParent(Application.dataPath).FullName;
            string sanitized = projectPath
                .Replace('\\', '_')
                .Replace('/', '_')
                .Replace(':', '_')
                .Replace(' ', '_');

            return "locus_unity_" + sanitized;
        }

        private static int ResolveCurrentProcessId()
        {
            try
            {
                using (var process = System.Diagnostics.Process.GetCurrentProcess())
                    return process.Id;
            }
            catch
            {
                return 0;
            }
        }

        private static string ResolveCurrentProcessPath()
        {
            try
            {
                using (var process = System.Diagnostics.Process.GetCurrentProcess())
                {
                    try
                    {
                        if (process.MainModule != null && !string.IsNullOrEmpty(process.MainModule.FileName))
                            return process.MainModule.FileName;
                    }
                    catch
                    {
                    }

                    try
                    {
                        if (!string.IsNullOrEmpty(EditorApplication.applicationPath))
                            return EditorApplication.applicationPath;
                    }
                    catch
                    {
                    }

                    return process.ProcessName ?? "";
                }
            }
            catch
            {
                return "";
            }
        }

        private static bool IsProjectAssetPath(string path)
        {
            if (string.IsNullOrEmpty(path))
                return false;

            string normalized = path.Replace('\\', '/');
            return normalized.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                || normalized.StartsWith("Packages/", StringComparison.OrdinalIgnoreCase);
        }

        private static bool IsProjectPrefabPath(string path)
        {
            return IsProjectAssetPath(path)
                && path.Replace('\\', '/').EndsWith(".prefab", StringComparison.OrdinalIgnoreCase);
        }

        private static string TrimToProjectAssetPath(string path)
        {
            if (string.IsNullOrEmpty(path))
                return path;

            string normalized = path.Replace('\\', '/');
            if (IsProjectAssetPath(normalized))
                return normalized;

            string[] prefixes = { "Assets/", "Packages/" };
            foreach (string prefix in prefixes)
            {
                int idx = normalized.IndexOf(prefix, StringComparison.OrdinalIgnoreCase);
                if (idx < 0)
                    continue;
                if (idx == 0 || normalized[idx - 1] == '/')
                    return normalized.Substring(idx);
            }

            return null;
        }

        public static void Start()
        {
            if (_serverTask != null && !_serverTask.IsCompleted)
                return;

            try
            {
                _cts = new CancellationTokenSource();

                _serverTask = Task.Factory
                    .StartNew(
                        () => ServerLoop(_cts.Token),
                        _cts.Token,
                        TaskCreationOptions.LongRunning,
                        TaskScheduler.Default)
                    .Unwrap();

                Debug.Log("[Locus] Bridge started, listening on pipe: " + PipeName);
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Bridge failed to start: " + ex);
            }
        }

        public static void Stop()
        {
            CancelActiveExecuteCode("bridge stopped");

            var cts = _cts;
            var task = _serverTask;

            _cts = null;
            _serverTask = null;

            try
            {
                lock (_connectionLock)
                {
                    try { if (_currentWriter != null) _currentWriter.Dispose(); } catch { }
                    try { if (_currentServer != null) _currentServer.Dispose(); } catch { }

                    _currentWriter = null;
                    _currentServer = null;
                    _desktopPipeConnected = false;
                }
            }
            catch
            {
            }

            if (cts != null)
            {
                try
                {
                    cts.Cancel();

                    if (task != null && !task.IsCompleted)
                        task.Wait(1000);
                }
                catch
                {
                }
                finally
                {
                    cts.Dispose();
                }
            }

            lock (_mainThreadQueueLock)
            {
                _mainThreadQueue.Clear();
            }

            Debug.Log("[Locus] Bridge stopped.");
        }

        // ───────────────── Compilation events ─────────────────

        private static void OnAssemblyCompilationFinished(string assemblyPath, CompilerMessage[] messages)
        {
            if (!_recompileRequested)
                return;

            lock (_recompileErrorsLock)
            {
                foreach (var msg in messages)
                {
                    if (msg.type == CompilerMessageType.Error)
                    {
                        _recompileErrors.Add(msg.message);
                    }
                }
            }
        }

        private static void OnCompilationFinished(object context)
        {
            InvalidateCompilationCaches();

            // Compilation did fire — cancel the "no compilation" check
            _recompileCheckFrames = -1;

            if (!_recompileRequested)
                return;

            _recompileRequested = false;

            lock (_recompileErrorsLock)
            {
                if (_recompileErrors.Count > 0)
                {
                    // Compilation failed. Persist the error so Rust can surface it after any reconnect.
                    SetCompileResult("error:" + string.Join("\n", _recompileErrors));
                    _recompileErrors.Clear();

                    // Failed compilations do not trigger a domain reload, so clear the in-progress flag here.
                    SessionState.SetBool(SessionKey_RecompileInProgress, false);
                    _domainReloadCheckFrames = -1;
                }
                else
                {
                    // Compilation finished successfully. Mark the result and wait for the real reload signal.
                    SetCompileResult("awaiting_reload");
                    _recompileErrors.Clear();
                    Debug.Log($"[Locus] Compilation succeeded, waiting for domain reload. isCompiling={EditorApplication.isCompiling}, isPlaying={EditorApplication.isPlaying}");
                    // If we are still in the same AppDomain after a few frames, reload did not fire.
                    _domainReloadCheckFrames = 0;
                }
            }
        }

        private static void OnAfterAssemblyReload()
        {
            // afterAssemblyReload is the authoritative completion point for a successful recompile.
            if (!SessionState.GetBool(SessionKey_RecompileInProgress, false))
                return;

            SessionState.SetBool(SessionKey_RecompileInProgress, false);
            SetCompileResult("ok");
        }

        private static void InvalidateExecuteCodeMetadataReferences()
        {
            lock (_compileCacheLock)
            {
                _metadataReferencesReady = false;
                _cachedMetadataReferences = null;
                _compileReferencePathsReady = false;
                _cachedCompileReferencePaths = null;
            }
        }

        private static void InvalidateCompilationCaches()
        {
            InvalidateExecuteCodeMetadataReferences();
            InvalidateViewScriptCache();
            InvalidateSkillPackageAssemblyCache();
        }

        // ───────────────── Main-thread dispatcher ─────────────────

        private static void SetCompileResult(string result)
        {
            _lastCompileResult = result;
            SessionState.SetString(SessionKey_RecompileResult, result ?? "");
        }

        private static string GetCompileResult()
        {
            if (!string.IsNullOrEmpty(_lastCompileResult))
                return _lastCompileResult;

            string result = SessionState.GetString(SessionKey_RecompileResult, "");
            if (!string.IsNullOrEmpty(result))
                _lastCompileResult = result;
            return result;
        }

        private static void ClearCompileResult()
        {
            _lastCompileResult = null;
            SessionState.SetString(SessionKey_RecompileResult, "");
        }

        private static void QueueChangedAssets(IEnumerable<string> assetPaths)
        {
            if (assetPaths == null)
                return;

            foreach (string rawPath in assetPaths)
            {
                string assetPath = (rawPath ?? "").Trim().Replace('\\', '/');
                if (string.IsNullOrEmpty(assetPath))
                    continue;
                if (!assetPath.StartsWith("Assets/", StringComparison.Ordinal) &&
                    !assetPath.StartsWith("Packages/", StringComparison.Ordinal))
                    continue;

                _pendingChangedAssetPaths.Add(assetPath);
            }
        }

        private static int FlushQueuedAssetImports()
        {
            if (_pendingChangedAssetPaths.Count == 0)
                return 0;

            string[] pendingPaths = new string[_pendingChangedAssetPaths.Count];
            _pendingChangedAssetPaths.CopyTo(pendingPaths);
            _pendingChangedAssetPaths.Clear();
            // Parents sort before their children, so a brand-new folder is in
            // the database before its files import.
            Array.Sort(pendingPaths, StringComparer.Ordinal);

            int importedCount = 0;
            bool needsRefresh = false;
            foreach (string assetPath in pendingPaths)
            {
                try
                {
                    if (!File.Exists(assetPath) && !Directory.Exists(assetPath))
                    {
                        // Deleted on disk. ImportAsset cannot drop a stale
                        // database entry; a Refresh pass below picks the
                        // removal up (leaving it would fail the next compile
                        // with a missing source file).
                        needsRefresh = true;
                        continue;
                    }
                    AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.ForceUpdate);
                    importedCount++;
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] Failed to import changed asset before compile: " + assetPath + "\n" + ex);
                }
            }

            if (needsRefresh)
            {
                try
                {
                    AssetDatabase.Refresh();
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] AssetDatabase.Refresh for deleted assets failed: " + ex);
                }
            }

            if (importedCount > 0 || needsRefresh)
                Debug.Log("[Locus] Flushed changed asset imports before compile: " + importedCount
                    + (needsRefresh ? " (+refresh for deletions)" : ""));

            return importedCount;
        }

        private static string BeginEditSession(string owner)
        {
            string normalizedOwner = string.IsNullOrEmpty(owner) ? "default" : owner.Trim();
            if (_activeEditSessionOwners.Add(normalizedOwner))
            {
                AssetDatabase.DisallowAutoRefresh();
                _autoRefreshSuppressionCount++;
                Debug.Log($"[Locus] Edit session started by '{normalizedOwner}', active owners={_activeEditSessionOwners.Count}");
            }

            return "active_edit_sessions:" + _activeEditSessionOwners.Count;
        }

        private static string EndEditSession(string owner)
        {
            if (string.IsNullOrEmpty(owner))
            {
                ReleaseAllEditSessions();
                return "active_edit_sessions:0";
            }

            string normalizedOwner = owner.Trim();
            if (_activeEditSessionOwners.Remove(normalizedOwner))
            {
                if (_autoRefreshSuppressionCount > 0)
                {
                    AssetDatabase.AllowAutoRefresh();
                    _autoRefreshSuppressionCount--;
                }

                Debug.Log($"[Locus] Edit session ended by '{normalizedOwner}', active owners={_activeEditSessionOwners.Count}");
            }

            return "active_edit_sessions:" + _activeEditSessionOwners.Count;
        }

        private static void ReleaseAllEditSessions()
        {
            if (_activeEditSessionOwners.Count == 0 && _autoRefreshSuppressionCount == 0)
                return;

            _activeEditSessionOwners.Clear();
            while (_autoRefreshSuppressionCount > 0)
            {
                AssetDatabase.AllowAutoRefresh();
                _autoRefreshSuppressionCount--;
            }

            Debug.Log("[Locus] Released all edit sessions.");
        }

        private static void PostToMainThread(Action action)
        {
            if (action == null)
                return;

            lock (_mainThreadQueueLock)
            {
                _mainThreadQueue.Enqueue(action);
            }
        }

        private static void PumpMainThreadQueue()
        {
            bool desktopConnected = HasDesktopPipeConnection();
            bool hasRuntimeWork = HasMainThreadRuntimeWork();
            if (desktopConnected || hasRuntimeWork)
                RefreshCachedEditorState();

            if (_activeRunStatesSession != null)
                PumpRunStates();
            if (HasActiveExecuteCodeAsyncRuntime())
                PumpExecuteCodeAsyncRuntime();
            if (desktopConnected)
                MaybeSendEditorUpdateEvent();

            // Detect "no compilation started" after request_recompile
            if (_recompileCheckFrames >= 0)
            {
                _recompileCheckFrames++;
                if (_recompileCheckFrames >= RecompileCheckDelayFrames)
                {
                    _recompileCheckFrames = -1;
                    if (_recompileRequested && !EditorApplication.isCompiling)
                    {
                        // Unity never started compilation — no script changes detected
                        _recompileRequested = false;
                        SetCompileResult("error:Unity 未检测到脚本变更，编译未触发。请确认 .cs 文件已正确写入且路径位于 Assets 目录内。");
                        SessionState.SetBool(SessionKey_RecompileInProgress, false);
                        _domainReloadCheckFrames = -1;
                    }
                }
            }

            // Detect "domain reload not triggered" after compilation succeeded
            // If domain reload happened, this AppDomain is destroyed and _domainReloadCheckFrames resets to -1.
            // Still counting here means we're in the same AppDomain — reload didn't fire.
            if (_domainReloadCheckFrames >= 0)
            {
                _domainReloadCheckFrames++;
                if (_domainReloadCheckFrames >= DomainReloadCheckDelayFrames)
                {
                    _domainReloadCheckFrames = -1;
                    SetCompileResult("error:编译成功但域重载未触发。请检查 Unity Editor 当前状态是否正常。");
                    SessionState.SetBool(SessionKey_RecompileInProgress, false);
                }
            }

            for (int i = 0; i < MaxMainThreadActionsPerUpdate; i++)
            {
                Action action = null;

                lock (_mainThreadQueueLock)
                {
                    if (_mainThreadQueue.Count > 0)
                        action = _mainThreadQueue.Dequeue();
                }

                if (action == null)
                    break;

                try
                {
                    action();
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] Main-thread action failed: " + ex);
                }
            }
        }

        private static bool HasDesktopPipeConnection()
        {
            return _desktopPipeConnected;
        }

        private static bool HasMainThreadRuntimeWork()
        {
            return _activeRunStatesSession != null
                || HasActiveExecuteCodeAsyncRuntime()
                || _recompileCheckFrames >= 0
                || _domainReloadCheckFrames >= 0;
        }

        private static bool HasActiveExecuteCodeAsyncRuntime()
        {
            return _activeAsyncExecuteCount > 0;
        }

        private static void RefreshCachedEditorState()
        {
            _isPlaying = EditorApplication.isPlaying;
            _isPaused = EditorApplication.isPaused;
            _activeScenePath = EditorSceneManager.GetActiveScene().path ?? "";
        }

        private static void MaybeSendEditorUpdateEvent()
        {
            double now = EditorApplication.timeSinceStartup;
            UnityEngine.Object selection = Selection.activeObject;
            string selectionKey = selection != null ? selection.GetInstanceID().ToString() : "none";
            bool selectionChanged = !string.Equals(selectionKey, _lastEditorUpdateSelectionKey, StringComparison.Ordinal);
            if (!selectionChanged && _lastEditorUpdateEventAt >= 0 && now - _lastEditorUpdateEventAt < EditorUpdateEventIntervalSeconds)
                return;

            _lastEditorUpdateSelectionKey = selectionKey;
            _lastEditorUpdateEventAt = now;
            _editorUpdateEventSequence++;

            var payload = new EditorUpdatePayload
            {
                sequence = _editorUpdateEventSequence,
                timeSinceStartup = now,
                isPlaying = _isPlaying,
                isPaused = _isPaused,
                activeScenePath = _activeScenePath,
                selection = BuildEditorSelectionSnapshot(selection)
            };
            SendEventToRust("unity-editor-update", JsonUtility.ToJson(payload));
        }

        private static EditorSelectionSnapshot BuildEditorSelectionSnapshot(UnityEngine.Object selection)
        {
            if (selection == null)
            {
                return new EditorSelectionSnapshot
                {
                    kind = "none",
                    name = "",
                    type = "",
                    path = "",
                    instanceId = 0
                };
            }

            string path = AssetDatabase.GetAssetPath(selection) ?? "";
            return new EditorSelectionSnapshot
            {
                kind = EditorSelectionKind(selection, path),
                name = selection.name ?? "",
                type = selection.GetType().FullName ?? selection.GetType().Name,
                path = path,
                instanceId = selection.GetInstanceID()
            };
        }

        private static string EditorSelectionKind(UnityEngine.Object selection, string path)
        {
            if (selection is Material)
                return "material";
            if (selection is GameObject)
                return "gameObject";
            if (selection is Component)
                return "component";
            if (!string.IsNullOrEmpty(path))
                return "asset";
            return "object";
        }

        // ───────────────── Pipe server loop ─────────────────

        /// <summary>
        /// </summary>
        private static async Task ServerLoop(CancellationToken ct)
        {
            while (!ct.IsCancellationRequested)
            {
                NamedPipeServerStream server = null;

                try
                {
                    server = new NamedPipeServerStream(
                        PipeName,
                        PipeDirection.InOut,
                        1,
                        PipeTransmissionMode.Byte,
                        ServerPipeOptions,
                        PipeBufferSize,
                        PipeBufferSize
                    );

#if UNITY_2020
                    WaitForConnectionCompat(server, ct);
#else
                    await server.WaitForConnectionAsync(ct);
#endif
                    Debug.Log("[Locus] Pipe client connected: " + PipeName);

                    await HandleConnectionAsync(server, ct);
                }
                catch (OperationCanceledException)
                {
                    try { if (server != null) server.Dispose(); } catch { }
                    break;
                }
                catch (Exception ex)
                {
                    try { if (server != null) server.Dispose(); } catch { }
                    Debug.LogError("[Locus] Bridge error: " + ex);

                    try
                    {
                        await Task.Delay(500, ct);
                    }
                    catch (OperationCanceledException)
                    {
                        break;
                    }
                }
            }
        }

        // Unity 2020's Mono runtime exposes WaitForConnectionAsync(CancellationToken) but throws NotImplementedException.
#if UNITY_2020
        private static void WaitForConnectionCompat(NamedPipeServerStream server, CancellationToken ct)
        {
            if (ct.IsCancellationRequested)
                throw new OperationCanceledException(ct);

            using (ct.Register(delegate
            {
                try { server.Dispose(); } catch { }
            }))
            {
                try
                {
                    server.WaitForConnection();
                }
                catch (ObjectDisposedException)
                {
                    if (ct.IsCancellationRequested)
                        throw new OperationCanceledException(ct);
                    throw;
                }
                catch (IOException)
                {
                    if (ct.IsCancellationRequested)
                        throw new OperationCanceledException(ct);
                    throw;
                }
            }

            if (ct.IsCancellationRequested)
                throw new OperationCanceledException(ct);
        }
#endif

        // ───────────────── Connection handling ─────────────────

        /// <summary>
        /// </summary>
        private static async Task HandleConnectionAsync(NamedPipeServerStream server, CancellationToken ct)
        {
            try
            {
                using (server)
                using (var reader = new StreamReader(server, Utf8NoBom, false, TextReaderWriterBufferSize, true))
                using (var writer = new StreamWriter(server, Utf8NoBom, TextReaderWriterBufferSize, true) { AutoFlush = false })
                {
                    lock (_connectionLock)
                    {
                        _currentServer = server;
                        _currentWriter = writer;
                    }

                    await SendEnvelopeAsync(new PipeEnvelope
                    {
                        type = "unity_connected",
                        message = "connected",
                        processId = _editorProcessId,
                        processPath = _editorProcessPath
                    });

                    lock (_connectionLock)
                    {
                        if (ReferenceEquals(_currentServer, server))
                            _desktopPipeConnected = true;
                    }

                    while (!ct.IsCancellationRequested)
                    {
                        string line = await reader.ReadLineAsync();
                        if (line == null)
                            break;

                        if (string.IsNullOrWhiteSpace(line))
                            continue;

                        string captured = line;
                        _ = ProcessIncomingLineAsync(captured);
                    }

                    lock (_connectionLock)
                    {
                        _currentWriter = null;
                        _currentServer = null;
                        _desktopPipeConnected = false;
                    }
                    await _writeLock.WaitAsync();
                    _writeLock.Release();
                }
            }
            catch (IOException)
            {
            }
            catch (ObjectDisposedException)
            {
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Bridge connection error: " + ex);
            }
            finally
            {
                bool wasCurrentConnection;
                lock (_connectionLock)
                {
                    wasCurrentConnection = ReferenceEquals(_currentServer, server);
                    if (wasCurrentConnection)
                    {
                        _currentWriter = null;
                        _currentServer = null;
                        _desktopPipeConnected = false;
                    }
                }

                // Only the still-current connection may cancel the active
                // execute. Mono can deliver this finally LATE — after the
                // client already reconnected and submitted a new request on
                // the next connection — and an unconditional cancel here
                // kills that fresh request, which re-triggers the client's
                // no-progress reconnect: a self-sustaining cancel loop.
                if (wasCurrentConnection)
                    CancelActiveExecuteCode("pipe disconnected");
                Debug.Log("[Locus] Pipe client disconnected: " + PipeName);
            }
        }

        private static async Task ProcessIncomingLineAsync(string json)
        {
            PipeEnvelope request = null;

            try
            {
                request = JsonUtility.FromJson<PipeEnvelope>(json);
            }
            catch (Exception ex)
            {
                string msg = "[Locus] Invalid JSON from client: " + ex.Message + " | raw=" + json;
                PostToMainThread(delegate { Debug.LogWarning(msg); });
                return;
            }

            if (request == null || string.IsNullOrEmpty(request.type))
            {
                string msg = "[Locus] Invalid message envelope: " + json;
                PostToMainThread(delegate { Debug.LogWarning(msg); });
                return;
            }

            bool traceExecuteLoaded = string.Equals(request.type, "execute_loaded", StringComparison.Ordinal);
            if (traceExecuteLoaded)
            {
                Debug.Log(
                    "[Locus] pipe received execute_loaded id=" +
                    (request.id ?? "?") +
                    ", line chars=" +
                    json.Length);
            }

            PipeEnvelope response = await HandleMessageAsync(request);

            if (response != null && !string.IsNullOrEmpty(response.reply_to))
            {
                if (traceExecuteLoaded)
                {
                    Debug.Log(
                        "[Locus] pipe sending execute_loaded response id=" +
                        response.reply_to +
                        ", ok=" +
                        response.ok +
                        ", message chars=" +
                        (response.message == null ? 0 : response.message.Length) +
                        ", error chars=" +
                        (response.error == null ? 0 : response.error.Length));
                }

                bool sent = await SendEnvelopeAsync(response);
                if (traceExecuteLoaded)
                {
                    Debug.Log(
                        "[Locus] pipe execute_loaded response sent id=" +
                        response.reply_to +
                        ", sent=" +
                        sent);
                }
            }
        }

        // ───────────────── Outbound messaging ─────────────────

        /// <summary>
        /// </summary>
        public static void SendEventToRust(string eventType, string message)
        {
            _ = SendEnvelopeAsync(new PipeEnvelope
            {
                type = eventType,
                message = message
            });
        }

        /// <summary>
        /// </summary>
        private static async Task<bool> SendEnvelopeAsync(PipeEnvelope env)
        {
            StreamWriter writer;
            NamedPipeServerStream server;

            lock (_connectionLock)
            {
                writer = _currentWriter;
                server = _currentServer;
            }

            if (writer == null)
                return false;

            string json;
            try
            {
                json = JsonUtility.ToJson(env);
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Failed to serialize envelope: " + ex);
                return false;
            }

            await _writeLock.WaitAsync();
            try
            {
                await writer.WriteLineAsync(json);
                await writer.FlushAsync();
                return true;
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to write to pipe: " + ex.Message);

                // A broken write means the client end is gone, but on Mono
                // the pending ReadLineAsync of this connection may take
                // seconds to notice — during which the single pipe instance
                // stays occupied and reconnect attempts fail with
                // ERROR_PIPE_BUSY. Dispose the dead server so its reader
                // unwinds and the listen loop can accept again immediately.
                if (server != null && (ex is IOException || ex is ObjectDisposedException))
                {
                    try { server.Dispose(); } catch { }
                }
                return false;
            }
            finally
            {
                _writeLock.Release();
            }
        }

        // ───────────────── Response helpers ─────────────────

        private static PipeEnvelope OkResponse(string replyTo, string message)
        {
            return new PipeEnvelope
            {
                type = "response",
                reply_to = replyTo,
                ok = true,
                message = message
            };
        }

        private static PipeEnvelope OkStatusResponse(string replyTo)
        {
            PipeEnvelope response = OkResponse(replyTo, BuildCachedEditorStatusMessage());
            response.processId = _editorProcessId;
            response.processPath = _editorProcessPath;
            return response;
        }

        private static PipeEnvelope OkResponse(string replyTo)
        {
            return OkResponse(replyTo, null);
        }

        private static PipeEnvelope ErrorResponse(string replyTo, string error)
        {
            return new PipeEnvelope
            {
                type = "response",
                reply_to = replyTo,
                ok = false,
                error = error
            };
        }

        private static SelectAssetRequest ParseSelectAssetRequest(string message)
        {
            string payload = (message ?? "").Trim();
            if (payload.StartsWith("{", StringComparison.Ordinal))
            {
                try
                {
                    SelectAssetRequest request = JsonUtility.FromJson<SelectAssetRequest>(payload);
                    if (request != null && !string.IsNullOrEmpty(request.assetPath))
                    {
                        request.assetPath = request.assetPath.Trim().Replace('\\', '/');
                        return request;
                    }
                }
                catch (Exception ex)
                {
                    Debug.LogWarning("[Locus] Failed to parse select_asset payload: " + ex.Message);
                }
            }

            return new SelectAssetRequest
            {
                assetPath = payload.Replace('\\', '/'),
                focusProjectWindow = true
            };
        }

        private static SceneObjectRequest ParseSceneObjectRequest(string message)
        {
            string payload = (message ?? "").Trim();
            if (!payload.StartsWith("{", StringComparison.Ordinal))
            {
                string normalized = payload.Replace('\\', '/');
                int marker = normalized.IndexOf(".unity/", StringComparison.OrdinalIgnoreCase);
                if (marker >= 0)
                {
                    int split = marker + ".unity".Length;
                    return new SceneObjectRequest
                    {
                        scenePath = normalized.Substring(0, split),
                        objectPath = normalized.Substring(split + 1)
                    };
                }

                return new SceneObjectRequest();
            }

            try
            {
                SceneObjectRequest request = JsonUtility.FromJson<SceneObjectRequest>(payload);
                if (request != null)
                {
                    request.scenePath = (request.scenePath ?? "").Trim().Replace('\\', '/');
                    request.objectPath = (request.objectPath ?? "").Trim().Replace('\\', '/').Trim('/');
                    return request;
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to parse scene object payload: " + ex.Message);
            }

            return new SceneObjectRequest();
        }

        private static StartAssetDragRequest ParseStartAssetDragRequest(string message)
        {
            string payload = (message ?? "").Trim();
            if (payload.StartsWith("{", StringComparison.Ordinal))
            {
                try
                {
                    StartAssetDragRequest request = JsonUtility.FromJson<StartAssetDragRequest>(payload);
                    if (request != null)
                    {
                        NormalizeStartAssetDragRefs(request.refs);
                        return request;
                    }
                }
                catch (Exception ex)
                {
                    Debug.LogWarning("[Locus] Failed to parse start_asset_drag payload: " + ex.Message);
                }
            }

            return new StartAssetDragRequest
            {
                refs = new LocusEditorWindow.DroppedAssetRef[0]
            };
        }

        private static void NormalizeStartAssetDragRefs(LocusEditorWindow.DroppedAssetRef[] refs)
        {
            if (refs == null)
                return;

            foreach (LocusEditorWindow.DroppedAssetRef assetRef in refs)
            {
                if (assetRef == null)
                    continue;
                assetRef.path = (assetRef.path ?? "").Trim().Replace('\\', '/').TrimEnd('/');
                assetRef.kind = (assetRef.kind ?? "").Trim();
                assetRef.name = (assetRef.name ?? "").Trim();
                assetRef.typeLabel = (assetRef.typeLabel ?? "").Trim();
                assetRef.source = (assetRef.source ?? "").Trim();
            }
        }

        // ───────────────── Message dispatch ─────────────────

        /// <summary>
        /// </summary>
        private static async Task<PipeEnvelope> HandleMessageAsync(PipeEnvelope msg)
        {
            string reqId = msg.id;

            try
            {
                switch (msg.type)
                {
                    case "log":
                    {
                        string logMsg = msg.message ?? "";
                        PostToMainThread(delegate { Debug.Log("[Locus Agent] " + logMsg); });
                        return OkResponse(reqId);
                    }

                    case "warn":
                    {
                        string warnMsg = msg.message ?? "";
                        PostToMainThread(delegate { Debug.LogWarning("[Locus Agent] " + warnMsg); });
                        return OkResponse(reqId);
                    }

                    case "error":
                    {
                        string errMsg = msg.message ?? "";
                        PostToMainThread(delegate { Debug.LogError("[Locus Agent] " + errMsg); });
                        return OkResponse(reqId);
                    }

                    case "ping":
                        return OkResponse(reqId, "pong");

                    case "status":
                        return await HandleStatus(reqId);

                    case "get_console_text":
                    {
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(reqId, BuildConsoleTextPayloadJson()));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "exit_play_mode":
                    {
                        if (!_isPlaying)
                            return OkResponse(reqId, "already_editing");

                        PostToMainThread(delegate
                        {
                            EditorApplication.isPlaying = false;
                        });

                        return OkResponse(reqId, "exit_play_mode_requested");
                    }

                    case "set_editor_status":
                        return await HandleSetEditorStatus(reqId, msg.message);

                    case "execute_code":
                        return await HandleExecuteCode(reqId, msg.message);

                    case "execute_loaded":
                        return await HandleExecuteLoaded(reqId, msg.message);

                    case "cancel_execute_code":
                        return HandleCancelExecuteCode(reqId);

                    case "execute_code_progress":
                        TouchActiveExecuteCodeClientHeartbeat();
                        return OkResponse(reqId, GetExecuteCodeProgressJson());

                    case "export_type_index":
                    {
                        // Pure reflection + string building (no Unity API,
                        // and JsonUtility is thread-safe for plain types):
                        // run directly on the pipe worker so a busy or
                        // import-blocked main thread cannot stall the export.
                        try
                        {
                            return OkResponse(reqId, ExportTypeIndexJson());
                        }
                        catch (Exception ex)
                        {
                            return ErrorResponse(reqId, ex.ToString());
                        }
                    }

                    case "export_type_index_fingerprint":
                    {
                        try
                        {
                            return OkResponse(reqId, ExportTypeIndexFingerprintJson());
                        }
                        catch (Exception ex)
                        {
                            return ErrorResponse(reqId, ex.ToString());
                        }
                    }

                    case "get_compile_params":
                        return await HandleGetCompileParams(reqId, msg.message);

                    case "hot_reload_probe":
                        return await HandleHotReloadProbe(reqId);

                    case "hot_patch_loaded":
                        return await HandleHotPatchLoaded(reqId, msg.message);

                    case "hot_patch_dispose":
                        return await HandleHotPatchDispose(reqId, msg.message);

                    case "run_states":
                        return await HandleRunStates(reqId, msg.message);

                    case "run_states_loaded":
                        return await HandleRunStatesLoaded(reqId, msg.message);

                    case "compile_run_states":
                        return await HandleCompileRunStates(reqId, msg.message);

                    case "compile_named":
                        return await HandleCompileNamed(reqId, msg.message);

                    case "compile_skill_package":
                        return await HandleCompileSkillPackage(reqId, msg.message);

                    case "invoke_skill_package":
                        return await HandleInvokeSkillPackage(reqId, msg.message);

                    case "invoke_named":
                        return await HandleInvokeNamed(reqId, msg.message);

                    case "invoke_named_cached":
                        return await HandleInvokeNamedCached(reqId, msg.message);

                    case "view_binding_read":
                        return await HandleViewBindingRead(reqId, msg.message);

                    case "view_binding_write":
                        return await HandleViewBindingWrite(reqId, msg.message);

                    case "view_binding_apply":
                        return await HandleViewBindingApply(reqId, msg.message);

                    case "view_binding_discover":
                        return await HandleViewBindingDiscover(reqId, msg.message);

                    case "capture_viewport":
                        return await HandleCaptureViewport(reqId, msg.message);

                    case "request_recompile":
                    {
                        // Hot-reload sessions write/delete files without telling
                        // the AssetDatabase; the desktop forwards every tracked
                        // dirty path here so created files import and deleted
                        // ones refresh away before the compile. Older callers
                        // send an empty message — unchanged behavior.
                        string changedPathsRaw = msg.message ?? "";
                        PostToMainThread(delegate
                        {
                            ReleaseAllEditSessions();
                            lock (_recompileErrorsLock) { _recompileErrors.Clear(); }
                            ClearCompileResult();
                            SetCompileResult("pending");
                            _recompileRequested = true;

                            SessionState.SetBool(SessionKey_RecompileInProgress, true);
                            if (changedPathsRaw.Length > 0)
                                QueueChangedAssets(changedPathsRaw.Split('\n'));
                            FlushQueuedAssetImports();
                            _domainReloadCheckFrames = -1;
                            CompilationPipeline.RequestScriptCompilation();

                            _recompileCheckFrames = 0;
                        });

                        return OkResponse(reqId, "recompile_started");
                    }

                    case "begin_edit_session":
                    {
                        string owner = msg.message ?? "";
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(reqId, BeginEditSession(owner)));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "end_edit_session":
                    {
                        string owner = msg.message ?? "";
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(reqId, EndEditSession(owner)));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "import_assets":
                    {
                        string paths = msg.message ?? "";
                        var lines = paths.Split(new[] { '\n' }, System.StringSplitOptions.RemoveEmptyEntries);
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                QueueChangedAssets(lines);

                                if (_activeEditSessionOwners.Count == 0)
                                    FlushQueuedAssetImports();

                                tcs.SetResult(OkResponse(reqId, lines.Length + " assets queued"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "get_compile_result":
                    {
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                string result = GetCompileResult();
                                if (string.IsNullOrEmpty(result) ||
                                    string.Equals(result, "pending", StringComparison.Ordinal) ||
                                    string.Equals(result, "awaiting_reload", StringComparison.Ordinal))
                                {
                                    tcs.SetResult(OkResponse(reqId, "pending"));
                                    return;
                                }

                                ClearCompileResult();

                                if (result.StartsWith("error:", StringComparison.Ordinal))
                                    tcs.SetResult(ErrorResponse(reqId, result.Substring("error:".Length)));
                                else
                                    tcs.SetResult(OkResponse(reqId, result));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "select_asset":
                    {
                        SelectAssetRequest request = ParseSelectAssetRequest(msg.message);
                        PostToMainThread(delegate
                        {
                            var obj = AssetDatabase.LoadMainAssetAtPath(request.assetPath);
                            if (obj != null)
                            {
                                Selection.activeObject = obj;
                                if (request.focusProjectWindow)
                                {
                                    EditorGUIUtility.PingObject(obj);
                                    EditorUtility.FocusProjectWindow();
                                }
                            }
                        });
                        return OkResponse(reqId);
                    }

                    case "open_asset_inspector":
                    {
                        SelectAssetRequest request = ParseSelectAssetRequest(msg.message);
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                LocusAssetInspectorUtility.OpenLockedInspector(request.assetPath);
                                tcs.SetResult(OkResponse(reqId, "ok"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "asset_thumbnail":
                        return await HandleAssetThumbnail(reqId, msg.message);

                    case "asset_preview_render":
                        return await HandleAssetPreviewRender(reqId, msg.message);

                    case "select_scene_object":
                    {
                        SceneObjectRequest request = ParseSceneObjectRequest(msg.message);
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                LocusSceneObjectUtility.SelectSceneObject(request.scenePath, request.objectPath);
                                tcs.SetResult(OkResponse(reqId, "ok"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "open_scene_object_inspector":
                    {
                        SceneObjectRequest request = ParseSceneObjectRequest(msg.message);
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                LocusSceneObjectUtility.OpenSceneObjectInspector(request.scenePath, request.objectPath);
                                tcs.SetResult(OkResponse(reqId, "ok"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "start_asset_drag":
                    {
                        StartAssetDragRequest request = ParseStartAssetDragRequest(msg.message);
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                string status;
                                if (LocusEditorWindow.QueueOutboundAssetDrag(request.refs, out status))
                                    tcs.SetResult(OkResponse(reqId, status));
                                else
                                    tcs.SetResult(ErrorResponse(reqId, status));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "cancel_asset_drag":
                    {
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                LocusEditorWindow.CancelOutboundAssetDrag();
                                tcs.SetResult(OkResponse(reqId, "cancelled"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "open_frontend_window":
                    {
                        var tcs = new TaskCompletionSource<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                string status = LocusEditorWindow.OpenFrontendWindowFromJson(msg.message);
                                tcs.SetResult(OkResponse(reqId, status));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task;
                    }

                    case "list_yaml":
                        return await HandleListYaml(reqId, msg.message);

                    case "search_yaml":
                        return await HandleSearchYaml(reqId, msg.message);

                    case "read_yaml":
                        return await HandleReadYaml(reqId, msg.message);

                    case "reload_open_scenes":
                    {
                        TaskCompletionSource<string> tcs = new TaskCompletionSource<string>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                var scenePaths = new List<string>();
                                for (int i = 0; i < UnityEditor.SceneManagement.EditorSceneManager.sceneCount; i++)
                                    scenePaths.Add(UnityEditor.SceneManagement.EditorSceneManager.GetSceneAt(i).path);

                                if (scenePaths.Count > 0)
                                {
                                    UnityEditor.SceneManagement.EditorSceneManager.OpenScene(scenePaths[0], UnityEditor.SceneManagement.OpenSceneMode.Single);
                                    for (int i = 1; i < scenePaths.Count; i++)
                                        UnityEditor.SceneManagement.EditorSceneManager.OpenScene(scenePaths[i], UnityEditor.SceneManagement.OpenSceneMode.Additive);
                                }
                                tcs.TrySetResult("reloaded:" + scenePaths.Count);
                            }
                            catch (Exception ex)
                            {
                                tcs.TrySetResult("error:" + ex.Message);
                            }
                        });
                        string result = await tcs.Task;
                        return OkResponse(reqId, result);
                    }

                    default:
                        return ErrorResponse(reqId, "unknown message type: " + (msg.type ?? ""));
                }
            }
            catch (Exception ex)
            {
                PostToMainThread(delegate { Debug.LogError("[Locus] HandleMessage exception for type '" + (msg.type ?? "null") + "': " + ex); });
                return ErrorResponse(reqId, ex.ToString());
            }
        }

        private static async Task<PipeEnvelope> HandleStatus(string requestId)
        {
            var tcs = new TaskCompletionSource<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    RefreshCachedEditorState();
                    tcs.SetResult(OkStatusResponse(requestId));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.ToString()));
                }
            });
            return await tcs.Task;
        }

        private static string BuildCachedEditorStatusMessage()
        {
            string status = _isPlaying
                ? (_isPaused ? "playing_paused" : "playing")
                : "editing";
            string scenePath = _activeScenePath;
            if (!string.IsNullOrEmpty(scenePath))
                status += "|" + scenePath;
            return status;
        }
    }
}
