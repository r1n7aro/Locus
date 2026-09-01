// 2026-03-28 - Introduce Unity edit sessions via Auto Refresh suppression and persist recompile results across domain reloads

using UnityEngine;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEditor.SceneManagement;

using System;
using System.Globalization;
using System.IO;
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

        private static readonly SemaphoreSlim _executeCodeLock = new SemaphoreSlim(1, 1);
        private static readonly SemaphoreSlim _runStatesLock = new SemaphoreSlim(1, 1);

        private static readonly int _editorProcessId = ResolveCurrentProcessId();
        private static readonly string _editorProcessPath = ResolveCurrentProcessPath();
        private static readonly bool _isUnityWorkerProcess = DetectUnityWorkerProcess();

        private static readonly UTF8Encoding Utf8NoBom = new UTF8Encoding(false);

        private static byte[] ReadAssemblyPayload(string assemblyB64, string assemblyPath)
        {
            if (!string.IsNullOrEmpty(assemblyPath))
                return File.ReadAllBytes(assemblyPath);
            return Convert.FromBase64String(assemblyB64);
        }

        // ───────────────── Constants ─────────────────

        private const int ExecuteTimeoutMs = 30000;
        private const int MaxMainThreadActionsPerUpdate = 32;

        // ───────────────── Main-thread dispatcher ─────────────────

        private static readonly object _mainThreadQueueLock = new object();
        private static readonly Queue<Action> _mainThreadQueue = new Queue<Action>(64);

        // ───────────────── Cached editor state (updated on main thread) ─────────────────

        private static volatile bool _isPlaying;
        private static volatile bool _isPaused;
        private static volatile string _activeScenePath = "";
        private static volatile string _additionalOpenScenePathsStatus = "";
        private static int _editorUpdateEventSequence;
        private static double _lastEditorUpdateEventAt = -1.0;
        private static int _lastEditorUpdateSelectionInstanceId = int.MinValue;
        private const double EditorUpdateEventIntervalSeconds = 0.25;

        // Reused across the 0.25s editor-update tick so the idle steady state does
        // not allocate a fresh payload/snapshot on every send. Written only on the
        // main thread (PumpMainThreadQueue), so no synchronization is needed.
        private static readonly EditorSelectionSnapshot _reusableSelectionSnapshot = new EditorSelectionSnapshot();
        private static readonly EditorUpdatePayload _reusableEditorUpdatePayload = new EditorUpdatePayload();
        private static readonly Dictionary<Type, string> _typeFullNameCache = new Dictionary<Type, string>();
        private static int _cachedSelectionPathInstanceId = int.MinValue;
        private static string _cachedSelectionPath = "";

        // ───────────────── Runtime compilation cache ─────────────────

        private static readonly object _compileCacheLock = new object();

        private static List<MetadataReference> _cachedMetadataReferences;
        private static bool _metadataReferencesReady;
        private static List<string> _cachedCompileReferencePaths;
        private static bool _compileReferencePathsReady;
        private static string _cachedCompileParamsFingerprint;
        private static bool _compileParamsFingerprintReady;
        private static long _cachedCompileParamsFingerprintCheckedAtTicks;
        private const long CompileParamsFingerprintAuditIntervalTicks = TimeSpan.TicksPerSecond * 5L;
        /// <summary>Any project script assembly compiles with "Allow unsafe
        /// code" — hot patches follow it (B4). Cached together with the
        /// reference paths (same CompilationPipeline walk, same lifetime).</summary>
        private static bool _cachedCompileAllowUnsafe;
        private static int _snippetAssemblyCounter;

        // ───────────────── Agent-controlled recompile ─────────────────

        private const string SessionKey_RecompileInProgress = "Locus_RecompileInProgress";
        private const string SessionKey_RecompileResult = "Locus_RecompileResult";
        // Convergence signalling read by the desktop via get_reload_state.
        // CompileAwaitingReload: set by OnCompilationFinished on a SUCCESSFUL
        // compile (any initiator), consumed by OnAfterAssemblyReload in the next
        // domain to advance ConvergedSerial — so the serial only moves once the
        // newly compiled assemblies are actually LOADED (true convergence),
        // never while the old domain still runs the old code. A no-compile
        // domain reload (e.g. entering play mode) leaves the serial untouched.
        // EditorSessionId is a per-process id (reset on restart) so the desktop
        // converges against a fresh editor instance even if it never observed
        // the old one exit.
        private const string SessionKey_CompileAwaitingReload = "Locus_CompileAwaitingReload";
        private const string SessionKey_ConvergedSerial = "Locus_ConvergedSerial";
        private const string SessionKey_EditorSessionId = "Locus_EditorSessionId";
        // Set when request_recompile issues a compilation, cleared by the first
        // OnCompilationFinished whose compilation STARTED at or after the
        // request (see the epoch pair below). While true, a domain reload is
        // loading an EARLIER compile's assemblies — not the requested one's — so
        // OnAfterAssemblyReload must not advance ConvergedSerial or complete the
        // request: a stale reload would otherwise converge edits that belong to
        // the still-running requested compile, reporting them applied before they
        // are loaded.
        private const string SessionKey_RecompilePendingCompile = "Locus_RecompilePendingCompile";
        // Monotonic count of compilations that ever STARTED this editor process
        // (SessionState: survives domain reloads, resets with the process).
        // RecompileTargetEpoch is stamped by request_recompile with the epoch
        // its compilation will carry (started + 1 at request time): a compile
        // finishing while started < target predates the request — a user
        // Ctrl+R already in flight when the request arrived — and its finish
        // must neither clear RecompilePendingCompile nor complete the request,
        // or its reload converges edits that only the queued follow-up compile
        // will build (the desktop would report them applied prematurely).
        private const string SessionKey_CompileEpochStarted = "Locus_CompileEpochStarted";
        private const string SessionKey_RecompileTargetEpoch = "Locus_RecompileTargetEpoch";

        private static volatile bool _recompileRequested;
        private static volatile string _lastCompileResult;
        private static TaskCompletionSource<PipeEnvelope> _recompileStartCompletion;
        private static string _recompileStartRequestId;
        private static bool _recompileStartWatchActive;
        private static double _recompileStartIdleSince = -1;
        // RequestScriptCompilation can remain queued behind a long asset import
        // (the Editor shows "Hold on") even after the request reaches the main
        // thread. Count only continuously idle editor time and leave enough room
        // for Unity to publish CompilationPipeline.compilationStarted.
        private const double RecompileStartIdleTimeoutSeconds = 30.0;
        // Unity's Bee build graph reports every output assembly through exactly
        // one of assemblyCompilationFinished (rebuilt) or
        // assemblyCompilationNotRequired (already up to date). These counters
        // let compilationFinished distinguish a real compile from an
        // incremental no-op without inferring from elapsed idle time.
        private static int _compiledAssemblyCount;
        private static int _upToDateAssemblyCount;
        private static readonly HashSet<string> _activeEditSessionOwners =
            new HashSet<string>(StringComparer.Ordinal);
        private static readonly HashSet<string> _pendingChangedAssetPaths =
            new HashSet<string>(StringComparer.Ordinal);
        private static int _autoRefreshSuppressionCount;

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

        // ConcurrentBuild is disabled on purpose: the in-process Roslyn fallback
        // must compile single-threaded. Roslyn's parallel build spins worker
        // threads that, if still in flight when a synchronous domain reload
        // begins (e.g. entering Play Mode), are not aborted and deadlock the
        // editor inside mono_domain_try_unload ("Begin MonoManager ReloadAssembly"
        // with 0% CPU). Single-threaded compiles run on the request's own worker
        // thread and are drained by CancelAndDrainInProcessCompiles on reload.
        private static readonly CSharpCompilationOptions SnippetCompilationOptions =
            new CSharpCompilationOptions(
                outputKind: OutputKind.DynamicallyLinkedLibrary,
                optimizationLevel: OptimizationLevel.Release,
                allowUnsafe: false,
                assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default
            ).WithConcurrentBuild(false);

        // ───────────────── In-process compile reload guard ─────────────────
        // Tracks in-flight in-process Roslyn compiles (execute_code / run_states /
        // compile_named / compile_skill_package fallbacks) so a domain reload can
        // cancel them and bounded-join before the domain unloads. Without this an
        // in-flight compile thread survives into mono_domain_try_unload and the
        // reload deadlocks forever.
        private const int InProcessCompileDrainTimeoutMs = 5000;
        private static readonly object _inProcessCompileGuardLock = new object();
        private static int _inProcessCompileActive;
        private static CancellationTokenSource _inProcessCompileReloadCts = new CancellationTokenSource();

        /// <summary>
        /// Token that trips when a domain reload begins. In-process compiles pass
        /// it to Roslyn's Emit so a reload interrupts a compile in progress.
        /// </summary>
        private static CancellationToken InProcessCompileReloadToken
        {
            get
            {
                lock (_inProcessCompileGuardLock)
                {
                    return _inProcessCompileReloadCts.Token;
                }
            }
        }

        private readonly struct InProcessCompileScope : IDisposable
        {
            public void Dispose()
            {
                Interlocked.Decrement(ref _inProcessCompileActive);
            }
        }

        /// <summary>
        /// Mark an in-process Roslyn compile as active for the lifetime of the
        /// returned scope. Wrap the CSharpCompilation.Emit call so a domain reload
        /// can wait for it to finish.
        /// </summary>
        private static InProcessCompileScope EnterInProcessCompile()
        {
            Interlocked.Increment(ref _inProcessCompileActive);
            return default(InProcessCompileScope);
        }

        /// <summary>
        /// Cancel any in-flight in-process Roslyn compiles and bounded-wait for
        /// them to unwind so none survive into mono_domain_try_unload. Called on
        /// the main thread from OnBeforeAssemblyReload; the compiles run on worker
        /// threads, observe the cancelled token, and finish quickly because they
        /// are single-threaded (see SnippetCompilationOptions).
        /// </summary>
        private static void CancelAndDrainInProcessCompiles(int timeoutMs)
        {
            CancellationTokenSource cts;
            lock (_inProcessCompileGuardLock)
            {
                cts = _inProcessCompileReloadCts;
            }

            try
            {
                cts.Cancel();
            }
            catch
            {
            }

            if (Volatile.Read(ref _inProcessCompileActive) > 0)
            {
                var sw = System.Diagnostics.Stopwatch.StartNew();
                while (Volatile.Read(ref _inProcessCompileActive) > 0 && sw.ElapsedMilliseconds < timeoutMs)
                    Thread.Sleep(10);

                int remaining = Volatile.Read(ref _inProcessCompileActive);
                if (remaining > 0)
                {
                    Debug.LogWarning(
                        "[Locus] in-process compile did not drain before assembly reload (active=" +
                        remaining + "); proceeding with reload.");
                }
            }

            // Fresh token for the next reload cycle (matters only when a reload is
            // aborted; a real reload resets these statics in the new domain).
            lock (_inProcessCompileGuardLock)
            {
                try
                {
                    _inProcessCompileReloadCts.Dispose();
                }
                catch
                {
                }
                _inProcessCompileReloadCts = new CancellationTokenSource();
            }
        }

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
#if UNITY_2021_2_OR_NEWER
                string raw = PlayerSettings.GetScriptingDefineSymbols(
                    UnityEditor.Build.NamedBuildTarget.FromBuildTargetGroup(group));
#else
                string raw = PlayerSettings.GetScriptingDefineSymbolsForGroup(group);
#endif
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
            if (_isUnityWorkerProcess)
            {
                NativeShutdownInWorkerProcess();
                return;
            }

            // Keep the bridge alive across edit sessions. Auto Refresh is only suppressed while a session is active.
            // The static ctor (InitializeOnLoad) runs on the Unity main thread; record it so the
            // LocusAsync.SwitchTo* primitives can tell which thread they are on.
            LocusAsync.CaptureMainThread();
            RefreshCachedEditorState();
            EditorApplication.update += PumpMainThreadQueue;
            EditorApplication.playModeStateChanged += OnPlayModeStateChanged;
            EditorApplication.pauseStateChanged += OnPauseStateChanged;
            EditorApplication.delayCall += Start;
            EditorApplication.quitting += OnQuitting;
            AssemblyReloadEvents.beforeAssemblyReload += OnBeforeAssemblyReload;
            AssemblyReloadEvents.afterAssemblyReload += OnAfterAssemblyReload;
            CompilationPipeline.compilationStarted += OnCompilationStarted;
            CompilationPipeline.compilationFinished += OnCompilationFinished;
#if UNITY_2022_1_OR_NEWER
            CompilationPipeline.assemblyCompilationNotRequired += OnAssemblyCompilationNotRequired;
#endif
            CompilationPipeline.assemblyCompilationFinished += OnAssemblyCompilationFinished;
            RestorePendingRecompileStartAfterReload();
            RegisterExtensionMessageHandler(
                "yaml_preview_cache_selftest",
                HandleYamlPreviewCacheSelfTest);
        }

        [Serializable]
        private sealed class RecompileRequestPayload
        {
            public int schema;
            public string syncMode;
            public string[] paths;
            public string reason;
        }

        private static void OnQuitting()
        {
            ClearYamlPreviewSceneCache();
            ReleaseAllEditSessions();
            NativeOnQuitting();
            Stop();
        }

        private static void OnPlayModeStateChanged(PlayModeStateChange state)
        {
            ClearYamlPreviewSceneCache();
            TickSchedulerOnPlayModeStateChanged(state);
            RefreshCachedEditorState();
            NativePublishEditorStatusNow();
        }

        private static void OnPauseStateChanged(PauseState state)
        {
            RefreshCachedEditorState();
            NativePublishEditorStatusNow();
        }

        private static void OnBeforeAssemblyReload()
        {
            ClearYamlPreviewSceneCache();
            RefreshCachedEditorState();
            // Cancel + bounded-join any in-flight in-process Roslyn compile before
            // the domain unloads, otherwise its worker thread deadlocks the reload
            // inside mono_domain_try_unload.
            CancelAndDrainInProcessCompiles(InProcessCompileDrainTimeoutMs);
            NativeOnBeforeReload();
            Stop();
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

        private static bool DetectUnityWorkerProcess()
        {
            try
            {
                string[] args = Environment.GetCommandLineArgs();
                for (int i = 0; i < args.Length; i++)
                {
                    string arg = args[i] ?? "";
                    if (arg.IndexOf("AssetImportWorker", StringComparison.OrdinalIgnoreCase) >= 0)
                        return true;
                }
            }
            catch
            {
            }
            return false;
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

            // Absolute paths are only trimmed when they actually live under
            // THIS project's root. Blindly cutting at the first "Assets/"
            // would alias D:/OtherProject/Assets/Scenes/Main.unity onto this
            // project's same-named scene and silently return wrong data.
            if (System.IO.Path.IsPathRooted(normalized))
            {
                string projectRoot = System.IO.Path
                    .GetDirectoryName(Application.dataPath)
                    ?.Replace('\\', '/');
                if (string.IsNullOrEmpty(projectRoot))
                    return null;
                if (!projectRoot.EndsWith("/", StringComparison.Ordinal))
                    projectRoot += "/";
                if (!normalized.StartsWith(projectRoot, StringComparison.OrdinalIgnoreCase))
                    return null;
                string relative = normalized.Substring(projectRoot.Length);
                return IsProjectAssetPath(relative) ? relative : null;
            }

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
            if (_isUnityWorkerProcess)
                return;

            NativeStartIfEnabled();
            if (!IsNativeBridgeActive)
                Debug.LogError("[Locus] Native broker bridge is required but did not start.");
        }

        public static void Stop()
        {
            CancelActiveExecuteCode("bridge stopped");
            ClearTickSchedulerNow();

            lock (_mainThreadQueueLock)
            {
                _mainThreadQueue.Clear();
            }
        }

        // ───────────────── Compilation events ─────────────────

        private static bool IsWaitingForRequestedCompilationStart()
        {
            if (!SessionState.GetBool(SessionKey_RecompileInProgress, false) ||
                !SessionState.GetBool(SessionKey_RecompilePendingCompile, false))
                return false;

            int targetEpoch = SessionState.GetInt(SessionKey_RecompileTargetEpoch, 0);
            return targetEpoch > 0 &&
                SessionState.GetInt(SessionKey_CompileEpochStarted, 0) < targetEpoch;
        }

        private static void CompleteRecompileStartResponse()
        {
            TaskCompletionSource<PipeEnvelope> completion = _recompileStartCompletion;
            string requestId = _recompileStartRequestId;
            _recompileStartCompletion = null;
            _recompileStartRequestId = null;

            if (completion != null && !string.IsNullOrEmpty(requestId))
                completion.TrySetResult(OkResponse(requestId, "recompile_started"));
        }

        private static void CompleteRecompileNotNeeded()
        {
            _recompileRequested = false;
            _recompileStartWatchActive = false;
            _recompileStartIdleSince = -1;
            _domainReloadCheckFrames = -1;
            SessionState.SetBool(SessionKey_RecompileInProgress, false);
            SessionState.SetBool(SessionKey_RecompilePendingCompile, false);
            SessionState.SetBool(SessionKey_CompileAwaitingReload, false);
            SetCompileResult("not_needed");
            Debug.Log("[Locus] Unity incremental build completed with no rebuilt assemblies ("
                + _upToDateAssemblyCount + " already up to date); no domain reload is required.");
        }

        private static void FailRecompileStart(string detail)
        {
            const string summary = "Unity 没有开始编译。";
            string message = string.IsNullOrEmpty(detail)
                ? summary
                : summary + " " + detail;

            _recompileRequested = false;
            _recompileStartWatchActive = false;
            _recompileStartIdleSince = -1;
            _domainReloadCheckFrames = -1;
            SessionState.SetBool(SessionKey_RecompileInProgress, false);
            SessionState.SetBool(SessionKey_RecompilePendingCompile, false);
            SessionState.SetBool(SessionKey_CompileAwaitingReload, false);
            SetCompileResult("error:" + message);

            TaskCompletionSource<PipeEnvelope> completion = _recompileStartCompletion;
            string requestId = _recompileStartRequestId;
            _recompileStartCompletion = null;
            _recompileStartRequestId = null;
            if (completion != null && !string.IsNullOrEmpty(requestId))
                completion.TrySetResult(ErrorResponse(requestId, message));

            Debug.LogWarning("[Locus] " + message);
        }

        private static void RestorePendingRecompileStartAfterReload()
        {
            if (!SessionState.GetBool(SessionKey_RecompileInProgress, false) ||
                !SessionState.GetBool(SessionKey_RecompilePendingCompile, false))
                return;

            int targetEpoch = SessionState.GetInt(SessionKey_RecompileTargetEpoch, 0);
            int startedEpoch = SessionState.GetInt(SessionKey_CompileEpochStarted, 0);
            if (targetEpoch <= 0)
            {
                FailRecompileStart("重编译请求缺少目标编译序号。");
                return;
            }

            if (startedEpoch >= targetEpoch)
            {
                FailRecompileStart("编译启动后的完成状态已中断。");
                return;
            }

            // An earlier compilation or unrelated reload destroyed the original
            // request handler before the requested compile started. Keep the
            // persisted request alive in the new domain and issue it again after
            // editor initialization finishes. Rust will reconnect and observe
            // "starting" until this reaches compilationStarted or fails.
            _recompileRequested = true;
            SetCompileResult("starting");
            _recompileStartWatchActive = true;
            _recompileStartIdleSince = -1;
            EditorApplication.delayCall += ResumePendingRecompileStart;
        }

        private static void ResumePendingRecompileStart()
        {
            if (!IsWaitingForRequestedCompilationStart())
                return;

            try
            {
                CompilationPipeline.RequestScriptCompilation();
                if (_recompileStartWatchActive && !EditorApplication.isCompiling)
                    _recompileStartIdleSince = EditorApplication.timeSinceStartup;
            }
            catch (Exception ex)
            {
                FailRecompileStart("请求编译失败：" + ex.Message);
            }
        }

        private static void OnCompilationStarted(object context)
        {
            _compiledAssemblyCount = 0;
            _upToDateAssemblyCount = 0;

            // Epoch for the request/finish pairing — see
            // SessionKey_CompileEpochStarted. SessionState survives the domain
            // reload between a pre-request compile and the requested one.
            int startedEpoch = SessionState.GetInt(SessionKey_CompileEpochStarted, 0) + 1;
            SessionState.SetInt(SessionKey_CompileEpochStarted, startedEpoch);

            int targetEpoch = SessionState.GetInt(SessionKey_RecompileTargetEpoch, 0);
            if (SessionState.GetBool(SessionKey_RecompileInProgress, false) &&
                targetEpoch > 0 && startedEpoch >= targetEpoch)
            {
                // This response is the start handshake: request_recompile does
                // not acknowledge success before Unity raises this event for the
                // requested epoch.
                _recompileStartWatchActive = false;
                _recompileStartIdleSince = -1;
                SetCompileResult("pending");
                CompleteRecompileStartResponse();
            }
        }

#if UNITY_2022_1_OR_NEWER
        private static void OnAssemblyCompilationNotRequired(string assemblyPath)
        {
            _upToDateAssemblyCount++;
        }
#endif

        private static void OnAssemblyCompilationFinished(string assemblyPath, CompilerMessage[] messages)
        {
            _compiledAssemblyCount++;
            // Collect errors for EVERY compilation, not just Locus-requested
            // ones, so OnCompilationFinished can tell a successful compile from
            // a failed one regardless of who triggered it (the convergence
            // serial must only advance on success). Reset each cycle in
            // OnCompilationFinished.
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

            // Whether THIS finish belongs to the requested compile (or a later
            // superset): a compile that STARTED before the request — a user
            // Ctrl+R already in flight when request_recompile arrived — finishes
            // with started < target and must not clear the in-flight marker nor
            // complete the request; the requested compile is still queued behind
            // it. With no request in play, target is stale-or-zero and this is
            // trivially true (any finish behaves as before).
            bool requestedCompileFinished =
                SessionState.GetInt(SessionKey_CompileEpochStarted, 0) >=
                SessionState.GetInt(SessionKey_RecompileTargetEpoch, 0);
            if (requestedCompileFinished)
            {
                // The requested compile (if any) has now finished: its reload is
                // the one allowed to converge. This frees OnAfterAssemblyReload
                // to converge again. Only NOW is the "no compilation started"
                // watchdog satisfied — a pre-request compile finishing must not
                // disarm it (the requested compile may still never start).
                SessionState.SetBool(SessionKey_RecompilePendingCompile, false);
                _recompileStartWatchActive = false;
                _recompileStartIdleSince = -1;
            }

            // Snapshot + reset the per-cycle error set (collected for every
            // compilation by OnAssemblyCompilationFinished) so success/failure
            // is known whoever triggered this compile.
            List<string> errors = null;
            lock (_recompileErrorsLock)
            {
                if (_recompileErrors.Count > 0)
                    errors = new List<string>(_recompileErrors);
                _recompileErrors.Clear();
            }
            bool succeeded = errors == null;

            // Initiator-agnostic convergence signal: a SUCCESSFUL compilation —
            // requested by Locus or triggered by Unity itself (manual Ctrl+R,
            // save, auto-refresh on focus, startup) — will make the loaded
            // assemblies reflect the current on-disk sources ONCE its domain
            // reload completes. Only flag it here; OnAfterAssemblyReload advances
            // the convergence serial after the new assemblies are actually
            // loaded, so the desktop never converges while the old domain still
            // runs the old code (or if the reload never fires). Write the flag
            // BOTH ways: a FAILED compile must clear a stale flag left by a prior
            // successful-but-not-yet-reloaded compile, otherwise a later bare
            // domain reload would consume it and falsely report convergence.
            bool rebuiltAssemblies = _compiledAssemblyCount > 0;
            SessionState.SetBool(SessionKey_CompileAwaitingReload, succeeded && rebuiltAssemblies);

            // Completion attribution is epoch-gated the same way: a pre-request
            // compile's finish must not consume the request (its success/errors
            // describe the WRONG compile) — the requested compile's own finish
            // completes it. The request is anchored on the PERSISTENT
            // RecompileInProgress flag, not only the volatile _recompileRequested:
            // when a pre-request compile reloads the domain before the requested
            // one runs, the volatile is lost, but the requested compile's finish
            // in the NEW domain must still attribute its errors to the request
            // (otherwise a failed follow-up leaves the result at "pending" until
            // the desktop times out, and a later unrelated reload would complete
            // it as a false "ok").
            bool requestActive = _recompileRequested
                || SessionState.GetBool(SessionKey_RecompileInProgress, false);
            if (!requestActive || !requestedCompileFinished)
                return;

            _recompileRequested = false;

            if (!succeeded)
            {
                // Compilation failed. Persist the error so Rust can surface it after any reconnect.
                SetCompileResult("error:" + string.Join("\n", errors));

                // Failed compilations do not trigger a domain reload, so clear the in-progress flag here.
                SessionState.SetBool(SessionKey_RecompileInProgress, false);
                _domainReloadCheckFrames = -1;
            }
            else if (!rebuiltAssemblies)
            {
                // Bee completed the requested incremental build and classified
                // every assembly as already up to date. This is Unity's own
                // dependency-graph verdict, so no assembly reload will follow.
                CompleteRecompileNotNeeded();
            }
            else
            {
                // Compilation finished successfully. Mark the result and wait for the real reload signal.
                SetCompileResult("awaiting_reload");
                Debug.Log($"[Locus] Compilation succeeded, waiting for domain reload. isCompiling={EditorApplication.isCompiling}, isPlaying={EditorApplication.isPlaying}");
                // If we are still in the same AppDomain after a few frames, reload did not fire.
                _domainReloadCheckFrames = 0;
            }
        }

        private static void OnAfterAssemblyReload()
        {
            RefreshCachedEditorState();
            NativeOnAfterReload();

            // A reload that fires while a requested compile is still in flight is
            // loading an EARLIER compile's assemblies, not the requested one's.
            // Advancing the serial or completing the request now would converge
            // edits that belong to the in-flight compile (not yet loaded). Defer
            // to that compile's own reload, which runs after its
            // OnCompilationFinished clears this marker.
            if (SessionState.GetBool(SessionKey_RecompilePendingCompile, false))
                return;

            // Initiator-agnostic convergence: if this reload was driven by a
            // successful compilation (flagged by OnCompilationFinished in the
            // previous domain), the new assemblies are now loaded — disk is the
            // loaded truth. Advance the convergence serial so the desktop clears
            // its "unapplied" tracking. This fires for Unity-initiated recompiles
            // (Ctrl+R, save, focus auto-refresh) too, not just Locus
            // request_recompile, and only AFTER load — never at compile time.
            if (SessionState.GetBool(SessionKey_CompileAwaitingReload, false))
            {
                SessionState.SetBool(SessionKey_CompileAwaitingReload, false);
                SessionState.SetInt(
                    SessionKey_ConvergedSerial,
                    SessionState.GetInt(SessionKey_ConvergedSerial, 0) + 1);
            }

            // afterAssemblyReload is also the completion point for a
            // Locus-requested recompile (drives get_compile_result polling).
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
                _compileParamsFingerprintReady = false;
                _cachedCompileParamsFingerprint = null;
                _cachedCompileParamsFingerprintCheckedAtTicks = 0;
                _cachedCompileAllowUnsafe = false;
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

        /// <summary>
        /// Stable id for this editor process, minted once and kept in
        /// SessionState (survives domain reloads, reset on restart). The desktop
        /// reads it via get_reload_state to recognize a fresh editor instance —
        /// whose loaded assemblies always reflect disk — and converge even when
        /// it never observed the previous instance exit.
        /// </summary>
        private static string EnsureEditorSessionId()
        {
            string id = SessionState.GetString(SessionKey_EditorSessionId, "");
            if (string.IsNullOrEmpty(id))
            {
                id = Guid.NewGuid().ToString("N");
                SessionState.SetString(SessionKey_EditorSessionId, id);
            }
            return id;
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

        private static int FlushQueuedAssetImports(bool forceFullRefresh = false)
        {
            string[] pendingPaths = new string[_pendingChangedAssetPaths.Count];
            _pendingChangedAssetPaths.CopyTo(pendingPaths);
            _pendingChangedAssetPaths.Clear();
            // Parents sort before their children, so a brand-new folder is in
            // the database before its files import.
            Array.Sort(pendingPaths, StringComparer.Ordinal);

            int importedCount = 0;
            bool needsFullRefresh = forceFullRefresh;
            if (!needsFullRefresh)
            {
                foreach (string assetPath in pendingPaths)
                {
                    if (!File.Exists(assetPath) && !Directory.Exists(assetPath))
                    {
                        // Deleted on disk. ImportAsset cannot drop a stale
                        // database entry, so one project refresh handles the
                        // whole queued batch, including the removal.
                        needsFullRefresh = true;
                        break;
                    }
                }
            }

            if (needsFullRefresh)
            {
                try
                {
                    // A compile request uses this path even when the explicit
                    // queue is empty, so out-of-band package/file changes also
                    // enter the AssetDatabase before dependency analysis.
                    AssetDatabase.Refresh(ImportAssetOptions.Default);
                    importedCount = pendingPaths.Length;
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] AssetDatabase.Refresh for changed assets failed: " + ex);
                    throw;
                }
            }
            else if (pendingPaths.Length > 0)
            {
                // ImportAsset starts an import process per call unless asset
                // editing is active. Queue every known path into one import
                // batch and let Unity run dependency analysis once at the end.
                AssetDatabase.StartAssetEditing();
                try
                {
                    foreach (string assetPath in pendingPaths)
                    {
                        try
                        {
                            AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.Default);
                            importedCount++;
                        }
                        catch (Exception ex)
                        {
                            Debug.LogError("[Locus] Failed to import changed asset before compile: " + assetPath + "\n" + ex);
                            throw;
                        }
                    }
                }
                finally
                {
                    AssetDatabase.StopAssetEditing();
                }
            }

            if (importedCount > 0 || needsFullRefresh)
                Debug.Log("[Locus] Flushed changed assets in one import pass: " + importedCount
                    + (needsFullRefresh ? " (project refresh)" : " (targeted batch)"));

            return importedCount;
        }

        private static RecompileRequestPayload ParseRecompileRequest(string raw)
        {
            string trimmed = (raw ?? "").Trim();
            if (trimmed.StartsWith("{", StringComparison.Ordinal))
            {
                RecompileRequestPayload parsed = JsonUtility.FromJson<RecompileRequestPayload>(trimmed);
                if (parsed == null || parsed.schema != 1)
                    throw new InvalidOperationException("Unsupported Locus recompile request schema.");

                string mode = (parsed.syncMode ?? "").Trim().ToLowerInvariant();
                if (mode != "none" && mode != "targeted" && mode != "full")
                    throw new InvalidOperationException("Unsupported Locus asset sync mode: " + parsed.syncMode);
                parsed.syncMode = mode;
                if (parsed.paths == null)
                    parsed.paths = new string[0];
                return parsed;
            }

            // Legacy desktop versions send newline-delimited paths. Preserve
            // their correctness contract with one full refresh.
            return new RecompileRequestPayload
            {
                schema = 0,
                syncMode = "full",
                paths = trimmed.Length == 0
                    ? new string[0]
                    : trimmed.Split(new[] { '\n' }, StringSplitOptions.RemoveEmptyEntries),
                reason = "legacy_request"
            };
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

                // Every queued path is added only after its filesystem mutation
                // completes. Import those completed paths when any owner exits,
                // even while another agent run is still active. This keeps one
                // long-running parallel session from holding finished work from
                // sibling sessions outside the AssetDatabase indefinitely.
                int importedCount = FlushQueuedAssetImports();
                Debug.Log($"[Locus] Edit session ended by '{normalizedOwner}', active owners={_activeEditSessionOwners.Count}, imported={importedCount}");
            }

            return "active_edit_sessions:" + _activeEditSessionOwners.Count;
        }

        private static void ReleaseAllEditSessions(bool flushQueuedAssets = true)
        {
            if (_activeEditSessionOwners.Count == 0 && _autoRefreshSuppressionCount == 0)
                return;

            _activeEditSessionOwners.Clear();
            while (_autoRefreshSuppressionCount > 0)
            {
                AssetDatabase.AllowAutoRefresh();
                _autoRefreshSuppressionCount--;
            }

            int importedCount = flushQueuedAssets ? FlushQueuedAssetImports() : 0;
            Debug.Log($"[Locus] Released all edit sessions, imported={importedCount}, deferred={!flushQueuedAssets}.");
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
            NativePump();
            PruneYamlPreviewSceneCache(false);
            PumpTickSchedulerEditorUpdate();

            bool desktopConnected = HasAnyDesktopConnection();
            bool hasRuntimeWork = HasMainThreadRuntimeWork();
            if (hasRuntimeWork)
                RefreshCachedEditorState();

            if (_activeRunStatesSession != null)
                PumpRunStates();
            if (HasActiveExecuteCodeAsyncRuntime())
                PumpExecuteCodeAsyncRuntime();
            if (desktopConnected)
                MaybeSendEditorUpdateEvent();

            // Keep the start handshake alive until the requested epoch really
            // enters CompilationPipeline.compilationStarted. Time only counts
            // while the editor is continuously idle; an earlier compilation may
            // legitimately occupy the pipeline before the requested one starts.
            if (_recompileStartWatchActive)
            {
                if (!IsWaitingForRequestedCompilationStart())
                {
                    _recompileStartWatchActive = false;
                    _recompileStartIdleSince = -1;
                }
                else if (EditorApplication.isCompiling || EditorApplication.isUpdating)
                {
                    _recompileStartIdleSince = -1;
                }
                else
                {
                    double now = EditorApplication.timeSinceStartup;
                    if (_recompileStartIdleSince < 0)
                        _recompileStartIdleSince = now;
                    else if (now - _recompileStartIdleSince >= RecompileStartIdleTimeoutSeconds)
                        FailRecompileStart("编辑器在请求后保持空闲，未触发 CompilationPipeline.compilationStarted。");
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
                    // The compile's reload never fired — its assemblies are on
                    // disk but unloaded. Drop the awaiting flag so a later
                    // unrelated reload does not claim this compile's convergence.
                    SessionState.SetBool(SessionKey_CompileAwaitingReload, false);
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

        private static bool HasAnyDesktopConnection()
        {
            return IsNativeBridgeActive;
        }

        private static bool HasMainThreadRuntimeWork()
        {
            return _activeRunStatesSession != null
                || HasActiveExecuteCodeAsyncRuntime()
                || _recompileStartWatchActive
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

            var additionalScenePaths = new List<string>();
            for (int i = 0; i < EditorSceneManager.sceneCount; i++)
            {
                string path = EditorSceneManager.GetSceneAt(i).path ?? "";
                if (string.IsNullOrEmpty(path)
                    || string.Equals(path, _activeScenePath, StringComparison.OrdinalIgnoreCase))
                    continue;
                additionalScenePaths.Add(path);
            }
            _additionalOpenScenePathsStatus = string.Join("|", additionalScenePaths.ToArray());
        }

        private static void MaybeSendEditorUpdateEvent()
        {
            double now = EditorApplication.timeSinceStartup;
            UnityEngine.Object selection = Selection.activeObject;
            int selectionInstanceId = LocusObjectIdentity.InstanceId(selection);
            bool selectionChanged = selectionInstanceId != _lastEditorUpdateSelectionInstanceId;
            if (!selectionChanged && _lastEditorUpdateEventAt >= 0 && now - _lastEditorUpdateEventAt < EditorUpdateEventIntervalSeconds)
                return;

            RefreshCachedEditorState();

            _lastEditorUpdateSelectionInstanceId = selectionInstanceId;
            _lastEditorUpdateEventAt = now;
            _editorUpdateEventSequence++;

            EditorUpdatePayload payload = _reusableEditorUpdatePayload;
            payload.sequence = _editorUpdateEventSequence;
            payload.timeSinceStartup = now;
            payload.isPlaying = _isPlaying;
            payload.isPaused = _isPaused;
            payload.activeScenePath = _activeScenePath;
            FillEditorSelectionSnapshot(_reusableSelectionSnapshot, selection, selectionInstanceId);
            payload.selection = _reusableSelectionSnapshot;
            SendEventToRust("unity-editor-update", JsonUtility.ToJson(payload));
        }

        private static void FillEditorSelectionSnapshot(
            EditorSelectionSnapshot target, UnityEngine.Object selection, int instanceId)
        {
            if (selection == null)
            {
                target.kind = "none";
                target.name = "";
                target.type = "";
                target.path = "";
                target.instanceId = 0;
                return;
            }

            // GetAssetPath hits the AssetDatabase and allocates a fresh string on
            // every call; reuse the last result while the selection is unchanged.
            string path;
            if (instanceId == _cachedSelectionPathInstanceId)
            {
                path = _cachedSelectionPath;
            }
            else
            {
                path = AssetDatabase.GetAssetPath(selection) ?? "";
                _cachedSelectionPathInstanceId = instanceId;
                _cachedSelectionPath = path;
            }

            target.kind = EditorSelectionKind(selection, path);
            target.name = selection.name ?? "";
            target.type = CachedTypeFullName(selection.GetType());
            target.path = path;
            target.instanceId = instanceId;
        }

        private static string CachedTypeFullName(Type type)
        {
            if (type == null)
                return "";
            string name;
            if (!_typeFullNameCache.TryGetValue(type, out name))
            {
                name = type.FullName ?? type.Name;
                _typeFullNameCache[type] = name;
            }
            return name;
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

        // ───────────────── Outbound messaging ─────────────────

        /// <summary>
        /// </summary>
        public static void SendEventToRust(string eventType, string message)
        {
            NativeEmitEvent(eventType, message);
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

                    case "configure_locus_external_editor":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(
                                    reqId,
                                    LocusExternalCodeEditor.ConfigureFromJson(msg.message)));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "sync_project_files":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(reqId, LocusProjectFiles.SyncAll()));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "bridge_capabilities":
                        return OkResponse(
                            reqId,
                            "managed_executor_v1,status_cached,set_editor_status_async,execute_idempotency_v1");

                    case "status":
                        return HandleStatus(reqId);

                    case "get_console_text":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "unity_get_console_log":
                    {
                        string requestJson = msg.message ?? "";
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                tcs.SetResult(OkResponse(reqId, BuildConsoleLogPayloadJson(requestJson)));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
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
                        return HandleSetEditorStatus(reqId, msg.message);

                    case "execute_code":
                        return await HandleExecuteCode(reqId, msg.message).ConfigureAwait(false);

                    case "execute_loaded":
                        return await HandleExecuteLoaded(reqId, msg.message).ConfigureAwait(false);

                    case "execute_code_wait":
                        return await HandleWaitExecuteCode(reqId, msg.message).ConfigureAwait(false);

                    case "cancel_execute_code":
                        return HandleCancelExecuteCode(reqId, msg.message);

                    case "execute_code_progress":
                        TouchExecuteCodeClientHeartbeat(msg.message);
                        return OkResponse(reqId, GetExecuteCodeProgressJson(msg.message));

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
                        return await HandleGetCompileParams(reqId, msg.message).ConfigureAwait(false);

                    case "hot_reload_probe":
                        return await HandleHotReloadProbe(reqId).ConfigureAwait(false);

                    case "hot_reload_set_code_optimization":
                        return await HandleHotReloadSetCodeOptimization(reqId, msg.message).ConfigureAwait(false);

                    case "hot_reload_set_debug":
                        return await HandleHotReloadSetDebug(reqId).ConfigureAwait(false);

                    case "hot_reload_set_play_mode_reload":
                        return await HandleHotReloadSetPlayModeReload(reqId, msg.message).ConfigureAwait(false);

                    case "hot_reload_access_probe":
                        return await HandleHotReloadAccessProbe(reqId, msg.message).ConfigureAwait(false);

                    case "hot_reload_inline_probe":
                        return await HandleHotReloadInlineProbe(reqId).ConfigureAwait(false);

                    case "hot_reload_inlining_active":
                        return await HandleHotReloadInliningActive(reqId).ConfigureAwait(false);

                    case "hot_patch_loaded":
                        return await HandleHotPatchLoaded(reqId, msg.message).ConfigureAwait(false);

                    case "hot_patch_dispose":
                        return await HandleHotPatchDispose(reqId, msg.message).ConfigureAwait(false);

                    case "run_states":
                        return await HandleRunStates(reqId, msg.message).ConfigureAwait(false);

                    case "run_states_loaded":
                        return await HandleRunStatesLoaded(reqId, msg.message).ConfigureAwait(false);

                    case "compile_run_states":
                        return await HandleCompileRunStates(reqId, msg.message).ConfigureAwait(false);

                    case "compile_named":
                        return await HandleCompileNamed(reqId, msg.message).ConfigureAwait(false);

                    case "compile_skill_package":
                        return await HandleCompileSkillPackage(reqId, msg.message).ConfigureAwait(false);

                    case "invoke_skill_package":
                        return await HandleInvokeSkillPackage(reqId, msg.message).ConfigureAwait(false);

                    case "invoke_named":
                        return await HandleInvokeNamed(reqId, msg.message).ConfigureAwait(false);

                    case "invoke_named_cached":
                        return await HandleInvokeNamedCached(reqId, msg.message).ConfigureAwait(false);

                    case "property_tree_read":
                    case "view_binding_read":
                        return await HandlePropertyTreeRead(reqId, msg.message).ConfigureAwait(false);

                    case "property_tree_write":
                    case "view_binding_write":
                        return await HandlePropertyTreeWrite(reqId, msg.message).ConfigureAwait(false);

                    case "property_tree_apply":
                    case "view_binding_apply":
                        return await HandlePropertyTreeApply(reqId, msg.message).ConfigureAwait(false);

                    case "property_tree_discover":
                    case "view_binding_discover":
                        return await HandlePropertyTreeDiscover(reqId, msg.message).ConfigureAwait(false);

                    case "capture_viewport":
                        return await HandleCaptureViewport(reqId, msg.message).ConfigureAwait(false);

                    case "request_recompile":
                    {
                        // Hot-reload sessions write/delete files without telling
                        // the AssetDatabase; the desktop forwards every tracked
                        // dirty path here so created files import and deleted
                        // ones refresh away before the compile. Older callers
                        // send an empty message — unchanged behavior.
                        string changedPathsRaw = msg.message ?? "";
                        var startCompletion = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                if (SessionState.GetBool(SessionKey_RecompileInProgress, false))
                                {
                                    startCompletion.TrySetResult(
                                        ErrorResponse(reqId, "Unity 重编译请求正在进行中。"));
                                    return;
                                }

                                lock (_recompileErrorsLock) { _recompileErrors.Clear(); }
                                ClearCompileResult();
                                SetCompileResult("starting");
                                _recompileRequested = true;
                                _recompileStartCompletion = startCompletion;
                                _recompileStartRequestId = reqId;
                                _recompileStartWatchActive = true;
                                _recompileStartIdleSince = -1;
                                _domainReloadCheckFrames = -1;

                                // Persist the target before releasing edit
                                // sessions or importing assets. Every operation
                                // below may itself start compilation; that first
                                // post-request epoch is a valid requested compile.
                                int targetEpoch =
                                    SessionState.GetInt(SessionKey_CompileEpochStarted, 0) + 1;
                                SessionState.SetInt(SessionKey_RecompileTargetEpoch, targetEpoch);
                                SessionState.SetBool(SessionKey_RecompilePendingCompile, true);
                                SessionState.SetBool(SessionKey_RecompileInProgress, true);
                                SessionState.SetBool(SessionKey_CompileAwaitingReload, false);

                                RecompileRequestPayload recompileRequest =
                                    ParseRecompileRequest(changedPathsRaw);

                                // Merge the session queue and the request's
                                // tracked paths before performing at most one
                                // AssetDatabase synchronization pass.
                                ReleaseAllEditSessions(false);
                                QueueChangedAssets(recompileRequest.paths);
                                FlushQueuedAssetImports(recompileRequest.syncMode == "full");
                                Debug.Log("[Locus] Recompile asset sync: mode="
                                    + recompileRequest.syncMode + ", paths="
                                    + recompileRequest.paths.Length + ", reason="
                                    + (recompileRequest.reason ?? ""));

                                // Refresh/import may already have started the
                                // target epoch. Request explicitly only while it
                                // is still pending.
                                if (SessionState.GetInt(SessionKey_CompileEpochStarted, 0) < targetEpoch)
                                    CompilationPipeline.RequestScriptCompilation();

                                if (_recompileStartWatchActive && !EditorApplication.isCompiling)
                                    _recompileStartIdleSince = EditorApplication.timeSinceStartup;
                            }
                            catch (Exception ex)
                            {
                                FailRecompileStart("启动请求执行失败：" + ex.Message);
                            }
                        });

                        return await startCompletion.Task.ConfigureAwait(false);
                    }

                    case "request_script_reload":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                // Complete the broker response before crossing
                                // the reload boundary. The next delay call then
                                // tears down this domain and removes live detours
                                // while keeping Unity's up-to-date assemblies.
                                tcs.SetResult(OkResponse(reqId, "reload_requested"));
                                EditorApplication.delayCall += EditorUtility.RequestScriptReload;
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "begin_edit_session":
                    {
                        string owner = msg.message ?? "";
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "end_edit_session":
                    {
                        string owner = msg.message ?? "";
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "import_assets":
                    {
                        string paths = msg.message ?? "";
                        var lines = paths.Split(new[] { '\n' }, System.StringSplitOptions.RemoveEmptyEntries);
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "get_reload_state":
                    {
                        // Lightweight reload-lifecycle probe the desktop polls to
                        // reconcile its hot-reload "unapplied changes" set: the
                        // per-domain generation (changes on every reload) plus a
                        // serial that advances on every SUCCESSFUL compilation
                        // (any initiator). A moved serial means a real compile
                        // converged disk into the loaded assemblies; a changed
                        // generation with an unchanged serial is a no-compile
                        // reload (e.g. entering play mode). SessionState read on
                        // the main thread, like get_compile_result.
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                var payload = new ReloadStatePayload
                                {
                                    session_id = EnsureEditorSessionId(),
                                    domain_generation = _compileDomainGeneration,
                                    converged_serial = SessionState.GetInt(SessionKey_ConvergedSerial, 0),
                                };
                                tcs.SetResult(OkResponse(reqId, JsonUtility.ToJson(payload)));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.ToString()));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "get_compile_result":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                string result = GetCompileResult();
                                if (string.IsNullOrEmpty(result))
                                {
                                    tcs.SetResult(ErrorResponse(
                                        reqId,
                                        "Unity 没有开始编译。未找到活动重编译请求。"));
                                    return;
                                }

                                if (string.Equals(result, "awaiting_reload", StringComparison.Ordinal))
                                {
                                    tcs.SetResult(OkResponse(reqId, "pending"));
                                    return;
                                }

                                if (string.Equals(result, "starting", StringComparison.Ordinal) ||
                                    string.Equals(result, "pending", StringComparison.Ordinal))
                                {
                                    tcs.SetResult(OkResponse(reqId, result));
                                    return;
                                }

                                if (string.Equals(result, "not_needed", StringComparison.Ordinal))
                                {
                                    ClearCompileResult();
                                    tcs.SetResult(OkResponse(reqId, result));
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
                        return await tcs.Task.ConfigureAwait(false);
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
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "asset_thumbnail":
                        return await HandleAssetThumbnail(reqId, msg.message).ConfigureAwait(false);

                    case "asset_preview_render":
                        return await HandleAssetPreviewRender(reqId, msg.message).ConfigureAwait(false);

                    case "validate_scene_object":
                    {
                        SceneObjectRequest request = ParseSceneObjectRequest(msg.message);
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
                        PostToMainThread(delegate
                        {
                            try
                            {
                                LocusSceneObjectUtility.ValidateSceneObject(request.scenePath, request.objectPath);
                                tcs.SetResult(OkResponse(reqId, "ok"));
                            }
                            catch (Exception ex)
                            {
                                tcs.SetResult(ErrorResponse(reqId, ex.Message));
                            }
                        });
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "select_scene_object":
                    {
                        SceneObjectRequest request = ParseSceneObjectRequest(msg.message);
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "open_scene_object_inspector":
                    {
                        SceneObjectRequest request = ParseSceneObjectRequest(msg.message);
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "start_asset_drag":
                    {
                        StartAssetDragRequest request = ParseStartAssetDragRequest(msg.message);
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "cancel_asset_drag":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "open_frontend_window":
                    {
                        var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
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
                        return await tcs.Task.ConfigureAwait(false);
                    }

                    case "search_yaml":
                        return await HandleSearchYaml(reqId, msg.message).ConfigureAwait(false);

                    case "read_yaml":
                        return await HandleReadYaml(reqId, msg.message).ConfigureAwait(false);

                    case "reload_open_scenes":
                    {
                        TaskCompletionSource<string> tcs = LocusAsync.CreateTcs<string>();
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
                        string result = await tcs.Task.ConfigureAwait(false);
                        return OkResponse(reqId, result);
                    }

                    default:
                        return await HandleExtensionMessageAsync(
                            reqId,
                            msg.type,
                            msg.message).ConfigureAwait(false);
                }
            }
            catch (Exception ex)
            {
                PostToMainThread(delegate { Debug.LogError("[Locus] HandleMessage exception for type '" + (msg.type ?? "null") + "': " + ex); });
                return ErrorResponse(reqId, ex.ToString());
            }
        }

        private static PipeEnvelope HandleStatus(string requestId)
        {
            return OkStatusResponse(requestId);
        }

        private static string BuildCachedEditorStatusMessage()
        {
            string status = _isPlaying
                ? (_isPaused ? "playing_paused" : "playing")
                : "editing";
            string scenePath = _activeScenePath;
            if (!string.IsNullOrEmpty(scenePath))
                status += "|" + scenePath;
            string additionalScenePaths = _additionalOpenScenePathsStatus;
            if (!string.IsNullOrEmpty(additionalScenePaths))
            {
                if (string.IsNullOrEmpty(scenePath))
                    status += "|";
                status += "|" + additionalScenePaths;
            }
            return status;
        }
    }
}
