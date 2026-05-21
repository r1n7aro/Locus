using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Text;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int CaptureViewportMaxLongEdge = 1280;

        private static async Task<PipeEnvelope> HandleCaptureViewport(string requestId, string message)
        {
            CaptureViewportRequest request = ParseCaptureViewportRequest(message);
            var tcs = new TaskCompletionSource<PipeEnvelope>();

            PostToMainThread(delegate
            {
                try
                {
                    string target;
                    string title;
                    EditorWindow window = ResolveCaptureWindow(request, out target, out title);
                    window.Focus();
                    window.Repaint();

                    EditorApplication.delayCall += delegate
                    {
                        try
                        {
                            CaptureViewportResponse response = CaptureWindowPng(window, target, title);
                            tcs.SetResult(OkResponse(requestId, JsonUtility.ToJson(response)));
                        }
                        catch (Exception ex)
                        {
                            tcs.SetResult(ErrorResponse(requestId, ex.Message));
                        }
                    };
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.Message));
                }
            });

            return await tcs.Task;
        }

        private static CaptureViewportRequest ParseCaptureViewportRequest(string message)
        {
            string payload = (message ?? "").Trim();
            CaptureViewportRequest request = null;
            if (payload.StartsWith("{", StringComparison.Ordinal))
            {
                try
                {
                    request = JsonUtility.FromJson<CaptureViewportRequest>(payload);
                }
                catch (Exception ex)
                {
                    Debug.LogWarning("[Locus] Failed to parse capture_viewport payload: " + ex.Message);
                }
            }

            if (request == null)
                request = new CaptureViewportRequest { target = payload };

            request.target = (request.target ?? "").Trim().ToLowerInvariant();
            request.windowTitle = (request.windowTitle ?? "").Trim();
            return request;
        }

        private static EditorWindow ResolveCaptureWindow(
            CaptureViewportRequest request,
            out string normalizedTarget,
            out string title)
        {
            normalizedTarget = (request != null ? request.target : "").Trim().ToLowerInvariant();
            title = "";

            if (normalizedTarget == "game")
            {
                Type gameViewType = typeof(EditorWindow).Assembly.GetType("UnityEditor.GameView");
                if (gameViewType == null)
                    throw new InvalidOperationException("Unity GameView type is unavailable.");
                EditorWindow gameView = EditorWindow.GetWindow(gameViewType);
                title = WindowTitle(gameView);
                return gameView;
            }

            if (normalizedTarget == "scene")
            {
                SceneView sceneView = SceneView.lastActiveSceneView;
                if (sceneView == null)
                    sceneView = EditorWindow.GetWindow<SceneView>();
                title = WindowTitle(sceneView);
                return sceneView;
            }

            if (normalizedTarget == "editor_window")
            {
                string query = request != null ? request.windowTitle : "";
                EditorWindow window = FindCaptureEditorWindow(query);
                if (window == null)
                {
                    if (string.IsNullOrEmpty(query))
                        throw new InvalidOperationException("No focused Editor window is available to capture.");
                    throw new InvalidOperationException("Editor window was not found: " + query);
                }
                title = WindowTitle(window);
                return window;
            }

            throw new InvalidOperationException(
                "Invalid capture target: " + normalizedTarget + ". Allowed values: game, scene, editor_window.");
        }

        private static EditorWindow FindCaptureEditorWindow(string query)
        {
            query = (query ?? "").Trim();
            if (string.IsNullOrEmpty(query))
            {
                if (EditorWindow.focusedWindow != null)
                    return EditorWindow.focusedWindow;
                if (EditorWindow.mouseOverWindow != null)
                    return EditorWindow.mouseOverWindow;
                return null;
            }

            EditorWindow[] windows = Resources.FindObjectsOfTypeAll<EditorWindow>();
            foreach (EditorWindow window in windows)
            {
                if (window == null)
                    continue;
                if (WindowMatches(window, query, true))
                    return window;
            }
            foreach (EditorWindow window in windows)
            {
                if (window == null)
                    continue;
                if (WindowMatches(window, query, false))
                    return window;
            }
            return null;
        }

        private static bool WindowMatches(EditorWindow window, string query, bool exact)
        {
            string title = WindowTitle(window);
            Type type = window.GetType();
            string typeName = type != null ? type.Name : "";
            string fullName = type != null ? type.FullName : "";
            StringComparison comparison = StringComparison.OrdinalIgnoreCase;

            if (exact)
            {
                return string.Equals(title, query, comparison)
                    || string.Equals(typeName, query, comparison)
                    || string.Equals(fullName, query, comparison);
            }

            return title.IndexOf(query, comparison) >= 0
                || typeName.IndexOf(query, comparison) >= 0
                || fullName.IndexOf(query, comparison) >= 0;
        }

        private static string WindowTitle(EditorWindow window)
        {
            if (window == null)
                return "";
            if (window.titleContent != null && !string.IsNullOrEmpty(window.titleContent.text))
                return window.titleContent.text;
            Type type = window.GetType();
            return type != null ? type.Name : "";
        }

        private static CaptureViewportResponse CaptureWindowPng(
            EditorWindow window,
            string target,
            string title)
        {
            if (window == null)
                throw new InvalidOperationException("Editor window is unavailable.");

            Rect rect = window.position;
            int width = Mathf.Max(1, Mathf.RoundToInt(rect.width));
            int height = Mathf.Max(1, Mathf.RoundToInt(rect.height));
            if (width <= 1 || height <= 1)
                throw new InvalidOperationException("Editor window has no visible capture area.");

            Color[] pixels = UnityEditorInternal.InternalEditorUtility.ReadScreenPixel(
                new Vector2(rect.x, rect.y),
                width,
                height);
            Texture2D texture = new Texture2D(width, height, TextureFormat.RGB24, false);
            Texture2D encodedTexture = null;
            try
            {
                texture.SetPixels(pixels);
                texture.Apply(false);

                encodedTexture = ResizeForCapture(texture, CaptureViewportMaxLongEdge);
                byte[] png = encodedTexture.EncodeToPNG();
                string dir = Path.Combine(
                    Directory.GetParent(Application.dataPath).FullName,
                    "Library",
                    "Locus",
                    "Screenshots");
                Directory.CreateDirectory(dir);
                string fileName = "locus_" + SafeCaptureFileName(target) + "_" +
                    DateTime.UtcNow.ToString("yyyyMMdd_HHmmss_fff") + ".png";
                string path = Path.Combine(dir, fileName);
                File.WriteAllBytes(path, png);

                return new CaptureViewportResponse
                {
                    target = target,
                    title = title,
                    path = path,
                    width = encodedTexture.width,
                    height = encodedTexture.height,
                    originalWidth = width,
                    originalHeight = height,
                    mimeType = "image/png"
                };
            }
            finally
            {
                if (encodedTexture != null && !object.ReferenceEquals(encodedTexture, texture))
                    UnityEngine.Object.DestroyImmediate(encodedTexture);
                UnityEngine.Object.DestroyImmediate(texture);
            }
        }

        private static Texture2D ResizeForCapture(Texture2D source, int maxLongEdge)
        {
            int longEdge = Mathf.Max(source.width, source.height);
            if (maxLongEdge <= 0 || longEdge <= maxLongEdge)
                return source;

            float scale = (float)maxLongEdge / (float)longEdge;
            int width = Mathf.Max(1, Mathf.RoundToInt(source.width * scale));
            int height = Mathf.Max(1, Mathf.RoundToInt(source.height * scale));
            RenderTexture rt = RenderTexture.GetTemporary(width, height, 0, RenderTextureFormat.ARGB32);
            RenderTexture previous = RenderTexture.active;
            try
            {
                Graphics.Blit(source, rt);
                RenderTexture.active = rt;
                Texture2D resized = new Texture2D(width, height, TextureFormat.RGB24, false);
                resized.ReadPixels(new Rect(0, 0, width, height), 0, 0);
                resized.Apply(false);
                return resized;
            }
            finally
            {
                RenderTexture.active = previous;
                RenderTexture.ReleaseTemporary(rt);
            }
        }

        private static string SafeCaptureFileName(string value)
        {
            string input = string.IsNullOrEmpty(value) ? "viewport" : value;
            char[] invalid = Path.GetInvalidFileNameChars();
            StringBuilder sb = new StringBuilder(input.Length);
            for (int i = 0; i < input.Length; i++)
            {
                char ch = input[i];
                bool ok = true;
                for (int j = 0; j < invalid.Length; j++)
                {
                    if (ch == invalid[j])
                    {
                        ok = false;
                        break;
                    }
                }
                sb.Append(ok ? ch : '_');
            }
            return sb.ToString();
        }
    }
}
