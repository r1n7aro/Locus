using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Reflection;
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
        private const int CaptureViewportDefaultMaxLongEdge = 1280;
        private const int CaptureViewportMaxAllowedLongEdge = 8192;
        private const int CaptureViewportLayoutRetryCount = 5;

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

                    ScheduleCaptureViewport(
                        requestId,
                        window,
                        target,
                        title,
                        request.maxLongEdge,
                        CaptureViewportLayoutRetryCount,
                        tcs);
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

        private static void ScheduleCaptureViewport(
            string requestId,
            EditorWindow window,
            string target,
            string title,
            int maxLongEdge,
            int layoutRetriesRemaining,
            TaskCompletionSource<PipeEnvelope> tcs)
        {
            EditorApplication.delayCall += delegate
            {
                try
                {
                    CaptureViewportResponse response = CaptureWindowPng(
                        window,
                        target,
                        title,
                        maxLongEdge,
                        layoutRetriesRemaining <= 0);
                    tcs.TrySetResult(OkResponse(requestId, JsonUtility.ToJson(response)));
                }
                catch (CaptureViewportLayoutException ex)
                {
                    if (layoutRetriesRemaining > 0 && window != null)
                    {
                        window.Focus();
                        window.Repaint();
                        ScheduleCaptureViewport(
                            requestId,
                            window,
                            target,
                            title,
                            maxLongEdge,
                            layoutRetriesRemaining - 1,
                            tcs);
                        return;
                    }

                    tcs.TrySetResult(ErrorResponse(requestId, ex.Message));
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult(ErrorResponse(requestId, ex.Message));
                }
            };
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

            if (payload.StartsWith("{", StringComparison.Ordinal)
                && payload.IndexOf("\"maxLongEdge\"", StringComparison.Ordinal) < 0)
            {
                request.maxLongEdge = CaptureViewportDefaultMaxLongEdge;
            }

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
            string title,
            int maxLongEdge,
            bool degradeToWindowOnLayoutFailure)
        {
            if (window == null)
                throw new InvalidOperationException("Editor window is unavailable.");
            if (maxLongEdge < 0 || maxLongEdge > CaptureViewportMaxAllowedLongEdge)
            {
                throw new InvalidOperationException(
                    "Invalid maxLongEdge. Expected an integer from 0 to "
                    + CaptureViewportMaxAllowedLongEdge + ".");
            }

            CaptureViewportRegion region = ResolveCaptureViewportRegion(
                window,
                target,
                degradeToWindowOnLayoutFailure);

            Texture2D texture = CaptureEditorWindowTexture(region);
            int sourceWidth = texture.width;
            int sourceHeight = texture.height;
            Texture2D encodedTexture = null;
            try
            {
                encodedTexture = ResizeForCapture(texture, maxLongEdge);
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
                    originalWidth = sourceWidth,
                    originalHeight = sourceHeight,
                    sourceWidth = sourceWidth,
                    sourceHeight = sourceHeight,
                    outputWidth = encodedTexture.width,
                    outputHeight = encodedTexture.height,
                    maxLongEdge = maxLongEdge,
                    pixelsPerPoint = region.pixelsPerPoint,
                    captureArea = region.captureArea,
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

        private struct CaptureViewportRegion
        {
            public Rect screenPoints;
            public RectInt screenPixels;
            public float pixelsPerPoint;
            public string captureArea;
            public bool hasContainer;
            public Rect containerScreenPoints;
        }

        private sealed class CaptureViewportLayoutException : InvalidOperationException
        {
            public CaptureViewportLayoutException(string message) : base(message)
            {
            }
        }

        private static CaptureViewportRegion ResolveCaptureViewportRegion(
            EditorWindow window,
            string target,
            bool degradeToWindowOnLayoutFailure)
        {
            // Anchor viewport rects on the host view's screen rect, mirroring
            // how Unity itself places them: viewInParent and worldBound are in
            // host/panel space, while EditorWindow.position has the tab border
            // stripped by DockArea and would double-count it.
            Rect hostScreenRect;
            bool hasHost = TryGetCaptureHostScreenRect(window, out hostScreenRect);
            Rect baseScreenRect = hasHost ? hostScreenRect : window.position;
            Rect boundsRect = new Rect(0f, 0f, baseScreenRect.width, baseScreenRect.height);

            string captureArea;
            Rect localRect;
            try
            {
                localRect = ResolveCaptureLocalRect(window, target, hasHost, boundsRect, out captureArea);
            }
            catch (CaptureViewportLayoutException)
            {
                if (!degradeToWindowOnLayoutFailure)
                    throw;
                captureArea = "window";
                localRect = boundsRect;
            }

            localRect = ClampCaptureLocalRect(localRect, boundsRect);
            if (localRect.width <= 1f || localRect.height <= 1f)
                throw new InvalidOperationException("Editor window has no visible capture area.");

            float pixelsPerPoint = CaptureWindowPixelsPerPoint(window);
            Rect screenPoints = new Rect(
                baseScreenRect.x + localRect.x,
                baseScreenRect.y + localRect.y,
                localRect.width,
                localRect.height);
            RectInt screenPixels = PointsToCapturePixelRect(screenPoints, pixelsPerPoint);
            if (screenPixels.width <= 1 || screenPixels.height <= 1)
                throw new InvalidOperationException("Editor window has no visible capture area.");

            Rect containerScreenPoints;
            bool hasContainer = TryGetCaptureContainerScreenRect(window, out containerScreenPoints);

            return new CaptureViewportRegion
            {
                screenPoints = screenPoints,
                screenPixels = screenPixels,
                pixelsPerPoint = pixelsPerPoint,
                captureArea = captureArea,
                hasContainer = hasContainer,
                containerScreenPoints = containerScreenPoints
            };
        }

        private static Rect ResolveCaptureLocalRect(
            EditorWindow window,
            string target,
            bool hostSpace,
            Rect boundsRect,
            out string captureArea)
        {
            Rect localRect;
            if (target == "game")
            {
                captureArea = "game_viewport";
                if (hostSpace && TryReadCaptureRectProperty(window, "viewInParent", out localRect))
                    return ClampGameCaptureRect(window, localRect, boundsRect, true);

                if (TryReadCaptureRectProperty(window, "viewInWindow", out localRect))
                {
                    // viewInWindow is in the window's own GUI space; shift it
                    // into host space the same way GameView.viewInParent does.
                    if (hostSpace)
                        localRect = OffsetCaptureRectByHostBorder(window, localRect);
                    return ClampGameCaptureRect(window, localRect, boundsRect, hostSpace);
                }

                throw new CaptureViewportLayoutException(
                    "Unity game viewport layout is not ready for capture.");
            }

            if (target == "scene")
            {
                captureArea = "scene_viewport";
                if (hostSpace
                    && TryReadNestedCaptureRectProperty(
                        window,
                        "cameraViewVisualElement",
                        "worldBound",
                        out localRect))
                {
                    return localRect;
                }

                throw new CaptureViewportLayoutException(
                    "Unity scene viewport layout is not ready for capture.");
            }

            captureArea = "window";
            return boundsRect;
        }

        private static bool TryGetCaptureHostScreenRect(EditorWindow window, out Rect screenRect)
        {
            screenRect = new Rect();
            return TryReadCaptureRectProperty(
                GetCaptureHostView(window),
                "screenPosition",
                out screenRect);
        }

        private static bool TryGetCaptureContainerScreenRect(EditorWindow window, out Rect screenRect)
        {
            screenRect = new Rect();
            object host = GetCaptureHostView(window);
            if (host == null)
                return false;

            try
            {
                PropertyInfo windowProperty = FindCaptureInstanceProperty(host.GetType(), "window");
                object container = windowProperty != null ? windowProperty.GetValue(host, null) : null;
                return TryReadCaptureRectProperty(container, "position", out screenRect);
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to resolve capture container window: " + ex.Message);
                return false;
            }
        }

        private static object GetCaptureHostView(EditorWindow window)
        {
            if (window == null)
                return null;

            try
            {
                FieldInfo parentField = typeof(EditorWindow).GetField(
                    "m_Parent",
                    BindingFlags.Instance | BindingFlags.NonPublic);
                return parentField != null ? parentField.GetValue(window) : null;
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to resolve capture host view: " + ex.Message);
                return null;
            }
        }

        private static bool TryReadNestedCaptureRectProperty(
            object instance,
            string propertyName,
            string rectPropertyName,
            out Rect rect)
        {
            rect = new Rect();
            if (instance == null)
                return false;

            try
            {
                PropertyInfo property = FindCaptureInstanceProperty(instance.GetType(), propertyName);
                object nested = property != null ? property.GetValue(instance, null) : null;
                return TryReadCaptureRectProperty(nested, rectPropertyName, out rect);
            }
            catch (Exception ex)
            {
                Debug.LogWarning(
                    "[Locus] Failed to resolve capture viewport property '"
                    + propertyName + "." + rectPropertyName + "': " + ex.Message);
                return false;
            }
        }

        private static bool TryReadCaptureRectProperty(
            object instance,
            string propertyName,
            out Rect rect)
        {
            rect = new Rect();
            if (instance == null)
                return false;

            try
            {
                PropertyInfo property = FindCaptureInstanceProperty(instance.GetType(), propertyName);
                if (property == null || property.PropertyType != typeof(Rect))
                    return false;

                object value = property.GetValue(instance, null);
                if (!(value is Rect))
                    return false;

                rect = (Rect)value;
                return IsFiniteCaptureRect(rect) && rect.width > 0f && rect.height > 0f;
            }
            catch (Exception ex)
            {
                Debug.LogWarning(
                    "[Locus] Failed to resolve capture viewport property '"
                    + propertyName + "': " + ex.Message);
                return false;
            }
        }

        // GetProperty on the runtime type misses private members declared on
        // base classes (e.g. GameView.viewInParent if Unity moves it), so walk
        // the hierarchy with DeclaredOnly.
        private static PropertyInfo FindCaptureInstanceProperty(Type type, string propertyName)
        {
            while (type != null)
            {
                PropertyInfo property = type.GetProperty(
                    propertyName,
                    BindingFlags.Instance
                        | BindingFlags.Public
                        | BindingFlags.NonPublic
                        | BindingFlags.DeclaredOnly);
                if (property != null)
                    return property;
                type = type.BaseType;
            }
            return null;
        }

        private static bool IsFiniteCaptureRect(Rect rect)
        {
            return !float.IsNaN(rect.x) && !float.IsInfinity(rect.x)
                && !float.IsNaN(rect.y) && !float.IsInfinity(rect.y)
                && !float.IsNaN(rect.width) && !float.IsInfinity(rect.width)
                && !float.IsNaN(rect.height) && !float.IsInfinity(rect.height);
        }

        // A zoomed/panned Game view reports a drawRect that can overflow the
        // visible area; clamp it to the host's content region so tab-bar rows
        // never leak into the capture.
        private static Rect ClampGameCaptureRect(
            EditorWindow window,
            Rect rect,
            Rect boundsRect,
            bool hostSpace)
        {
            if (!hostSpace)
                return rect;

            RectOffset border;
            if (!TryReadCaptureHostBorder(window, out border))
                return rect;

            float top = boundsRect.yMin + border.top + border.bottom;
            Rect contentRect = Rect.MinMaxRect(
                boundsRect.xMin + border.left,
                Mathf.Min(top, boundsRect.yMax),
                Mathf.Max(boundsRect.xMin + border.left, boundsRect.xMax - border.right),
                boundsRect.yMax);
            return ClampCaptureLocalRect(rect, contentRect);
        }

        private static Rect OffsetCaptureRectByHostBorder(EditorWindow window, Rect rect)
        {
            RectOffset border;
            if (!TryReadCaptureHostBorder(window, out border))
                return rect;

            // Mirrors GameView.viewInParent: DockArea keeps the extra space
            // above the view in borderSize.bottom, so it belongs on top here.
            rect.x += border.left;
            rect.y += border.top + border.bottom;
            return rect;
        }

        private static bool TryReadCaptureHostBorder(EditorWindow window, out RectOffset border)
        {
            border = null;
            object parent = GetCaptureHostView(window);
            if (parent == null)
                return false;

            try
            {
                PropertyInfo borderProperty = FindCaptureInstanceProperty(parent.GetType(), "borderSize");
                border = borderProperty != null
                    ? borderProperty.GetValue(parent, null) as RectOffset
                    : null;
                return border != null;
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to resolve capture host border: " + ex.Message);
                return false;
            }
        }

        private static Rect ClampCaptureLocalRect(Rect rect, Rect bounds)
        {
            float xMin = Mathf.Clamp(rect.xMin, bounds.xMin, bounds.xMax);
            float yMin = Mathf.Clamp(rect.yMin, bounds.yMin, bounds.yMax);
            float xMax = Mathf.Clamp(rect.xMax, bounds.xMin, bounds.xMax);
            float yMax = Mathf.Clamp(rect.yMax, bounds.yMin, bounds.yMax);
            return Rect.MinMaxRect(xMin, yMin, Mathf.Max(xMin, xMax), Mathf.Max(yMin, yMax));
        }

        private static float CaptureWindowPixelsPerPoint(EditorWindow window)
        {
            try
            {
                PropertyInfo rootProperty = typeof(EditorWindow).GetProperty(
                    "rootVisualElement",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                object root = rootProperty != null ? rootProperty.GetValue(window, null) : null;
                if (root != null)
                {
                    PropertyInfo scaleProperty = root.GetType().GetProperty(
                        "scaledPixelsPerPoint",
                        BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                    object value = scaleProperty != null ? scaleProperty.GetValue(root, null) : null;
                    if (value is float && (float)value > 0f)
                        return (float)value;
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Failed to resolve target window scale: " + ex.Message);
            }

            return EditorGUIUtility.pixelsPerPoint > 0f
                ? EditorGUIUtility.pixelsPerPoint
                : 1f;
        }

        private static RectInt PointsToCapturePixelRect(Rect rect, float pixelsPerPoint)
        {
            int left = Mathf.RoundToInt(rect.xMin * pixelsPerPoint);
            int top = Mathf.RoundToInt(rect.yMin * pixelsPerPoint);
            int right = Mathf.RoundToInt(rect.xMax * pixelsPerPoint);
            int bottom = Mathf.RoundToInt(rect.yMax * pixelsPerPoint);
            return new RectInt(left, top, Mathf.Max(1, right - left), Mathf.Max(1, bottom - top));
        }

        private static Texture2D CaptureEditorWindowTexture(CaptureViewportRegion region)
        {
#if UNITY_EDITOR_WIN
            Texture2D nativeTexture;
            if (TryCaptureEditorWindowTextureWin32(region, out nativeTexture))
                return nativeTexture;
#endif

            return CaptureEditorWindowTextureFromScreen(region.screenPoints);
        }

        private static Texture2D CaptureEditorWindowTextureFromScreen(Rect screenPoints)
        {
            // ReadScreenPixel takes point coordinates (see Unity's EyeDropper,
            // which feeds it GUIToScreenPoint), so this fallback captures at
            // point resolution rather than physical pixels.
            int width = Mathf.Max(1, Mathf.RoundToInt(screenPoints.width));
            int height = Mathf.Max(1, Mathf.RoundToInt(screenPoints.height));
            Color[] pixels = UnityEditorInternal.InternalEditorUtility.ReadScreenPixel(
                new Vector2(screenPoints.x, screenPoints.y),
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
        private const int CaptureDwmExtendedFrameBounds = 9;
        private const int CaptureCropTolerancePixels = 8;
        private static readonly IntPtr CapturePerMonitorAwareV2 = new IntPtr(-4);

        // Capture the Unity process window off-screen, then crop to the target EditorWindow rect.
        private static bool TryCaptureEditorWindowTextureWin32(
            CaptureViewportRegion region,
            out Texture2D texture)
        {
            texture = null;
            IntPtr previousDpiContext = EnterCaptureDpiContext();
            try
            {
                RectInt screenPixels = region.screenPixels;
                IntPtr hwnd = FindUnityWindowForCapture(region);
                if (hwnd == IntPtr.Zero)
                    return false;

                CaptureNativeRect windowRect;
                if (!GetWindowRect(hwnd, out windowRect))
                    return false;

                int windowWidth = windowRect.right - windowRect.left;
                int windowHeight = windowRect.bottom - windowRect.top;
                if (windowWidth <= 0 || windowHeight <= 0)
                    return false;
                if (screenPixels.width > windowWidth || screenPixels.height > windowHeight)
                    return false;

                // Tolerate small point-to-pixel rounding drift by nudging the
                // crop back inside the window instead of failing the capture.
                int cropX = screenPixels.x - windowRect.left;
                int cropY = screenPixels.y - windowRect.top;
                int clampedX = Mathf.Clamp(cropX, 0, windowWidth - screenPixels.width);
                int clampedY = Mathf.Clamp(cropY, 0, windowHeight - screenPixels.height);
                if (Mathf.Abs(clampedX - cropX) > CaptureCropTolerancePixels
                    || Mathf.Abs(clampedY - cropY) > CaptureCropTolerancePixels)
                {
                    return false;
                }

                byte[] bgra;
                if (!TryCaptureWindowBgra(hwnd, windowWidth, windowHeight, out bgra))
                    return false;

                texture = CreateTextureFromBgraCrop(
                    bgra,
                    windowWidth,
                    clampedX,
                    clampedY,
                    screenPixels.width,
                    screenPixels.height);
                return texture != null;
            }
            finally
            {
                ExitCaptureDpiContext(previousDpiContext);
            }
        }

        private static IntPtr EnterCaptureDpiContext()
        {
            try
            {
                return SetThreadDpiAwarenessContext(CapturePerMonitorAwareV2);
            }
            catch (EntryPointNotFoundException)
            {
                return IntPtr.Zero;
            }
            catch (DllNotFoundException)
            {
                return IntPtr.Zero;
            }
        }

        private static void ExitCaptureDpiContext(IntPtr previousDpiContext)
        {
            if (previousDpiContext == IntPtr.Zero)
                return;

            try
            {
                SetThreadDpiAwarenessContext(previousDpiContext);
            }
            catch (EntryPointNotFoundException)
            {
            }
            catch (DllNotFoundException)
            {
            }
        }

        private static IntPtr FindUnityWindowForCapture(CaptureViewportRegion region)
        {
            uint unityProcessId = (uint)Process.GetCurrentProcess().Id;

            // Prefer matching the target window's own container rect: overlap
            // against the capture region alone can pick an unrelated floating
            // window hovering above the viewport.
            if (region.hasContainer)
            {
                RectInt expectedClient = PointsToCapturePixelRect(
                    region.containerScreenPoints,
                    region.pixelsPerPoint);
                IntPtr anchored = FindCaptureWindowByClientRect(unityProcessId, expectedClient);
                if (anchored != IntPtr.Zero)
                    return anchored;
            }

            return FindCaptureWindowByRegionOverlap(unityProcessId, region.screenPixels);
        }

        private static IntPtr FindCaptureWindowByClientRect(uint unityProcessId, RectInt expectedClient)
        {
            CaptureNativeRect expected = new CaptureNativeRect
            {
                left = expectedClient.x,
                top = expectedClient.y,
                right = expectedClient.xMax,
                bottom = expectedClient.yMax
            };
            if (RectArea(expected) <= 0)
                return IntPtr.Zero;

            IntPtr bestHwnd = IntPtr.Zero;
            long bestIntersection = 0;

            EnumWindows(delegate (IntPtr hwnd, IntPtr lParam)
            {
                if (!IsWindowVisible(hwnd))
                    return true;

                uint processId;
                GetWindowThreadProcessId(hwnd, out processId);
                if (processId != unityProcessId)
                    return true;

                CaptureNativeRect clientRect;
                if (!TryGetCaptureWindowClientRect(hwnd, out clientRect))
                    return true;

                long intersection = IntersectionArea(expected, clientRect);
                if (intersection <= 0)
                    return true;

                // Require a dominant match (IoU >= 1/2) so the anchor also
                // validates the point-to-pixel mapping before it is trusted.
                long union = RectArea(expected) + RectArea(clientRect) - intersection;
                if (union <= 0 || intersection * 2 < union)
                    return true;

                if (intersection > bestIntersection)
                {
                    bestHwnd = hwnd;
                    bestIntersection = intersection;
                }

                return true;
            }, IntPtr.Zero);

            return bestHwnd;
        }

        private static bool TryGetCaptureWindowClientRect(IntPtr hwnd, out CaptureNativeRect rect)
        {
            rect = new CaptureNativeRect();
            CaptureNativeRect client;
            if (!GetClientRect(hwnd, out client))
                return false;

            CapturePoint origin = new CapturePoint { x = 0, y = 0 };
            if (!ClientToScreen(hwnd, ref origin))
                return false;

            rect.left = origin.x;
            rect.top = origin.y;
            rect.right = origin.x + (client.right - client.left);
            rect.bottom = origin.y + (client.bottom - client.top);
            return RectArea(rect) > 0;
        }

        private static IntPtr FindCaptureWindowByRegionOverlap(uint unityProcessId, RectInt screenPixels)
        {
            CaptureNativeRect target = new CaptureNativeRect
            {
                left = screenPixels.x,
                top = screenPixels.y,
                right = screenPixels.xMax,
                bottom = screenPixels.yMax
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

                CaptureNativeRect visibleRect;
                if (!TryGetCaptureWindowVisibleRect(hwnd, out visibleRect))
                    return true;

                long intersection = IntersectionArea(target, visibleRect);
                if (intersection <= 0)
                    return true;

                long area = RectArea(visibleRect);
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

        private static bool TryGetCaptureWindowVisibleRect(
            IntPtr hwnd,
            out CaptureNativeRect rect)
        {
            rect = new CaptureNativeRect();
            try
            {
                int result = DwmGetWindowAttribute(
                    hwnd,
                    CaptureDwmExtendedFrameBounds,
                    out rect,
                    Marshal.SizeOf(typeof(CaptureNativeRect)));
                if (result == 0 && RectArea(rect) > 0)
                    return true;
            }
            catch (EntryPointNotFoundException)
            {
            }
            catch (DllNotFoundException)
            {
            }

            return GetWindowRect(hwnd, out rect);
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
            for (int textureY = 0; textureY < height; textureY++)
            {
                int sourceY = cropY + (height - 1 - textureY);
                int sourceRow = (sourceY * sourceWidth + cropX) * 4;
                int targetRow = textureY * width * 4;
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
        private struct CapturePoint
        {
            public int x;
            public int y;
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
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetClientRect(IntPtr hWnd, out CaptureNativeRect lpRect);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool ClientToScreen(IntPtr hWnd, ref CapturePoint lpPoint);

        [DllImport("user32.dll")]
        private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr dpiContext);

        [DllImport("dwmapi.dll")]
        private static extern int DwmGetWindowAttribute(
            IntPtr hwnd,
            int attribute,
            out CaptureNativeRect attributeValue,
            int attributeSize);

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
