using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace Locus
{
    [InitializeOnLoad]
    public static class LocusEmbedHttpServer
    {
        private const string HeaderSeparator = "\r\n\r\n";
        private const int ReadBufferSize = 32 * 1024;

        private static readonly object Sync = new object();

        private static TcpListener _listener;
        private static CancellationTokenSource _cts;
        private static Task _acceptTask;
        private static int _port;
        private static readonly Queue<Action> _mainThreadQueue = new Queue<Action>(32);

        private static string _projectPath = "";
        private static string _activeScenePath = "";
        private static string _unityVersion = "";
        private static bool _isPlaying;
        private static bool _isPaused;

        [Serializable]
        private sealed class PingResponse
        {
            public bool ok;
            public string runtime;
            public string message;
            public int port;
        }

        [Serializable]
        private sealed class EditorInfoResponse
        {
            public bool ok;
            public string runtime;
            public string unityVersion;
            public string projectPath;
            public string activeScenePath;
            public bool isPlaying;
            public bool isPaused;
            public int port;
        }

        [Serializable]
        private sealed class InvokeRequest
        {
            public string command;
            public InvokeArgs args;
        }

        [Serializable]
        private sealed class InvokeArgs
        {
            public string assetPath;
            public string filePath;
            public string scenePath;
            public string objectPath;
            public bool focusProjectWindow;
            public int line;
        }

        [Serializable]
        private sealed class WorkspaceFilePreviewResponse
        {
            public string displayPath;
            public bool exists;
            public string kind;
            public string language;
            public string snippet;
            public bool truncated;
            public bool isUnityAsset;
            public string preferredAction;
            public long fileSize;
            public int snippetStartLine;
            public string previewSuppressed;
        }

        [Serializable]
        private sealed class ErrorResponse
        {
            public bool ok;
            public string error;
        }

        static LocusEmbedHttpServer()
        {
            EditorApplication.update += OnEditorUpdate;
            EditorApplication.quitting += Stop;
            AssemblyReloadEvents.beforeAssemblyReload += Stop;
        }

        public static bool IsRunning
        {
            get
            {
                lock (Sync)
                    return _listener != null;
            }
        }

        public static int Port
        {
            get
            {
                lock (Sync)
                    return _port;
            }
        }

        public static string BaseUrl
        {
            get
            {
                int port = Port;
                return port > 0 ? "http://127.0.0.1:" + port : "";
            }
        }

        public static string EnsureStarted()
        {
            lock (Sync)
            {
                if (_listener != null)
                    return BaseUrl;

                UpdateSnapshot();

                _cts = new CancellationTokenSource();
                _listener = new TcpListener(IPAddress.Loopback, 0);
                _listener.Start();
                _port = ((IPEndPoint)_listener.LocalEndpoint).Port;
                _acceptTask = Task.Factory
                    .StartNew(
                        () => AcceptLoop(_cts.Token),
                        _cts.Token,
                        TaskCreationOptions.LongRunning,
                        TaskScheduler.Default)
                    .Unwrap();

                Debug.Log("[Locus] Embed HTTP bridge started: " + BaseUrl);
                return BaseUrl;
            }
        }

        public static void Stop()
        {
            TcpListener listener;
            CancellationTokenSource cts;

            lock (Sync)
            {
                listener = _listener;
                cts = _cts;

                _listener = null;
                _cts = null;
                _acceptTask = null;
                _port = 0;
                _mainThreadQueue.Clear();
            }

            try
            {
                if (cts != null)
                    cts.Cancel();
            }
            catch
            {
            }

            try
            {
                if (listener != null)
                    listener.Stop();
            }
            catch
            {
            }

            if (cts != null)
                cts.Dispose();
        }

        private static void UpdateSnapshot()
        {
            try
            {
                _projectPath = Directory.GetParent(Application.dataPath).FullName;
                _activeScenePath = EditorSceneManager.GetActiveScene().path ?? "";
                _unityVersion = Application.unityVersion;
                _isPlaying = EditorApplication.isPlaying;
                _isPaused = EditorApplication.isPaused;
            }
            catch
            {
            }
        }

        private static void OnEditorUpdate()
        {
            UpdateSnapshot();
            PumpMainThreadQueue();
        }

        private static Task<T> RunOnMainThread<T>(Func<T> action)
        {
            var tcs = new TaskCompletionSource<T>();
            lock (Sync)
            {
                _mainThreadQueue.Enqueue(delegate
                {
                    try
                    {
                        tcs.TrySetResult(action());
                    }
                    catch (Exception ex)
                    {
                        tcs.TrySetException(ex);
                    }
                });
            }
            return tcs.Task;
        }

        private static void PumpMainThreadQueue()
        {
            for (int i = 0; i < 16; i++)
            {
                Action action = null;
                lock (Sync)
                {
                    if (_mainThreadQueue.Count > 0)
                        action = _mainThreadQueue.Dequeue();
                }

                if (action == null)
                    return;

                action();
            }
        }

        private static async Task AcceptLoop(CancellationToken ct)
        {
            while (!ct.IsCancellationRequested)
            {
                TcpClient client = null;
                try
                {
                    TcpListener listener;
                    lock (Sync)
                        listener = _listener;

                    if (listener == null)
                        break;

                    client = await listener.AcceptTcpClientAsync();
                    TcpClient acceptedClient = client;
                    client = null;
                    _ = Task.Run(() => HandleClient(acceptedClient, ct), ct);
                }
                catch (ObjectDisposedException)
                {
                    break;
                }
                catch (SocketException)
                {
                    if (ct.IsCancellationRequested)
                        break;
                }
                catch (Exception ex)
                {
                    if (!ct.IsCancellationRequested)
                        Debug.LogWarning("[Locus] Embed HTTP bridge accept failed: " + ex.Message);
                }
                finally
                {
                    if (client != null)
                        client.Close();
                }
            }
        }

        private static async Task HandleClient(TcpClient client, CancellationToken ct)
        {
            using (client)
            {
                try
                {
                    client.NoDelay = true;
                    using (NetworkStream stream = client.GetStream())
                    {
                        byte[] buffer = new byte[ReadBufferSize];
                        int bytesRead = await stream.ReadAsync(buffer, 0, buffer.Length);
                        if (bytesRead <= 0)
                            return;

                        string requestText = Encoding.UTF8.GetString(buffer, 0, bytesRead);
                        string[] lines = requestText.Split(new[] { "\r\n", "\n" }, StringSplitOptions.None);
                        if (lines.Length == 0)
                            return;

                        string[] requestParts = lines[0].Split(' ');
                        if (requestParts.Length < 2)
                        {
                            await WriteJson(stream, 400, ToJsonError("bad_request"), ct);
                            return;
                        }

                        string method = requestParts[0].ToUpperInvariant();
                        string path = requestParts[1];
                        string body = ExtractBody(requestText);

                        if (method == "OPTIONS")
                        {
                            await WriteRaw(stream, 204, "text/plain; charset=utf-8", "", ct);
                            return;
                        }

                        await Dispatch(stream, method, path, body, ct);
                    }
                }
                catch
                {
                }
            }
        }

        private static async Task Dispatch(
            NetworkStream stream,
            string method,
            string path,
            string body,
            CancellationToken ct)
        {
            string normalizedPath = NormalizePath(path);

            if (method == "GET" && normalizedPath == "/ping")
            {
                await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                {
                    ok = true,
                    runtime = "unity",
                    message = "pong",
                    port = Port
                }), ct);
                return;
            }

            if (method == "GET" && normalizedPath == "/editor-info")
            {
                await WriteJson(stream, 200, JsonUtility.ToJson(new EditorInfoResponse
                {
                    ok = true,
                    runtime = "unity",
                    unityVersion = _unityVersion,
                    projectPath = _projectPath,
                    activeScenePath = _activeScenePath,
                    isPlaying = _isPlaying,
                    isPaused = _isPaused,
                    port = Port
                }), ct);
                return;
            }

            if (method == "POST" && normalizedPath == "/invoke")
            {
                InvokeRequest request = null;
                try
                {
                    request = JsonUtility.FromJson<InvokeRequest>(body);
                }
                catch
                {
                }

                if (request != null && request.command == "unity_embed_ping")
                {
                    await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                    {
                        ok = true,
                        runtime = "unity",
                        message = "pong",
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "unity_editor_info")
                {
                    await WriteJson(stream, 200, JsonUtility.ToJson(new EditorInfoResponse
                    {
                        ok = true,
                        runtime = "unity",
                        unityVersion = _unityVersion,
                        projectPath = _projectPath,
                        activeScenePath = _activeScenePath,
                        isPlaying = _isPlaying,
                        isPaused = _isPaused,
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "select_unity_asset")
                {
                    string assetPath = request.args != null ? request.args.assetPath : "";
                    bool focusProjectWindow = request.args != null && request.args.focusProjectWindow;
                    await RunOnMainThread(delegate
                    {
                        SelectUnityAsset(assetPath, focusProjectWindow);
                        return true;
                    });
                    await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                    {
                        ok = true,
                        runtime = "unity",
                        message = "ok",
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "open_unity_asset_inspector")
                {
                    string assetPath = request.args != null ? request.args.assetPath : "";
                    try
                    {
                        await RunOnMainThread(delegate
                        {
                            LocusAssetInspectorUtility.OpenLockedInspector(assetPath);
                            return true;
                        });
                    }
                    catch (Exception ex)
                    {
                        await WriteJson(stream, 400, ToJsonError(ex.Message), ct);
                        return;
                    }

                    await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                    {
                        ok = true,
                        runtime = "unity",
                        message = "ok",
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "select_unity_scene_object")
                {
                    string scenePath = request.args != null ? request.args.scenePath : "";
                    string objectPath = request.args != null ? request.args.objectPath : "";
                    try
                    {
                        await RunOnMainThread(delegate
                        {
                            LocusSceneObjectUtility.SelectSceneObject(scenePath, objectPath);
                            return true;
                        });
                    }
                    catch (Exception ex)
                    {
                        await WriteJson(stream, 400, ToJsonError(ex.Message), ct);
                        return;
                    }

                    await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                    {
                        ok = true,
                        runtime = "unity",
                        message = "ok",
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "open_unity_scene_object_inspector")
                {
                    string scenePath = request.args != null ? request.args.scenePath : "";
                    string objectPath = request.args != null ? request.args.objectPath : "";
                    try
                    {
                        await RunOnMainThread(delegate
                        {
                            LocusSceneObjectUtility.OpenSceneObjectInspector(scenePath, objectPath);
                            return true;
                        });
                    }
                    catch (Exception ex)
                    {
                        await WriteJson(stream, 400, ToJsonError(ex.Message), ct);
                        return;
                    }

                    await WriteJson(stream, 200, JsonUtility.ToJson(new PingResponse
                    {
                        ok = true,
                        runtime = "unity",
                        message = "ok",
                        port = Port
                    }), ct);
                    return;
                }

                if (request != null && request.command == "preview_workspace_file")
                {
                    string filePath = request.args != null ? request.args.filePath : "";
                    int line = request.args != null ? request.args.line : 0;
                    WorkspaceFilePreviewResponse preview = PreviewWorkspaceFile(filePath, line);
                    await WriteJson(stream, 200, JsonUtility.ToJson(preview), ct);
                    return;
                }

                await WriteJson(stream, 404, ToJsonError("unknown_command"), ct);
                return;
            }

            await WriteJson(stream, 404, ToJsonError("not_found"), ct);
        }

        private static void SelectUnityAsset(string assetPath, bool focusProjectWindow)
        {
            string normalized = (assetPath ?? "").Trim().Replace('\\', '/');
            if (string.IsNullOrEmpty(normalized))
                return;

            UnityEngine.Object obj = AssetDatabase.LoadMainAssetAtPath(normalized);
            if (obj == null)
                return;

            Selection.activeObject = obj;
            if (focusProjectWindow)
            {
                EditorGUIUtility.PingObject(obj);
                EditorUtility.FocusProjectWindow();
            }
        }

        private const long HoverPreviewMaxFileBytes = 256 * 1024;
        private const int HoverPreviewMaxLines = 50;
        private const long HoverPreviewMaxBytes = 5 * 1024;

        private static readonly HashSet<string> BinaryExts = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tga", ".psd", ".tif", ".tiff", ".exr", ".hdr",
            ".webp", ".ico", ".svg", ".fbx", ".obj", ".blend", ".dae", ".3ds", ".wav", ".mp3", ".ogg",
            ".aif", ".aiff", ".flac", ".mp4", ".avi", ".mov", ".wmv", ".webm", ".dll", ".so", ".dylib",
            ".exe", ".a", ".lib", ".ttf", ".otf", ".woff", ".woff2", ".zip", ".rar", ".7z", ".gz", ".tar",
            ".pdf", ".doc", ".docx", ".xls", ".xlsx"
        };

        private static readonly HashSet<string> CodeExts = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            ".cs", ".js", ".ts", ".jsx", ".tsx", ".py", ".rs", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
            ".lua", ".rb", ".sh", ".bat", ".ps1", ".json", ".xml", ".yaml", ".yml", ".toml", ".ini",
            ".cfg", ".html", ".css", ".scss", ".less", ".vue", ".svelte", ".md", ".txt", ".log", ".csv",
            ".csproj", ".sln", ".asmdef", ".asmref", ".shader", ".hlsl", ".glsl", ".cginc", ".compute"
        };

        private static WorkspaceFilePreviewResponse NotFoundPreview(string filePath)
        {
            return new WorkspaceFilePreviewResponse
            {
                displayPath = filePath ?? "",
                exists = false,
                kind = "not_found",
                language = "",
                snippet = "",
                truncated = false,
                isUnityAsset = false,
                preferredAction = "external",
                fileSize = 0,
                snippetStartLine = 1,
                previewSuppressed = ""
            };
        }

        private static WorkspaceFilePreviewResponse PreviewWorkspaceFile(string filePath, int line)
        {
            string normalizedPath = (filePath ?? "").Trim().Replace('\\', '/');
            if (!IsSafeWorkspaceRelativePath(normalizedPath))
                return NotFoundPreview(normalizedPath);

            string projectRoot = _projectPath ?? "";
            if (string.IsNullOrEmpty(projectRoot))
                return NotFoundPreview(normalizedPath);

            string fullPath;
            try
            {
                fullPath = Path.GetFullPath(Path.Combine(projectRoot, normalizedPath));
            }
            catch
            {
                return NotFoundPreview(normalizedPath);
            }

            if (!IsUnderProjectRoot(fullPath, projectRoot) || !File.Exists(fullPath))
                return NotFoundPreview(normalizedPath);

            FileInfo info;
            try
            {
                info = new FileInfo(fullPath);
            }
            catch
            {
                return NotFoundPreview(normalizedPath);
            }

            string ext = Path.GetExtension(fullPath).ToLowerInvariant();
            bool isBinary = BinaryExts.Contains(ext);
            bool isCode = CodeExts.Contains(ext);
            bool isUnityAsset = IsUnityAssetPath(normalizedPath) && !isCode;
            string preferredAction = isCode ? "editor" : isUnityAsset ? "unity" : "external";
            string language = LanguageFromExt(ext);

            if (info.Length > HoverPreviewMaxFileBytes)
            {
                return new WorkspaceFilePreviewResponse
                {
                    displayPath = normalizedPath,
                    exists = true,
                    kind = isBinary ? "binary" : "text",
                    language = language,
                    snippet = "",
                    truncated = true,
                    isUnityAsset = isUnityAsset,
                    preferredAction = preferredAction,
                    fileSize = info.Length,
                    snippetStartLine = 1,
                    previewSuppressed = "largeFile"
                };
            }

            if (isBinary)
            {
                return new WorkspaceFilePreviewResponse
                {
                    displayPath = normalizedPath,
                    exists = true,
                    kind = "binary",
                    language = language,
                    snippet = "",
                    truncated = false,
                    isUnityAsset = isUnityAsset,
                    preferredAction = preferredAction,
                    fileSize = info.Length,
                    snippetStartLine = 1,
                    previewSuppressed = ""
                };
            }

            int snippetStartLine;
            bool truncated;
            string snippet = ReadTextSnippet(fullPath, line, out snippetStartLine, out truncated);
            return new WorkspaceFilePreviewResponse
            {
                displayPath = normalizedPath,
                exists = true,
                kind = "text",
                language = language,
                snippet = snippet,
                truncated = truncated,
                isUnityAsset = isUnityAsset,
                preferredAction = preferredAction,
                fileSize = info.Length,
                snippetStartLine = snippetStartLine,
                previewSuppressed = ""
            };
        }

        private static bool IsSafeWorkspaceRelativePath(string path)
        {
            if (string.IsNullOrEmpty(path))
                return false;
            if (Path.IsPathRooted(path) || path.StartsWith("\\\\", StringComparison.Ordinal))
                return false;
            return !path.Contains("..");
        }

        private static bool IsUnderProjectRoot(string fullPath, string projectRoot)
        {
            string root = Path.GetFullPath(projectRoot).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            string target = Path.GetFullPath(fullPath);
            return string.Equals(target, root, StringComparison.OrdinalIgnoreCase)
                || target.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)
                || target.StartsWith(root + Path.AltDirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
        }

        private static bool IsUnityAssetPath(string path)
        {
            return path.StartsWith("Assets/", StringComparison.Ordinal)
                || path.StartsWith("Packages/", StringComparison.Ordinal);
        }

        private static string LanguageFromExt(string ext)
        {
            switch (ext)
            {
                case ".rs": return "rust";
                case ".ts":
                case ".tsx": return "typescript";
                case ".js":
                case ".jsx": return "javascript";
                case ".cs": return "csharp";
                case ".py": return "python";
                case ".go": return "go";
                case ".java": return "java";
                case ".c":
                case ".h": return "c";
                case ".cpp":
                case ".hpp": return "cpp";
                case ".lua": return "lua";
                case ".rb": return "ruby";
                case ".sh":
                case ".bat": return "shell";
                case ".json": return "json";
                case ".xml":
                case ".csproj":
                case ".sln": return "xml";
                case ".yaml":
                case ".yml": return "yaml";
                case ".toml": return "toml";
                case ".html": return "html";
                case ".css":
                case ".scss":
                case ".less": return "css";
                case ".vue":
                case ".svelte": return "html";
                case ".md": return "markdown";
                case ".sql": return "sql";
                case ".shader":
                case ".hlsl":
                case ".glsl":
                case ".cginc":
                case ".compute": return "hlsl";
                case ".unity":
                case ".prefab":
                case ".asset":
                case ".mat":
                case ".anim":
                case ".controller":
                case ".physicmaterial":
                case ".preset":
                case ".fontsettings":
                case ".guiskin":
                case ".mask":
                case ".flare":
                case ".rendertexture":
                case ".lighting":
                case ".meta":
                case ".locus-meta": return "yaml";
                default: return "";
            }
        }

        private static string ReadTextSnippet(
            string fullPath,
            int line,
            out int snippetStartLine,
            out bool truncated)
        {
            string[] lines;
            try
            {
                lines = File.ReadAllLines(fullPath);
            }
            catch
            {
                snippetStartLine = 1;
                truncated = false;
                return "";
            }

            int totalLines = lines.Length;
            int startIndex;
            int endIndex;
            if (line > 0)
            {
                int target = Math.Max(0, line - 1);
                int half = HoverPreviewMaxLines / 2;
                startIndex = Math.Max(0, target - half);
                endIndex = Math.Min(totalLines, startIndex + HoverPreviewMaxLines);
            }
            else
            {
                startIndex = 0;
                endIndex = Math.Min(totalLines, HoverPreviewMaxLines);
            }

            StringBuilder snippet = new StringBuilder();
            long byteCount = 0;
            truncated = endIndex < totalLines;
            for (int i = startIndex; i < endIndex; i++)
            {
                string current = lines[i] ?? "";
                long nextBytes = Encoding.UTF8.GetByteCount(current) + 1;
                if (byteCount + nextBytes > HoverPreviewMaxBytes)
                {
                    truncated = true;
                    break;
                }
                if (snippet.Length > 0)
                    snippet.Append('\n');
                snippet.Append(current);
                byteCount += nextBytes;
            }

            snippetStartLine = startIndex + 1;
            return snippet.ToString();
        }

        private static string NormalizePath(string path)
        {
            int queryIndex = path.IndexOf('?');
            return queryIndex >= 0 ? path.Substring(0, queryIndex) : path;
        }

        private static string ExtractBody(string requestText)
        {
            int index = requestText.IndexOf(HeaderSeparator, StringComparison.Ordinal);
            if (index < 0)
                return "";
            return requestText.Substring(index + HeaderSeparator.Length);
        }

        private static string ToJsonError(string error)
        {
            return JsonUtility.ToJson(new ErrorResponse
            {
                ok = false,
                error = error
            });
        }

        private static Task WriteJson(NetworkStream stream, int statusCode, string body, CancellationToken ct)
        {
            return WriteRaw(stream, statusCode, "application/json; charset=utf-8", body, ct);
        }

        private static async Task WriteRaw(
            NetworkStream stream,
            int statusCode,
            string contentType,
            string body,
            CancellationToken ct)
        {
            byte[] bodyBytes = Encoding.UTF8.GetBytes(body ?? "");
            string header =
                "HTTP/1.1 " + statusCode + " " + ReasonPhrase(statusCode) + "\r\n" +
                "Content-Type: " + contentType + "\r\n" +
                "Content-Length: " + bodyBytes.Length + "\r\n" +
                "Access-Control-Allow-Origin: *\r\n" +
                "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n" +
                "Access-Control-Allow-Headers: Content-Type\r\n" +
                "Connection: close\r\n\r\n";

            byte[] headerBytes = Encoding.ASCII.GetBytes(header);
            await stream.WriteAsync(headerBytes, 0, headerBytes.Length, ct);
            if (bodyBytes.Length > 0)
                await stream.WriteAsync(bodyBytes, 0, bodyBytes.Length, ct);
        }

        private static string ReasonPhrase(int statusCode)
        {
            switch (statusCode)
            {
                case 200:
                    return "OK";
                case 204:
                    return "No Content";
                case 400:
                    return "Bad Request";
                case 404:
                    return "Not Found";
                default:
                    return "OK";
            }
        }
    }
}
