using UnityEngine;
using UnityEditor;

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
#if UNITY_EDITOR_WIN
using Microsoft.Win32;
#endif

namespace Locus
{
    public sealed class LocusEditorWindow : EditorWindow
    {
        private const string PipeName = "locus_tauri_unity_embed";
        private const string FullPipeName = @"\\.\pipe\locus_tauri_unity_embed";
        private const double SyncIntervalSeconds = 0.12d;
        private const double HeartbeatIntervalSeconds = 2d;
        private const double DesktopProbeIntervalSeconds = 2d;
        private const int PipeConnectTimeoutMs = 500;

        private static readonly Encoding Utf8NoBom = new UTF8Encoding(false);
        private static Texture2D _titleIcon;

        private double _nextSyncAt;
        private volatile bool _sendInFlight;
        private volatile bool _sentOpen;
        private volatile int _failedSends;
        private string _statusMessage = "Waiting for Locus desktop.";
        private readonly object _pipeLock = new object();
        private NamedPipeClientStream _pipeClient;
        private StreamWriter _pipeWriter;
        private bool _hasScreenRect;
        private int _screenX;
        private int _screenY;
        private int _screenWidth;
        private int _screenHeight;
        private double _nextHeartbeatAt;
        private bool _hasLastSent;
        private int _lastSentX;
        private int _lastSentY;
        private int _lastSentWidth;
        private int _lastSentHeight;
        private bool _lastSentVisible;
        private long _lastSentParentHwnd;
        private double _nextDesktopProbeAt;
        private LocusDesktopInstall _desktopInstall = LocusDesktopInstall.NotFound;
        private bool _desktopProcessRunning;
        private volatile bool _desktopLaunchInFlight;

        [Serializable]
        private sealed class EmbedControlMessage
        {
            public string type;
            public int x;
            public int y;
            public int width;
            public int height;
            public bool visible;
            public long parentHwnd;
        }

        private sealed class LocusDesktopInstall
        {
            public static readonly LocusDesktopInstall NotFound = new LocusDesktopInstall(false, "");

            public readonly bool IsInstalled;
            public readonly string ExecutablePath;

            public LocusDesktopInstall(bool isInstalled, string executablePath)
            {
                IsInstalled = isInstalled;
                ExecutablePath = executablePath ?? "";
            }
        }

        [MenuItem("Window/Locus")]
        public static void OpenWindow()
        {
            LocusEditorWindow window = GetWindow<LocusEditorWindow>();
            window.titleContent = CreateTitleContent();
            window.minSize = new Vector2(360f, 420f);
            window.Show();
        }

        private void OnEnable()
        {
            titleContent = CreateTitleContent();
            minSize = new Vector2(360f, 420f);
            RefreshDesktopState(true);
            EditorApplication.update += SyncOverlay;
            SendOpenOrUpdate(true);
        }

        private void OnDisable()
        {
            EditorApplication.update -= SyncOverlay;
            SendClose();
            DisconnectPipe();
        }

        private void OnFocus()
        {
            SendOpenOrUpdate(true);
        }

        private void OnGUI()
        {
            UpdateScreenRectFromGUI();
            RefreshDesktopState(false);
            DrawPlaceholder();

            if (Event.current.type == EventType.Repaint)
                SendOpenOrUpdate(false);
        }

        private void SyncOverlay()
        {
            double now = EditorApplication.timeSinceStartup;
            if (now < _nextSyncAt)
                return;

            _nextSyncAt = now + SyncIntervalSeconds;
            RefreshDesktopState(false);
            SendOpenOrUpdate(false);

            if (_failedSends > 0 || ShouldShowStartButton() || _desktopLaunchInFlight)
                Repaint();
        }

        private void SendOpenOrUpdate(bool force)
        {
            if (_sendInFlight && !force)
                return;

            EmbedControlMessage message = BuildMessage(_sentOpen ? "update" : "open", true);
            if (!force && !ShouldSendMessage(message))
                return;

            _nextHeartbeatAt = EditorApplication.timeSinceStartup + HeartbeatIntervalSeconds;
            SendControlMessage(message, false);
        }

        private void SendClose()
        {
            SendControlMessage(BuildMessage("close", false), true);
            _sentOpen = false;
        }

        private EmbedControlMessage BuildMessage(string type, bool visible)
        {
            if (!_hasScreenRect)
                UpdateScreenRectFromPosition();

            return new EmbedControlMessage
            {
                type = type,
                x = _screenX,
                y = _screenY,
                width = _screenWidth,
                height = _screenHeight,
                visible = visible && _screenWidth > 12 && _screenHeight > 12 && IsSelectedDockTab(),
                parentHwnd = GetUnityMainHwnd()
            };
        }

        private bool ShouldSendMessage(EmbedControlMessage message)
        {
            if (!_sentOpen || !_hasLastSent || _failedSends > 0)
                return true;

            if (EditorApplication.timeSinceStartup >= _nextHeartbeatAt)
                return true;

            return message.x != _lastSentX
                || message.y != _lastSentY
                || message.width != _lastSentWidth
                || message.height != _lastSentHeight
                || message.visible != _lastSentVisible
                || message.parentHwnd != _lastSentParentHwnd;
        }

        private void RecordLastSent(EmbedControlMessage message)
        {
            _hasLastSent = true;
            _lastSentX = message.x;
            _lastSentY = message.y;
            _lastSentWidth = message.width;
            _lastSentHeight = message.height;
            _lastSentVisible = message.visible;
            _lastSentParentHwnd = message.parentHwnd;
        }

        private void UpdateScreenRectFromGUI()
        {
            Vector2 topLeft = GUIUtility.GUIToScreenPoint(Vector2.zero);
            Vector2 bottomRight = GUIUtility.GUIToScreenPoint(new Vector2(
                position.width,
                position.height));
            StoreScreenRect(topLeft, bottomRight);
        }

        private void UpdateScreenRectFromPosition()
        {
            Vector2 topLeft = new Vector2(position.x, position.y);
            Vector2 bottomRight = new Vector2(position.xMax, position.yMax);
            StoreScreenRect(topLeft, bottomRight);
        }

        private void StoreScreenRect(Vector2 topLeft, Vector2 bottomRight)
        {
            float scale = EditorGUIUtility.pixelsPerPoint;
            _screenX = Mathf.RoundToInt(topLeft.x * scale);
            _screenY = Mathf.RoundToInt(topLeft.y * scale);
            _screenWidth = Mathf.Max(1, Mathf.RoundToInt((bottomRight.x - topLeft.x) * scale));
            _screenHeight = Mathf.Max(1, Mathf.RoundToInt((bottomRight.y - topLeft.y) * scale));
            _hasScreenRect = true;
        }

        private void SendControlMessage(EmbedControlMessage message, bool force)
        {
            if (_sendInFlight && !force)
                return;

            string json = JsonUtility.ToJson(message);
            _sendInFlight = true;

            Task.Run(() =>
            {
                try
                {
                    WritePipeLine(json);

                    if (message.type != "close")
                    {
                        _sentOpen = true;
                        RecordLastSent(message);
                        _failedSends = 0;
                        _statusMessage = "Overlay signal sent.";
                    }
                }
                catch (Exception ex)
                {
                    DisconnectPipe();
                    if (message.type != "close")
                    {
                        int failures = _failedSends + 1;
                        _failedSends = failures;
                        _statusMessage = failures <= 1
                            ? "Waiting for Locus desktop."
                            : "Waiting for Locus desktop: " + ex.Message;
                    }
                }
                finally
                {
                    _sendInFlight = false;
                }
            });
        }

        private void WritePipeLine(string json)
        {
            lock (_pipeLock)
            {
                EnsurePipeConnected();
                _pipeWriter.WriteLine(json);
                _pipeWriter.Flush();
            }
        }

        private void EnsurePipeConnected()
        {
            if (_pipeClient != null && _pipeClient.IsConnected && _pipeWriter != null)
                return;

            DisconnectPipe();
            _pipeClient = new NamedPipeClientStream(
                ".",
                PipeName,
                PipeDirection.Out,
                PipeOptions.Asynchronous);
            _pipeClient.Connect(PipeConnectTimeoutMs);
            _pipeWriter = new StreamWriter(_pipeClient, Utf8NoBom, 4096)
            {
                NewLine = "\n",
                AutoFlush = true
            };
        }

        private void DisconnectPipe()
        {
            lock (_pipeLock)
            {
                try { if (_pipeWriter != null) _pipeWriter.Dispose(); } catch { }
                try { if (_pipeClient != null) _pipeClient.Dispose(); } catch { }
                _pipeWriter = null;
                _pipeClient = null;
            }
        }

        private void DrawPlaceholder()
        {
            Rect rect = new Rect(0f, 0f, position.width, position.height);
            Color bg = EditorGUIUtility.isProSkin
                ? new Color(0.18f, 0.18f, 0.18f, 1f)
                : new Color(0.78f, 0.78f, 0.78f, 1f);
            EditorGUI.DrawRect(rect, bg);

            Rect titleRect = new Rect(8f, 5f, Mathf.Max(0f, rect.width - 16f), 16f);
            Rect inner = new Rect(
                14f,
                28f,
                Mathf.Max(0f, rect.width - 28f),
                rect.height - 38f);
            Rect statusRect = new Rect(inner.x, titleRect.yMax + 8f, inner.width, 34f);
            Rect pipeRect = new Rect(inner.x, statusRect.yMax + 10f, inner.width, 18f);
            Rect buttonRect = new Rect(
                inner.x,
                pipeRect.yMax + 12f,
                Mathf.Min(116f, inner.width),
                24f);

            GUI.Label(titleRect, "Locus", EditorStyles.boldLabel);
            GUI.Label(statusRect, _statusMessage, EditorStyles.wordWrappedLabel);
            EditorGUI.SelectableLabel(pipeRect, FullPipeName, EditorStyles.miniLabel);

            if (ShouldShowStartButton())
            {
                using (new EditorGUI.DisabledScope(_desktopLaunchInFlight))
                {
                    if (GUI.Button(buttonRect, _desktopLaunchInFlight ? "启动中..." : "启动 Locus"))
                        StartLocusDesktop();
                }
            }
        }

        private void RefreshDesktopState(bool force)
        {
            double now = EditorApplication.timeSinceStartup;
            if (!force && now < _nextDesktopProbeAt)
                return;

            _nextDesktopProbeAt = now + DesktopProbeIntervalSeconds;
            _desktopInstall = FindLocusDesktopInstall();
            _desktopProcessRunning = IsLocusDesktopProcessRunning(_desktopInstall.ExecutablePath);
        }

        private bool ShouldShowStartButton()
        {
            return _desktopInstall.IsInstalled && !_desktopProcessRunning;
        }

        private void StartLocusDesktop()
        {
            if (_desktopLaunchInFlight)
                return;

            RefreshDesktopState(true);
            if (!_desktopInstall.IsInstalled)
            {
                _statusMessage = "Locus desktop install was not found.";
                return;
            }

            if (_desktopProcessRunning)
            {
                _statusMessage = "Locus desktop is running.";
                SendOpenOrUpdate(true);
                return;
            }

            string executablePath = _desktopInstall.ExecutablePath;
            if (string.IsNullOrEmpty(executablePath) || !File.Exists(executablePath))
            {
                _statusMessage = "Locus desktop executable was not found.";
                return;
            }

            _desktopLaunchInFlight = true;
            _statusMessage = "Starting Locus desktop.";

            Task.Run(async () =>
            {
                try
                {
                    ProcessStartInfo startInfo = new ProcessStartInfo
                    {
                        FileName = executablePath,
                        WorkingDirectory = Path.GetDirectoryName(executablePath),
                        UseShellExecute = true
                    };

                    Process.Start(startInfo);
                    _desktopProcessRunning = true;
                    await Task.Delay(2000);
                }
                catch (Exception ex)
                {
                    _statusMessage = "Failed to start Locus desktop: " + ex.Message;
                }
                finally
                {
                    _desktopLaunchInFlight = false;
                }
            });
        }

        private static LocusDesktopInstall FindLocusDesktopInstall()
        {
#if UNITY_EDITOR_WIN
            string executablePath = FindWindowsLocusExecutable();
            if (!string.IsNullOrEmpty(executablePath))
                return new LocusDesktopInstall(true, executablePath);
#endif

            return LocusDesktopInstall.NotFound;
        }

        private static bool IsLocusDesktopProcessRunning(string executablePath)
        {
            string processName = "locus";
            if (!string.IsNullOrEmpty(executablePath))
            {
                try
                {
                    string fileName = Path.GetFileNameWithoutExtension(executablePath);
                    if (!string.IsNullOrEmpty(fileName))
                        processName = fileName;
                }
                catch
                {
                }
            }

            if (HasProcessByName(processName))
                return true;

            return !string.Equals(processName, "locus", StringComparison.OrdinalIgnoreCase)
                && HasProcessByName("locus");
        }

        private static bool HasProcessByName(string processName)
        {
            if (string.IsNullOrEmpty(processName))
                return false;

            try
            {
                Process[] processes = Process.GetProcessesByName(processName);
                bool found = processes.Length > 0;
                for (int i = 0; i < processes.Length; i++)
                    processes[i].Dispose();
                return found;
            }
            catch
            {
                return false;
            }
        }

#if UNITY_EDITOR_WIN
        private static string FindWindowsLocusExecutable()
        {
            foreach (string path in GetWindowsRegistryExecutableCandidates())
            {
                string normalized = NormalizeLocusExecutablePath(path);
                if (!string.IsNullOrEmpty(normalized))
                    return normalized;
            }

            foreach (string path in GetWindowsFileSystemExecutableCandidates())
            {
                string normalized = NormalizeLocusExecutablePath(path);
                if (!string.IsNullOrEmpty(normalized))
                    return normalized;
            }

            return "";
        }

        private static IEnumerable<string> GetWindowsRegistryExecutableCandidates()
        {
            List<string> candidates = new List<string>();

            foreach (RegistryHive hive in new[] { RegistryHive.CurrentUser, RegistryHive.LocalMachine })
            {
                foreach (RegistryView view in new[] { RegistryView.Registry64, RegistryView.Registry32 })
                {
                    RegistryKey baseKey = null;
                    try
                    {
                        baseKey = RegistryKey.OpenBaseKey(hive, view);
                    }
                    catch
                    {
                    }

                    if (baseKey == null)
                        continue;

                    try
                    {
                        AddWindowsRegistryExecutableCandidates(candidates, baseKey);
                    }
                    finally
                    {
                        baseKey.Dispose();
                    }
                }
            }

            return candidates;
        }

        private static void AddWindowsRegistryExecutableCandidates(
            List<string> candidates,
            RegistryKey baseKey)
        {
            AddWindowsAppPathCandidates(candidates, baseKey);
            AddWindowsUninstallCandidates(candidates, baseKey);
        }

        private static void AddWindowsAppPathCandidates(
            List<string> candidates,
            RegistryKey baseKey)
        {
            using (RegistryKey key = baseKey.OpenSubKey(
                @"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\locus.exe"))
            {
                if (key == null)
                    return;

                candidates.Add(Convert.ToString(key.GetValue("")));
                candidates.Add(Convert.ToString(key.GetValue("Path")));
            }
        }

        private static void AddWindowsUninstallCandidates(
            List<string> candidates,
            RegistryKey baseKey)
        {
            using (RegistryKey uninstallKey = baseKey.OpenSubKey(
                @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"))
            {
                if (uninstallKey == null)
                    return;

                string[] subKeyNames;
                try
                {
                    subKeyNames = uninstallKey.GetSubKeyNames();
                }
                catch
                {
                    return;
                }

                for (int i = 0; i < subKeyNames.Length; i++)
                {
                    using (RegistryKey appKey = uninstallKey.OpenSubKey(subKeyNames[i]))
                    {
                        if (appKey == null || !IsLocusUninstallEntry(appKey))
                            continue;

                        candidates.Add(Convert.ToString(appKey.GetValue("DisplayIcon")));
                        candidates.Add(Convert.ToString(appKey.GetValue("InstallLocation")));
                    }
                }
            }
        }

        private static bool IsLocusUninstallEntry(RegistryKey appKey)
        {
            string displayName = Convert.ToString(appKey.GetValue("DisplayName")) ?? "";
            string publisher = Convert.ToString(appKey.GetValue("Publisher")) ?? "";

            if (string.Equals(displayName, "locus", StringComparison.OrdinalIgnoreCase))
                return true;

            return displayName.IndexOf("Locus", StringComparison.OrdinalIgnoreCase) >= 0
                && publisher.IndexOf("FarLocus", StringComparison.OrdinalIgnoreCase) >= 0;
        }

        private static IEnumerable<string> GetWindowsFileSystemExecutableCandidates()
        {
            string localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            string programFiles = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles);
            string programFilesX86 = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86);

            foreach (string root in new[] { localAppData, programFiles, programFilesX86 })
            {
                if (string.IsNullOrEmpty(root))
                    continue;

                yield return Path.Combine(root, "locus", "locus.exe");
                yield return Path.Combine(root, "Locus", "locus.exe");
                yield return Path.Combine(root, "Programs", "locus", "locus.exe");
                yield return Path.Combine(root, "Programs", "Locus", "locus.exe");
            }
        }

        private static string NormalizeLocusExecutablePath(string rawPath)
        {
            string path = ExtractWindowsPath(rawPath);
            if (string.IsNullOrEmpty(path))
                return "";

            try
            {
                path = Environment.ExpandEnvironmentVariables(path);

                if (Directory.Exists(path))
                    path = Path.Combine(path, "locus.exe");

                if (!File.Exists(path))
                    return "";

                if (!string.Equals(Path.GetFileName(path), "locus.exe", StringComparison.OrdinalIgnoreCase))
                    return "";

                return Path.GetFullPath(path);
            }
            catch
            {
                return "";
            }
        }

        private static string ExtractWindowsPath(string rawPath)
        {
            if (string.IsNullOrEmpty(rawPath))
                return "";

            string path = rawPath.Trim();
            if (path.Length == 0)
                return "";

            if (path[0] == '"')
            {
                int endQuote = path.IndexOf('"', 1);
                path = endQuote > 1 ? path.Substring(1, endQuote - 1) : path.Trim('"');
            }
            else
            {
                int exeIndex = path.IndexOf(".exe", StringComparison.OrdinalIgnoreCase);
                if (exeIndex >= 0)
                    path = path.Substring(0, exeIndex + 4);
            }

            int iconSuffixIndex = path.LastIndexOf(',');
            if (iconSuffixIndex > 0)
                path = path.Substring(0, iconSuffixIndex);

            return path.Trim();
        }
#endif

        private static long GetUnityMainHwnd()
        {
            IntPtr hwnd = IntPtr.Zero;

            try
            {
                hwnd = Process.GetCurrentProcess().MainWindowHandle;
            }
            catch
            {
            }

            if (hwnd == IntPtr.Zero)
                hwnd = GetActiveWindow();

            if (hwnd != IntPtr.Zero)
            {
                IntPtr root = GetAncestor(hwnd, 2);
                if (root != IntPtr.Zero)
                    hwnd = root;
            }

            return hwnd.ToInt64();
        }

        private static GUIContent CreateTitleContent()
        {
            return new GUIContent("Locus", GetTitleIcon());
        }

        private static Texture2D GetTitleIcon()
        {
            if (_titleIcon != null)
                return _titleIcon;

            _titleIcon = new Texture2D(16, 16, TextureFormat.RGBA32, false)
            {
                hideFlags = HideFlags.HideAndDontSave,
                filterMode = FilterMode.Point,
                wrapMode = TextureWrapMode.Clamp
            };

            Color clear = new Color(0f, 0f, 0f, 0f);
            Color line = EditorGUIUtility.isProSkin
                ? new Color(0.78f, 0.82f, 0.88f, 1f)
                : new Color(0.18f, 0.22f, 0.28f, 1f);
            Color accent = EditorGUIUtility.isProSkin
                ? new Color(0.46f, 0.63f, 0.95f, 1f)
                : new Color(0.18f, 0.36f, 0.72f, 1f);

            Color[] pixels = new Color[16 * 16];
            for (int i = 0; i < pixels.Length; i++)
                pixels[i] = clear;

            DrawIconCircle(pixels, 5, 8, 3, line);
            DrawIconCircle(pixels, 11, 8, 3, line);
            DrawIconLine(pixels, 6, 7, 10, 9, accent);
            DrawIconLine(pixels, 6, 9, 10, 7, accent);

            _titleIcon.SetPixels(pixels);
            _titleIcon.Apply(false, true);
            return _titleIcon;
        }

        private static void DrawIconCircle(Color[] pixels, int cx, int cy, int radius, Color color)
        {
            int radiusSquared = radius * radius;
            int innerSquared = (radius - 1) * (radius - 1);
            for (int y = cy - radius; y <= cy + radius; y++)
            {
                for (int x = cx - radius; x <= cx + radius; x++)
                {
                    int dx = x - cx;
                    int dy = y - cy;
                    int distanceSquared = dx * dx + dy * dy;
                    if (distanceSquared <= radiusSquared && distanceSquared >= innerSquared)
                        SetIconPixel(pixels, x, y, color);
                }
            }
        }

        private static void DrawIconLine(Color[] pixels, int x0, int y0, int x1, int y1, Color color)
        {
            int dx = Mathf.Abs(x1 - x0);
            int dy = -Mathf.Abs(y1 - y0);
            int sx = x0 < x1 ? 1 : -1;
            int sy = y0 < y1 ? 1 : -1;
            int error = dx + dy;

            while (true)
            {
                SetIconPixel(pixels, x0, y0, color);
                if (x0 == x1 && y0 == y1)
                    break;

                int doubledError = 2 * error;
                if (doubledError >= dy)
                {
                    error += dy;
                    x0 += sx;
                }

                if (doubledError <= dx)
                {
                    error += dx;
                    y0 += sy;
                }
            }
        }

        private static void SetIconPixel(Color[] pixels, int x, int y, Color color)
        {
            if (x < 0 || x >= 16 || y < 0 || y >= 16)
                return;

            pixels[y * 16 + x] = color;
        }

        private bool IsSelectedDockTab()
        {
            try
            {
                FieldInfo parentField = typeof(EditorWindow).GetField(
                    "m_Parent",
                    BindingFlags.Instance | BindingFlags.NonPublic);
                object parent = parentField != null ? parentField.GetValue(this) : null;
                if (parent == null)
                    return true;

                PropertyInfo actualViewProperty = parent.GetType().GetProperty(
                    "actualView",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                object actualView = actualViewProperty != null
                    ? actualViewProperty.GetValue(parent, null)
                    : null;

                return actualView == null || ReferenceEquals(actualView, this);
            }
            catch
            {
                return true;
            }
        }

        [DllImport("user32.dll")]
        private static extern IntPtr GetActiveWindow();

        [DllImport("user32.dll")]
        private static extern IntPtr GetAncestor(IntPtr hwnd, uint gaFlags);
    }
}
