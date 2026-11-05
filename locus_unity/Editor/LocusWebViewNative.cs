using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Reflection;
using System.Collections.Generic;
using System.Text;
using UnityEditor;
using UnityEngine;

namespace Locus
{
    internal static class LocusWebViewNative
    {
        const string Dll = "locus_unity_native";
        internal const string RuntimeDownload = "https://developer.microsoft.com/microsoft-edge/webview2/";
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_start(IntPtr parent, byte[] profile, int n, byte[] url, int un);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_bind(int id, ulong generation, IntPtr parent, byte[] binding, int n);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_message(byte[] value, int n);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_rect(int id, int x, int y, int w, int h, int cx, int cy, int cw, int ch, int visible);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_focus(int id, int reason);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int locus_webview_parent(int id, IntPtr parent);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void locus_webview_suspend(int id);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void locus_webview_park(int id);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void locus_webview_close(int id);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void locus_webview_shutdown();
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        static extern int locus_webview_poll(byte[] value, int size);
        [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
        static extern int locus_webview_status(byte[] value, int size);
        [StructLayout(LayoutKind.Sequential)]
        struct NativeRect
        {
            public int left, top, right, bottom;
        }

        delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr unused);
        [DllImport("user32.dll")]
        static extern bool EnumWindows(EnumWindowsProc callback, IntPtr unused);
        [DllImport("user32.dll")]
        static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")]
        static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);
        [DllImport("user32.dll")]
        internal static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int capacity);
        static readonly byte[] Buffer = new byte[1024 * 1024];
        internal static bool Supported
        {
            get
            {
                return Application.platform == RuntimePlatform.WindowsEditor;
            }
        }

        internal static string Status()
        {
            if (!Supported)
                return "{\"running\":false}";
            int n = locus_webview_status(Buffer, Buffer.Length);
            return n > 0 ? Encoding.UTF8.GetString(Buffer, 0, n) : "{}";
        }

        internal static string Poll()
        {
            int n = locus_webview_poll(Buffer, Buffer.Length);
            if (n < 0)
                throw new InvalidOperationException("Native View message exceeds limit");
            return n > 0 ? Encoding.UTF8.GetString(Buffer, 0, n) : null;
        }

        internal static void Send(object value)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(JsonUtility.ToJson(value));
            locus_webview_message(bytes, bytes.Length);
        }

        internal static void Hide(int id)
        {
            locus_webview_rect(id, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        }

        internal static bool Finite(Rect rect)
        {
            return !(float.IsNaN(rect.x) || float.IsNaN(rect.y) || float.IsNaN(rect.width) || float.IsNaN(rect.height) || float.IsInfinity(rect.x) || float.IsInfinity(rect.y) || float.IsInfinity(rect.width) || float.IsInfinity(rect.height));
        }

        internal static Rect Intersect(Rect a, Rect b)
        {
            float x = Math.Max(a.xMin, b.xMin), y = Math.Max(a.yMin, b.yMin);
            return new Rect(x, y, Math.Max(0, Math.Min(a.xMax, b.xMax) - x), Math.Max(0, Math.Min(a.yMax, b.yMax) - y));
        }

        internal static void Sync(int id, Rect full, Rect clip, float scale)
        {
            locus_webview_rect(id, Mathf.RoundToInt(full.x * scale), Mathf.RoundToInt(full.y * scale), Mathf.RoundToInt(full.width * scale), Mathf.RoundToInt(full.height * scale), Mathf.RoundToInt(clip.x * scale), Mathf.RoundToInt(clip.y * scale), Mathf.RoundToInt(clip.width * scale), Mathf.RoundToInt(clip.height * scale), 1);
        }

        static readonly List<EditorWindow> Popups = new List<EditorWindow>(4);
        static EditorWindow lastFocusedPopupProbe;
        static double nextPopupProbe;

        static bool IsPopup(EditorWindow window)
        {
            if (window == null) return false;
            string type = window.GetType().FullName;
            return type == "UnityEditor.ObjectSelector" || type == "UnityEditor.ColorPicker"
                || type == "UnityEditor.Search.SearchPickerWindow"
                || type == "UnityEditor.PopupWindow" || type == "UnityEditor.PopupWindowWithoutFocus";
        }

        internal static bool HasBlockingPopup(EditorWindow owner, Rect screenRect)
        {
            var focused = EditorWindow.focusedWindow;
            double now = EditorApplication.timeSinceStartup;
            if (focused != lastFocusedPopupProbe || now >= nextPopupProbe)
            {
                lastFocusedPopupProbe = focused;
                nextPopupProbe = now + 0.1;
                Popups.Clear();
                // Some auxiliary pickers are visible without being focused.
                // Share one bounded cache across controls instead of scanning per control/frame.
                foreach (var window in Resources.FindObjectsOfTypeAll<EditorWindow>())
                    if (IsPopup(window) && Popups.Count < 16) Popups.Add(window);
            }
            foreach (var popup in Popups)
                if (popup != null && popup != owner && popup.rootVisualElement.panel != null
                    && popup.position.Overlaps(screenRect) && IsWindowVisible(FindOwner(popup))) return true;
            return false;
        }

        static readonly FieldInfo ParentField = typeof(EditorWindow).GetField("m_Parent", BindingFlags.Instance | BindingFlags.NonPublic);
        static readonly PropertyInfo NativeHandle = typeof(EditorWindow).Assembly.GetType("UnityEditor.GUIView")?.GetProperty("nativeHandle", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
        internal static object OwnerParent(EditorWindow owner)
        {
            var parent = ParentField?.GetValue(owner);
            return parent?.GetType().GetProperty("window", BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)?.GetValue(parent, null) ?? parent;
        }

        // nativeHandle is not an HWND in all Editor builds. The fallback matches
        // the explicit owner's ContainerWindow, never the currently focused view.
        internal static IntPtr FindOwner(EditorWindow owner)
        {
            if (owner == null)
                return IntPtr.Zero;
            var parent = ParentField?.GetValue(owner);
            if (parent == null)
                return IntPtr.Zero;
            uint own = (uint)Process.GetCurrentProcess().Id;
            var value = NativeHandle?.GetValue(parent, null);
            if (value is IntPtr native && native != IntPtr.Zero && IsWindow(native))
            {
                uint nativePid;
                GetWindowThreadProcessId(native, out nativePid);
                if (nativePid == own)
                    return native;
            }

            var container = parent.GetType().GetProperty("window", BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)?.GetValue(parent, null);
            if (container == null)
                return IntPtr.Zero;
            var type = container.GetType();
            var title = type.GetProperty("displayedTitle")?.GetValue(container, null) as string ?? type.GetProperty("title")?.GetValue(container, null) as string;
            bool main = Convert.ToInt32(type.GetProperty("showMode", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(container, null) ?? -1) == 4;
            if (!(type.GetProperty("position")?.GetValue(container, null)is Rect bounds) || (!main && string.IsNullOrEmpty(title)))
                return IntPtr.Zero;
            var candidates = new List<IntPtr>();
            float best = float.MaxValue;
            EnumWindows((hwnd, unused) =>
            {
                uint pid;
                NativeRect r;
                GetWindowThreadProcessId(hwnd, out pid);
                if (pid != own || !IsWindowVisible(hwnd) || !GetWindowRect(hwnd, out r))
                    return true;
                var text = new StringBuilder(512);
                GetWindowText(hwnd, text, text.Capacity);
                if (!main && text.ToString() != title)
                    return true;
                float score = Math.Abs(r.left - bounds.x) + Math.Abs(r.top - bounds.y) + Math.Abs((r.right - r.left) - bounds.width) + Math.Abs((r.bottom - r.top) - bounds.height);
                if (score < best - 1)
                {
                    best = score;
                    candidates.Clear();
                    candidates.Add(hwnd);
                }
                else if (Math.Abs(score - best) < 1)
                    candidates.Add(hwnd);
                return true;
            }, IntPtr.Zero);
            if (best >= 160 || candidates.Count == 0)
                return IntPtr.Zero;
            if (candidates.Count == 1)
                return candidates[0];
            // Both lists are front-to-back. This disambiguates identical locked
            // Inspectors restored at exactly the same position and size.
            int ordinal = -1, count = 0;
            var containers = type.GetProperty("windows", BindingFlags.Static | BindingFlags.Public)?.GetValue(null, null) as Array;
            if (containers == null)
                return IntPtr.Zero;
            foreach (var other in containers)
            {
                var otherTitle = type.GetProperty("displayedTitle")?.GetValue(other, null) as string;
                if (otherTitle == title && type.GetProperty("position")?.GetValue(other, null)is Rect otherBounds && otherBounds == bounds)
                {
                    if (Equals(other, container))
                        ordinal = count;
                    count++;
                }
            }

            return count == candidates.Count && ordinal >= 0 ? candidates[ordinal] : IntPtr.Zero;
        }
    }
}
