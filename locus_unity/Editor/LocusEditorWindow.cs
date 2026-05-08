using UnityEngine;
using UnityEditor;
using UnityEngine.UIElements;

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
        private const bool OverlaySyncEnabled = true;
        private const string PipeNamePrefix = "locus_tauri_unity_embed_";
        private const string FullPipeNamePrefix = @"\\.\pipe\";
        private const double SyncIntervalSeconds = 0.12d;
        private const double ResizeSyncIntervalSeconds = 1d / 60d;
        private const double ResizeBoostDurationSeconds = 0.35d;
        private const double AssetDragStateRefreshSeconds = 0.35d;
        private const double HeartbeatIntervalSeconds = 2d;
        private const double DesktopProbeIntervalSeconds = 2d;
        private const int PipeConnectTimeoutMs = 500;
        private const double ConsoleToolbarProbeIntervalSeconds = 0.5d;
        private const int MaxConsoleEntriesToSend = 200;
        private const int MaxConsoleCharsToSend = 60000;
        private const string ConsoleSendButtonName = "locus-console-send-button";
        private const string CloseReasonWindowClosed = "windowClosed";
        private const string CloseReasonEditorQuit = "editorQuit";
        private const string CloseReasonDomainReload = "domainReload";

        private static readonly Encoding Utf8NoBom = new UTF8Encoding(false);
        private static Texture2D _titleIcon;
        private static bool _lifecycleHooksRegistered;
        private static bool _assemblyReloadInProgress;
        private static bool _editorQuitting;
        private static double _nextConsoleToolbarProbeAt;
        private static Type _consoleWindowType;
        private static string _consoleSendButtonToken = "";

        private double _nextSyncAt;
        private double _resizeBoostUntil;
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
        private double _nextAssetDragStateAt;
        private string _lastAssetDragSignature = "";
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
        private volatile bool _assetDragStateSendInFlight;
        private string _connectedPipeName = "";

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
            public string reason;
            public DroppedAssetRef[] assetRefs;
            public string text;
            public ConsoleTextEntry[] textEntries;
            public string title;
            public string source;
        }

        [Serializable]
        private sealed class ConsoleTextEntry
        {
            public string title;
            public string text;
            public string source;
            public string level;
        }

        [Serializable]
        private sealed class DroppedAssetRef
        {
            public string path;
            public string kind;
            public string name;
            public string typeLabel;
            public string source;
        }

        [InitializeOnLoadMethod]
        private static void InitializeConsoleIntegration()
        {
            _consoleSendButtonToken = Guid.NewGuid().ToString("N");
            EditorApplication.update -= EnsureConsoleToolbarButtons;
            EditorApplication.update += EnsureConsoleToolbarButtons;
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

        [MenuItem("Assets/Send to Locus", false, 0)]
        private static void SendSelectedAssetsToLocusMenu()
        {
            SendSelectedRefsToLocus();
        }

        [MenuItem("Assets/Send to Locus", true)]
        private static bool ValidateSendSelectedAssetsToLocusMenu()
        {
            return BuildSelectedAssetRefs().Length > 0;
        }

        [MenuItem("GameObject/Send to Locus", false, 0)]
        private static void SendSelectedGameObjectsToLocusMenu()
        {
            SendSelectedRefsToLocus();
        }

        [MenuItem("GameObject/Send to Locus", true)]
        private static bool ValidateSendSelectedGameObjectsToLocusMenu()
        {
            return BuildSelectedAssetRefs().Length > 0;
        }

        private static void SendSelectedRefsToLocus()
        {
            DroppedAssetRef[] assetRefs = BuildSelectedAssetRefs();
            if (assetRefs.Length == 0)
                return;

            string json = JsonUtility.ToJson(new EmbedControlMessage
            {
                type = "assetDrop",
                assetRefs = assetRefs
            });
            string pipeName = GetControlPipeName();

            Task.Run(() =>
            {
                try
                {
                    WritePipeLineOnce(pipeName, json);
                }
                catch
                {
                }
            });
        }

        private static void SendConsoleToLocus()
        {
            ConsoleTextEntry[] consoleEntries = BuildConsoleTextEntries();
            if (consoleEntries.Length == 0)
            {
                return;
            }
            string consoleText = JoinConsoleTextEntries(consoleEntries);

            string json = JsonUtility.ToJson(new EmbedControlMessage
            {
                type = "consoleText",
                text = consoleText,
                textEntries = consoleEntries,
                title = "Unity Console",
                source = "unity-console"
            });
            string pipeName = GetControlPipeName();

            Task.Run(() =>
            {
                try
                {
                    WritePipeLineOnce(pipeName, json);
                }
                catch
                {
                }
            });
        }

        private static void EnsureConsoleToolbarButtons()
        {
            double now = EditorApplication.timeSinceStartup;
            if (now < _nextConsoleToolbarProbeAt)
                return;

            _nextConsoleToolbarProbeAt = now + ConsoleToolbarProbeIntervalSeconds;
            Type consoleWindowType = GetConsoleWindowType();
            if (consoleWindowType == null)
                return;

            UnityEngine.Object[] windows = Resources.FindObjectsOfTypeAll(consoleWindowType);
            if (windows == null || windows.Length == 0)
                return;

            foreach (UnityEngine.Object obj in windows)
            {
                EditorWindow window = obj as EditorWindow;
                if (window == null || window.rootVisualElement == null)
                    continue;

                AddConsoleSendButton(window.rootVisualElement);
            }
        }

        private static Type GetConsoleWindowType()
        {
            if (_consoleWindowType != null)
                return _consoleWindowType;

            _consoleWindowType = typeof(EditorWindow).Assembly.GetType("UnityEditor.ConsoleWindow");
            return _consoleWindowType;
        }

        private static void AddConsoleSendButton(VisualElement root)
        {
            Button existing = root.Q<Button>(ConsoleSendButtonName);
            if (existing != null)
            {
                if (string.Equals(Convert.ToString(existing.userData), _consoleSendButtonToken, StringComparison.Ordinal))
                {
                    existing.BringToFront();
                    return;
                }

                existing.RemoveFromHierarchy();
            }

            Button button = new Button()
            {
                name = ConsoleSendButtonName,
                text = "Send to Locus",
                tooltip = "Send Unity Console contents to Locus",
                userData = _consoleSendButtonToken
            };
            button.pickingMode = PickingMode.Position;
            button.style.position = Position.Absolute;
            button.style.left = 248;
            button.style.top = 1;
            button.style.width = 92;
            button.style.height = 18;
            button.style.fontSize = 11;
            button.style.paddingLeft = 4;
            button.style.paddingRight = 4;
            button.style.paddingTop = 0;
            button.style.paddingBottom = 0;
            button.style.marginLeft = 0;
            button.style.marginRight = 0;
            button.style.marginTop = 0;
            button.style.marginBottom = 0;
            button.style.borderTopLeftRadius = 0;
            button.style.borderTopRightRadius = 0;
            button.style.borderBottomLeftRadius = 0;
            button.style.borderBottomRightRadius = 0;
            button.style.borderLeftWidth = 1;
            button.style.borderRightWidth = 1;
            button.style.borderTopWidth = 1;
            button.style.borderBottomWidth = 1;
            button.style.unityTextAlign = TextAnchor.MiddleCenter;
            StyleConsoleToolbarButtonColors(button);
            button.RegisterCallback<MouseDownEvent>(HandleConsoleSendButtonMouseDown, TrickleDown.TrickleDown);
            root.Add(button);
            button.BringToFront();
        }

        private static void HandleConsoleSendButtonMouseDown(MouseDownEvent evt)
        {
            if (evt.button != 0)
                return;

            evt.StopImmediatePropagation();
            evt.PreventDefault();
            SendConsoleToLocus();
        }

        private static void StyleConsoleToolbarButtonColors(Button button)
        {
            Color background = EditorGUIUtility.isProSkin
                ? new Color(0.22f, 0.22f, 0.22f, 1f)
                : new Color(0.76f, 0.76f, 0.76f, 1f);
            Color border = EditorGUIUtility.isProSkin
                ? new Color(0.13f, 0.13f, 0.13f, 1f)
                : new Color(0.54f, 0.54f, 0.54f, 1f);
            Color text = EditorGUIUtility.isProSkin
                ? new Color(0.78f, 0.78f, 0.78f, 1f)
                : new Color(0.13f, 0.13f, 0.13f, 1f);

            button.style.backgroundColor = background;
            button.style.borderLeftColor = border;
            button.style.borderRightColor = border;
            button.style.borderTopColor = border;
            button.style.borderBottomColor = border;
            button.style.color = text;
        }

        private static ConsoleTextEntry[] BuildConsoleTextEntries()
        {
            ConsoleTextEntry[] entries = TryBuildConsoleTextEntriesFromLogEntries();
            if (entries == null || entries.Length == 0)
                return new ConsoleTextEntry[0];

            return entries;
        }

        private static string JoinConsoleTextEntries(ConsoleTextEntry[] entries)
        {
            if (entries == null || entries.Length == 0)
                return "";

            StringBuilder sb = new StringBuilder(Math.Min(MaxConsoleCharsToSend, entries.Length * 256));
            sb.AppendLine("Unity Console");
            bool hasEntry = false;
            for (int i = 0; i < entries.Length; i++)
            {
                ConsoleTextEntry entry = entries[i];
                if (entry == null || string.IsNullOrWhiteSpace(entry.text))
                    continue;
                if (hasEntry)
                    sb.AppendLine();
                sb.AppendLine(entry.text.TrimEnd());
                hasEntry = true;
                TrimStringBuilderStart(sb);
            }

            return TrimConsoleText(sb.ToString());
        }

        private static ConsoleTextEntry[] TryBuildConsoleTextEntriesFromLogEntries()
        {
            try
            {
                Type logEntriesType = FindEditorType("UnityEditor.LogEntries", "UnityEditorInternal.LogEntries");
                Type logEntryType = FindEditorType("UnityEditor.LogEntry", "UnityEditorInternal.LogEntry");
                if (logEntriesType == null)
                    return new ConsoleTextEntry[0];

                MethodInfo getCount = FindStaticMethod(logEntriesType, "GetCount", 0);
                if (getCount == null)
                    return new ConsoleTextEntry[0];

                int count = Convert.ToInt32(getCount.Invoke(null, null));
                if (count <= 0)
                    return new ConsoleTextEntry[0];

                MethodInfo startGettingEntries = FindStaticMethod(logEntriesType, "StartGettingEntries", 0);
                MethodInfo endGettingEntries = FindStaticMethod(logEntriesType, "EndGettingEntries", 0);
                MethodInfo getEntryInternal = FindStaticMethod(logEntriesType, "GetEntryInternal", 2);
                if (logEntryType != null && getEntryInternal != null)
                {
                    return BuildConsoleTextFromLogEntryObjects(
                        count,
                        logEntryType,
                        getEntryInternal,
                        startGettingEntries,
                        endGettingEntries);
                }

                MethodInfo getLinesAndMode = FindStaticMethod(logEntriesType, "GetLinesAndModeFromEntryInternal", 4);
                if (getLinesAndMode != null)
                    return BuildConsoleTextFromLinesAndMode(count, getLinesAndMode);
            }
            catch
            {
            }

            return new ConsoleTextEntry[0];
        }

        private static ConsoleTextEntry[] BuildConsoleTextFromLogEntryObjects(
            int count,
            Type logEntryType,
            MethodInfo getEntryInternal,
            MethodInfo startGettingEntries,
            MethodInfo endGettingEntries)
        {
            List<ConsoleTextEntry> entries = new List<ConsoleTextEntry>();
            int startIndex = Math.Max(0, count - MaxConsoleEntriesToSend);
            object logEntry = Activator.CreateInstance(logEntryType);

            try
            {
                if (startGettingEntries != null)
                    startGettingEntries.Invoke(null, null);

                for (int i = startIndex; i < count; i++)
                {
                    object result = getEntryInternal.Invoke(null, new[] { (object)i, logEntry });
                    if (result is bool && !(bool)result)
                        continue;

                    string condition = ReadStringMember(logEntry, "message", "condition");
                    string stackTrace = ReadStringMember(logEntry, "stacktrace", "stackTrace");
                    int mode = ReadIntMember(logEntry, "mode");
                    AddConsoleEntry(entries, LogModeLabel(mode), condition, stackTrace);
                    TrimConsoleEntries(entries);
                }
            }
            finally
            {
                if (endGettingEntries != null)
                    endGettingEntries.Invoke(null, null);
            }

            return entries.ToArray();
        }

        private static ConsoleTextEntry[] BuildConsoleTextFromLinesAndMode(int count, MethodInfo getLinesAndMode)
        {
            List<ConsoleTextEntry> entries = new List<ConsoleTextEntry>();
            int startIndex = Math.Max(0, count - MaxConsoleEntriesToSend);
            for (int i = startIndex; i < count; i++)
            {
                string lines;
                int mode;
                if (!TryGetLinesAndMode(getLinesAndMode, i, out lines, out mode))
                    continue;
                AddConsoleEntry(entries, LogModeLabel(mode), lines, "");
                TrimConsoleEntries(entries);
            }
            return entries.ToArray();
        }

        private static Type FindEditorType(params string[] names)
        {
            Assembly editorAssembly = typeof(EditorWindow).Assembly;
            foreach (string name in names)
            {
                Type type = editorAssembly.GetType(name);
                if (type != null)
                    return type;
            }
            return null;
        }

        private static bool TryGetLinesAndMode(MethodInfo method, int row, out string lines, out int mode)
        {
            lines = "";
            mode = 0;
            ParameterInfo[] parameters = method.GetParameters();
            if (parameters.Length != 4)
                return false;

            object[] args = new object[4];
            args[0] = row;
            args[1] = 1000;
            int modeIndex = -1;
            int textIndex = -1;

            for (int i = 2; i < parameters.Length; i++)
            {
                Type parameterType = parameters[i].ParameterType;
                Type valueType = parameterType.IsByRef ? parameterType.GetElementType() : parameterType;
                if (valueType == typeof(int) || (valueType != null && valueType.IsEnum))
                {
                    args[i] = valueType != null && valueType.IsEnum ? Enum.ToObject(valueType, 0) : (object)0;
                    modeIndex = i;
                }
                else if (valueType == typeof(string))
                {
                    args[i] = "";
                    textIndex = i;
                }
                else
                {
                    args[i] = null;
                }
            }

            method.Invoke(null, args);
            if (modeIndex >= 0)
                mode = Convert.ToInt32(args[modeIndex]);
            if (textIndex >= 0)
                lines = Convert.ToString(args[textIndex]) ?? "";
            return !string.IsNullOrWhiteSpace(lines);
        }

        private static MethodInfo FindStaticMethod(Type type, string name, int parameterCount)
        {
            MethodInfo[] methods = type.GetMethods(BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
            foreach (MethodInfo method in methods)
            {
                if (!string.Equals(method.Name, name, StringComparison.Ordinal))
                    continue;
                if (method.GetParameters().Length == parameterCount)
                    return method;
            }
            return null;
        }

        private static string ReadStringMember(object target, params string[] names)
        {
            if (target == null || names == null)
                return "";

            Type type = target.GetType();
            foreach (string name in names)
            {
                FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (field != null)
                    return Convert.ToString(field.GetValue(target)) ?? "";

                PropertyInfo property = type.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (property != null)
                    return Convert.ToString(property.GetValue(target, null)) ?? "";
            }
            return "";
        }

        private static int ReadIntMember(object target, params string[] names)
        {
            if (target == null || names == null)
                return 0;

            Type type = target.GetType();
            foreach (string name in names)
            {
                FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (field != null)
                    return ConvertMemberToInt(field.GetValue(target));

                PropertyInfo property = type.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (property != null)
                    return ConvertMemberToInt(property.GetValue(target, null));
            }
            return 0;
        }

        private static int ConvertMemberToInt(object value)
        {
            if (value == null)
                return 0;
            try
            {
                return Convert.ToInt32(value);
            }
            catch
            {
                int parsed;
                return int.TryParse(Convert.ToString(value), out parsed) ? parsed : 0;
            }
        }

        private static void AddConsoleEntry(List<ConsoleTextEntry> entries, string type, string condition, string stackTrace)
        {
            condition = (condition ?? "").TrimEnd();
            stackTrace = (stackTrace ?? "").TrimEnd();
            if (string.IsNullOrEmpty(condition) && string.IsNullOrEmpty(stackTrace))
                return;

            string level = string.IsNullOrEmpty(type) ? "Log" : type;
            StringBuilder sb = new StringBuilder(condition.Length + stackTrace.Length + level.Length + 8);
            sb.Append("[").Append(level).Append("] ");
            sb.AppendLine(condition);
            if (!string.IsNullOrEmpty(stackTrace))
                sb.AppendLine(stackTrace);

            entries.Add(new ConsoleTextEntry
            {
                title = ConsoleEntryTitle(level, condition),
                text = sb.ToString().TrimEnd(),
                source = "unity-console",
                level = level
            });
        }

        private static string ConsoleEntryTitle(string level, string condition)
        {
            string summary = FirstNonEmptyLine(condition);
            if (string.IsNullOrEmpty(summary))
                summary = "Unity Console";
            if (summary.Length > 96)
                summary = summary.Substring(0, 93) + "...";
            return "[" + (string.IsNullOrEmpty(level) ? "Log" : level) + "] " + summary;
        }

        private static string FirstNonEmptyLine(string text)
        {
            if (string.IsNullOrEmpty(text))
                return "";

            string[] lines = text.Split(new[] { "\r\n", "\n" }, StringSplitOptions.None);
            foreach (string line in lines)
            {
                string trimmed = line.Trim();
                if (!string.IsNullOrEmpty(trimmed))
                    return trimmed;
            }
            return "";
        }

        private static void TrimConsoleEntries(List<ConsoleTextEntry> entries)
        {
            int total = 0;
            for (int i = 0; i < entries.Count; i++)
            {
                ConsoleTextEntry entry = entries[i];
                total += entry == null || entry.text == null ? 0 : entry.text.Length;
            }

            while (total > MaxConsoleCharsToSend && entries.Count > 1)
            {
                ConsoleTextEntry removed = entries[0];
                total -= removed == null || removed.text == null ? 0 : removed.text.Length;
                entries.RemoveAt(0);
            }

            if (total <= MaxConsoleCharsToSend || entries.Count == 0)
                return;

            ConsoleTextEntry first = entries[0];
            if (first == null || string.IsNullOrEmpty(first.text))
                return;

            int overflow = total - MaxConsoleCharsToSend;
            if (overflow <= 0 || overflow >= first.text.Length)
                return;

            first.text = first.text.Substring(overflow).TrimStart();
        }

        private static string LogModeLabel(int mode)
        {
            const int Error = 1 << 0;
            const int Assert = 1 << 1;
            const int Log = 1 << 2;
            const int Fatal = 1 << 4;
            const int AssetImportError = 1 << 6;
            const int AssetImportWarning = 1 << 7;
            const int ScriptingError = 1 << 8;
            const int ScriptingWarning = 1 << 9;
            const int ScriptingLog = 1 << 10;
            const int ScriptCompileError = 1 << 11;
            const int ScriptCompileWarning = 1 << 12;
            const int ScriptingException = 1 << 17;
            const int GraphCompileError = 1 << 20;
            const int ScriptingAssertion = 1 << 21;
            const int VisualScriptingError = 1 << 22;

            const int ErrorMask =
                Error
                | Assert
                | Fatal
                | AssetImportError
                | ScriptingError
                | ScriptCompileError
                | ScriptingException
                | GraphCompileError
                | ScriptingAssertion
                | VisualScriptingError;
            const int WarningMask = AssetImportWarning | ScriptingWarning | ScriptCompileWarning;
            const int LogMask = Log | ScriptingLog;

            if ((mode & ErrorMask) != 0)
                return "Error";
            if ((mode & WarningMask) != 0)
                return "Warning";
            if ((mode & LogMask) != 0)
                return "Log";
            return "Log";
        }

        private static string TrimConsoleText(string text)
        {
            if (string.IsNullOrEmpty(text) || text.Length <= MaxConsoleCharsToSend)
                return text ?? "";

            return "[older console output truncated]\n" + text.Substring(text.Length - MaxConsoleCharsToSend);
        }

        private static void TrimStringBuilderStart(StringBuilder sb)
        {
            if (sb.Length <= MaxConsoleCharsToSend)
                return;

            sb.Remove(0, sb.Length - MaxConsoleCharsToSend);
        }

        private static void EnsureLifecycleHooks()
        {
            if (_lifecycleHooksRegistered)
                return;

            _lifecycleHooksRegistered = true;
            AssemblyReloadEvents.beforeAssemblyReload += OnBeforeAssemblyReload;
            AssemblyReloadEvents.afterAssemblyReload += OnAfterAssemblyReload;
            EditorApplication.quitting += OnEditorQuitting;
        }

        private static void OnBeforeAssemblyReload()
        {
            _assemblyReloadInProgress = true;
        }

        private static void OnAfterAssemblyReload()
        {
            _assemblyReloadInProgress = false;
        }

        private static void OnEditorQuitting()
        {
            _editorQuitting = true;
        }

        private void OnEnable()
        {
            EnsureLifecycleHooks();
            titleContent = CreateTitleContent();
            minSize = new Vector2(360f, 420f);
            RefreshDesktopState(true);
            if (OverlaySyncEnabled)
            {
                EditorApplication.update += SyncOverlay;
                SendOpenOrUpdate(true);
            }
        }

        private void OnDisable()
        {
            if (OverlaySyncEnabled)
            {
                EditorApplication.update -= SyncOverlay;
                SendClose(GetCloseReason());
            }
            DisconnectPipe();
        }

        private void OnFocus()
        {
            if (OverlaySyncEnabled)
                SendOpenOrUpdate(true);
        }

        private void OnGUI()
        {
            UpdateScreenRectFromGUI();
            HandleUnityObjectDrag();
            RefreshDesktopState(false);
            DrawPlaceholder();

            if (OverlaySyncEnabled && Event.current.type == EventType.Repaint)
                SendOpenOrUpdate(false);
        }

        private void SyncOverlay()
        {
            double now = EditorApplication.timeSinceStartup;
            if (now < _nextSyncAt)
                return;

            bool resizeBoostActive = IsResizeSyncBoostActive(now);
            _nextSyncAt = now + (resizeBoostActive ? ResizeSyncIntervalSeconds : SyncIntervalSeconds);
            RefreshDesktopState(false);
            SendOpenOrUpdate(false);
            SendAssetDragState(false);

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

        private void SendClose(string reason)
        {
            SendControlMessage(BuildMessage("close", false, reason), true);
            _sentOpen = false;
        }

        private void SendAssetDrop(DroppedAssetRef[] assetRefs)
        {
            if (assetRefs == null || assetRefs.Length == 0)
                return;

            SendControlMessage(new EmbedControlMessage
            {
                type = "assetDrop",
                assetRefs = assetRefs
            }, true);
        }

        private void SendAssetDragState(bool force)
        {
            DroppedAssetRef[] assetRefs = BuildDroppedAssetRefs();
            if (assetRefs.Length == 0)
            {
                if (_lastAssetDragSignature.Length > 0)
                {
                    _lastAssetDragSignature = "";
                    _nextAssetDragStateAt = 0d;
                    SendAssetDragStateMessage(assetRefs);
                }
                _lastAssetDragSignature = "";
                return;
            }

            double now = EditorApplication.timeSinceStartup;
            string signature = BuildAssetRefsSignature(assetRefs);
            if (!force && string.Equals(signature, _lastAssetDragSignature, StringComparison.Ordinal) && now < _nextAssetDragStateAt)
                return;

            _lastAssetDragSignature = signature;
            _nextAssetDragStateAt = now + AssetDragStateRefreshSeconds;
            SendAssetDragStateMessage(assetRefs);
        }

        private void SendAssetDragStateMessage(DroppedAssetRef[] assetRefs)
        {
            if (assetRefs == null || _assetDragStateSendInFlight)
                return;

            string json = JsonUtility.ToJson(new EmbedControlMessage
            {
                type = "assetDrag",
                assetRefs = assetRefs
            });
            string pipeName = GetControlPipeName();
            _assetDragStateSendInFlight = true;

            Task.Run(() =>
            {
                try
                {
                    WritePipeLine(pipeName, json);
                }
                catch
                {
                    DisconnectPipe();
                }
                finally
                {
                    _assetDragStateSendInFlight = false;
                }
            });
        }

        private string GetCloseReason()
        {
            if (_editorQuitting)
                return CloseReasonEditorQuit;
            if (_assemblyReloadInProgress)
                return CloseReasonDomainReload;
            return CloseReasonWindowClosed;
        }

        private EmbedControlMessage BuildMessage(string type, bool visible, string reason = "")
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
                parentHwnd = GetUnityHostHwnd(_screenX, _screenY, _screenWidth, _screenHeight),
                reason = reason ?? ""
            };
        }

        private void HandleUnityObjectDrag()
        {
            Event evt = Event.current;
            if (evt == null)
                return;
            if (evt.type != EventType.DragUpdated && evt.type != EventType.DragPerform)
                return;

            DroppedAssetRef[] assetRefs = BuildDroppedAssetRefs();
            if (assetRefs.Length == 0)
                return;

            DragAndDrop.visualMode = DragAndDropVisualMode.Link;
            if (evt.type == EventType.DragPerform)
            {
                DragAndDrop.AcceptDrag();
                SendAssetDrop(assetRefs);
            }
            evt.Use();
        }

        private static DroppedAssetRef[] BuildDroppedAssetRefs()
        {
            List<DroppedAssetRef> refs = new List<DroppedAssetRef>();
            HashSet<string> seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            UnityEngine.Object[] objects = DragAndDrop.objectReferences;
            if (objects != null)
            {
                foreach (UnityEngine.Object obj in objects)
                {
                    DroppedAssetRef assetRef = BuildDroppedObjectRef(obj);
                    AddDroppedAssetRef(refs, seen, assetRef);
                }
            }

            string[] paths = DragAndDrop.paths;
            if (paths != null)
            {
                foreach (string path in paths)
                {
                    string normalizedPath = NormalizeProjectRelativePath(path);
                    if (!IsSupportedUnityRefPath(normalizedPath))
                        continue;
                    AddDroppedAssetRef(refs, seen, new DroppedAssetRef
                    {
                        path = normalizedPath,
                        kind = "asset",
                        name = Path.GetFileNameWithoutExtension(normalizedPath),
                        typeLabel = "",
                        source = "unity"
                    });
                }
            }

            return refs.ToArray();
        }

        private static DroppedAssetRef[] BuildSelectedAssetRefs()
        {
            List<DroppedAssetRef> refs = new List<DroppedAssetRef>();
            HashSet<string> seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            UnityEngine.Object[] objects = Selection.objects;
            if (objects != null)
            {
                foreach (UnityEngine.Object obj in objects)
                {
                    DroppedAssetRef assetRef = BuildDroppedObjectRef(obj);
                    AddDroppedAssetRef(refs, seen, assetRef);
                }
            }

            string[] assetGuids = Selection.assetGUIDs;
            if (assetGuids != null)
            {
                foreach (string guid in assetGuids)
                {
                    string path = NormalizeUnityPath(AssetDatabase.GUIDToAssetPath(guid));
                    if (!IsSupportedUnityRefPath(path))
                        continue;

                    UnityEngine.Object obj = AssetDatabase.LoadMainAssetAtPath(path);
                    AddDroppedAssetRef(refs, seen, new DroppedAssetRef
                    {
                        path = path,
                        kind = "asset",
                        name = obj != null ? obj.name : Path.GetFileNameWithoutExtension(path),
                        typeLabel = obj != null ? obj.GetType().Name : "",
                        source = "unity"
                    });
                }
            }

            return refs.ToArray();
        }

        private static string BuildAssetRefsSignature(DroppedAssetRef[] assetRefs)
        {
            if (assetRefs == null || assetRefs.Length == 0)
                return "";

            StringBuilder sb = new StringBuilder(assetRefs.Length * 64);
            foreach (DroppedAssetRef assetRef in assetRefs)
            {
                if (assetRef == null)
                    continue;
                sb.Append(assetRef.kind).Append('\n').Append(assetRef.path).Append('\n');
            }
            return sb.ToString();
        }

        private static DroppedAssetRef BuildDroppedObjectRef(UnityEngine.Object obj)
        {
            if (obj == null)
                return null;

            string assetPath = NormalizeUnityPath(AssetDatabase.GetAssetPath(obj));
            if (IsSupportedUnityRefPath(assetPath))
            {
                return new DroppedAssetRef
                {
                    path = assetPath,
                    kind = "asset",
                    name = obj.name ?? Path.GetFileNameWithoutExtension(assetPath),
                    typeLabel = obj.GetType().Name,
                    source = "unity"
                };
            }

            GameObject gameObject = obj as GameObject;
            if (gameObject == null)
            {
                Component component = obj as Component;
                if (component != null)
                    gameObject = component.gameObject;
            }

            if (gameObject == null || !gameObject.scene.IsValid())
                return null;

            string scenePath = NormalizeUnityPath(gameObject.scene.path);
            if (string.IsNullOrEmpty(scenePath))
                return null;

            string hierarchyPath = BuildHierarchyPath(gameObject.transform);
            if (string.IsNullOrEmpty(hierarchyPath))
                return null;

            return new DroppedAssetRef
            {
                path = scenePath + "/" + hierarchyPath,
                kind = "sceneObject",
                name = gameObject.name,
                typeLabel = "GameObject",
                source = "unity"
            };
        }

        private static void AddDroppedAssetRef(
            List<DroppedAssetRef> refs,
            HashSet<string> seen,
            DroppedAssetRef assetRef)
        {
            if (assetRef == null || string.IsNullOrEmpty(assetRef.path))
                return;

            string key = assetRef.kind + "\n" + assetRef.path;
            if (seen.Contains(key))
                return;

            seen.Add(key);
            refs.Add(assetRef);
        }

        private static string BuildHierarchyPath(Transform transform)
        {
            if (transform == null)
                return "";

            List<string> parts = new List<string>();
            Transform current = transform;
            while (current != null)
            {
                parts.Add(current.name);
                current = current.parent;
            }
            parts.Reverse();
            return string.Join("/", parts.ToArray());
        }

        private static string NormalizeProjectRelativePath(string path)
        {
            string normalized = NormalizeUnityPath(path);
            if (string.IsNullOrEmpty(normalized))
                return "";

            DirectoryInfo projectRootInfo = Directory.GetParent(Application.dataPath);
            if (projectRootInfo == null)
                return normalized;

            string projectRoot = projectRootInfo.FullName.Replace('\\', '/');
            if (normalized.StartsWith(projectRoot + "/", StringComparison.OrdinalIgnoreCase))
                normalized = normalized.Substring(projectRoot.Length + 1);

            return NormalizeUnityPath(normalized);
        }

        private static string NormalizeUnityPath(string path)
        {
            return string.IsNullOrEmpty(path)
                ? ""
                : path.Trim().Replace('\\', '/').TrimEnd('/');
        }

        private static bool IsSupportedUnityRefPath(string path)
        {
            return path.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                || path.StartsWith("Packages/", StringComparison.OrdinalIgnoreCase)
                || path.StartsWith("ProjectSettings/", StringComparison.OrdinalIgnoreCase);
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
            int nextX = Mathf.RoundToInt(topLeft.x * scale);
            int nextY = Mathf.RoundToInt(topLeft.y * scale);
            int nextWidth = Mathf.Max(1, Mathf.RoundToInt((bottomRight.x - topLeft.x) * scale));
            int nextHeight = Mathf.Max(1, Mathf.RoundToInt((bottomRight.y - topLeft.y) * scale));
            bool changed = !_hasScreenRect
                || _screenX != nextX
                || _screenY != nextY
                || _screenWidth != nextWidth
                || _screenHeight != nextHeight;

            _screenX = nextX;
            _screenY = nextY;
            _screenWidth = nextWidth;
            _screenHeight = nextHeight;
            _hasScreenRect = true;

            if (changed)
                MarkResizeSyncBoost();
        }

        private void MarkResizeSyncBoost()
        {
            double now = EditorApplication.timeSinceStartup;
            _resizeBoostUntil = Math.Max(_resizeBoostUntil, now + ResizeBoostDurationSeconds);
            if (_nextSyncAt > now)
                _nextSyncAt = now;
        }

        private bool IsResizeSyncBoostActive(double now)
        {
            return now < _resizeBoostUntil;
        }

        private void SendControlMessage(EmbedControlMessage message, bool force)
        {
            if (_sendInFlight && !force)
                return;

            string json = JsonUtility.ToJson(message);
            string pipeName = GetControlPipeName();
            _sendInFlight = true;
            bool isGeometryMessage = message.type == "open" || message.type == "update";
            bool isAssetDropMessage = message.type == "assetDrop";

            Task.Run(() =>
            {
                try
                {
                    WritePipeLine(pipeName, json);

                    if (isGeometryMessage)
                    {
                        _sentOpen = true;
                        RecordLastSent(message);
                        _failedSends = 0;
                        _statusMessage = "Overlay signal sent.";
                    }
                    else if (isAssetDropMessage)
                    {
                        _failedSends = 0;
                        _statusMessage = "Asset reference sent.";
                    }
                }
                catch (Exception ex)
                {
                    DisconnectPipe();
                    if (isGeometryMessage)
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

        private void WritePipeLine(string pipeName, string json)
        {
            lock (_pipeLock)
            {
                EnsurePipeConnected(pipeName);
                _pipeWriter.WriteLine(json);
                _pipeWriter.Flush();
            }
        }

        private static void WritePipeLineOnce(string pipeName, string json)
        {
            using (NamedPipeClientStream client = new NamedPipeClientStream(
                ".",
                pipeName,
                PipeDirection.Out,
                PipeOptions.Asynchronous))
            {
                client.Connect(PipeConnectTimeoutMs);
                using (StreamWriter writer = new StreamWriter(client, Utf8NoBom, 4096))
                {
                    writer.NewLine = "\n";
                    writer.AutoFlush = true;
                    writer.WriteLine(json);
                    writer.Flush();
                }
            }
        }

        private void EnsurePipeConnected(string pipeName)
        {
            if (_pipeClient != null
                && _pipeClient.IsConnected
                && _pipeWriter != null
                && string.Equals(_connectedPipeName, pipeName, StringComparison.Ordinal))
                return;

            DisconnectPipe();
            _pipeClient = new NamedPipeClientStream(
                ".",
                pipeName,
                PipeDirection.Out,
                PipeOptions.Asynchronous);
            _pipeClient.Connect(PipeConnectTimeoutMs);
            _connectedPipeName = pipeName;
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
                _connectedPipeName = "";
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
            GUIStyle executablePathStyle = new GUIStyle(EditorStyles.miniLabel)
            {
                wordWrap = true,
                clipping = TextClipping.Clip
            };
            string executablePathText = GetDesktopExecutablePathText();
            float executablePathHeight = string.IsNullOrEmpty(executablePathText)
                ? 0f
                : Mathf.Max(
                    18f,
                    executablePathStyle.CalcHeight(new GUIContent(executablePathText), inner.width));
            Rect executablePathRect = new Rect(
                inner.x,
                pipeRect.yMax + 8f,
                inner.width,
                executablePathHeight);
            float buttonY = string.IsNullOrEmpty(executablePathText)
                ? pipeRect.yMax + 12f
                : executablePathRect.yMax + 10f;
            Rect buttonRect = new Rect(
                inner.x,
                buttonY,
                Mathf.Min(116f, inner.width),
                24f);

            GUI.Label(titleRect, "Locus", EditorStyles.boldLabel);
            GUI.Label(statusRect, _statusMessage, EditorStyles.wordWrappedLabel);
            EditorGUI.SelectableLabel(pipeRect, GetFullControlPipeName(), EditorStyles.miniLabel);
            if (!string.IsNullOrEmpty(executablePathText))
                EditorGUI.SelectableLabel(executablePathRect, executablePathText, executablePathStyle);

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

        private string GetDesktopExecutablePathText()
        {
            if (!_desktopInstall.IsInstalled || string.IsNullOrEmpty(_desktopInstall.ExecutablePath))
                return "";

            return "EXE: " + _desktopInstall.ExecutablePath;
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
            _statusMessage = "Starting Locus desktop: " + executablePath;

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

        private static long GetUnityHostHwnd(int screenX, int screenY, int width, int height)
        {
#if UNITY_EDITOR_WIN
            IntPtr host = FindUnityHostWindowForRect(screenX, screenY, width, height);
            if (host != IntPtr.Zero)
                return host.ToInt64();
#endif

            return GetUnityMainHwnd();
        }

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

#if UNITY_EDITOR_WIN
        private static IntPtr FindUnityHostWindowForRect(int screenX, int screenY, int width, int height)
        {
            if (width <= 0 || height <= 0)
                return IntPtr.Zero;

            uint unityProcessId = (uint)Process.GetCurrentProcess().Id;
            NativeRect target = new NativeRect
            {
                left = screenX,
                top = screenY,
                right = screenX + width,
                bottom = screenY + height
            };
            IntPtr bestHwnd = IntPtr.Zero;
            long bestIntersection = 0;
            long bestArea = long.MaxValue;

            EnumWindows(delegate (IntPtr hwnd, IntPtr lParam)
            {
                if (!IsWindowVisible(hwnd))
                    return true;

                uint processId;
                GetWindowThreadProcessId(hwnd, out processId);
                if (processId != unityProcessId)
                    return true;

                NativeRect rect;
                if (!GetWindowRect(hwnd, out rect))
                    return true;

                long intersection = IntersectionArea(target, rect);
                if (intersection <= 0)
                    return true;

                long area = RectArea(rect);
                if (intersection > bestIntersection
                    || (intersection == bestIntersection && area < bestArea))
                {
                    bestHwnd = hwnd;
                    bestIntersection = intersection;
                    bestArea = area;
                }

                return true;
            }, IntPtr.Zero);

            return bestHwnd;
        }

        private static long IntersectionArea(NativeRect a, NativeRect b)
        {
            int left = Math.Max(a.left, b.left);
            int top = Math.Max(a.top, b.top);
            int right = Math.Min(a.right, b.right);
            int bottom = Math.Min(a.bottom, b.bottom);
            if (right <= left || bottom <= top)
                return 0;
            return (long)(right - left) * (bottom - top);
        }

        private static long RectArea(NativeRect rect)
        {
            int width = Math.Max(0, rect.right - rect.left);
            int height = Math.Max(0, rect.bottom - rect.top);
            return (long)width * height;
        }
#endif

        private static string GetProjectPath()
        {
            try
            {
                DirectoryInfo projectDir = Directory.GetParent(Application.dataPath);
                return projectDir != null ? projectDir.FullName : "";
            }
            catch
            {
                return "";
            }
        }

        private static string GetControlPipeName()
        {
            string projectPath = GetProjectPath();
            string sanitized = string.IsNullOrEmpty(projectPath)
                ? "unknown"
                : projectPath
                    .TrimEnd('\\', '/')
                    .Replace('\\', '_')
                    .Replace('/', '_')
                    .Replace(':', '_')
                    .Replace(' ', '_');

            return PipeNamePrefix + sanitized;
        }

        private static string GetFullControlPipeName()
        {
            return FullPipeNamePrefix + GetControlPipeName();
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

#if UNITY_EDITOR_WIN
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeRect
        {
            public int left;
            public int top;
            public int right;
            public int bottom;
        }

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsWindowVisible(IntPtr hWnd);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetWindowRect(IntPtr hWnd, out NativeRect lpRect);
#endif
    }
}
