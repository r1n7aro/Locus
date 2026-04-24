using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;

using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
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
        }

        [Serializable]
        private sealed class ErrorResponse
        {
            public bool ok;
            public string error;
        }

        static LocusEmbedHttpServer()
        {
            EditorApplication.update += UpdateSnapshot;
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

                await WriteJson(stream, 404, ToJsonError("unknown_command"), ct);
                return;
            }

            await WriteJson(stream, 404, ToJsonError("not_found"), ct);
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
