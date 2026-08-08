using System;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

using UnityEditor;
using UnityEngine;

namespace Locus.Skills
{
    public static class RenderDocCapture
    {
        private const int MinimumUnityMajor = 6000;
        private const int MinimumUnityMinor = 3;
        private const int RenderDocApiVersion_1_6_0 = 10600;
        private const int LoadLibrarySearchDllLoadDir = 0x00000100;
        private const int LoadLibrarySearchDefaultDirs = 0x00001000;
        private const double CaptureTimeoutSeconds = 45.0;

        public sealed class SkillContext
        {
            public string skillPackageRoot;
            public string workingDirectory;
        }

        public sealed class CaptureRequest
        {
            public string target;
            public string captureName;
            public SkillContext __locus;
        }

        public sealed class CaptureResult
        {
            public string target;
            public string windowTitle;
            public string capturePath;
            public long captureBytes;
            public long captureTimestamp;
            public int captureIndex;
            public string renderDocVersion;
            public string renderDocApiVersion;
            public string renderDocModule;
            public string unityVersion;
            public string graphicsApi;
            public string pythonExecutable;
            public string inspectionScript;
            public string textureExportScript;
        }

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
        private delegate void StartFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint IsFrameCapturingDelegate();

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint EndFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate uint DiscardFrameCaptureDelegate(IntPtr device, IntPtr window);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void SetCaptureTitleDelegate(IntPtr utf8Title);

        private sealed class RenderDocApi
        {
            public GetApiVersionDelegate GetVersion;
            public SetCaptureFilePathTemplateDelegate SetCapturePathTemplate;
            public GetNumCapturesDelegate GetNumCaptures;
            public GetCaptureDelegate GetCapture;
            public StartFrameCaptureDelegate StartFrameCapture;
            public IsFrameCapturingDelegate IsFrameCapturing;
            public EndFrameCaptureDelegate EndFrameCapture;
            public DiscardFrameCaptureDelegate DiscardFrameCapture;
            public SetCaptureTitleDelegate SetCaptureTitle;
            public IntPtr Module;
        }

        public static async Task<CaptureResult> CaptureFrame(CaptureRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("RenderDoc capture request is empty.");

            string target = (request.target ?? "").Trim().ToLowerInvariant();
            if (target != "game" && target != "scene")
                throw new InvalidOperationException("RenderDoc target must be 'game' or 'scene'.");
            if (request.__locus == null || string.IsNullOrWhiteSpace(request.__locus.skillPackageRoot))
                throw new InvalidOperationException(
                    "Locus did not provide the Skill package runtime context. Update Locus and reload the Skill.");

            string packageRoot = Path.GetFullPath(request.__locus.skillPackageRoot);
            string runtimeRoot = Path.Combine(packageRoot, "runtime", "windows-x64");
            string renderDocDll = Path.Combine(runtimeRoot, "renderdoc.dll");
            string pythonExecutable = Path.Combine(runtimeRoot, "qrenderdoc.exe");
            string inspectionScript = Path.Combine(packageRoot, "scripts", "inspect_capture.py");
            string textureExportScript = Path.Combine(packageRoot, "scripts", "export_texture.py");
            RequireFile(renderDocDll, "RenderDoc runtime");
            RequireFile(pythonExecutable, "RenderDoc embedded Python host");
            RequireFile(inspectionScript, "RenderDoc inspection script");
            RequireFile(textureExportScript, "RenderDoc texture export script");

            IntPtr module = GetModuleHandleW("renderdoc.dll");
            bool loadedBundledRuntime = module == IntPtr.Zero;
            if (module == IntPtr.Zero)
            {
                module = LoadLibraryExW(
                    renderDocDll,
                    IntPtr.Zero,
                    LoadLibrarySearchDllLoadDir | LoadLibrarySearchDefaultDirs);
                if (module == IntPtr.Zero)
                {
                    int error = Marshal.GetLastWin32Error();
                    throw new InvalidOperationException(
                        "Failed to load bundled renderdoc.dll (Win32 error " + error + "): " + renderDocDll);
                }
            }

            if (loadedBundledRuntime)
            {
                RecreateGraphicsDeviceForRenderDoc();
                await WaitForEditorUpdates(3);
            }

            RenderDocApi api = ResolveRenderDocApi(module);
            EditorWindow previousWindow = EditorWindow.focusedWindow;
            EditorWindow captureWindow = ResolveWindow(target);
            string windowTitle = WindowTitle(captureWindow);
            string projectRoot = ResolveProjectRoot(request.__locus.workingDirectory);
            string outputDirectory = Path.Combine(projectRoot, "Library", "Locus", "RenderDoc");
            Directory.CreateDirectory(outputDirectory);
            string prefix = SafeFileName(request.captureName);
            if (string.IsNullOrEmpty(prefix))
                prefix = "locus";
            string template = Path.Combine(
                outputDirectory,
                prefix + "_" + target + "_" + DateTime.UtcNow.ToString("yyyyMMdd_HHmmss_fff"));

            SetCapturePathTemplate(api, template);
            uint previousCaptureCount = api.GetNumCaptures();
            try
            {
                captureWindow.Focus();
                captureWindow.Repaint();
                await WaitForEditorUpdates(2);
                await CaptureWindowFrame(api, captureWindow, prefix + " " + target);
                string capturePath;
                ulong captureTimestamp;
                uint captureIndex;
                await WaitForCapture(api, previousCaptureCount);
                ReadNewestCapture(api, previousCaptureCount, out captureIndex, out capturePath, out captureTimestamp);

                FileInfo captureFile = new FileInfo(capturePath);
                if (!captureFile.Exists || captureFile.Length <= 0)
                    throw new InvalidOperationException("RenderDoc returned an empty capture: " + capturePath);

                int major;
                int minor;
                int patch;
                api.GetVersion(out major, out minor, out patch);
                string modulePath = ModulePath(api.Module);
                return new CaptureResult
                {
                    target = target,
                    windowTitle = windowTitle,
                    capturePath = captureFile.FullName,
                    captureBytes = captureFile.Length,
                    captureTimestamp = captureTimestamp > long.MaxValue
                        ? long.MaxValue
                        : (long)captureTimestamp,
                    captureIndex = (int)captureIndex,
                    renderDocVersion = System.Diagnostics.FileVersionInfo
                        .GetVersionInfo(modulePath)
                        .ProductVersion,
                    renderDocApiVersion = major + "." + minor + "." + patch,
                    renderDocModule = modulePath,
                    unityVersion = Application.unityVersion,
                    graphicsApi = SystemInfo.graphicsDeviceType.ToString(),
                    pythonExecutable = pythonExecutable,
                    inspectionScript = inspectionScript,
                    textureExportScript = textureExportScript
                };
            }
            finally
            {
                if (previousWindow != null && previousWindow != captureWindow)
                    previousWindow.Focus();
            }
        }

        private static void ValidateUnityVersion()
        {
            if (Application.platform != RuntimePlatform.WindowsEditor)
                throw new PlatformNotSupportedException("RenderDoc Frame Capture requires Windows x64.");
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
                    "RenderDoc Frame Capture requires Unity 6000.3 or newer; current version is "
                    + Application.unityVersion + ".");
            }
        }

        private static void RequireFile(string path, string label)
        {
            if (!File.Exists(path))
                throw new FileNotFoundException(label + " is missing. Run 'bun run renderdoc:bundle'.", path);
        }

        private static string ResolveProjectRoot(string contextWorkingDirectory)
        {
            string fromUnity = Directory.GetParent(Application.dataPath).FullName;
            if (string.IsNullOrWhiteSpace(contextWorkingDirectory))
                return fromUnity;
            string requested = Path.GetFullPath(contextWorkingDirectory);
            return string.Equals(requested.TrimEnd(Path.DirectorySeparatorChar),
                    fromUnity.TrimEnd(Path.DirectorySeparatorChar),
                    StringComparison.OrdinalIgnoreCase)
                ? requested
                : fromUnity;
        }

        private static void RecreateGraphicsDeviceForRenderDoc()
        {
            MethodInfo recreate = typeof(ShaderUtil).GetMethod(
                "RecreateGfxDevice",
                BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
            if (recreate == null)
                throw new MissingMethodException("UnityEditor.ShaderUtil.RecreateGfxDevice is unavailable.");
            try
            {
                recreate.Invoke(null, null);
            }
            catch (TargetInvocationException ex)
            {
                throw new InvalidOperationException(
                    "Unity failed to recreate the graphics device for RenderDoc: "
                    + (ex.InnerException ?? ex).Message,
                    ex.InnerException ?? ex);
            }
        }

        private static EditorWindow ResolveWindow(string target)
        {
            if (target == "scene")
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

        private static async Task CaptureWindowFrame(
            RenderDocApi api,
            EditorWindow window,
            string title)
        {
            api.StartFrameCapture(IntPtr.Zero, IntPtr.Zero);
            if (api.IsFrameCapturing() == 0)
            {
                throw new InvalidOperationException(
                    "RenderDoc could not match an active graphics device for the Unity Editor.");
            }
            InvokeWithUtf8(
                title,
                delegate(IntPtr pointer) { api.SetCaptureTitle(pointer); });
            try
            {
                window.Repaint();
                RepaintImmediately(window);
                await WaitForEditorUpdates(2);
            }
            catch
            {
                api.DiscardFrameCapture(IntPtr.Zero, IntPtr.Zero);
                throw;
            }
            if (api.EndFrameCapture(IntPtr.Zero, IntPtr.Zero) == 0)
                throw new InvalidOperationException("RenderDoc failed to finish the Unity frame capture.");
        }

        private static void RepaintImmediately(EditorWindow window)
        {
            FieldInfo parentField = typeof(EditorWindow).GetField(
                "m_Parent",
                BindingFlags.Instance | BindingFlags.NonPublic);
            object parent = parentField != null ? parentField.GetValue(window) : null;
            if (parent == null)
                return;
            MethodInfo repaint = parent.GetType().GetMethod(
                "RepaintImmediately",
                BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (repaint != null)
                repaint.Invoke(parent, null);
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

        private static async Task WaitForCapture(RenderDocApi api, uint previousCaptureCount)
        {
            double deadline = EditorApplication.timeSinceStartup + CaptureTimeoutSeconds;
            while (api.GetNumCaptures() <= previousCaptureCount)
            {
                if (EditorApplication.timeSinceStartup >= deadline)
                    throw new TimeoutException("RenderDoc timed out while capturing the requested Unity view.");
                await WaitForEditorUpdates(1);
            }
        }

        private static void ReadNewestCapture(
            RenderDocApi api,
            uint previousCaptureCount,
            out uint captureIndex,
            out string capturePath,
            out ulong timestamp)
        {
            uint count = api.GetNumCaptures();
            if (count <= previousCaptureCount)
                throw new InvalidOperationException("RenderDoc did not register a new capture.");
            captureIndex = count - 1;
            uint pathLength = 0;
            timestamp = 0;
            if (api.GetCapture(captureIndex, IntPtr.Zero, ref pathLength, out timestamp) == 0
                || pathLength == 0)
            {
                throw new InvalidOperationException("RenderDoc did not return the capture path.");
            }

            IntPtr buffer = Marshal.AllocHGlobal((int)pathLength + 1);
            try
            {
                Marshal.WriteByte(buffer, (int)pathLength, 0);
                if (api.GetCapture(captureIndex, buffer, ref pathLength, out timestamp) == 0)
                    throw new InvalidOperationException("RenderDoc could not read the capture path.");
                byte[] bytes = new byte[pathLength];
                Marshal.Copy(buffer, bytes, 0, bytes.Length);
                int length = bytes.Length;
                while (length > 0 && bytes[length - 1] == 0)
                    length--;
                capturePath = Encoding.UTF8.GetString(bytes, 0, length);
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
            if (string.IsNullOrWhiteSpace(capturePath))
                throw new InvalidOperationException("RenderDoc returned an empty capture path.");
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
            if (getApi(RenderDocApiVersion_1_6_0, out apiPointer) == 0 || apiPointer == IntPtr.Zero)
                throw new InvalidOperationException("RenderDoc in-application API 1.6.0 is unavailable.");

            return new RenderDocApi
            {
                GetVersion = ReadApiFunction<GetApiVersionDelegate>(apiPointer, 0),
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
            InvokeWithUtf8(
                path,
                delegate(IntPtr pointer) { api.SetCapturePathTemplate(pointer); });
        }

        private static void InvokeWithUtf8(string value, Action<IntPtr> callback)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(value + "\0");
            IntPtr pointer = Marshal.AllocHGlobal(bytes.Length);
            try
            {
                Marshal.Copy(bytes, 0, pointer, bytes.Length);
                callback(pointer);
            }
            finally
            {
                Marshal.FreeHGlobal(pointer);
            }
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
                bool blocked = false;
                for (int j = 0; j < invalid.Length; j++)
                {
                    if (ch == invalid[j])
                    {
                        blocked = true;
                        break;
                    }
                }
                output.Append(blocked || char.IsWhiteSpace(ch) ? '_' : ch);
            }
            return output.ToString().Trim('_', '.');
        }

        private static string ModulePath(IntPtr module)
        {
            StringBuilder path = new StringBuilder(32768);
            uint length = GetModuleFileNameW(module, path, path.Capacity);
            return length == 0 ? "renderdoc.dll" : path.ToString();
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr LoadLibraryExW(string fileName, IntPtr file, int flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        private static extern IntPtr GetModuleHandleW(string moduleName);

        [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true)]
        private static extern IntPtr GetProcAddress(IntPtr module, string procedureName);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        private static extern uint GetModuleFileNameW(IntPtr module, StringBuilder fileName, int size);
    }
}
