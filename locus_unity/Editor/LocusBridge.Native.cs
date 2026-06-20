// Native broker bridge. The command channel that Locus uses to talk to this
// editor normally lives on a managed named pipe (see LocusBridge.cs), which
// dies on every domain reload. When the native broker DLL (`locus_native`) is
// present and enabled, that long-lived pipe instead lives in native code that
// survives reloads; this file is the managed half: it registers lifecycle
// state with the broker and pumps queued requests through the same
// `HandleMessageAsync` executor, on the main thread via
// `EditorApplication.update`.
//
// Gating is intentionally out-of-band: the Tauri side writes a per-project
// marker file (Library/Locus/NativeBridge.enabled) that records the native
// broker pipe name. The native broker is the required desktop command
// transport.

using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;

using UnityEngine;
using UnityEditor;

namespace Locus
{
    public static partial class LocusBridge
    {
        // ───────────────── Native bridge state ─────────────────

        private const string NativeDll = "locus_native";
        private const int NativeProtocolVersion = 1;
        private const int NativeInitialBufferSize = 64 * 1024;
        private const int MaxNativeRequestsPerUpdate = 16;
        private const int MaxNativeActiveManagedRequests = 32;
        private const double NativeStatusPublishIntervalSeconds = 0.25;
        private const double NativeBackgroundHookRefreshIntervalSeconds = 1.0;
        private const string SessionKey_NativeGeneration = "Locus_NativeGeneration";
        private const string NativePipePrefix = @"\\.\pipe\";

        // Must match the `MANAGED_STATE_*` constants in locus_native (lib.rs).
        private const int ManagedStateInitializing = 0;
        private const int ManagedStateReady = 1;
        private const int ManagedStateReloading = 2;
        private const int ManagedStateQuitting = 3;

        // Capabilities the managed executor advertises (mirrors the managed
        // executor features the broker can route through this domain).
        private const string ManagedCapabilities = "managed_executor_v1,status_cached,set_editor_status_async";

        private static volatile bool _nativeBridgeActive;
        private static long _nativeGeneration;
        private static byte[] _nativeBuffer;
        private static int _nativeActiveRequests;
        private static double _lastNativeStatusPublishAt = -1.0;
        private static double _lastNativeBackgroundHookCheckAt = -1.0;
        private static bool _nativeBackgroundHookApplied;

        // Reused UTF-8 encode buffers for the high-frequency outbound calls
        // (status heartbeat + editor-update events both fire every 0.25s). Guarded
        // by a dedicated lock because an event can be emitted off the main thread.
        private static readonly object _nativeEncodeLock = new object();
        private static byte[] _nativeEncodeBufferA;
        private static byte[] _nativeEncodeBufferB;

        // ───────────────── P/Invoke surface (see lib.rs) ─────────────────

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int locus_init(
            byte[] project, int projectLen, byte[] pipe, int pipeLen, int protocolVersion);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_shutdown();

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_set_managed_state(
            int state, long generation, byte[] editorStatus, int editorStatusLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_managed_heartbeat(long generation);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_set_capabilities(byte[] caps, int capsLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int locus_poll_request(byte[] buffer, int bufferLen, ref int outRequiredLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_complete_request(
            byte[] id, int idLen, byte[] response, int responseLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_emit_event(
            byte[] eventType, int typeLen, byte[] payload, int payloadLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int locus_set_background_active(int active);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern int locus_overlay_connect(byte[] pipe, int pipeLen);

        [DllImport(NativeDll, CallingConvention = CallingConvention.Cdecl)]
        private static extern void locus_overlay_push(byte[] line, int lineLen);

        private static volatile bool _nativeOverlayConnected;

        // ───────────────── Lifecycle (called from LocusBridge.cs) ─────────────────

        /// <summary>
        /// Start the native broker bridge for this domain if the feature is
        /// enabled. Idempotent within a domain; the underlying broker is started
        /// once per process and reused across reloads.
        /// </summary>
        private static void NativeStartIfEnabled()
        {
            if (_isUnityWorkerProcess)
                return;
            if (_nativeBridgeActive)
                return;
            if (!NativeBridgeEnabled())
                return;
            if (!NativeLoadAndInit())
            {
                Debug.LogError("[Locus] Native broker bridge is required and failed to initialize.");
                return;
            }

            if (_nativeGeneration <= 0)
            {
                long persisted = SessionState.GetInt(SessionKey_NativeGeneration, 0);
                _nativeGeneration = persisted > 0 ? persisted : 1;
                SessionState.SetInt(SessionKey_NativeGeneration, (int)_nativeGeneration);
            }

            if (_nativeBuffer == null)
                _nativeBuffer = new byte[NativeInitialBufferSize];

            NativeSetCapabilities(ManagedCapabilities);
            NativeSetManagedState(ManagedStateReady);
            _nativeBridgeActive = true;
            Debug.Log("[Locus] Native broker bridge active (generation " + _nativeGeneration + ").");

            // Reconcile the in-process background hook (Phase 6). The native
            // patch can survive domain reloads, so a fresh domain must restore
            // it too when the marker was removed while managed code was down.
            NativeRefreshBackgroundHook(true);
        }

        private static void NativeOnBeforeReload()
        {
            if (!_nativeBridgeActive)
                return;
            // Tell the broker the executor is gone; it will fail in-flight
            // requests with `domain_reload_interrupted` and answer new ones with
            // `managed_reloading` until we re-register.
            NativeSetManagedState(ManagedStateReloading);
        }

        private static void NativeOnAfterReload()
        {
            if (!NativeBridgeEnabled())
                return;
            long generation = SessionState.GetInt(SessionKey_NativeGeneration, 0) + 1;
            SessionState.SetInt(SessionKey_NativeGeneration, (int)generation);
            _nativeGeneration = generation;
            // _nativeBridgeActive is false in this fresh domain, so this
            // re-binds the DLL and re-registers as ready at the new generation.
            NativeStartIfEnabled();
        }

        private static void NativeOnQuitting()
        {
            if (!_nativeBridgeActive)
                return;
            try
            {
                NativeSetManagedState(ManagedStateQuitting);
                NativeRestoreBackgroundHook();
                locus_shutdown();
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native broker shutdown failed: " + ex.Message);
            }
            _nativeBridgeActive = false;
        }

        private static void NativeShutdownInWorkerProcess()
        {
            try
            {
                locus_shutdown();
            }
            catch
            {
            }
            _nativeBridgeActive = false;
        }

        // ───────────────── Per-frame pump (from PumpMainThreadQueue) ─────────────────

        private static void NativePump()
        {
            if (!_nativeBridgeActive)
                return;

            try
            {
                NativeRefreshBackgroundHook();
                NativePublishHeartbeat();

                for (int i = 0; i < MaxNativeRequestsPerUpdate; i++)
                {
                    if (Volatile.Read(ref _nativeActiveRequests) >= MaxNativeActiveManagedRequests)
                        break;

                    int required = 0;
                    int n = locus_poll_request(_nativeBuffer, _nativeBuffer.Length, ref required);
                    if (n == 0)
                        break;
                    if (n < 0)
                    {
                        // Buffer too small — grow to fit the pending request and retry once.
                        int grow = Math.Max(required, _nativeBuffer.Length * 2);
                        _nativeBuffer = new byte[grow];
                        n = locus_poll_request(_nativeBuffer, _nativeBuffer.Length, ref required);
                        if (n <= 0)
                            break;
                    }

                    string json = Utf8NoBom.GetString(_nativeBuffer, 0, n);
                    DispatchNativeRequest(json);
                }
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Native bridge pump failed; disabling native transport: " + ex);
                NativeSetManagedState(ManagedStateInitializing);
                _nativeBridgeActive = false;
            }
        }

        private static void DispatchNativeRequest(string json)
        {
            PipeEnvelope request;
            try
            {
                request = JsonUtility.FromJson<PipeEnvelope>(json);
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native request parse failed: " + ex.Message + " | raw=" + json);
                return;
            }

            if (request == null || string.IsNullOrEmpty(request.id))
            {
                Debug.LogWarning("[Locus] Native request missing id: " + json);
                return;
            }

            int active = Interlocked.Increment(ref _nativeActiveRequests);
            if (active > MaxNativeActiveManagedRequests)
            {
                Interlocked.Decrement(ref _nativeActiveRequests);
                NativeComplete(request.id, ErrorResponse(request.id, "native_managed_executor_full"));
                return;
            }

            // Fire-and-forget: HandleMessageAsync marshals its own main-thread
            // work through PostToMainThread, which this same PumpMainThreadQueue
            // drains, so awaiting here is unnecessary and would stall the pump.
            _ = HandleNativeRequestAsync(request.id, request);
        }

        private static async Task HandleNativeRequestAsync(string id, PipeEnvelope request)
        {
            try
            {
                // Leave the main-thread pump immediately. The request executor and
                // the synchronous prefix of every handler (reflection-heavy type
                // index export, compile-params fingerprint hashing, Roslyn Emit,
                // JsonUtility on plain types) now run on a background thread.
                // Handlers marshal their Unity-API work back to the main thread via
                // LocusAsync.SwitchToMainThread / PostToMainThread, which the pump
                // drains. This restores the off-main-thread execution contract the
                // old managed-pipe worker provided before the native broker became
                // the sole transport.
                await LocusAsync.SwitchToThreadPool();

                PipeEnvelope response;
                try
                {
                    response = await HandleMessageAsync(request).ConfigureAwait(false);
                }
                catch (Exception ex)
                {
                    response = ErrorResponse(id, "native executor exception: " + ex.Message);
                }

                if (response == null)
                    response = ErrorResponse(id, "native executor returned no response");
                if (string.IsNullOrEmpty(response.reply_to))
                    response.reply_to = id;

                // A handler may have left us on the main thread (its last hop was
                // SwitchToMainThread). Serialize + native write must not run on the
                // main thread, so hop back off it first.
                await LocusAsync.SwitchToThreadPool();
                NativeComplete(id, response);
            }
            finally
            {
                Interlocked.Decrement(ref _nativeActiveRequests);
            }
        }

        private static void NativeComplete(string id, PipeEnvelope response)
        {
            if (!_nativeBridgeActive)
                return;

            string json;
            try
            {
                json = JsonUtility.ToJson(response);
            }
            catch (Exception ex)
            {
                try
                {
                    json = JsonUtility.ToJson(ErrorResponse(id, "native serialize failed: " + ex.Message));
                }
                catch
                {
                    return;
                }
            }

            try
            {
                byte[] idBytes = Utf8NoBom.GetBytes(id);
                byte[] responseBytes = Utf8NoBom.GetBytes(json);
                locus_complete_request(idBytes, idBytes.Length, responseBytes, responseBytes.Length);
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native complete_request failed: " + ex.Message);
            }
        }

        /// <summary>
        /// Mirror an unsolicited event onto the native transport too, so a
        /// client connected via the broker still receives editor pushes. Safe
        /// no-op when the native bridge is inactive.
        /// </summary>
        private static void NativeEmitEvent(string eventType, string message)
        {
            if (!_nativeBridgeActive || string.IsNullOrEmpty(eventType))
                return;
            try
            {
                lock (_nativeEncodeLock)
                {
                    int typeLen = EncodeUtf8Reusing(eventType, ref _nativeEncodeBufferA);
                    int payloadLen = EncodeUtf8Reusing(message ?? "", ref _nativeEncodeBufferB);
                    locus_emit_event(_nativeEncodeBufferA, typeLen, _nativeEncodeBufferB, payloadLen);
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native emit_event failed: " + ex.Message);
            }
        }

        // ───────────────── Helpers ─────────────────

        private static void NativePublishHeartbeat()
        {
            double now = EditorApplication.timeSinceStartup;
            if (_lastNativeStatusPublishAt < 0 || now - _lastNativeStatusPublishAt >= NativeStatusPublishIntervalSeconds)
            {
                RefreshCachedEditorState();
                NativeSetManagedState(ManagedStateReady);
                _lastNativeStatusPublishAt = now;
                return;
            }

            locus_managed_heartbeat(_nativeGeneration);
        }

        private static void NativePublishEditorStatusNow()
        {
            if (!_nativeBridgeActive)
                return;

            RefreshCachedEditorState();
            NativeSetManagedState(ManagedStateReady);
            _lastNativeStatusPublishAt = EditorApplication.timeSinceStartup;
        }

        private static void NativeSetManagedState(int state)
        {
            try
            {
                string status = BuildCachedEditorStatusMessage();
                lock (_nativeEncodeLock)
                {
                    int statusLen = EncodeUtf8Reusing(status, ref _nativeEncodeBufferA);
                    locus_set_managed_state(state, _nativeGeneration, _nativeEncodeBufferA, statusLen);
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native set_managed_state failed: " + ex.Message);
            }
        }

        /// <summary>
        /// Encode <paramref name="value"/> as UTF-8 into a reused buffer (grown as
        /// needed) and return the byte length. The caller must hold
        /// <see cref="_nativeEncodeLock"/> for the buffer it passes. Replaces the
        /// per-call <c>byte[]</c> the 0.25s status/event path used to churn.
        /// </summary>
        private static int EncodeUtf8Reusing(string value, ref byte[] buffer)
        {
            value = value ?? "";
            int maxBytes = Utf8NoBom.GetMaxByteCount(value.Length);
            if (buffer == null || buffer.Length < maxBytes)
                buffer = new byte[Math.Max(maxBytes, 256)];
            return Utf8NoBom.GetBytes(value, 0, value.Length, buffer, 0);
        }

        private static void NativeSetCapabilities(string caps)
        {
            try
            {
                byte[] bytes = Utf8NoBom.GetBytes(caps ?? "");
                locus_set_capabilities(bytes, bytes.Length);
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native set_capabilities failed: " + ex.Message);
            }
        }

        private static bool NativeLoadAndInit()
        {
            try
            {
                string projectPath = Directory.GetParent(Application.dataPath).FullName;
                string pipeName = ResolveNativePipeName();
                if (string.IsNullOrEmpty(pipeName))
                    return false;

                byte[] projectBytes = Utf8NoBom.GetBytes(projectPath);
                byte[] pipeBytes = Utf8NoBom.GetBytes(pipeName);
                int rc = locus_init(
                    projectBytes, projectBytes.Length, pipeBytes, pipeBytes.Length, NativeProtocolVersion);
                if (rc != 0)
                {
                    Debug.LogError("[Locus] locus_init returned " + rc + "; native bridge is unavailable.");
                    return false;
                }
                return true;
            }
            catch (DllNotFoundException ex)
            {
                Debug.LogError("[Locus] Native broker DLL not found: " + ex.Message);
                return false;
            }
            catch (EntryPointNotFoundException ex)
            {
                Debug.LogError("[Locus] Native broker DLL is incompatible: " + ex.Message);
                return false;
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Native broker init failed: " + ex.Message);
                return false;
            }
        }

        private static bool NativeBridgeEnabled()
        {
            try
            {
                string env = Environment.GetEnvironmentVariable("LOCUS_UNITY_NATIVE_BRIDGE");
                if (env == "1" || string.Equals(env, "true", StringComparison.OrdinalIgnoreCase))
                    return true;
                string marker = NativeMarkerPath();
                return !string.IsNullOrEmpty(marker) && File.Exists(marker);
            }
            catch
            {
                return false;
            }
        }

        private static string NativeMarkerPath()
        {
            string projectPath = Directory.GetParent(Application.dataPath).FullName;
            return Path.Combine(projectPath, "Library", "Locus", "NativeBridge.enabled");
        }

        /// <summary>
        /// Ask the broker to patch Unity's background-activity symbols in-process
        /// (Phase 6) when the feature marker is present. Fails open: on any error
        /// the Tauri side still applies the cross-process patch.
        /// </summary>
        private static void NativeApplyBackgroundHook()
        {
            if (!_nativeBridgeActive || !NativeBackgroundHookEnabled())
                return;

            int rc = NativeSetBackgroundHookActive(true);
            if (rc < 0)
            {
                Debug.LogWarning(
                    "[Locus] Native background hook unavailable; the cross-process hook will handle it.");
                return;
            }

            bool wasApplied = _nativeBackgroundHookApplied;
            _nativeBackgroundHookApplied = rc > 0;
            if (_nativeBackgroundHookApplied && !wasApplied)
                Debug.Log("[Locus] Native background hook active (" + rc + " symbols).");
        }

        private static void NativeRefreshBackgroundHook(bool force)
        {
            if (!_nativeBridgeActive)
                return;

            double now = EditorApplication.timeSinceStartup;
            if (!force && _lastNativeBackgroundHookCheckAt >= 0 && now - _lastNativeBackgroundHookCheckAt < NativeBackgroundHookRefreshIntervalSeconds)
                return;
            _lastNativeBackgroundHookCheckAt = now;

            if (NativeBackgroundHookEnabled())
                NativeApplyBackgroundHook();
            else
                NativeRestoreBackgroundHook();
        }

        private static void NativeRefreshBackgroundHook()
        {
            NativeRefreshBackgroundHook(false);
        }

        private static void NativeRestoreBackgroundHook()
        {
            if (!_nativeBridgeActive)
                return;

            int rc = NativeSetBackgroundHookActive(false);
            if (rc >= 0)
                _nativeBackgroundHookApplied = false;
        }

        private static int NativeSetBackgroundHookActive(bool active)
        {
            try
            {
                return locus_set_background_active(active ? 1 : 0);
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native background hook call failed: " + ex.Message);
                return -1;
            }
        }

        private static bool NativeBackgroundHookEnabled()
        {
            try
            {
                string marker = NativeBackgroundHookMarkerPath();
                return !string.IsNullOrEmpty(marker) && File.Exists(marker);
            }
            catch
            {
                return false;
            }
        }

        private static string NativeBackgroundHookMarkerPath()
        {
            string projectPath = Directory.GetParent(Application.dataPath).FullName;
            return Path.Combine(projectPath, "Library", "Locus", "BackgroundHook.enabled");
        }

        // ───────────────── Overlay control (Phase 5, used by LocusEditorWindow) ─────────────────

        /// <summary>True when the native broker bridge is active for this domain.</summary>
        internal static bool IsNativeBridgeActive
        {
            get { return _nativeBridgeActive; }
        }

        /// <summary>
        /// Open (idempotently) the persistent native overlay client to the given
        /// Tauri control pipe so it survives domain reloads. Returns false when
        /// the native bridge is inactive, so the caller keeps owning its managed
        /// pipe (fail-open). Safe to call from any thread.
        /// </summary>
        internal static bool NativeOverlayConnect(string controlPipeName)
        {
            if (!_nativeBridgeActive || string.IsNullOrEmpty(controlPipeName))
                return false;
            if (_nativeOverlayConnected)
                return true;
            try
            {
                byte[] pipe = Utf8NoBom.GetBytes(controlPipeName);
                _nativeOverlayConnected = locus_overlay_connect(pipe, pipe.Length) == 0;
                return _nativeOverlayConnected;
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native overlay connect failed: " + ex.Message);
                return false;
            }
        }

        /// <summary>
        /// Forward one overlay control line through the persistent native client.
        /// Returns false when native overlay is unavailable so the caller writes
        /// through native overlay client. Safe to call from any thread (the
        /// native side serializes and buffers).
        /// </summary>
        internal static bool NativeOverlayPush(string json)
        {
            if (!_nativeBridgeActive || !_nativeOverlayConnected || string.IsNullOrEmpty(json))
                return false;
            try
            {
                byte[] line = Utf8NoBom.GetBytes(json);
                locus_overlay_push(line, line.Length);
                return true;
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] Native overlay push failed: " + ex.Message);
                return false;
            }
        }

        /// <summary>
        /// The pipe name the broker should serve. Prefer the exact name the
        /// Tauri side recorded in the marker (so both ends agree byte-for-byte
        /// regardless of path-formatting differences); otherwise compute it.
        /// </summary>
        private static string ResolveNativePipeName()
        {
            try
            {
                string marker = NativeMarkerPath();
                if (File.Exists(marker))
                {
                    foreach (string raw in File.ReadAllLines(marker))
                    {
                        string line = raw == null ? null : raw.Trim();
                        if (!string.IsNullOrEmpty(line))
                            return NormalizeNativePipeName(line);
                    }
                }
            }
            catch
            {
            }
            return NormalizeNativePipeName(GenerateNativePipeName());
        }

        private static string GenerateNativePipeName()
        {
            string projectPath = Directory.GetParent(Application.dataPath).FullName;
            return "locus_unity_native_" + NativeProjectKey(projectPath);
        }

        private static string NativeProjectKey(string projectPath)
        {
            string normalized = NormalizeProjectPathForNativeKey(projectPath);
            byte[] bytes = Utf8NoBom.GetBytes(normalized);
            using (SHA256 sha = SHA256.Create())
            {
                return HexPrefix(sha.ComputeHash(bytes), 16);
            }
        }

        private static string NormalizeProjectPathForNativeKey(string projectPath)
        {
            string value = (projectPath ?? "").Trim();
            if (value.StartsWith(@"\\?\", StringComparison.Ordinal))
                value = value.Substring(4);
            value = value.Replace('/', '\\');
            while (value.EndsWith("\\", StringComparison.Ordinal) && value.Length > 3)
                value = value.Substring(0, value.Length - 1);
            return value.ToLowerInvariant();
        }

        private static string HexPrefix(byte[] bytes, int bytesToTake)
        {
            int count = Math.Min(bytesToTake, bytes == null ? 0 : bytes.Length);
            char[] chars = new char[count * 2];
            const string hex = "0123456789abcdef";
            for (int i = 0; i < count; i++)
            {
                byte value = bytes[i];
                chars[i * 2] = hex[value >> 4];
                chars[i * 2 + 1] = hex[value & 0x0F];
            }
            return new string(chars);
        }

        private static string NormalizeNativePipeName(string pipeName)
        {
            string value = pipeName == null ? "" : pipeName.Trim();
            if (string.IsNullOrEmpty(value))
                return "";
            if (value.StartsWith(NativePipePrefix, StringComparison.OrdinalIgnoreCase))
                return value;
            return NativePipePrefix + value.TrimStart('\\');
        }
    }
}
