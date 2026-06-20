using UnityEngine;
using UnityEditor;

using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private sealed class AssetThumbnailRequest
        {
            public string assetPath;
            public int maxSize = 192;
        }

        [Serializable]
        private sealed class AssetPreviewRenderRequest
        {
            public string assetPath;
            public int width = 320;
            public int height = 220;
            public float yaw = 25f;
            public float pitch = -12f;
            public float distance = 1.15f;
            public float panX;
            public float panY;
            public float panZ;
        }

        [Serializable]
        private sealed class AssetThumbnailResponse
        {
            public string assetPath;
            public int width;
            public int height;
            public string mimeType;
            public string pngBase64;
        }

        [Serializable]
        private sealed class AssetPreviewRenderResponse
        {
            public string assetPath;
            public int width;
            public int height;
            public string mimeType;
            public string dataBase64;
        }

        private sealed class AssetPreviewRenderSession : IDisposable
        {
            public readonly string assetPath;
            public readonly string dependencyHash;
            public readonly GameObject instance;
            public readonly PreviewRenderUtility preview;
            public readonly Bounds bounds;
            public double lastUsedAt;

            public AssetPreviewRenderSession(string assetPath, string dependencyHash, GameObject source)
            {
                this.assetPath = assetPath;
                this.dependencyHash = dependencyHash;
                lastUsedAt = EditorApplication.timeSinceStartup;

                instance = UnityEngine.Object.Instantiate(source);
                instance.hideFlags = HideFlags.HideAndDontSave;
                SetHideFlagsRecursive(instance.transform, HideFlags.HideAndDontSave);
                instance.transform.position = Vector3.zero;
                instance.transform.rotation = Quaternion.identity;
                instance.transform.localScale = Vector3.one;

                preview = new PreviewRenderUtility();
                preview.cameraFieldOfView = 30f;
                preview.camera.clearFlags = CameraClearFlags.Color;
                preview.camera.backgroundColor = new Color(0.31f, 0.31f, 0.31f, 1f);
                if (preview.lights != null && preview.lights.Length > 0)
                {
                    preview.lights[0].intensity = 1.2f;
                    preview.lights[0].transform.rotation = Quaternion.Euler(35f, 35f, 0f);
                }
                if (preview.lights != null && preview.lights.Length > 1)
                {
                    preview.lights[1].intensity = 0.65f;
                }

                bounds = CalculatePreviewBounds(instance);
                preview.AddSingleGO(instance);
            }

            public string Render(AssetPreviewRenderRequest request)
            {
                lastUsedAt = EditorApplication.timeSinceStartup;
                Vector3 center = bounds.center;
                float radius = Mathf.Max(bounds.extents.magnitude, 0.5f);
                float fov = Mathf.Max(preview.camera.fieldOfView, 1f) * Mathf.Deg2Rad;
                float cameraDistance = radius / Mathf.Sin(fov * 0.5f) * request.distance;
                Quaternion orbit = Quaternion.Euler(request.pitch, request.yaw, 0f);
                Vector3 direction = orbit * Vector3.forward;
                Vector3 right = orbit * Vector3.right;
                Vector3 horizontalForward = Vector3.ProjectOnPlane(direction, Vector3.up);
                if (horizontalForward.sqrMagnitude <= 0.0001f)
                    horizontalForward = Vector3.forward;
                else
                    horizontalForward.Normalize();
                Vector3 panOffset = radius * (
                    right * request.panX
                    + Vector3.up * request.panY
                    + horizontalForward * request.panZ);
                Vector3 target = center + panOffset;

                preview.camera.transform.position = target - direction * cameraDistance;
                preview.camera.transform.rotation = Quaternion.LookRotation(direction, Vector3.up);
                preview.camera.nearClipPlane = Mathf.Max(0.01f, cameraDistance - radius * 2.5f);
                preview.camera.farClipPlane = cameraDistance + radius * 3.5f;

                Texture2D rendered = null;
                try
                {
                    preview.BeginStaticPreview(new Rect(0, 0, request.width, request.height));
                    preview.Render();
                    rendered = preview.EndStaticPreview();
                    return EncodeAssetPreviewRenderTexture(assetPath, rendered, request.width, request.height);
                }
                finally
                {
                    if (rendered != null)
                        UnityEngine.Object.DestroyImmediate(rendered);
                }
            }

            public void Dispose()
            {
                if (preview != null)
                    preview.Cleanup();
                if (instance != null)
                    UnityEngine.Object.DestroyImmediate(instance);
            }
        }

        private const int AssetThumbnailMinSize = 64;
        private const int AssetThumbnailMaxSize = 512;
        private const int AssetThumbnailMaxFrames = 45;
        private const double AssetThumbnailMaxSeconds = 3.0;
        private const int AssetPreviewRenderMinSize = 96;
        private const int AssetPreviewRenderMaxSize = 640;
        private const int AssetPreviewRenderMaxSessions = 8;
        private const double AssetPreviewRenderSessionTtlSeconds = 600.0;
        private static readonly Dictionary<string, AssetPreviewRenderSession> AssetPreviewRenderSessions =
            new Dictionary<string, AssetPreviewRenderSession>();
        private static bool AssetPreviewRenderCleanupRegistered;

        private static async Task<PipeEnvelope> HandleAssetThumbnail(string requestId, string message)
        {
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    BeginAssetThumbnailRequest(requestId, ParseAssetThumbnailRequest(message), tcs);
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.Message));
                }
            });

            try
            {
                return await LocusAsync.WithTimeout(tcs.Task, ExecuteTimeoutMs, "asset_thumbnail");
            }
            catch (TimeoutException)
            {
                return ErrorResponse(requestId, "asset_thumbnail timed out");
            }
        }

        private static async Task<PipeEnvelope> HandleAssetPreviewRender(string requestId, string message)
        {
            var tcs = LocusAsync.CreateTcs<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    tcs.SetResult(OkResponse(requestId, RenderAssetPreview(ParseAssetPreviewRenderRequest(message))));
                }
                catch (Exception ex)
                {
                    tcs.SetResult(ErrorResponse(requestId, ex.Message));
                }
            });

            try
            {
                return await LocusAsync.WithTimeout(tcs.Task, ExecuteTimeoutMs, "asset_preview_render");
            }
            catch (TimeoutException)
            {
                return ErrorResponse(requestId, "asset_preview_render timed out");
            }
        }

        private static AssetThumbnailRequest ParseAssetThumbnailRequest(string json)
        {
            AssetThumbnailRequest request = null;
            if (!string.IsNullOrEmpty(json))
                request = JsonUtility.FromJson<AssetThumbnailRequest>(json);

            if (request == null)
                request = new AssetThumbnailRequest();

            request.assetPath = TrimToProjectAssetPath(request.assetPath);
            request.maxSize = Mathf.Clamp(
                request.maxSize <= 0 ? 192 : request.maxSize,
                AssetThumbnailMinSize,
                AssetThumbnailMaxSize);
            return request;
        }

        private static AssetPreviewRenderRequest ParseAssetPreviewRenderRequest(string json)
        {
            AssetPreviewRenderRequest request = null;
            if (!string.IsNullOrEmpty(json))
                request = JsonUtility.FromJson<AssetPreviewRenderRequest>(json);

            if (request == null)
                request = new AssetPreviewRenderRequest();

            request.assetPath = TrimToProjectAssetPath(request.assetPath);
            request.width = Mathf.Clamp(
                request.width <= 0 ? 320 : request.width,
                AssetPreviewRenderMinSize,
                AssetPreviewRenderMaxSize);
            request.height = Mathf.Clamp(
                request.height <= 0 ? 220 : request.height,
                AssetPreviewRenderMinSize,
                AssetPreviewRenderMaxSize);
            request.yaw = Mathf.Repeat(request.yaw, 360f);
            request.pitch = Mathf.Clamp(request.pitch, -80f, 80f);
            request.distance = Mathf.Clamp(request.distance <= 0f ? 1.15f : request.distance, 0.7f, 3.5f);
            request.panX = Mathf.Clamp(request.panX, -8f, 8f);
            request.panY = Mathf.Clamp(request.panY, -8f, 8f);
            request.panZ = Mathf.Clamp(request.panZ, -8f, 8f);
            return request;
        }

        private static void BeginAssetThumbnailRequest(
            string requestId,
            AssetThumbnailRequest request,
            TaskCompletionSource<PipeEnvelope> tcs)
        {
            string assetPath = (request.assetPath ?? "").Trim().Replace('\\', '/');
            if (string.IsNullOrEmpty(assetPath))
                throw new InvalidOperationException("Asset path is empty.");
            if (!IsProjectAssetPath(assetPath))
                throw new InvalidOperationException("Asset thumbnail path must start with Assets/ or Packages/.");

            UnityEngine.Object asset = AssetDatabase.LoadMainAssetAtPath(assetPath);
            if (asset == null)
                throw new InvalidOperationException("Asset was not found: " + assetPath);

            int instanceId = asset.GetInstanceID();
            int frame = 0;
            double startedAt = EditorApplication.timeSinceStartup;
            EditorApplication.CallbackFunction poll = null;
            poll = delegate
            {
                frame++;
                try
                {
                    Texture2D preview = AssetPreview.GetAssetPreview(asset);
                    if (preview != null)
                    {
                        EditorApplication.update -= poll;
                        tcs.SetResult(OkResponse(requestId, EncodeAssetThumbnail(assetPath, preview, request.maxSize)));
                        return;
                    }

                    bool stillLoading = AssetPreview.IsLoadingAssetPreview(instanceId);
                    bool timedOut = frame >= AssetThumbnailMaxFrames
                        || EditorApplication.timeSinceStartup - startedAt >= AssetThumbnailMaxSeconds;
                    if (timedOut || (!stillLoading && frame >= 8))
                    {
                        EditorApplication.update -= poll;
                        tcs.SetResult(ErrorResponse(requestId, "Asset thumbnail unavailable: " + assetPath));
                    }
                }
                catch (Exception ex)
                {
                    EditorApplication.update -= poll;
                    tcs.SetResult(ErrorResponse(requestId, ex.Message));
                }
            };

            poll();
            if (!tcs.Task.IsCompleted)
                EditorApplication.update += poll;
        }

        private static string RenderAssetPreview(AssetPreviewRenderRequest request)
        {
            string assetPath = (request.assetPath ?? "").Trim().Replace('\\', '/');
            if (string.IsNullOrEmpty(assetPath))
                throw new InvalidOperationException("Asset path is empty.");
            if (!IsProjectAssetPath(assetPath))
                throw new InvalidOperationException("Asset preview path must start with Assets/ or Packages/.");

            SweepAssetPreviewRenderSessions();
            return GetAssetPreviewRenderSession(assetPath).Render(request);
        }

        private static AssetPreviewRenderSession GetAssetPreviewRenderSession(string assetPath)
        {
            EnsureAssetPreviewRenderCleanupRegistered();

            UnityEngine.Object asset = AssetDatabase.LoadMainAssetAtPath(assetPath);
            GameObject source = asset as GameObject;
            if (source == null)
                throw new InvalidOperationException("Asset interactive preview only supports prefab or model assets: " + assetPath);

            string dependencyHash = AssetDatabase.GetAssetDependencyHash(assetPath).ToString();
            AssetPreviewRenderSession session;
            if (AssetPreviewRenderSessions.TryGetValue(assetPath, out session))
            {
                if (session.dependencyHash == dependencyHash)
                {
                    session.lastUsedAt = EditorApplication.timeSinceStartup;
                    return session;
                }

                session.Dispose();
                AssetPreviewRenderSessions.Remove(assetPath);
            }

            session = new AssetPreviewRenderSession(assetPath, dependencyHash, source);
            AssetPreviewRenderSessions[assetPath] = session;
            TrimAssetPreviewRenderSessions();
            return session;
        }

        private static void EnsureAssetPreviewRenderCleanupRegistered()
        {
            if (AssetPreviewRenderCleanupRegistered)
                return;

            AssetPreviewRenderCleanupRegistered = true;
            EditorApplication.quitting += ClearAssetPreviewRenderSessions;
        }

        private static void SweepAssetPreviewRenderSessions()
        {
            double now = EditorApplication.timeSinceStartup;
            var expired = new List<string>();
            foreach (var entry in AssetPreviewRenderSessions)
            {
                if (now - entry.Value.lastUsedAt > AssetPreviewRenderSessionTtlSeconds)
                    expired.Add(entry.Key);
            }

            for (int i = 0; i < expired.Count; i++)
                RemoveAssetPreviewRenderSession(expired[i]);
        }

        private static void TrimAssetPreviewRenderSessions()
        {
            while (AssetPreviewRenderSessions.Count > AssetPreviewRenderMaxSessions)
            {
                string oldestKey = null;
                double oldestTime = double.MaxValue;
                foreach (var entry in AssetPreviewRenderSessions)
                {
                    if (entry.Value.lastUsedAt < oldestTime)
                    {
                        oldestKey = entry.Key;
                        oldestTime = entry.Value.lastUsedAt;
                    }
                }

                if (string.IsNullOrEmpty(oldestKey))
                    break;
                RemoveAssetPreviewRenderSession(oldestKey);
            }
        }

        private static void RemoveAssetPreviewRenderSession(string assetPath)
        {
            AssetPreviewRenderSession session;
            if (!AssetPreviewRenderSessions.TryGetValue(assetPath, out session))
                return;

            AssetPreviewRenderSessions.Remove(assetPath);
            session.Dispose();
        }

        private static void ClearAssetPreviewRenderSessions()
        {
            var keys = new List<string>(AssetPreviewRenderSessions.Keys);
            for (int i = 0; i < keys.Count; i++)
                RemoveAssetPreviewRenderSession(keys[i]);
        }

        private static void SetHideFlagsRecursive(Transform transform, HideFlags flags)
        {
            transform.gameObject.hideFlags = flags;
            for (int i = 0; i < transform.childCount; i++)
                SetHideFlagsRecursive(transform.GetChild(i), flags);
        }

        private static Bounds CalculatePreviewBounds(GameObject instance)
        {
            Renderer[] renderers = instance.GetComponentsInChildren<Renderer>(true);
            if (renderers == null || renderers.Length == 0)
                return new Bounds(instance.transform.position, Vector3.one);

            Bounds bounds = renderers[0].bounds;
            for (int i = 1; i < renderers.Length; i++)
                bounds.Encapsulate(renderers[i].bounds);
            if (bounds.extents.sqrMagnitude <= 0.0001f)
                return new Bounds(bounds.center, Vector3.one);
            return bounds;
        }

        private static string EncodeAssetThumbnail(string assetPath, Texture source, int maxSize)
        {
            int sourceWidth = Mathf.Max(1, source.width);
            int sourceHeight = Mathf.Max(1, source.height);
            float scale = Mathf.Min(1f, maxSize / (float)Mathf.Max(sourceWidth, sourceHeight));
            int width = Mathf.Max(1, Mathf.RoundToInt(sourceWidth * scale));
            int height = Mathf.Max(1, Mathf.RoundToInt(sourceHeight * scale));

            return EncodeAssetPreviewTexture(assetPath, source, width, height);
        }

        private static string EncodeAssetPreviewRenderTexture(string assetPath, Texture source, int width, int height)
        {
            RenderTexture previous = RenderTexture.active;
            RenderTexture rt = RenderTexture.GetTemporary(width, height, 0, RenderTextureFormat.ARGB32);
            Texture2D readable = null;
            try
            {
                Graphics.Blit(source, rt);
                RenderTexture.active = rt;
                readable = new Texture2D(width, height, TextureFormat.RGBA32, false);
                readable.ReadPixels(new Rect(0, 0, width, height), 0, 0);
                readable.Apply(false, false);
                byte[] png = readable.EncodeToPNG();

                var response = new AssetPreviewRenderResponse
                {
                    assetPath = assetPath,
                    width = width,
                    height = height,
                    mimeType = "image/png",
                    dataBase64 = Convert.ToBase64String(png)
                };
                return JsonUtility.ToJson(response);
            }
            finally
            {
                RenderTexture.active = previous;
                RenderTexture.ReleaseTemporary(rt);
                if (readable != null)
                    UnityEngine.Object.DestroyImmediate(readable);
            }
        }

        private static string EncodeAssetPreviewTexture(string assetPath, Texture source, int width, int height)
        {
            RenderTexture previous = RenderTexture.active;
            RenderTexture rt = RenderTexture.GetTemporary(width, height, 0, RenderTextureFormat.ARGB32);
            Texture2D readable = null;
            try
            {
                Graphics.Blit(source, rt);
                RenderTexture.active = rt;
                readable = new Texture2D(width, height, TextureFormat.RGBA32, false);
                readable.ReadPixels(new Rect(0, 0, width, height), 0, 0);
                readable.Apply(false, false);
                byte[] png = readable.EncodeToPNG();

                var response = new AssetThumbnailResponse
                {
                    assetPath = assetPath,
                    width = width,
                    height = height,
                    mimeType = "image/png",
                    pngBase64 = Convert.ToBase64String(png)
                };
                return JsonUtility.ToJson(response);
            }
            finally
            {
                RenderTexture.active = previous;
                RenderTexture.ReleaseTemporary(rt);
                if (readable != null)
                    UnityEngine.Object.DestroyImmediate(readable);
            }
        }
    }
}
