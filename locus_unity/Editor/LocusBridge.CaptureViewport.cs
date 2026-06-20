using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Text;
using System.Threading.Tasks;
#if UNITY_EDITOR_WIN
using System.Runtime.InteropServices;
using Process = System.Diagnostics.Process;
#endif

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int CaptureViewportMaxLongEdge = 1280;

        private static async Task<PipeEnvelope> HandleCaptureViewport(string requestId, string message)
        {
            CaptureViewportRequest request = ParseCaptureViewportRequest(message);
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();

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

            try
            {
                return await LocusAsync.WithTimeout(tcs.Task, ExecuteTimeoutMs, "capture_viewport");
            }
            catch (TimeoutException)
            {
                return ErrorResponse(requestId, "capture_viewport timed out");
            }
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

            Texture2D texture = CaptureEditorWindowTexture(rect, width, height);
            Texture2D encodedTexture = null;
            try
            {
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

        private static Texture2D CaptureEditorWindowTexture(Rect rect, int width, int height)
        {
#if UNITY_EDITOR_WIN
            Texture2D nativeTexture;
            if (TryCaptureEditorWindowTextureWin32(rect, width, height, out nativeTexture))
                return nativeTexture;
#endif

            return CaptureEditorWindowTextureFromScreen(rect, width, height);
        }

        private static Texture2D CaptureEditorWindowTextureFromScreen(Rect rect, int width, int height)
        {
            Color[] pixels = UnityEditorInternal.InternalEditorUtility.ReadScreenPixel(
                new Vector2(rect.x, rect.y),
                width,
                height);
            Texture2D texture = new Texture2D(width, height, TextureFormat.RGB24, false);
            texture.SetPixels(pixels);
            texture.Apply(false);
            return texture;
        }

#if UNITY_EDITOR_WIN
        private const uint CapturePrintWindowRenderFullContent = 0x00000002;
        private const uint CaptureDibRgbColors = 0;
        private const uint CaptureBiRgb = 0;
        private const int CaptureSrcCopy = 0x00CC0020;

        // Capture the Unity process window off-screen, then crop to the target EditorWindow rect.
        private static bool TryCaptureEditorWindowTextureWin32(
            Rect rect,
            int width,
            int height,
            out Texture2D texture)
        {
            texture = null;

            IntPtr hwnd = FindUnityWindowForCapture(rect, width, height);
            if (hwnd == IntPtr.Zero)
                return false;

            CaptureNativeRect windowRect;
            if (!GetWindowRect(hwnd, out windowRect))
                return false;

            int windowWidth = windowRect.right - windowRect.left;
            int windowHeight = windowRect.bottom - windowRect.top;
            if (windowWidth <= 0 || windowHeight <= 0)
                return false;

            int cropX = Mathf.RoundToInt(rect.x) - windowRect.left;
            int cropY = Mathf.RoundToInt(rect.y) - windowRect.top;
            if (cropX < 0 || cropY < 0 || cropX + width > windowWidth || cropY + height > windowHeight)
                return false;

            byte[] bgra;
            if (!TryCaptureWindowBgra(hwnd, windowWidth, windowHeight, out bgra))
                return false;

            texture = CreateTextureFromBgraCrop(bgra, windowWidth, cropX, cropY, width, height);
            return texture != null;
        }

        private static IntPtr FindUnityWindowForCapture(Rect rect, int width, int height)
        {
            uint unityProcessId = (uint)Process.GetCurrentProcess().Id;
            CaptureNativeRect target = new CaptureNativeRect
            {
                left = Mathf.RoundToInt(rect.x),
                top = Mathf.RoundToInt(rect.y),
                right = Mathf.RoundToInt(rect.x) + width,
                bottom = Mathf.RoundToInt(rect.y) + height
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

                CaptureNativeRect windowRect;
                if (!GetWindowRect(hwnd, out windowRect))
                    return true;

                long intersection = IntersectionArea(target, windowRect);
                if (intersection <= 0)
                    return true;

                long area = RectArea(windowRect);
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

        private static bool TryCaptureWindowBgra(
            IntPtr hwnd,
            int width,
            int height,
            out byte[] bgra)
        {
            bgra = null;

            IntPtr sourceDc = GetWindowDC(hwnd);
            if (sourceDc == IntPtr.Zero)
                return false;

            IntPtr memoryDc = IntPtr.Zero;
            IntPtr bitmap = IntPtr.Zero;
            IntPtr previousBitmap = IntPtr.Zero;
            try
            {
                memoryDc = CreateCompatibleDC(sourceDc);
                if (memoryDc == IntPtr.Zero)
                    return false;

                bitmap = CreateCompatibleBitmap(sourceDc, width, height);
                if (bitmap == IntPtr.Zero)
                    return false;

                previousBitmap = SelectObject(memoryDc, bitmap);
                if (previousBitmap == IntPtr.Zero)
                    return false;

                bool painted = PrintWindow(hwnd, memoryDc, CapturePrintWindowRenderFullContent);
                if (!painted)
                    painted = BitBlt(memoryDc, 0, 0, width, height, sourceDc, 0, 0, CaptureSrcCopy);
                if (!painted)
                    return false;

                if (SelectObject(memoryDc, previousBitmap) == IntPtr.Zero)
                    return false;
                previousBitmap = IntPtr.Zero;

                CaptureBitmapInfo info = new CaptureBitmapInfo
                {
                    bmiHeader = new CaptureBitmapInfoHeader
                    {
                        biSize = (uint)Marshal.SizeOf(typeof(CaptureBitmapInfoHeader)),
                        biWidth = width,
                        biHeight = -height,
                        biPlanes = 1,
                        biBitCount = 32,
                        biCompression = CaptureBiRgb,
                        biSizeImage = (uint)(width * height * 4)
                    },
                    bmiColors = 0
                };

                byte[] pixels = new byte[width * height * 4];
                int scanLines = GetDIBits(
                    memoryDc,
                    bitmap,
                    0,
                    (uint)height,
                    pixels,
                    ref info,
                    CaptureDibRgbColors);
                if (scanLines != height)
                    return false;

                bgra = pixels;
                return true;
            }
            finally
            {
                if (previousBitmap != IntPtr.Zero)
                    SelectObject(memoryDc, previousBitmap);
                if (bitmap != IntPtr.Zero)
                    DeleteObject(bitmap);
                if (memoryDc != IntPtr.Zero)
                    DeleteDC(memoryDc);
                ReleaseDC(hwnd, sourceDc);
            }
        }

        private static Texture2D CreateTextureFromBgraCrop(
            byte[] bgra,
            int sourceWidth,
            int cropX,
            int cropY,
            int width,
            int height)
        {
            byte[] rgba = new byte[width * height * 4];
            for (int y = 0; y < height; y++)
            {
                int sourceRow = ((cropY + y) * sourceWidth + cropX) * 4;
                int targetRow = y * width * 4;
                for (int x = 0; x < width; x++)
                {
                    int sourceIndex = sourceRow + x * 4;
                    int targetIndex = targetRow + x * 4;
                    rgba[targetIndex] = bgra[sourceIndex + 2];
                    rgba[targetIndex + 1] = bgra[sourceIndex + 1];
                    rgba[targetIndex + 2] = bgra[sourceIndex];
                    rgba[targetIndex + 3] = 255;
                }
            }

            Texture2D texture = new Texture2D(width, height, TextureFormat.RGBA32, false);
            texture.LoadRawTextureData(rgba);
            texture.Apply(false);
            return texture;
        }

        private static long IntersectionArea(CaptureNativeRect a, CaptureNativeRect b)
        {
            int left = Math.Max(a.left, b.left);
            int top = Math.Max(a.top, b.top);
            int right = Math.Min(a.right, b.right);
            int bottom = Math.Min(a.bottom, b.bottom);
            if (right <= left || bottom <= top)
                return 0;
            return (long)(right - left) * (bottom - top);
        }

        private static long RectArea(CaptureNativeRect rect)
        {
            int width = Math.Max(0, rect.right - rect.left);
            int height = Math.Max(0, rect.bottom - rect.top);
            return (long)width * height;
        }

        private delegate bool CaptureEnumWindowsProc(IntPtr hwnd, IntPtr lParam);

        [StructLayout(LayoutKind.Sequential)]
        private struct CaptureNativeRect
        {
            public int left;
            public int top;
            public int right;
            public int bottom;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct CaptureBitmapInfoHeader
        {
            public uint biSize;
            public int biWidth;
            public int biHeight;
            public ushort biPlanes;
            public ushort biBitCount;
            public uint biCompression;
            public uint biSizeImage;
            public int biXPelsPerMeter;
            public int biYPelsPerMeter;
            public uint biClrUsed;
            public uint biClrImportant;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct CaptureBitmapInfo
        {
            public CaptureBitmapInfoHeader bmiHeader;
            public uint bmiColors;
        }

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumWindows(CaptureEnumWindowsProc lpEnumFunc, IntPtr lParam);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsWindowVisible(IntPtr hWnd);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetWindowRect(IntPtr hWnd, out CaptureNativeRect lpRect);

        [DllImport("user32.dll")]
        private static extern IntPtr GetWindowDC(IntPtr hWnd);

        [DllImport("user32.dll")]
        private static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);

        [DllImport("gdi32.dll")]
        private static extern IntPtr CreateCompatibleDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        private static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int cx, int cy);

        [DllImport("gdi32.dll")]
        private static extern IntPtr SelectObject(IntPtr hdc, IntPtr h);

        [DllImport("gdi32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DeleteObject(IntPtr ho);

        [DllImport("gdi32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DeleteDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool BitBlt(
            IntPtr hdc,
            int x,
            int y,
            int cx,
            int cy,
            IntPtr hdcSrc,
            int x1,
            int y1,
            int rop);

        [DllImport("gdi32.dll")]
        private static extern int GetDIBits(
            IntPtr hdc,
            IntPtr hbm,
            uint start,
            uint cLines,
            byte[] lpvBits,
            ref CaptureBitmapInfo lpbmi,
            uint usage);
#endif

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
