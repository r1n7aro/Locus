// 2026-03-28 - Introduce Unity edit sessions via Auto Refresh suppression and persist recompile results across domain reloads

using UnityEngine;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEditor.SceneManagement;

using System;
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

        private static readonly UTF8Encoding Utf8NoBom = new UTF8Encoding(false);

        // ───────────────── Constants ─────────────────

        private const int ExecuteTimeoutMs = 30000;
        private const int PipeBufferSize = 64 * 1024;
        private const int TextReaderWriterBufferSize = 16 * 1024;
        private const int MaxMainThreadActionsPerUpdate = 32;

        // ───────────────── Main-thread dispatcher ─────────────────

        private static readonly object _mainThreadQueueLock = new object();
        private static readonly Queue<Action> _mainThreadQueue = new Queue<Action>(64);

        // ───────────────── Cached editor state (updated on main thread) ─────────────────

        private static volatile bool _isPlaying;
        private static volatile bool _isPaused;
        private static volatile string _activeScenePath = "";

        // ───────────────── Runtime compilation cache ─────────────────

        private static readonly object _compileCacheLock = new object();

        private static List<MetadataReference> _cachedMetadataReferences;
        private static bool _metadataReferencesReady;
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

        private static readonly CSharpParseOptions SnippetParseOptions =
            new CSharpParseOptions(
                kind: SourceCodeKind.Regular,
                documentationMode: DocumentationMode.None,
                languageVersion: LanguageVersion.CSharp9
            );

        private static readonly CSharpCompilationOptions SnippetCompilationOptions =
            new CSharpCompilationOptions(
                outputKind: OutputKind.DynamicallyLinkedLibrary,
                optimizationLevel: OptimizationLevel.Release,
                allowUnsafe: false,
                assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default
            );

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

        private static void InvalidateCompilationCaches()
        {
            lock (_compileCacheLock)
            {
                _metadataReferencesReady = false;
                _cachedMetadataReferences = null;
            }
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

            int importedCount = 0;
            foreach (string assetPath in pendingPaths)
            {
                try
                {
                    AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.ForceUpdate);
                    importedCount++;
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] Failed to import changed asset before compile: " + assetPath + "\n" + ex);
                }
            }

            if (importedCount > 0)
                Debug.Log("[Locus] Flushed changed asset imports before compile: " + importedCount);

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
            _isPlaying = EditorApplication.isPlaying;
            _isPaused = EditorApplication.isPaused;
            _activeScenePath = EditorSceneManager.GetActiveScene().path ?? "";

            PumpRunStates();
            PumpExecuteCodeAsyncRuntime();

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
                        PipeOptions.Asynchronous,
                        PipeBufferSize,
                        PipeBufferSize
                    );

                    await server.WaitForConnectionAsync(ct);
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
                        message = "connected"
                    });

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
                lock (_connectionLock)
                {
                    if (ReferenceEquals(_currentServer, server))
                    {
                        _currentWriter = null;
                        _currentServer = null;
                    }
                }

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

            PipeEnvelope response = await HandleMessageAsync(request);

            if (response != null && !string.IsNullOrEmpty(response.reply_to))
                await SendEnvelopeAsync(response);
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

            lock (_connectionLock)
            {
                writer = _currentWriter;
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
                    {
                        string status = _isPlaying
                            ? (_isPaused ? "playing_paused" : "playing")
                            : "editing";
                        string scenePath = _activeScenePath;
                        if (!string.IsNullOrEmpty(scenePath))
                            status += "|" + scenePath;
                        return OkResponse(reqId, status);
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

                    case "execute_code_progress":
                        return OkResponse(reqId, GetExecuteCodeProgressJson());

                    case "run_states":
                        return await HandleRunStates(reqId, msg.message);

                    case "compile_run_states":
                        return await HandleCompileRunStates(reqId, msg.message);

                    case "request_recompile":
                    {
                        PostToMainThread(delegate
                        {
                            ReleaseAllEditSessions();
                            lock (_recompileErrorsLock) { _recompileErrors.Clear(); }
                            ClearCompileResult();
                            SetCompileResult("pending");
                            _recompileRequested = true;

                            SessionState.SetBool(SessionKey_RecompileInProgress, true);
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

                    case "list_yaml":
                        return await HandleListYaml(reqId, msg.message);

                    case "search_yaml":
                        return await HandleSearchYaml(reqId, msg.message);

                    case "read_yaml":
                        return await HandleReadYaml(reqId, msg.message);

                    case "set_serialized_data":
                        return await HandleSetSerializedData(reqId, msg.message);

                    case "get_serialized_data":
                        return await HandleGetSerializedData(reqId, msg.message);

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
    }
}
