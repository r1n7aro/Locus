using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

using UnityEditor;
using UnityEngine;

namespace Locus.Skills
{
    /// <summary>
    /// Public RenderDoc capture facade for unity_execute and unity_run_states.
    /// Normal capture resolves an exact UnityGUIView HWND and uses RenderDoc's
    /// in-application API. Wildcard capture is reserved for stale-state recovery.
    /// </summary>
    public static class RenderDocCaptureApi
    {
        public static Task<RenderDocInitializeResult> InitializeAsync(string skillPackageRoot)
        {
            return RenderDocCaptureRuntime.InitializeAsync(skillPackageRoot);
        }

        public static RenderDocCaptureStatus Status()
        {
            return RenderDocCaptureRuntime.Status();
        }

        public static RenderDocBeginCaptureResult Begin(RenderDocBeginCaptureOptions options = null)
        {
            return RenderDocCaptureRuntime.Begin(options ?? new RenderDocBeginCaptureOptions());
        }

        public static Task<RenderDocCaptureOnceResult> CaptureOnceAsync(
            RenderDocBeginCaptureOptions options = null,
            int maxEditorUpdates = 16)
        {
            return RenderDocCaptureRuntime.CaptureOnceAsync(
                options ?? new RenderDocBeginCaptureOptions(),
                maxEditorUpdates);
        }

        public static RenderDocRepaintResult RequestTargetRepaint()
        {
            return RenderDocCaptureRuntime.RequestTargetRepaint();
        }

        public static RenderDocTriggerCaptureResult Trigger()
        {
            return RenderDocCaptureRuntime.Trigger();
        }

        public static RenderDocEndCaptureResult End()
        {
            return RenderDocCaptureRuntime.End();
        }

        public static RenderDocCaptureLookupResult LastCapture()
        {
            return RenderDocCaptureRuntime.LastCapture();
        }

        public static RenderDocDiscardCaptureResult Discard()
        {
            return RenderDocCaptureRuntime.Discard();
        }

        public static RenderDocRecoveryResult RecoverStaleCaptures(int maxAttempts = 8)
        {
            return RenderDocCaptureRuntime.RecoverStaleCaptures(maxAttempts);
        }

        public static string Json(object value)
        {
            return RenderDocJson.Serialize(value);
        }
    }

    public enum RenderDocCaptureTarget
    {
        Game,
        Scene
    }

    public sealed class RenderDocBeginCaptureOptions
    {
        public RenderDocCaptureTarget Target { get; set; } = RenderDocCaptureTarget.Game;
        public string CaptureName { get; set; }
        public string CaptureTitle { get; set; }
        public string OutputDirectory { get; set; }
        public bool FocusWindow { get; set; } = true;
        public bool RestorePreviousFocus { get; set; } = true;
    }

    public sealed class RenderDocRuntimeInfo
    {
        public bool Initialized { get; internal set; }
        public bool UnityIntegrationLoaded { get; internal set; }
        public bool LoadedBundledRuntime { get; internal set; }
        public string SkillPackageRoot { get; internal set; }
        public string RuntimeRoot { get; internal set; }
        public string RenderDocModule { get; internal set; }
        public string RenderDocVersion { get; internal set; }
        public string RenderDocApiVersion { get; internal set; }
        public string PythonExecutable { get; internal set; }
        public string PythonModule { get; internal set; }
        public string PythonModuleDirectory { get; internal set; }
        public string PythonWorker { get; internal set; }
        public string ProjectRoot { get; internal set; }
        public string DefaultOutputDirectory { get; internal set; }
        public string UnityVersion { get; internal set; }
        public string GraphicsApi { get; internal set; }
        public string TargetSelection { get; internal set; }
        public bool UsesUnityInternalWindowMapping { get; internal set; }
        public bool OverlayHidden { get; internal set; }
    }

    public sealed class RenderDocInitializeResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public bool AlreadyInitialized { get; internal set; }
        public bool RecreatedGraphicsDevice { get; internal set; }
        public RenderDocRuntimeInfo Runtime { get; internal set; }
    }

    public sealed class RenderDocCaptureFile
    {
        public bool Exists { get; internal set; }
        public string Path { get; internal set; }
        public long Bytes { get; internal set; }
        public long Timestamp { get; internal set; }
        public int CaptureIndex { get; internal set; }
        public string ReplayValidation { get; internal set; } = "not_run";
    }

    public sealed class RenderDocCaptureStatus
    {
        public bool Initialized { get; internal set; }
        public string State { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
        public bool TrackedCaptureActive { get; internal set; }
        public string CaptureId { get; internal set; }
        public RenderDocCaptureTarget? Target { get; internal set; }
        public string WindowTitle { get; internal set; }
        public int WindowInstanceId { get; internal set; }
        public string NativeWindowHandle { get; internal set; }
        public string StartedUtc { get; internal set; }
        public double ElapsedSeconds { get; internal set; }
        public int CaptureCount { get; internal set; }
        public RenderDocCaptureFile LatestCapture { get; internal set; }
        public RenderDocRuntimeInfo Runtime { get; internal set; }
    }

    public sealed class RenderDocBeginCaptureResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public string CaptureId { get; internal set; }
        public RenderDocCaptureTarget Target { get; internal set; }
        public string WindowTitle { get; internal set; }
        public int WindowInstanceId { get; internal set; }
        public string NativeWindowHandle { get; internal set; }
        public string OutputTemplate { get; internal set; }
        public string CaptureTitle { get; internal set; }
        public string StartedUtc { get; internal set; }
        public int CaptureCountBefore { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
    }

    public sealed class RenderDocRepaintResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public string CaptureId { get; internal set; }
        public string WindowTitle { get; internal set; }
        public int WindowInstanceId { get; internal set; }
        public int RequestedRepaintCount { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
    }

    public sealed class RenderDocTriggerCaptureResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public string CaptureId { get; internal set; }
        public int TriggerCount { get; internal set; }
        public string TriggeredUtc { get; internal set; }
        public int CaptureCountBefore { get; internal set; }
        public int CaptureCountAfterRepaint { get; internal set; }
        public bool CaptureRegisteredImmediately { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
        public RenderDocCaptureFile Capture { get; internal set; }
    }

    public sealed class RenderDocCaptureOnceResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public RenderDocBeginCaptureResult Begin { get; internal set; }
        public RenderDocTriggerCaptureResult Trigger { get; internal set; }
        public RenderDocCaptureLookupResult Lookup { get; internal set; }
        public RenderDocEndCaptureResult End { get; internal set; }
        public RenderDocCaptureFile Capture { get; internal set; }
    }

    public sealed class RenderDocEndCaptureResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public string CaptureId { get; internal set; }
        public RenderDocCaptureTarget Target { get; internal set; }
        public string WindowTitle { get; internal set; }
        public string StartedUtc { get; internal set; }
        public string EndedUtc { get; internal set; }
        public double DurationSeconds { get; internal set; }
        public int CaptureCountBefore { get; internal set; }
        public int CaptureCountAfter { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
        public bool CaptureRegistered { get; internal set; }
        public bool CapturePending { get; internal set; }
        public RenderDocCaptureFile Capture { get; internal set; }
    }

    public sealed class RenderDocCaptureLookupResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public int CaptureCount { get; internal set; }
        public int ExpectedAfterIndex { get; internal set; }
        public bool NewSinceLastBegin { get; internal set; }
        public bool CapturePending { get; internal set; }
        public RenderDocCaptureFile Capture { get; internal set; }
    }

    public sealed class RenderDocDiscardCaptureResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public string CaptureId { get; internal set; }
        public bool EndedExactWindowCapture { get; internal set; }
        public bool GlobalCaptureActive { get; internal set; }
        public bool SavedBeforeDeletion { get; internal set; }
        public bool DeletedCaptureFile { get; internal set; }
        public string DeletedPath { get; internal set; }
    }

    public sealed class RenderDocRecoveryResult
    {
        public bool Success { get; internal set; }
        public string ErrorCode { get; internal set; }
        public string Message { get; internal set; }
        public int MaxAttempts { get; internal set; }
        public int Attempts { get; internal set; }
        public bool CaptureActiveBefore { get; internal set; }
        public bool CaptureActiveAfter { get; internal set; }
        public int SuccessfulDiscardCalls { get; internal set; }
    }

    internal static class RenderDocCaptureRuntime
    {
        private const int MinimumUnityMajor = 6000;
        private const int MinimumUnityMinor = 3;
        private const int RenderDocApiVersion_1_7_0 = 10700;
        private const int RenderDocApiVersion_1_6_0 = 10600;
        private const int LoadLibrarySearchDllLoadDir = 0x00000100;
        private const int LoadLibrarySearchDefaultDirs = 0x00001000;

        private static RenderDocApi _api;
        private static RenderDocRuntimeInfo _runtime;
        private static ActiveCapture _active;
        private static uint _lastCaptureFloor;
        private static bool _captureExpected;
        private static readonly object Gate = new object();

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate int GetRenderDocApiDelegate(int version, out IntPtr api);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void GetApiVersionDelegate(out int major, out int minor, out int patch);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void SetCaptureFilePathTemplateDelegate(IntPtr utf8Path);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint GetNumCapturesDelegate();

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint GetCaptureDelegate(
            uint index,
            IntPtr utf8Path,
            ref uint pathLength,
            out ulong timestamp);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void MaskOverlayBitsDelegate(uint andMask, uint orMask);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void StartFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint EndFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint IsFrameCapturingDelegate();

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint DiscardFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void SetCaptureTitleDelegate(IntPtr utf8Title);

        [UnmanagedFunctionPointer(CallingConvention.Winapi)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeRect
        {
            public int Left;
            public int Top;
            public int Right;
            public int Bottom;
        }

        private sealed class RenderDocApi
        {
            public GetApiVersionDelegate GetVersion;
            public SetCaptureFilePathTemplateDelegate SetCapturePathTemplate;
            public GetNumCapturesDelegate GetNumCaptures;
            public GetCaptureDelegate GetCapture;
            public MaskOverlayBitsDelegate MaskOverlayBits;
            public StartFrameCaptureDelegate StartFrameCapture;
            public IsFrameCapturingDelegate IsFrameCapturing;
            public EndFrameCaptureDelegate EndFrameCapture;
            public DiscardFrameCaptureDelegate DiscardFrameCapture;
            public SetCaptureTitleDelegate SetCaptureTitle;
            public IntPtr Module;
        }

        private sealed class ActiveCapture
        {
            public string Id;
            public RenderDocCaptureTarget Target;
            public EditorWindow Window;
            public EditorWindow PreviousWindow;
            public bool RestorePreviousFocus;
            public IntPtr NativeWindowHandle;
            public bool NativeCaptureActive;
            public string WindowTitle;
            public string OutputTemplate;
            public string CaptureTitle;
            public uint CaptureCountBefore;
            public DateTime StartedUtc;
            public double StartedEditorTime;
            public int RepaintCount;
            public int TriggerCount;
        }

        internal static async Task<RenderDocInitializeResult> InitializeAsync(string skillPackageRoot)
        {
            try
            {
                ValidateUnityVersion();
                string root = string.IsNullOrWhiteSpace(skillPackageRoot)
                    ? null
                    : Path.GetFullPath(skillPackageRoot.Trim());
                if (string.IsNullOrEmpty(root) || !Directory.Exists(root))
                    return InitFailure("invalid_skill_root", "Skill package Root is missing or unavailable.");

                string runtimeRoot = Path.Combine(root, "runtime", "windows-x64");
                string renderDocDll = Path.Combine(runtimeRoot, "renderdoc.dll");
                string pythonExecutable = Path.Combine(runtimeRoot, "qrenderdoc.exe");
                string pythonModule = Path.Combine(root, "scripts", "locus_renderdoc.py");
                string pythonWorker = Path.Combine(root, "scripts", "renderdoc_worker.py");
                RequireFile(renderDocDll, "RenderDoc runtime");
                RequireFile(pythonExecutable, "RenderDoc embedded Python host");
                RequireFile(pythonModule, "RenderDoc Python module");
                RequireFile(pythonWorker, "RenderDoc Python worker");

                if (_api != null && _runtime != null)
                {
                    bool sameRoot = PathsEqual(_runtime.SkillPackageRoot, root);
                    if (!sameRoot)
                    {
                        return InitFailure(
                            "runtime_already_initialized",
                            "RenderDoc is already initialized from another Skill package Root. Restart Unity before switching runtimes.");
                    }
                    _api.MaskOverlayBits(0, 0);
                    _runtime.OverlayHidden = true;
                    return new RenderDocInitializeResult
                    {
                        Success = true,
                        Message = "RenderDoc runtime is already initialized.",
                        AlreadyInitialized = true,
                        Runtime = SnapshotRuntime()
                    };
                }

                IntPtr module = GetModuleHandleW("renderdoc.dll");
                bool loadedBundledRuntime = module == IntPtr.Zero;
                if (module != IntPtr.Zero)
                {
                    string loadedPath = ModulePath(module);
                    if (!PathsEqual(loadedPath, renderDocDll))
                    {
                        return InitFailure(
                            "renderdoc_module_conflict",
                            "A different renderdoc.dll is already loaded: " + loadedPath
                            + ". Restart Unity, then initialize this Skill before other RenderDoc integrations.");
                    }
                }
                else
                {
                    module = LoadLibraryExW(
                        renderDocDll,
                        IntPtr.Zero,
                        LoadLibrarySearchDllLoadDir | LoadLibrarySearchDefaultDirs);
                    if (module == IntPtr.Zero)
                    {
                        int error = Marshal.GetLastWin32Error();
                        return InitFailure(
                            "load_library_failed",
                            "Failed to load bundled renderdoc.dll (Win32 error " + error + "): " + renderDocDll);
                    }
                }

                RenderDocApi api = ResolveRenderDocApi(module);
                api.MaskOverlayBits(0, 0);
                bool recreated = false;
                if (!UnityEditorInternal.RenderDoc.IsLoaded())
                {
                    // Register the already-loaded module with Unity's native
                    // RenderDoc integration before recreating the device.
                    UnityEditorInternal.RenderDoc.Load();
                    RecreateGraphicsDeviceForRenderDoc();
                    recreated = true;
                    await WaitForEditorUpdates(3);
                }
                api.MaskOverlayBits(0, 0);

                int major;
                int minor;
                int patch;
                api.GetVersion(out major, out minor, out patch);
                string modulePath = ModulePath(module);
                string projectRoot = Directory.GetParent(Application.dataPath).FullName;
                _api = api;
                _runtime = new RenderDocRuntimeInfo
                {
                    Initialized = true,
                    UnityIntegrationLoaded = UnityEditorInternal.RenderDoc.IsLoaded(),
                    LoadedBundledRuntime = loadedBundledRuntime,
                    SkillPackageRoot = root,
                    RuntimeRoot = runtimeRoot,
                    RenderDocModule = modulePath,
                    RenderDocVersion = FileVersionInfo.GetVersionInfo(modulePath).ProductVersion,
                    RenderDocApiVersion = major + "." + minor + "." + patch,
                    PythonExecutable = pythonExecutable,
                    PythonModule = pythonModule,
                    PythonModuleDirectory = Path.GetDirectoryName(pythonModule),
                    PythonWorker = pythonWorker,
                    ProjectRoot = projectRoot,
                    DefaultOutputDirectory = Path.Combine(projectRoot, "Library", "Locus", "RenderDoc"),
                    UnityVersion = Application.unityVersion,
                    GraphicsApi = SystemInfo.graphicsDeviceType.ToString(),
                    TargetSelection = "FocusedUnityGUIViewHwnd",
                    UsesUnityInternalWindowMapping = false,
                    OverlayHidden = true
                };

                if (!_runtime.UnityIntegrationLoaded)
                {
                    return new RenderDocInitializeResult
                    {
                        Success = false,
                        ErrorCode = "unity_renderdoc_integration_unavailable",
                        Message = "Unity did not activate its RenderDoc integration after graphics-device recreation.",
                        RecreatedGraphicsDevice = recreated,
                        Runtime = SnapshotRuntime()
                    };
                }

                return new RenderDocInitializeResult
                {
                    Success = true,
                    Message = "RenderDoc runtime initialized.",
                    RecreatedGraphicsDevice = recreated,
                    Runtime = SnapshotRuntime()
                };
            }
            catch (Exception exception)
            {
                return InitFailure("initialization_failed", ExceptionMessage(exception));
            }
        }

        internal static RenderDocCaptureStatus Status()
        {
            bool global = IsGlobalCaptureActive();
            ActiveCapture active = _active;
            int count = SafeCaptureCount();
            RenderDocCaptureFile latest = count > 0 ? TryReadCapture((uint)(count - 1)) : null;
            string state = _api == null
                ? "uninitialized"
                : active != null
                    ? active.NativeCaptureActive
                        ? "capturing"
                        : _captureExpected ? "capture_pending" : "armed"
                    : global ? "untracked_capture_active" : "ready";
            return new RenderDocCaptureStatus
            {
                Initialized = _api != null,
                State = state,
                GlobalCaptureActive = global,
                TrackedCaptureActive = active != null,
                CaptureId = active == null ? null : active.Id,
                Target = active == null ? (RenderDocCaptureTarget?)null : active.Target,
                WindowTitle = active == null ? null : active.WindowTitle,
                WindowInstanceId = active == null || active.Window == null ? 0 : active.Window.GetHashCode(),
                NativeWindowHandle = active == null ? null : NativeHandleText(active.NativeWindowHandle),
                StartedUtc = active == null ? null : Utc(active.StartedUtc),
                ElapsedSeconds = active == null
                    ? 0.0
                    : Math.Max(0.0, EditorApplication.timeSinceStartup - active.StartedEditorTime),
                CaptureCount = count,
                LatestCapture = latest,
                Runtime = SnapshotRuntime()
            };
        }

        internal static RenderDocBeginCaptureResult Begin(RenderDocBeginCaptureOptions options)
        {
            if (_api == null || _runtime == null)
                return BeginFailure("not_initialized", "Call RenderDocCaptureApi.InitializeAsync(Root) first.");
            if (!_runtime.UnityIntegrationLoaded || !UnityEditorInternal.RenderDoc.IsLoaded())
                return BeginFailure("unity_integration_unavailable", "Unity's RenderDoc integration is unavailable.");
            if (_active != null)
                return BeginFailure("capture_already_tracked", "A tracked RenderDoc capture is already active.");
            if (IsGlobalCaptureActive())
            {
                return BeginFailure(
                    "untracked_capture_active",
                    "RenderDoc reports an active capture without a tracked session. Call RecoverStaleCaptures explicitly before starting another capture.");
            }

            EditorWindow window = null;
            try
            {
                window = ResolveWindow(options.Target);
                string windowTitle = WindowTitle(window);
                string outputDirectory = ResolveOutputDirectory(options.OutputDirectory);
                Directory.CreateDirectory(outputDirectory);
                string prefix = SafeFileName(options.CaptureName);
                if (string.IsNullOrEmpty(prefix))
                    prefix = "locus";
                string target = options.Target == RenderDocCaptureTarget.Scene ? "scene" : "game";
                string outputTemplate = Path.Combine(
                    outputDirectory,
                    prefix + "_" + target + "_" + DateTime.UtcNow.ToString("yyyyMMdd_HHmmss_fff"));
                string captureTitle = string.IsNullOrWhiteSpace(options.CaptureTitle)
                    ? prefix + " " + target
                    : options.CaptureTitle.Trim();
                uint captureCountBefore = _api.GetNumCaptures();
                SetCapturePathTemplate(_api, outputTemplate);
                EditorWindow previousWindow = EditorWindow.focusedWindow;

                if (options.FocusWindow)
                    window.Focus();
                window.Repaint();
                RepaintImmediately(window);
                IntPtr nativeWindowHandle = ResolveNativeWindowHandle(window);
                if (nativeWindowHandle == IntPtr.Zero)
                {
                    return BeginFailure(
                        "native_window_not_found",
                        "Could not resolve the target UnityGUIView HWND for " + window.GetType().FullName + ".");
                }

                ActiveCapture session = new ActiveCapture
                {
                    Id = Guid.NewGuid().ToString("N"),
                    Target = options.Target,
                    Window = window,
                    PreviousWindow = previousWindow,
                    RestorePreviousFocus = options.RestorePreviousFocus,
                    NativeWindowHandle = nativeWindowHandle,
                    WindowTitle = windowTitle,
                    OutputTemplate = outputTemplate,
                    CaptureTitle = captureTitle,
                    CaptureCountBefore = captureCountBefore,
                    StartedUtc = DateTime.UtcNow,
                    StartedEditorTime = EditorApplication.timeSinceStartup
                };

                lock (Gate)
                {
                    _active = session;
                    _lastCaptureFloor = captureCountBefore;
                    _captureExpected = false;
                }
                window.Repaint();
                session.RepaintCount++;
                return new RenderDocBeginCaptureResult
                {
                    Success = true,
                    Message = "Background RenderDoc target armed for one exact UnityGUIView HWND.",
                    CaptureId = session.Id,
                    Target = session.Target,
                    WindowTitle = session.WindowTitle,
                    WindowInstanceId = window.GetHashCode(),
                    NativeWindowHandle = NativeHandleText(session.NativeWindowHandle),
                    OutputTemplate = session.OutputTemplate,
                    CaptureTitle = session.CaptureTitle,
                    StartedUtc = Utc(session.StartedUtc),
                    CaptureCountBefore = (int)captureCountBefore,
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
            catch (Exception exception)
            {
                return BeginFailure("begin_failed", ExceptionMessage(exception));
            }
        }

        internal static async Task<RenderDocCaptureOnceResult> CaptureOnceAsync(
            RenderDocBeginCaptureOptions options,
            int maxEditorUpdates)
        {
            RenderDocBeginCaptureResult begin = Begin(options);
            if (!begin.Success)
            {
                return new RenderDocCaptureOnceResult
                {
                    Success = false,
                    ErrorCode = begin.ErrorCode,
                    Message = begin.Message,
                    Begin = begin
                };
            }

            RenderDocTriggerCaptureResult trigger = null;
            RenderDocCaptureLookupResult lookup = null;
            RenderDocEndCaptureResult end = null;
            try
            {
                trigger = Trigger();
                if (trigger.Success)
                {
                    int bounded = Math.Max(0, Math.Min(120, maxEditorUpdates));
                    lookup = LastCapture();
                    for (int index = 0;
                        index < bounded && (!lookup.Success || !lookup.NewSinceLastBegin);
                        index++)
                    {
                        await WaitForEditorUpdates(1);
                        lookup = LastCapture();
                    }
                }

                end = End();
                RenderDocCaptureFile capture = trigger == null ? null : trigger.Capture;
                if (lookup != null && lookup.NewSinceLastBegin && lookup.Capture != null)
                    capture = lookup.Capture;
                if (end != null && end.Capture != null)
                    capture = end.Capture;
                bool success = trigger != null && trigger.Success
                    && end != null && end.Success
                    && capture != null && capture.Exists;
                return new RenderDocCaptureOnceResult
                {
                    Success = success,
                    ErrorCode = success
                        ? null
                        : trigger != null && !trigger.Success
                            ? trigger.ErrorCode
                            : end != null && !end.Success
                                ? end.ErrorCode
                                : "capture_not_registered",
                    Message = success
                        ? "One exact Unity view frame was captured in the background."
                        : trigger != null && !trigger.Success
                            ? trigger.Message
                            : end != null && !end.Success
                                ? end.Message
                                : "The background capture completed without a registered file.",
                    Begin = begin,
                    Trigger = trigger,
                    Lookup = lookup,
                    End = end,
                    Capture = capture
                };
            }
            catch (Exception exception)
            {
                return new RenderDocCaptureOnceResult
                {
                    Success = false,
                    ErrorCode = "capture_once_failed",
                    Message = ExceptionMessage(exception),
                    Begin = begin,
                    Trigger = trigger,
                    Lookup = lookup,
                    End = end
                };
            }
            finally
            {
                ActiveCapture active = _active;
                if (active != null && active.Id == begin.CaptureId)
                    Discard();
            }
        }

        internal static RenderDocRepaintResult RequestTargetRepaint()
        {
            ActiveCapture active = _active;
            if (active == null)
            {
                return new RenderDocRepaintResult
                {
                    Success = false,
                    ErrorCode = "no_tracked_capture",
                    Message = "No tracked RenderDoc capture is active.",
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
            if (active.Window == null)
            {
                return new RenderDocRepaintResult
                {
                    Success = false,
                    ErrorCode = "target_window_closed",
                    Message = "The tracked capture target window has been closed.",
                    CaptureId = active.Id,
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
            active.Window.Repaint();
            active.RepaintCount++;
            return new RenderDocRepaintResult
            {
                Success = true,
                Message = "Capture target repaint requested.",
                CaptureId = active.Id,
                WindowTitle = active.WindowTitle,
                WindowInstanceId = active.Window.GetHashCode(),
                RequestedRepaintCount = active.RepaintCount,
                GlobalCaptureActive = IsGlobalCaptureActive()
            };
        }

        internal static RenderDocTriggerCaptureResult Trigger()
        {
            ActiveCapture active = _active;
            if (_api == null)
            {
                return new RenderDocTriggerCaptureResult
                {
                    Success = false,
                    ErrorCode = "not_initialized",
                    Message = "Call RenderDocCaptureApi.InitializeAsync(Root) first."
                };
            }
            if (active == null)
            {
                return new RenderDocTriggerCaptureResult
                {
                    Success = false,
                    ErrorCode = "no_tracked_capture",
                    Message = "Call Begin before triggering a frame.",
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
            if (active.Window == null)
            {
                return new RenderDocTriggerCaptureResult
                {
                    Success = false,
                    ErrorCode = "target_window_closed",
                    Message = "The tracked capture target window has been closed.",
                    CaptureId = active.Id
                };
            }
            if (_captureExpected && _api.GetNumCaptures() <= _lastCaptureFloor)
            {
                return new RenderDocTriggerCaptureResult
                {
                    Success = false,
                    ErrorCode = "previous_trigger_pending",
                    Message = "The previous capture trigger has not registered a frame yet.",
                    CaptureId = active.Id,
                    TriggerCount = active.TriggerCount,
                    CaptureCountBefore = (int)_lastCaptureFloor,
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }

            try
            {
                uint before = _api.GetNumCaptures();
                _lastCaptureFloor = before;
                _captureExpected = false;
                active.TriggerCount++;
                DateTime triggered = DateTime.UtcNow;
                _api.StartFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                active.NativeCaptureActive = IsGlobalCaptureActive();
                if (!active.NativeCaptureActive)
                {
                    return new RenderDocTriggerCaptureResult
                    {
                        Success = false,
                        ErrorCode = "native_window_not_capturable",
                        Message = "RenderDoc did not start a capture for HWND "
                            + NativeHandleText(active.NativeWindowHandle) + ".",
                        CaptureId = active.Id,
                        TriggerCount = active.TriggerCount,
                        CaptureCountBefore = (int)before,
                        GlobalCaptureActive = false
                    };
                }
                SetCaptureTitle(_api, active.CaptureTitle);
                active.Window.Repaint();
                RepaintImmediately(active.Window);
                active.RepaintCount++;
                uint ended = _api.EndFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                active.NativeCaptureActive = false;
                if (ended == 0)
                {
                    if (IsGlobalCaptureActive())
                        _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    return new RenderDocTriggerCaptureResult
                    {
                        Success = false,
                        ErrorCode = "native_capture_end_failed",
                        Message = "RenderDoc could not finish the exact HWND capture.",
                        CaptureId = active.Id,
                        TriggerCount = active.TriggerCount,
                        CaptureCountBefore = (int)before,
                        GlobalCaptureActive = IsGlobalCaptureActive()
                    };
                }
                uint after = _api.GetNumCaptures();
                _captureExpected = after <= before;
                RenderDocCaptureFile capture = after > before ? TryReadCapture(after - 1) : null;
                return new RenderDocTriggerCaptureResult
                {
                    Success = true,
                    Message = capture == null
                        ? "Exact HWND frame captured; file registration is pending."
                        : "Exact HWND frame captured and registered in the background.",
                    CaptureId = active.Id,
                    TriggerCount = active.TriggerCount,
                    TriggeredUtc = Utc(triggered),
                    CaptureCountBefore = (int)before,
                    CaptureCountAfterRepaint = (int)after,
                    CaptureRegisteredImmediately = capture != null,
                    GlobalCaptureActive = IsGlobalCaptureActive(),
                    Capture = capture
                };
            }
            catch (Exception exception)
            {
                if (active.NativeCaptureActive)
                {
                    try
                    {
                        _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    }
                    catch { }
                    active.NativeCaptureActive = false;
                }
                _captureExpected = false;
                return new RenderDocTriggerCaptureResult
                {
                    Success = false,
                    ErrorCode = "trigger_failed",
                    Message = ExceptionMessage(exception),
                    CaptureId = active.Id,
                    TriggerCount = active.TriggerCount,
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
        }

        internal static RenderDocEndCaptureResult End()
        {
            ActiveCapture active = _active;
            if (active == null)
            {
                return new RenderDocEndCaptureResult
                {
                    Success = false,
                    ErrorCode = "no_tracked_capture",
                    Message = "No tracked RenderDoc capture is active.",
                    GlobalCaptureActive = IsGlobalCaptureActive(),
                    CaptureCountAfter = SafeCaptureCount()
                };
            }

            try
            {
                if (active.NativeCaptureActive)
                {
                    _api.EndFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    active.NativeCaptureActive = false;
                }
                bool globalActive = IsGlobalCaptureActive();
                if (globalActive)
                {
                    _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    globalActive = IsGlobalCaptureActive();
                }
                uint countAfter = _api.GetNumCaptures();
                RenderDocCaptureFile capture = countAfter > active.CaptureCountBefore
                    ? TryReadCapture(countAfter - 1)
                    : null;
                DateTime ended = DateTime.UtcNow;
                lock (Gate)
                {
                    _active = null;
                    _captureExpected = false;
                }
                RestoreFocus(active);
                return new RenderDocEndCaptureResult
                {
                    Success = !globalActive,
                    ErrorCode = globalActive ? "capture_still_active" : null,
                    Message = globalActive
                        ? "The exact HWND capture session closed, but RenderDoc still reports native capture state."
                        : capture == null
                            ? "Background capture session closed; capture registration is pending."
                            : "Background capture session closed and registered.",
                    CaptureId = active.Id,
                    Target = active.Target,
                    WindowTitle = active.WindowTitle,
                    StartedUtc = Utc(active.StartedUtc),
                    EndedUtc = Utc(ended),
                    DurationSeconds = Math.Max(0.0, (ended - active.StartedUtc).TotalSeconds),
                    CaptureCountBefore = (int)active.CaptureCountBefore,
                    CaptureCountAfter = (int)countAfter,
                    GlobalCaptureActive = globalActive,
                    CaptureRegistered = capture != null,
                    CapturePending = !globalActive && capture == null,
                    Capture = capture
                };
            }
            catch (Exception exception)
            {
                if (active.NativeCaptureActive)
                {
                    try { _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle); }
                    catch { }
                    active.NativeCaptureActive = false;
                }
                lock (Gate)
                {
                    _active = null;
                    _captureExpected = false;
                }
                RestoreFocus(active);
                return new RenderDocEndCaptureResult
                {
                    Success = false,
                    ErrorCode = "end_failed",
                    Message = ExceptionMessage(exception),
                    CaptureId = active.Id,
                    Target = active.Target,
                    WindowTitle = active.WindowTitle,
                    StartedUtc = Utc(active.StartedUtc),
                    CaptureCountBefore = (int)active.CaptureCountBefore,
                    CaptureCountAfter = SafeCaptureCount(),
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
        }

        internal static RenderDocCaptureLookupResult LastCapture()
        {
            if (_api == null)
            {
                return new RenderDocCaptureLookupResult
                {
                    Success = false,
                    ErrorCode = "not_initialized",
                    Message = "Call RenderDocCaptureApi.InitializeAsync(Root) first."
                };
            }
            try
            {
                uint count = _api.GetNumCaptures();
                bool isNew = count > _lastCaptureFloor;
                RenderDocCaptureFile capture = count > 0 ? TryReadCapture(count - 1) : null;
                return new RenderDocCaptureLookupResult
                {
                    Success = capture != null,
                    ErrorCode = capture == null ? "capture_not_registered" : null,
                    Message = capture == null ? "RenderDoc has not registered a capture." : "Latest capture resolved.",
                    CaptureCount = (int)count,
                    ExpectedAfterIndex = (int)_lastCaptureFloor,
                    NewSinceLastBegin = isNew,
                    CapturePending = _captureExpected && !isNew,
                    Capture = capture
                };
            }
            catch (Exception exception)
            {
                return new RenderDocCaptureLookupResult
                {
                    Success = false,
                    ErrorCode = "capture_lookup_failed",
                    Message = ExceptionMessage(exception),
                    CaptureCount = SafeCaptureCount(),
                    ExpectedAfterIndex = (int)_lastCaptureFloor
                };
            }
        }

        internal static RenderDocDiscardCaptureResult Discard()
        {
            ActiveCapture active = _active;
            if (active == null)
            {
                return new RenderDocDiscardCaptureResult
                {
                    Success = false,
                    ErrorCode = "no_tracked_capture",
                    Message = "No tracked capture exists. Use RecoverStaleCaptures only for an untracked native capture.",
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }

            try
            {
                if (active.NativeCaptureActive)
                {
                    _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    active.NativeCaptureActive = false;
                }
                bool globalActive = IsGlobalCaptureActive();
                if (globalActive)
                {
                    _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle);
                    globalActive = IsGlobalCaptureActive();
                }
                uint count = _api.GetNumCaptures();
                RenderDocCaptureFile saved = count > active.CaptureCountBefore
                    ? TryReadCapture(count - 1)
                    : null;
                bool deleted = false;
                if (saved != null && saved.Exists)
                {
                    File.Delete(saved.Path);
                    deleted = !File.Exists(saved.Path);
                }
                lock (Gate)
                {
                    _active = null;
                    _captureExpected = false;
                }
                RestoreFocus(active);
                return new RenderDocDiscardCaptureResult
                {
                    Success = !globalActive && (saved == null || deleted),
                    ErrorCode = globalActive ? "capture_still_active" : saved != null && !deleted ? "capture_delete_failed" : null,
                    Message = globalActive
                        ? "The background HWND session closed, but native capture state remains active."
                        : saved == null ? "Background capture session discarded." : "Background capture session discarded and its file was deleted.",
                    CaptureId = active.Id,
                    EndedExactWindowCapture = true,
                    GlobalCaptureActive = globalActive,
                    SavedBeforeDeletion = saved != null,
                    DeletedCaptureFile = deleted,
                    DeletedPath = saved == null ? null : saved.Path
                };
            }
            catch (Exception exception)
            {
                if (active.NativeCaptureActive)
                {
                    try { _api.DiscardFrameCapture(IntPtr.Zero, active.NativeWindowHandle); }
                    catch { }
                    active.NativeCaptureActive = false;
                }
                lock (Gate)
                {
                    _active = null;
                    _captureExpected = false;
                }
                RestoreFocus(active);
                return new RenderDocDiscardCaptureResult
                {
                    Success = false,
                    ErrorCode = "discard_failed",
                    Message = ExceptionMessage(exception),
                    CaptureId = active.Id,
                    GlobalCaptureActive = IsGlobalCaptureActive()
                };
            }
        }

        internal static RenderDocRecoveryResult RecoverStaleCaptures(int maxAttempts)
        {
            if (_api == null)
            {
                return RecoveryFailure("not_initialized", "Call RenderDocCaptureApi.InitializeAsync(Root) first.", maxAttempts);
            }
            if (_active != null)
            {
                return RecoveryFailure(
                    "tracked_capture_active",
                    "A tracked capture is active. Use RenderDocCaptureApi.Discard() for exact-window cleanup.",
                    maxAttempts);
            }
            int bounded = Math.Max(1, Math.Min(32, maxAttempts));
            bool before = IsGlobalCaptureActive();
            int attempts = 0;
            int successful = 0;
            try
            {
                while (attempts < bounded && IsGlobalCaptureActive())
                {
                    attempts++;
                    // Explicit recovery only: wildcard matching is undefined when
                    // several device/window pairs are active, so normal capture
                    // never calls this API.
                    if (_api.DiscardFrameCapture(IntPtr.Zero, IntPtr.Zero) != 0)
                        successful++;
                }
                bool after = IsGlobalCaptureActive();
                return new RenderDocRecoveryResult
                {
                    Success = !after,
                    ErrorCode = after ? "stale_capture_remains" : null,
                    Message = after
                        ? "RenderDoc still reports an active capture after bounded recovery. Restart Unity before capturing again."
                        : before ? "Untracked native capture state cleared." : "No stale capture was active.",
                    MaxAttempts = bounded,
                    Attempts = attempts,
                    CaptureActiveBefore = before,
                    CaptureActiveAfter = after,
                    SuccessfulDiscardCalls = successful
                };
            }
            catch (Exception exception)
            {
                RenderDocRecoveryResult failure = RecoveryFailure("recovery_failed", ExceptionMessage(exception), bounded);
                failure.Attempts = attempts;
                failure.CaptureActiveBefore = before;
                failure.CaptureActiveAfter = IsGlobalCaptureActive();
                failure.SuccessfulDiscardCalls = successful;
                return failure;
            }
        }

        private static RenderDocInitializeResult InitFailure(string code, string message)
        {
            return new RenderDocInitializeResult
            {
                Success = false,
                ErrorCode = code,
                Message = message,
                Runtime = SnapshotRuntime()
            };
        }

        private static RenderDocBeginCaptureResult BeginFailure(string code, string message)
        {
            return new RenderDocBeginCaptureResult
            {
                Success = false,
                ErrorCode = code,
                Message = message,
                GlobalCaptureActive = IsGlobalCaptureActive()
            };
        }

        private static RenderDocRecoveryResult RecoveryFailure(string code, string message, int maxAttempts)
        {
            return new RenderDocRecoveryResult
            {
                Success = false,
                ErrorCode = code,
                Message = message,
                MaxAttempts = Math.Max(1, Math.Min(32, maxAttempts)),
                CaptureActiveAfter = IsGlobalCaptureActive()
            };
        }

        private static RenderDocRuntimeInfo SnapshotRuntime()
        {
            if (_runtime == null)
                return null;
            return new RenderDocRuntimeInfo
            {
                Initialized = _runtime.Initialized,
                UnityIntegrationLoaded = _runtime.UnityIntegrationLoaded,
                LoadedBundledRuntime = _runtime.LoadedBundledRuntime,
                SkillPackageRoot = _runtime.SkillPackageRoot,
                RuntimeRoot = _runtime.RuntimeRoot,
                RenderDocModule = _runtime.RenderDocModule,
                RenderDocVersion = _runtime.RenderDocVersion,
                RenderDocApiVersion = _runtime.RenderDocApiVersion,
                PythonExecutable = _runtime.PythonExecutable,
                PythonModule = _runtime.PythonModule,
                PythonModuleDirectory = _runtime.PythonModuleDirectory,
                PythonWorker = _runtime.PythonWorker,
                ProjectRoot = _runtime.ProjectRoot,
                DefaultOutputDirectory = _runtime.DefaultOutputDirectory,
                UnityVersion = _runtime.UnityVersion,
                GraphicsApi = _runtime.GraphicsApi,
                TargetSelection = _runtime.TargetSelection,
                UsesUnityInternalWindowMapping = _runtime.UsesUnityInternalWindowMapping,
                OverlayHidden = _runtime.OverlayHidden
            };
        }

        private static bool IsGlobalCaptureActive()
        {
            try { return _api != null && _api.IsFrameCapturing() != 0; }
            catch { return false; }
        }

        private static int SafeCaptureCount()
        {
            try { return _api == null ? 0 : (int)_api.GetNumCaptures(); }
            catch { return 0; }
        }

        private static RenderDocCaptureFile TryReadCapture(uint index)
        {
            try
            {
                uint pathLength = 0;
                ulong timestamp;
                if (_api.GetCapture(index, IntPtr.Zero, ref pathLength, out timestamp) == 0 || pathLength == 0)
                    return null;
                IntPtr buffer = Marshal.AllocHGlobal((int)pathLength + 1);
                string path;
                try
                {
                    Marshal.WriteByte(buffer, (int)pathLength, 0);
                    if (_api.GetCapture(index, buffer, ref pathLength, out timestamp) == 0)
                        return null;
                    byte[] bytes = new byte[pathLength];
                    Marshal.Copy(buffer, bytes, 0, bytes.Length);
                    int length = bytes.Length;
                    while (length > 0 && bytes[length - 1] == 0)
                        length--;
                    path = Encoding.UTF8.GetString(bytes, 0, length);
                }
                finally { Marshal.FreeHGlobal(buffer); }
                if (string.IsNullOrWhiteSpace(path))
                    return null;
                FileInfo file = new FileInfo(path);
                return new RenderDocCaptureFile
                {
                    Exists = file.Exists,
                    Path = file.FullName,
                    Bytes = file.Exists ? file.Length : 0,
                    Timestamp = timestamp > long.MaxValue ? long.MaxValue : (long)timestamp,
                    CaptureIndex = (int)index,
                    ReplayValidation = "not_run"
                };
            }
            catch { return null; }
        }

        private static string ResolveOutputDirectory(string requested)
        {
            if (string.IsNullOrWhiteSpace(requested))
                return _runtime.DefaultOutputDirectory;
            string path = requested.Trim();
            return Path.GetFullPath(Path.IsPathRooted(path) ? path : Path.Combine(_runtime.ProjectRoot, path));
        }

        private static EditorWindow ResolveWindow(RenderDocCaptureTarget target)
        {
            if (target == RenderDocCaptureTarget.Scene)
            {
                SceneView scene = SceneView.lastActiveSceneView;
                return scene != null ? scene : EditorWindow.GetWindow<SceneView>();
            }
            Type gameViewType = typeof(EditorWindow).Assembly.GetType("UnityEditor.GameView");
            if (gameViewType == null)
                throw new InvalidOperationException("Unity GameView type is unavailable.");
            return EditorWindow.GetWindow(gameViewType);
        }

        private static string WindowTitle(EditorWindow window)
        {
            if (window == null)
                return "";
            if (window.titleContent != null && !string.IsNullOrEmpty(window.titleContent.text))
                return window.titleContent.text;
            return window.GetType().Name;
        }

        private static void RestoreFocus(ActiveCapture active)
        {
            if (active.RestorePreviousFocus
                && active.PreviousWindow != null
                && active.PreviousWindow != active.Window)
                active.PreviousWindow.Focus();
        }

        private static IntPtr ResolveNativeWindowHandle(EditorWindow window)
        {
            string expectedTitle = window.GetType().FullName;
            uint processId = (uint)Process.GetCurrentProcess().Id;
            IntPtr focused = GetFocus();
            IntPtr current = focused;
            while (current != IntPtr.Zero)
            {
                if (IsUnityGuiView(current, processId, expectedTitle))
                    return current;
                current = GetParent(current);
            }

            List<IntPtr> matches = new List<IntPtr>();
            EnumWindowsProc collect = delegate(IntPtr handle, IntPtr parameter)
            {
                uint ownerProcessId;
                GetWindowThreadProcessId(handle, out ownerProcessId);
                if (ownerProcessId != processId)
                    return true;
                if (IsUnityGuiView(handle, processId, expectedTitle))
                    matches.Add(handle);
                EnumWindowsProc collectChild = delegate(IntPtr child, IntPtr childParameter)
                {
                    if (IsUnityGuiView(child, processId, expectedTitle))
                        matches.Add(child);
                    return true;
                };
                EnumChildWindows(handle, collectChild, IntPtr.Zero);
                return true;
            };
            EnumWindows(collect, IntPtr.Zero);
            if (matches.Count == 0)
                return IntPtr.Zero;
            if (matches.Count == 1)
                return matches[0];

            Rect target = window.position;
            IntPtr best = matches[0];
            double bestDistance = double.MaxValue;
            for (int index = 0; index < matches.Count; index++)
            {
                NativeRect rect;
                if (!GetWindowRect(matches[index], out rect))
                    continue;
                double deltaX = (rect.Left + rect.Right) * 0.5 - target.center.x;
                double deltaY = (rect.Top + rect.Bottom) * 0.5 - target.center.y;
                double distance = deltaX * deltaX + deltaY * deltaY;
                if (distance < bestDistance)
                {
                    best = matches[index];
                    bestDistance = distance;
                }
            }
            return best;
        }

        private static bool IsUnityGuiView(IntPtr handle, uint processId, string expectedTitle)
        {
            uint ownerProcessId;
            GetWindowThreadProcessId(handle, out ownerProcessId);
            if (ownerProcessId != processId)
                return false;
            StringBuilder className = new StringBuilder(128);
            if (GetClassNameW(handle, className, className.Capacity) == 0
                || !string.Equals(className.ToString(), "UnityGUIViewWndClass", StringComparison.Ordinal))
                return false;
            int length = GetWindowTextLengthW(handle);
            StringBuilder title = new StringBuilder(Math.Max(1, length + 1));
            GetWindowTextW(handle, title, title.Capacity);
            return string.Equals(title.ToString(), expectedTitle, StringComparison.Ordinal);
        }

        private static string NativeHandleText(IntPtr handle)
        {
            return handle == IntPtr.Zero
                ? null
                : "0x" + handle.ToInt64().ToString("X", CultureInfo.InvariantCulture);
        }

        private static void RepaintImmediately(EditorWindow window)
        {
            object parent = ResolveGuiView(window);
            if (parent == null)
                return;
            MethodInfo repaint = parent.GetType().GetMethod(
                "RepaintImmediately",
                BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (repaint != null)
                repaint.Invoke(parent, null);
        }

        private static object ResolveGuiView(EditorWindow window)
        {
            FieldInfo parentField = typeof(EditorWindow).GetField(
                "m_Parent",
                BindingFlags.Instance | BindingFlags.NonPublic);
            return parentField == null ? null : parentField.GetValue(window);
        }

        private static void ValidateUnityVersion()
        {
            if (Application.platform != RuntimePlatform.WindowsEditor)
                throw new PlatformNotSupportedException("RenderDoc capture requires Windows x64.");
            string[] parts = (Application.unityVersion ?? "").Split('.');
            int major;
            int minor;
            if (parts.Length < 2
                || !int.TryParse(parts[0], out major)
                || !int.TryParse(parts[1], out minor)
                || major < MinimumUnityMajor
                || (major == MinimumUnityMajor && minor < MinimumUnityMinor))
            {
                throw new InvalidOperationException(
                    "RenderDoc capture requires Unity 6000.3 or newer; current version is "
                    + Application.unityVersion + ".");
            }
        }

        private static void RequireFile(string path, string label)
        {
            if (!File.Exists(path))
                throw new FileNotFoundException(label + " is missing. Run 'bun run renderdoc:bundle'.", path);
        }

        private static void RecreateGraphicsDeviceForRenderDoc()
        {
            MethodInfo recreate = typeof(ShaderUtil).GetMethod(
                "RecreateGfxDevice",
                BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
            if (recreate == null)
                throw new MissingMethodException("UnityEditor.ShaderUtil.RecreateGfxDevice is unavailable.");
            try { recreate.Invoke(null, null); }
            catch (TargetInvocationException exception)
            {
                Exception inner = exception.InnerException ?? exception;
                throw new InvalidOperationException(
                    "Unity failed to recreate the graphics device for RenderDoc: " + inner.Message,
                    inner);
            }
        }

        private static Task WaitForEditorUpdates(int count)
        {
            TaskCompletionSource<bool> source = new TaskCompletionSource<bool>();
            int remaining = Math.Max(1, count);
            EditorApplication.CallbackFunction callback = null;
            callback = delegate
            {
                remaining--;
                if (remaining > 0)
                    return;
                EditorApplication.update -= callback;
                source.TrySetResult(true);
            };
            EditorApplication.update += callback;
            return source.Task;
        }

        private static RenderDocApi ResolveRenderDocApi(IntPtr module)
        {
            IntPtr getApiPointer = GetProcAddress(module, "RENDERDOC_GetAPI");
            if (getApiPointer == IntPtr.Zero)
                throw new InvalidOperationException("renderdoc.dll does not export RENDERDOC_GetAPI.");
            GetRenderDocApiDelegate getApi = (GetRenderDocApiDelegate)Marshal.GetDelegateForFunctionPointer(
                getApiPointer,
                typeof(GetRenderDocApiDelegate));
            IntPtr apiPointer;
            if ((getApi(RenderDocApiVersion_1_7_0, out apiPointer) == 0 || apiPointer == IntPtr.Zero)
                && (getApi(RenderDocApiVersion_1_6_0, out apiPointer) == 0 || apiPointer == IntPtr.Zero))
                throw new InvalidOperationException("RenderDoc in-application API 1.6.0 or newer is unavailable.");
            return new RenderDocApi
            {
                GetVersion = ReadApiFunction<GetApiVersionDelegate>(apiPointer, 0),
                MaskOverlayBits = ReadApiFunction<MaskOverlayBitsDelegate>(apiPointer, 8),
                SetCapturePathTemplate = ReadApiFunction<SetCaptureFilePathTemplateDelegate>(apiPointer, 11),
                GetNumCaptures = ReadApiFunction<GetNumCapturesDelegate>(apiPointer, 13),
                GetCapture = ReadApiFunction<GetCaptureDelegate>(apiPointer, 14),
                StartFrameCapture = ReadApiFunction<StartFrameCaptureDelegate>(apiPointer, 19),
                IsFrameCapturing = ReadApiFunction<IsFrameCapturingDelegate>(apiPointer, 20),
                EndFrameCapture = ReadApiFunction<EndFrameCaptureDelegate>(apiPointer, 21),
                DiscardFrameCapture = ReadApiFunction<DiscardFrameCaptureDelegate>(apiPointer, 24),
                SetCaptureTitle = ReadApiFunction<SetCaptureTitleDelegate>(apiPointer, 26),
                Module = module
            };
        }

        private static T ReadApiFunction<T>(IntPtr api, int index) where T : class
        {
            IntPtr pointer = Marshal.ReadIntPtr(api, index * IntPtr.Size);
            if (pointer == IntPtr.Zero)
                throw new InvalidOperationException("RenderDoc API function " + index + " is unavailable.");
            T callback = Marshal.GetDelegateForFunctionPointer(pointer, typeof(T)) as T;
            if (callback == null)
                throw new InvalidOperationException("RenderDoc API function " + index + " has an invalid signature.");
            return callback;
        }

        private static void SetCapturePathTemplate(RenderDocApi api, string path)
        {
            InvokeWithUtf8(path, delegate(IntPtr pointer) { api.SetCapturePathTemplate(pointer); });
        }

        private static void SetCaptureTitle(RenderDocApi api, string title)
        {
            InvokeWithUtf8(title, delegate(IntPtr pointer) { api.SetCaptureTitle(pointer); });
        }

        private static void InvokeWithUtf8(string value, Action<IntPtr> callback)
        {
            byte[] bytes = Encoding.UTF8.GetBytes((value ?? "") + "\0");
            IntPtr pointer = Marshal.AllocHGlobal(bytes.Length);
            try
            {
                Marshal.Copy(bytes, 0, pointer, bytes.Length);
                callback(pointer);
            }
            finally { Marshal.FreeHGlobal(pointer); }
        }

        private static string SafeFileName(string value)
        {
            value = (value ?? "").Trim();
            if (value.Length > 48)
                value = value.Substring(0, 48);
            char[] invalid = Path.GetInvalidFileNameChars();
            StringBuilder output = new StringBuilder(value.Length);
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                bool blocked = Array.IndexOf(invalid, ch) >= 0;
                output.Append(blocked || char.IsWhiteSpace(ch) ? '_' : ch);
            }
            return output.ToString().Trim('_', '.');
        }

        private static bool PathsEqual(string left, string right)
        {
            try
            {
                return string.Equals(
                    Path.GetFullPath(left).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar),
                    Path.GetFullPath(right).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar),
                    StringComparison.OrdinalIgnoreCase);
            }
            catch { return false; }
        }

        private static string ModulePath(IntPtr module)
        {
            StringBuilder path = new StringBuilder(32768);
            uint length = GetModuleFileNameW(module, path, path.Capacity);
            return length == 0 ? "renderdoc.dll" : path.ToString();
        }

        private static string ExceptionMessage(Exception exception)
        {
            TargetInvocationException invocation = exception as TargetInvocationException;
            Exception source = invocation != null && invocation.InnerException != null
                ? invocation.InnerException
                : exception;
            return source.GetType().Name + ": " + source.Message;
        }

        private static string Utc(DateTime value)
        {
            return value.ToUniversalTime().ToString("o", CultureInfo.InvariantCulture);
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr LoadLibraryExW(string fileName, IntPtr file, int flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        private static extern IntPtr GetModuleHandleW(string moduleName);

        [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true)]
        private static extern IntPtr GetProcAddress(IntPtr module, string procedureName);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        private static extern uint GetModuleFileNameW(IntPtr module, StringBuilder fileName, int size);

        [DllImport("user32.dll")]
        private static extern IntPtr GetFocus();

        [DllImport("user32.dll")]
        private static extern IntPtr GetParent(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr parameter);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetClassNameW(IntPtr window, StringBuilder className, int maxCount);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetWindowTextLengthW(IntPtr window);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetWindowTextW(IntPtr window, StringBuilder title, int maxCount);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetWindowRect(IntPtr window, out NativeRect rect);

    }

    internal static class RenderDocJson
    {
        internal static string Serialize(object value)
        {
            StringBuilder output = new StringBuilder(512);
            HashSet<object> path = new HashSet<object>(ReferenceComparer.Instance);
            Write(output, value, path, 0);
            return output.ToString();
        }

        private static void Write(StringBuilder output, object value, HashSet<object> path, int depth)
        {
            if (value == null) { output.Append("null"); return; }
            if (depth > 12) { Quote(output, "<max-depth>"); return; }
            if (value is string || value is char) { Quote(output, Convert.ToString(value, CultureInfo.InvariantCulture)); return; }
            if (value is bool) { output.Append((bool)value ? "true" : "false"); return; }
            if (IsNumber(value)) { output.Append(Convert.ToString(value, CultureInfo.InvariantCulture)); return; }
            if (value is Enum) { Quote(output, value.ToString()); return; }
            if (value is UnityEngine.Object)
            {
                UnityEngine.Object unityObject = (UnityEngine.Object)value;
                Quote(output, unityObject == null ? null : unityObject.name);
                return;
            }
            bool track = !value.GetType().IsValueType;
            if (track && !path.Add(value)) { Quote(output, "<cycle>"); return; }
            try
            {
                IDictionary dictionary = value as IDictionary;
                if (dictionary != null) { WriteDictionary(output, dictionary, path, depth); return; }
                IEnumerable enumerable = value as IEnumerable;
                if (enumerable != null) { WriteList(output, enumerable, path, depth); return; }
                WriteObject(output, value, path, depth);
            }
            finally { if (track) path.Remove(value); }
        }

        private static void WriteDictionary(StringBuilder output, IDictionary value, HashSet<object> path, int depth)
        {
            output.Append('{');
            bool first = true;
            foreach (DictionaryEntry item in value)
            {
                if (!first) output.Append(',');
                first = false;
                Quote(output, Convert.ToString(item.Key, CultureInfo.InvariantCulture));
                output.Append(':');
                Write(output, item.Value, path, depth + 1);
            }
            output.Append('}');
        }

        private static void WriteList(StringBuilder output, IEnumerable value, HashSet<object> path, int depth)
        {
            output.Append('[');
            bool first = true;
            foreach (object item in value)
            {
                if (!first) output.Append(',');
                first = false;
                Write(output, item, path, depth + 1);
            }
            output.Append(']');
        }

        private static void WriteObject(StringBuilder output, object value, HashSet<object> path, int depth)
        {
            output.Append('{');
            bool first = true;
            PropertyInfo[] properties = value.GetType().GetProperties(BindingFlags.Instance | BindingFlags.Public);
            for (int i = 0; i < properties.Length; i++)
            {
                PropertyInfo property = properties[i];
                if (!property.CanRead || property.GetIndexParameters().Length != 0)
                    continue;
                object item;
                try { item = property.GetValue(value, null); }
                catch { continue; }
                if (!first) output.Append(',');
                first = false;
                Quote(output, char.ToLowerInvariant(property.Name[0]) + property.Name.Substring(1));
                output.Append(':');
                Write(output, item, path, depth + 1);
            }
            output.Append('}');
        }

        private static bool IsNumber(object value)
        {
            return value is byte || value is sbyte || value is short || value is ushort
                || value is int || value is uint || value is long || value is ulong
                || value is float || value is double || value is decimal;
        }

        private static void Quote(StringBuilder output, string value)
        {
            if (value == null) { output.Append("null"); return; }
            output.Append('"');
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                switch (ch)
                {
                    case '"': output.Append("\\\""); break;
                    case '\\': output.Append("\\\\"); break;
                    case '\b': output.Append("\\b"); break;
                    case '\f': output.Append("\\f"); break;
                    case '\n': output.Append("\\n"); break;
                    case '\r': output.Append("\\r"); break;
                    case '\t': output.Append("\\t"); break;
                    default:
                        if (ch < 32) output.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                        else output.Append(ch);
                        break;
                }
            }
            output.Append('"');
        }

        private sealed class ReferenceComparer : IEqualityComparer<object>
        {
            internal static readonly ReferenceComparer Instance = new ReferenceComparer();
            public new bool Equals(object left, object right) { return ReferenceEquals(left, right); }
            public int GetHashCode(object value)
            {
                return System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(value);
            }
        }
    }
}
