using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using UnityEditor;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.Rendering;

namespace Locus.Skills
{
    /// <summary>
    /// Stable public facade over Unity 6.3/6.5's built-in Frame Debugger.
    /// Unity keeps its detailed API internal, so all version-sensitive access
    /// is isolated in FrameDebuggerReflection.
    /// </summary>
    public static class FrameDebuggerApi
    {
        public static FrameDebuggerStatus Status()
        {
            return FrameDebuggerRuntime.Status();
        }

        public static FrameDebuggerStatus Enable(int? remotePlayerId = null)
        {
            return FrameDebuggerRuntime.Enable(remotePlayerId);
        }

        public static FrameDebuggerStatus Disable()
        {
            return FrameDebuggerRuntime.Disable();
        }

        public static Task<FrameDebuggerStatus> CaptureAsync(
            int timeoutMs = 15000,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            return FrameDebuggerRuntime.CaptureAsync(timeoutMs, cancellationToken);
        }

        public static FrameEventList Events(FrameEventQuery query = null)
        {
            return FrameDebuggerRuntime.Events(query ?? new FrameEventQuery());
        }

        public static FrameEventSummary Select(int index)
        {
            return FrameDebuggerRuntime.Select(index);
        }

        public static FrameEventDetail Event(int index, FrameEventOptions options = null)
        {
            return FrameDebuggerRuntime.Event(index, options ?? new FrameEventOptions());
        }

        public static Task<FrameTextureExportResult> ExportRenderTargetAsync(
            int index,
            FrameTextureExportOptions options = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            return FrameDebuggerRuntime.ExportRenderTargetAsync(
                index,
                options ?? new FrameTextureExportOptions(),
                cancellationToken);
        }

        public static string Json(object value)
        {
            return FrameDebuggerJson.Serialize(value);
        }
    }

    public enum FrameTextureFormat
    {
        Png,
        Exr,
        Tga
    }

    public sealed class FrameEventQuery
    {
        public int FromIndex { get; set; }
        public int MaxEvents { get; set; } = 120;
        public string NameContains { get; set; }
        public string TypeContains { get; set; }
        public string ObjectNameContains { get; set; }
        public bool Reverse { get; set; }
    }

    public sealed class FrameEventOptions
    {
        public bool IncludeRenderState { get; set; } = true;
        public bool IncludeShaderProperties { get; set; }
        public int MaxShaderProperties { get; set; } = 48;
    }

    public sealed class FrameTextureExportOptions
    {
        public FrameTextureFormat Format { get; set; } = FrameTextureFormat.Png;
        public string OutputDirectory { get; set; }
        public string FileName { get; set; }
        public int RenderTargetIndex { get; set; }
        public string Channels { get; set; } = "RGBA";
        public float BlackLevel { get; set; }
        public float WhiteLevel { get; set; } = 1f;
        public bool? FlipY { get; set; }
        public bool WriteMetadata { get; set; } = true;
    }

    public sealed class FrameDebuggerStatus
    {
        public bool Enabled { get; internal set; }
        public bool LocallySupported { get; internal set; }
        public bool ReceivingRemoteData { get; internal set; }
        public int Count { get; internal set; }
        public int SelectedIndex { get; internal set; }
        public int EventsHash { get; internal set; }
        public int RemotePlayerId { get; internal set; }
        public string UnityVersion { get; internal set; }
        public string EditorStatus { get; internal set; }
    }

    public sealed class FrameEventList
    {
        public int TotalCount { get; internal set; }
        public int MatchedCount { get; internal set; }
        public bool Truncated { get; internal set; }
        public IList<FrameEventSummary> Events { get; internal set; }
        public int Count { get { return Events == null ? 0 : Events.Count; } }
    }

    public sealed class FrameEventSummary
    {
        public int Index { get; internal set; }
        public string Type { get; internal set; }
        public string Name { get; internal set; }
        public string ObjectName { get; internal set; }
        public string ObjectType { get; internal set; }
    }

    public sealed class FrameRenderTargetInfo
    {
        public string Name { get; internal set; }
        public int Width { get; internal set; }
        public int Height { get; internal set; }
        public string Format { get; internal set; }
        public int FormatCode { get; internal set; }
        public string Dimension { get; internal set; }
        public int CubemapFace { get; internal set; }
        public int TargetCount { get; internal set; }
        public bool HasDepth { get; internal set; }
        public bool HasStencil { get; internal set; }
        public bool IsBackBuffer { get; internal set; }
        public bool Memoryless { get; internal set; }
        public int DisplayIndex { get; internal set; }
        public string LoadAction { get; internal set; }
        public string StoreAction { get; internal set; }
    }

    public sealed class FrameShaderProperty
    {
        public string Category { get; internal set; }
        public string Name { get; internal set; }
        public object Value { get; internal set; }
        public string TextureName { get; internal set; }
        public int Flags { get; internal set; }
    }

    public sealed class FrameEventDetail
    {
        public int Index { get; internal set; }
        public string Type { get; internal set; }
        public string Name { get; internal set; }
        public string ObjectName { get; internal set; }
        public string ObjectType { get; internal set; }
        public int? VertexCount { get; internal set; }
        public int? IndexCount { get; internal set; }
        public int? InstanceCount { get; internal set; }
        public int? DrawCallCount { get; internal set; }
        public string Shader { get; internal set; }
        public string OriginalShader { get; internal set; }
        public string Pass { get; internal set; }
        public string LightMode { get; internal set; }
        public int? SubShaderIndex { get; internal set; }
        public int? ShaderPassIndex { get; internal set; }
        public string ShaderKeywords { get; internal set; }
        public string Mesh { get; internal set; }
        public int? MeshSubset { get; internal set; }
        public string ComputeShader { get; internal set; }
        public string ComputeKernel { get; internal set; }
        public int[] ComputeThreadGroups { get; internal set; }
        public int[] ComputeGroupSize { get; internal set; }
        public string RayTracingShader { get; internal set; }
        public string RayTracingPass { get; internal set; }
        public int[] RayTracingDispatch { get; internal set; }
        public FrameRenderTargetInfo RenderTarget { get; internal set; }
        public IDictionary<string, object> BlendState { get; internal set; }
        public IDictionary<string, object> RasterState { get; internal set; }
        public IDictionary<string, object> DepthState { get; internal set; }
        public IDictionary<string, object> StencilState { get; internal set; }
        public IList<FrameShaderProperty> ShaderProperties { get; internal set; }
        public bool ShaderPropertiesTruncated { get; internal set; }
    }

    public sealed class FrameTextureExportResult
    {
        public int EventIndex { get; internal set; }
        public int RenderTargetIndex { get; internal set; }
        public string TexturePath { get; internal set; }
        public string MetadataPath { get; internal set; }
        public string Format { get; internal set; }
        public int Width { get; internal set; }
        public int Height { get; internal set; }
        public string SourceFormat { get; internal set; }
        public string Channels { get; internal set; }
        public bool FlippedY { get; internal set; }
        public FrameEventSummary Event { get; internal set; }
    }

    internal static class FrameDebuggerRuntime
    {
        internal static FrameDebuggerStatus Status()
        {
            FrameDebuggerReflection.EnsureAvailable();
            int limit = FrameDebuggerReflection.IntProperty("limit", 0);
            return new FrameDebuggerStatus
            {
                Enabled = UnityEngine.FrameDebugger.enabled,
                LocallySupported = FrameDebuggerReflection.BoolProperty("locallySupported", false),
                ReceivingRemoteData = FrameDebuggerReflection.BoolProperty("receivingRemoteFrameEventData", false),
                Count = FrameDebuggerReflection.IntProperty("count", 0),
                SelectedIndex = limit <= 0 ? -1 : limit - 1,
                EventsHash = FrameDebuggerReflection.IntProperty("eventsHash", 0),
                RemotePlayerId = Convert.ToInt32(FrameDebuggerReflection.Invoke("GetRemotePlayerGUID"), CultureInfo.InvariantCulture),
                UnityVersion = Application.unityVersion,
                EditorStatus = EditorApplication.isPlaying
                    ? (EditorApplication.isPaused ? "playing_paused" : "playing")
                    : "editing"
            };
        }

        internal static FrameDebuggerStatus Enable(int? remotePlayerId)
        {
            FrameDebuggerReflection.EnsureAvailable();
            if (UnityEngine.FrameDebugger.enabled)
                return Status();
            if (!remotePlayerId.HasValue && !FrameDebuggerReflection.BoolProperty("locallySupported", false))
                throw new InvalidOperationException("Unity reports that local Frame Debugger capture is unsupported for the current graphics device.");
            EnsurePlayModeView();
            if (!remotePlayerId.HasValue && EditorApplication.isPlaying && !EditorApplication.isPaused)
                EditorApplication.isPaused = true;
            if (!remotePlayerId.HasValue && FrameDebuggerReflection.TryEnableWithBuiltInWindow())
                return Status();
            int target = remotePlayerId ?? FrameDebuggerReflection.ConnectedProfilerId();
            if (target < 0)
                target = 0;
            FrameDebuggerReflection.Invoke("SetEnabled", true, target);
            return Status();
        }

        internal static FrameDebuggerStatus Disable()
        {
            FrameDebuggerReflection.EnsureAvailable();
            if (UnityEngine.FrameDebugger.enabled)
            {
                if (!FrameDebuggerReflection.TryDisableWithBuiltInWindow())
                {
                    int target = Convert.ToInt32(
                        FrameDebuggerReflection.Invoke("GetRemotePlayerGUID"),
                        CultureInfo.InvariantCulture);
                    FrameDebuggerReflection.Invoke("SetEnabled", false, target);
                }
                SceneView.RepaintAll();
            }
            return Status();
        }

        internal static async Task<FrameDebuggerStatus> CaptureAsync(
            int timeoutMs,
            CancellationToken cancellationToken)
        {
            timeoutMs = Math.Max(100, Math.Min(60000, timeoutMs));
            Enable(null);
            double deadline = EditorApplication.timeSinceStartup + timeoutMs / 1000.0;
            await WaitUntilAsync(
                delegate
                {
                    RepaintRenderingViews();
                    return UnityEngine.FrameDebugger.enabled
                        && FrameDebuggerReflection.IntProperty("count", 0) > 0
                        && !FrameDebuggerReflection.BoolProperty("receivingRemoteFrameEventData", false);
                },
                deadline,
                cancellationToken,
                "Timed out waiting for Unity Frame Debugger events.");
            return Status();
        }

        internal static FrameEventList Events(FrameEventQuery query)
        {
            RequireCapturedFrame();
            Array events = FrameDebuggerReflection.Events();
            int from = Math.Max(0, query.FromIndex);
            int maximum = Math.Max(1, Math.Min(5000, query.MaxEvents));
            List<FrameEventSummary> matched = new List<FrameEventSummary>();
            int matchedCount = 0;
            if (!query.Reverse)
            {
                for (int i = from; i < events.Length; i++)
                    AddMatchingEvent(events, i, query, maximum, matched, ref matchedCount);
            }
            else
            {
                int start = query.FromIndex > 0 ? Math.Min(query.FromIndex, events.Length - 1) : events.Length - 1;
                for (int i = start; i >= 0; i--)
                    AddMatchingEvent(events, i, query, maximum, matched, ref matchedCount);
            }
            return new FrameEventList
            {
                TotalCount = events.Length,
                MatchedCount = matchedCount,
                Truncated = matchedCount > matched.Count,
                Events = matched
            };
        }

        internal static FrameEventSummary Select(int index)
        {
            RequireCapturedFrame();
            Array events = FrameDebuggerReflection.Events();
            ValidateEventIndex(index, events.Length);
            FrameDebuggerReflection.SetProperty("limit", index + 1);
            SceneView.RepaintAll();
            return Summary(events, index);
        }

        internal static FrameEventDetail Event(int index, FrameEventOptions options)
        {
            RequireCapturedFrame();
            Array events = FrameDebuggerReflection.Events();
            ValidateEventIndex(index, events.Length);
            FrameDebuggerReflection.SetProperty("limit", index + 1);
            object data = FrameDebuggerReflection.EventData(index);
            FrameEventSummary summary = Summary(events, index);
            FrameEventDetail detail = new FrameEventDetail
            {
                Index = index,
                Type = summary.Type,
                Name = summary.Name,
                ObjectName = summary.ObjectName,
                ObjectType = summary.ObjectType,
                VertexCount = PositiveInt(data, "m_VertexCount"),
                IndexCount = PositiveInt(data, "m_IndexCount"),
                InstanceCount = PositiveInt(data, "m_InstanceCount"),
                DrawCallCount = PositiveInt(data, "m_DrawCallCount"),
                Shader = Text(data, "m_RealShaderName"),
                OriginalShader = Text(data, "m_OriginalShaderName"),
                Pass = Text(data, "m_PassName"),
                LightMode = Text(data, "m_PassLightMode"),
                SubShaderIndex = NonNegativeInt(data, "m_SubShaderIndex"),
                ShaderPassIndex = NonNegativeInt(data, "m_ShaderPassIndex"),
                ShaderKeywords = Text(data, "shaderKeywords"),
                Mesh = ObjectName(FrameDebuggerReflection.Field(data, "m_Mesh")),
                MeshSubset = NonNegativeInt(data, "m_MeshSubset"),
                ComputeShader = Text(data, "m_ComputeShaderName"),
                ComputeKernel = Text(data, "m_ComputeShaderKernelName"),
                RayTracingShader = Text(data, "m_RayTracingShaderName"),
                RayTracingPass = Text(data, "m_RayTracingShaderPassName"),
                RenderTarget = RenderTarget(data)
            };
            if (!string.IsNullOrEmpty(detail.ComputeShader))
            {
                detail.ComputeThreadGroups = IntArray(data,
                    "m_ComputeShaderThreadGroupsX", "m_ComputeShaderThreadGroupsY", "m_ComputeShaderThreadGroupsZ");
                detail.ComputeGroupSize = IntArray(data,
                    "m_ComputeShaderGroupSizeX", "m_ComputeShaderGroupSizeY", "m_ComputeShaderGroupSizeZ");
            }
            if (!string.IsNullOrEmpty(detail.RayTracingShader))
            {
                detail.RayTracingDispatch = IntArray(data,
                    "m_RayTracingShaderWidth", "m_RayTracingShaderHeight", "m_RayTracingShaderDepth");
            }
            if (options.IncludeRenderState)
            {
                detail.BlendState = State(FrameDebuggerReflection.Field(data, "m_BlendState"));
                detail.RasterState = State(FrameDebuggerReflection.Field(data, "m_RasterState"));
                detail.DepthState = State(FrameDebuggerReflection.Field(data, "m_DepthState"));
                detail.StencilState = State(FrameDebuggerReflection.Field(data, "m_StencilState"));
            }
            if (options.IncludeShaderProperties)
            {
                int maximum = Math.Max(1, Math.Min(512, options.MaxShaderProperties));
                detail.ShaderProperties = ShaderProperties(data, maximum, out bool truncated);
                detail.ShaderPropertiesTruncated = truncated;
            }
            return detail;
        }

        internal static async Task<FrameTextureExportResult> ExportRenderTargetAsync(
            int index,
            FrameTextureExportOptions options,
            CancellationToken cancellationToken)
        {
            RequireCapturedFrame();
            if (options.WhiteLevel <= options.BlackLevel)
                throw new ArgumentException("WhiteLevel must be greater than BlackLevel.");
            Select(index);
            Vector4 channels = Channels(options.Channels);
            FrameDebuggerReflection.Invoke(
                "SetRenderTargetDisplayOptions",
                options.RenderTargetIndex,
                channels,
                options.BlackLevel,
                options.WhiteLevel);
            await WaitEditorUpdatesAsync(2, cancellationToken);
            object data = FrameDebuggerReflection.EventData(index);
            RenderTexture source = FrameDebuggerReflection.Field(data, "m_RenderTargetRenderTexture") as RenderTexture;
            FrameRenderTargetInfo sourceInfo = RenderTarget(data);
            if (source == null)
            {
                throw new InvalidOperationException(
                    "Frame event " + index + " has no readable Render Target for display index "
                    + options.RenderTargetIndex + ".");
            }

            int width = sourceInfo.Width > 0 ? sourceInfo.Width : source.width;
            int height = sourceInfo.Height > 0 ? sourceInfo.Height : source.height;
            bool flipY = options.FlipY ?? (sourceInfo.IsBackBuffer && SystemInfo.graphicsUVStartsAtTop);
            string directory = ResolveOutputDirectory(options.OutputDirectory);
            Directory.CreateDirectory(directory);
            string extension = Extension(options.Format);
            string fileName = ResolveFileName(options.FileName, index, extension);
            string texturePath = Path.GetFullPath(Path.Combine(directory, fileName));

            RenderTexture copy = null;
            Texture2D readable = null;
            RenderTexture previous = RenderTexture.active;
            try
            {
                RenderTextureFormat copyFormat = options.Format == FrameTextureFormat.Exr
                    ? RenderTextureFormat.ARGBHalf
                    : RenderTextureFormat.ARGB32;
                copy = RenderTexture.GetTemporary(
                    width,
                    height,
                    0,
                    copyFormat,
                    RenderTextureReadWrite.Linear);
                FrameDebuggerReflection.BlitRenderTarget(
                    source,
                    copy,
                    width,
                    height,
                    channels,
                    new Vector4(options.BlackLevel, options.WhiteLevel, 0f, 0f),
                    flipY,
                    QualitySettings.activeColorSpace == ColorSpace.Linear);
                RenderTexture.active = copy;
                TextureFormat readableFormat = options.Format == FrameTextureFormat.Exr
                    ? TextureFormat.RGBAHalf
                    : TextureFormat.RGBA32;
                readable = new Texture2D(width, height, readableFormat, false, true);
                readable.ReadPixels(new Rect(0, 0, width, height), 0, 0, false);
                readable.Apply(false, false);
                byte[] bytes;
                switch (options.Format)
                {
                    case FrameTextureFormat.Exr:
                        bytes = readable.EncodeToEXR(Texture2D.EXRFlags.CompressZIP);
                        break;
                    case FrameTextureFormat.Tga:
                        bytes = readable.EncodeToTGA();
                        break;
                    default:
                        bytes = readable.EncodeToPNG();
                        break;
                }
                if (bytes == null || bytes.Length == 0)
                    throw new InvalidOperationException("Unity returned an empty encoded texture.");
                File.WriteAllBytes(texturePath, bytes);
            }
            finally
            {
                RenderTexture.active = previous;
                if (readable != null)
                    UnityEngine.Object.DestroyImmediate(readable);
                if (copy != null)
                    RenderTexture.ReleaseTemporary(copy);
            }

            FrameEventSummary summary = Summary(FrameDebuggerReflection.Events(), index);
            FrameTextureExportResult result = new FrameTextureExportResult
            {
                EventIndex = index,
                RenderTargetIndex = options.RenderTargetIndex,
                TexturePath = texturePath.Replace('\\', '/'),
                Format = options.Format.ToString().ToLowerInvariant(),
                Width = width,
                Height = height,
                SourceFormat = sourceInfo.Format,
                Channels = NormalizeChannels(options.Channels),
                FlippedY = flipY,
                Event = summary
            };
            if (options.WriteMetadata)
            {
                string metadataPath = Path.ChangeExtension(texturePath, null) + ".metadata.json";
                File.WriteAllText(metadataPath, FrameDebuggerJson.Serialize(new
                {
                    export = result,
                    renderTarget = sourceInfo,
                    unityVersion = Application.unityVersion,
                    graphicsDevice = SystemInfo.graphicsDeviceType.ToString(),
                    colorSpace = QualitySettings.activeColorSpace.ToString()
                }), new UTF8Encoding(false));
                result.MetadataPath = metadataPath.Replace('\\', '/');
            }
            return result;
        }

        private static void AddMatchingEvent(
            Array events,
            int index,
            FrameEventQuery query,
            int maximum,
            List<FrameEventSummary> output,
            ref int matchedCount)
        {
            FrameEventSummary summary = Summary(events, index);
            if (!Contains(summary.Name, query.NameContains)
                || !Contains(summary.Type, query.TypeContains)
                || !Contains(summary.ObjectName, query.ObjectNameContains))
                return;
            matchedCount++;
            if (output.Count < maximum)
                output.Add(summary);
        }

        private static FrameEventSummary Summary(Array events, int index)
        {
            object item = events.GetValue(index);
            object type = FrameDebuggerReflection.Field(item, "m_Type");
            UnityEngine.Object owner = FrameDebuggerReflection.Field(item, "m_Obj") as UnityEngine.Object;
            if (owner == null)
                owner = FrameDebuggerReflection.Invoke("GetFrameEventObject", index) as UnityEngine.Object;
            string name = Convert.ToString(
                FrameDebuggerReflection.Invoke("GetFrameEventInfoName", index),
                CultureInfo.InvariantCulture);
            return new FrameEventSummary
            {
                Index = index,
                Type = type == null ? null : type.ToString(),
                Name = name,
                ObjectName = owner == null ? null : owner.name,
                ObjectType = owner == null ? null : owner.GetType().Name
            };
        }

        private static FrameRenderTargetInfo RenderTarget(object data)
        {
            int formatCode = Int(data, "m_RenderTargetFormat");
            string format = EnumName(typeof(GraphicsFormat), formatCode);
            RenderTexture rt = FrameDebuggerReflection.Field(data, "m_RenderTargetRenderTexture") as RenderTexture;
            if (rt != null && rt.graphicsFormat != GraphicsFormat.None)
                format = rt.graphicsFormat.ToString();
            return new FrameRenderTargetInfo
            {
                Name = Text(data, "m_RenderTargetName") ?? (rt == null ? null : rt.name),
                Width = Int(data, "m_RenderTargetWidth"),
                Height = Int(data, "m_RenderTargetHeight"),
                Format = format,
                FormatCode = formatCode,
                Dimension = EnumName(typeof(TextureDimension), Int(data, "m_RenderTargetDimension")),
                CubemapFace = Int(data, "m_RenderTargetCubemapFace"),
                TargetCount = Int(data, "m_RenderTargetCount"),
                HasDepth = Int(data, "m_RenderTargetHasDepthTexture") != 0,
                HasStencil = Int(data, "m_RenderTargetHasStencilBits") != 0,
                IsBackBuffer = Bool(data, "m_RenderTargetIsBackBuffer"),
                Memoryless = Int(data, "m_RenderTargetMemoryless") != 0,
                DisplayIndex = Int(data, "m_RTDisplayIndex"),
                LoadAction = EnumName(typeof(RenderBufferLoadAction), Int(data, "m_RenderTargetLoadAction")),
                StoreAction = EnumName(typeof(RenderBufferStoreAction), Int(data, "m_RenderTargetStoreAction"))
            };
        }

        private static IDictionary<string, object> State(object state)
        {
            if (state == null)
                return null;
            Dictionary<string, object> output = new Dictionary<string, object>(StringComparer.Ordinal);
            FieldInfo[] fields = state.GetType().GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            for (int i = 0; i < fields.Length; i++)
            {
                string name = fields[i].Name;
                if (name.StartsWith("m_", StringComparison.Ordinal))
                    name = name.Substring(2);
                object value = fields[i].GetValue(state);
                output[name] = value is Enum ? value.ToString() : value;
            }
            return output;
        }

        private static IList<FrameShaderProperty> ShaderProperties(
            object data,
            int maximum,
            out bool truncated)
        {
            List<FrameShaderProperty> output = new List<FrameShaderProperty>();
            object info = FrameDebuggerReflection.Field(data, "m_ShaderInfo");
            int total = 0;
            AddShaderProperties(info, "Keyword", "m_Keywords", output, maximum, ref total);
            AddShaderProperties(info, "Texture", "m_Textures", output, maximum, ref total);
            AddShaderProperties(info, "Int", "m_Ints", output, maximum, ref total);
            AddShaderProperties(info, "Float", "m_Floats", output, maximum, ref total);
            AddShaderProperties(info, "Vector", "m_Vectors", output, maximum, ref total);
            AddShaderProperties(info, "Matrix", "m_Matrices", output, maximum, ref total);
            AddShaderProperties(info, "Buffer", "m_Buffers", output, maximum, ref total);
            AddShaderProperties(info, "ConstantBuffer", "m_CBuffers", output, maximum, ref total);
            truncated = total > output.Count;
            return output;
        }

        private static void AddShaderProperties(
            object info,
            string category,
            string field,
            List<FrameShaderProperty> output,
            int maximum,
            ref int total)
        {
            Array values = FrameDebuggerReflection.Field(info, field) as Array;
            if (values == null)
                return;
            for (int i = 0; i < values.Length; i++)
            {
                total++;
                if (output.Count >= maximum)
                    continue;
                object value = values.GetValue(i);
                object propertyValue = FrameDebuggerReflection.Field(value, "m_Value");
                if (propertyValue is UnityEngine.Object)
                    propertyValue = ObjectName(propertyValue);
                output.Add(new FrameShaderProperty
                {
                    Category = category,
                    Name = Text(value, "m_Name"),
                    Value = CompactValue(propertyValue),
                    TextureName = Text(value, "m_TextureName"),
                    Flags = Int(value, "m_Flags")
                });
            }
        }

        private static object CompactValue(object value)
        {
            if (value is Matrix4x4)
            {
                Matrix4x4 matrix = (Matrix4x4)value;
                float[] output = new float[16];
                for (int i = 0; i < 16; i++)
                    output[i] = matrix[i];
                return output;
            }
            if (value is Vector4)
            {
                Vector4 vector = (Vector4)value;
                return new float[] { vector.x, vector.y, vector.z, vector.w };
            }
            return value;
        }

        private static async Task WaitUntilAsync(
            Func<bool> predicate,
            double deadline,
            CancellationToken cancellationToken,
            string timeoutMessage)
        {
            TaskCompletionSource<bool> source = new TaskCompletionSource<bool>();
            EditorApplication.CallbackFunction callback = null;
            CancellationTokenRegistration registration = default(CancellationTokenRegistration);
            callback = delegate
            {
                try
                {
                    if (cancellationToken.IsCancellationRequested)
                    {
                        EditorApplication.update -= callback;
                        source.TrySetCanceled();
                    }
                    else if (predicate())
                    {
                        EditorApplication.update -= callback;
                        source.TrySetResult(true);
                    }
                    else if (EditorApplication.timeSinceStartup >= deadline)
                    {
                        EditorApplication.update -= callback;
                        source.TrySetException(new TimeoutException(timeoutMessage));
                    }
                }
                catch (Exception exception)
                {
                    EditorApplication.update -= callback;
                    source.TrySetException(exception);
                }
            };
            if (cancellationToken.CanBeCanceled)
            {
                registration = cancellationToken.Register(delegate
                {
                    EditorApplication.delayCall += delegate
                    {
                        EditorApplication.update -= callback;
                        source.TrySetCanceled();
                    };
                });
            }
            EditorApplication.update += callback;
            callback();
            try { await source.Task; }
            finally { registration.Dispose(); }
        }

        private static async Task WaitEditorUpdatesAsync(int count, CancellationToken cancellationToken)
        {
            int remaining = Math.Max(1, count);
            await WaitUntilAsync(
                delegate { return --remaining <= 0; },
                EditorApplication.timeSinceStartup + 5.0,
                cancellationToken,
                "Timed out waiting for Frame Debugger Render Target update.");
        }

        private static void EnsurePlayModeView()
        {
            Type gameViewType = Type.GetType("UnityEditor.GameView,UnityEditor", false);
            if (gameViewType == null)
                gameViewType = FrameDebuggerReflection.FindType("UnityEditor.GameView");
            if (gameViewType == null)
                return;
            UnityEngine.Object[] existing = Resources.FindObjectsOfTypeAll(gameViewType);
            if (existing == null || existing.Length == 0)
                EditorWindow.GetWindow(gameViewType, false, "Game", false);
            RepaintRenderingViews();
        }

        private static void RepaintRenderingViews()
        {
            EditorApplication.QueuePlayerLoopUpdate();
            SceneView.RepaintAll();
            EditorWindow[] windows = Resources.FindObjectsOfTypeAll<EditorWindow>();
            for (int i = 0; i < windows.Length; i++)
            {
                EditorWindow window = windows[i];
                if (window == null)
                    continue;
                string name = window.GetType().Name;
                if (name.IndexOf("GameView", StringComparison.OrdinalIgnoreCase) >= 0
                    || name.IndexOf("PlayModeView", StringComparison.OrdinalIgnoreCase) >= 0)
                    window.Repaint();
            }
        }

        private static void RequireCapturedFrame()
        {
            FrameDebuggerReflection.EnsureAvailable();
            if (!UnityEngine.FrameDebugger.enabled)
                throw new InvalidOperationException("Frame Debugger is disabled. Call FrameDebuggerApi.CaptureAsync() first.");
            if (FrameDebuggerReflection.IntProperty("count", 0) <= 0)
                throw new InvalidOperationException("Frame Debugger has no captured events yet. Await CaptureAsync().");
        }

        private static void ValidateEventIndex(int index, int count)
        {
            if (index < 0 || index >= count)
                throw new ArgumentOutOfRangeException("index", "Frame event index must be between 0 and " + (count - 1) + ".");
        }

        private static bool Contains(string value, string query)
        {
            return string.IsNullOrWhiteSpace(query)
                || (!string.IsNullOrEmpty(value)
                    && value.IndexOf(query.Trim(), StringComparison.OrdinalIgnoreCase) >= 0);
        }

        private static int Int(object value, string field)
        {
            object raw = FrameDebuggerReflection.Field(value, field);
            return raw == null ? 0 : Convert.ToInt32(raw, CultureInfo.InvariantCulture);
        }

        private static int? PositiveInt(object value, string field)
        {
            int raw = Int(value, field);
            return raw > 0 ? (int?)raw : null;
        }

        private static int? NonNegativeInt(object value, string field)
        {
            object raw = FrameDebuggerReflection.Field(value, field);
            if (raw == null)
                return null;
            int converted = Convert.ToInt32(raw, CultureInfo.InvariantCulture);
            return converted >= 0 ? (int?)converted : null;
        }

        private static bool Bool(object value, string field)
        {
            object raw = FrameDebuggerReflection.Field(value, field);
            return raw != null && Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
        }

        private static string Text(object value, string field)
        {
            object raw = FrameDebuggerReflection.Field(value, field);
            string text = raw == null ? null : Convert.ToString(raw, CultureInfo.InvariantCulture);
            return string.IsNullOrEmpty(text) ? null : text;
        }

        private static int[] IntArray(object value, params string[] fields)
        {
            int[] output = new int[fields.Length];
            for (int i = 0; i < fields.Length; i++)
                output[i] = Int(value, fields[i]);
            return output;
        }

        private static string ObjectName(object value)
        {
            UnityEngine.Object unityObject = value as UnityEngine.Object;
            return unityObject == null ? null : unityObject.name;
        }

        private static string EnumName(Type type, int value)
        {
            return Enum.IsDefined(type, value)
                ? Enum.GetName(type, value)
                : value.ToString(CultureInfo.InvariantCulture);
        }

        private static Vector4 Channels(string value)
        {
            switch (NormalizeChannels(value))
            {
                case "R": return new Vector4(1, 0, 0, 0);
                case "G": return new Vector4(0, 1, 0, 0);
                case "B": return new Vector4(0, 0, 1, 0);
                case "A": return new Vector4(0, 0, 0, 1);
                case "RGB": return new Vector4(1, 1, 1, 0);
                default: return Vector4.one;
            }
        }

        private static string NormalizeChannels(string value)
        {
            string normalized = (value ?? "RGBA").Trim().ToUpperInvariant();
            if (normalized != "RGBA" && normalized != "RGB" && normalized != "R"
                && normalized != "G" && normalized != "B" && normalized != "A")
                throw new ArgumentException("Channels must be RGBA, RGB, R, G, B, or A.");
            return normalized;
        }

        private static string ResolveOutputDirectory(string value)
        {
            string directory = string.IsNullOrWhiteSpace(value)
                ? Path.Combine("Library", "Locus", "FrameDebugger")
                : value.Trim();
            if (!Path.IsPathRooted(directory))
            {
                string projectRoot = Directory.GetParent(Application.dataPath).FullName;
                directory = Path.Combine(projectRoot, directory);
            }
            return Path.GetFullPath(directory);
        }

        private static string ResolveFileName(string value, int index, string extension)
        {
            string fileName = string.IsNullOrWhiteSpace(value)
                ? DateTime.Now.ToString("yyyyMMdd-HHmmss-fff", CultureInfo.InvariantCulture)
                    + "-event-" + index.ToString("D4", CultureInfo.InvariantCulture)
                : Path.GetFileName(value.Trim());
            string currentExtension = Path.GetExtension(fileName);
            if (!string.IsNullOrEmpty(currentExtension))
                fileName = fileName.Substring(0, fileName.Length - currentExtension.Length);
            foreach (char invalid in Path.GetInvalidFileNameChars())
                fileName = fileName.Replace(invalid, '_');
            return fileName + extension;
        }

        private static string Extension(FrameTextureFormat format)
        {
            switch (format)
            {
                case FrameTextureFormat.Exr: return ".exr";
                case FrameTextureFormat.Tga: return ".tga";
                default: return ".png";
            }
        }
    }

    internal static class FrameDebuggerReflection
    {
        private const BindingFlags AllStatic = BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic;
        private const BindingFlags AllInstance = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        private static Type _utilityType;
        private static Type _eventDataType;
        private static Type _helperType;

        internal static void EnsureAvailable()
        {
            if (_utilityType != null)
                return;
            _utilityType = FindType("UnityEditorInternal.FrameDebuggerInternal.FrameDebuggerUtility");
            _eventDataType = FindType("UnityEditorInternal.FrameDebuggerInternal.FrameDebuggerEventData");
            _helperType = FindType("UnityEditorInternal.FrameDebuggerInternal.FrameDebuggerHelper");
            if (_utilityType == null || _eventDataType == null)
                throw new NotSupportedException("Unity Frame Debugger internals were not found. Unity 6000.3 or newer is required.");
        }

        internal static Type FindType(string fullName)
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Type type = assemblies[i].GetType(fullName, false);
                if (type != null)
                    return type;
            }
            return null;
        }

        internal static object Invoke(string name, params object[] arguments)
        {
            EnsureAvailable();
            MethodInfo method = FindMethod(_utilityType, name, arguments == null ? 0 : arguments.Length);
            if (method == null)
                throw new MissingMethodException(_utilityType.FullName, name);
            try { return method.Invoke(null, arguments); }
            catch (TargetInvocationException exception)
            {
                throw new InvalidOperationException(
                    "Unity Frame Debugger " + name + " failed: "
                    + (exception.InnerException == null ? exception.Message : exception.InnerException.Message),
                    exception.InnerException ?? exception);
            }
        }

        internal static bool BoolProperty(string name, bool fallback)
        {
            object value = Property(name);
            return value == null ? fallback : Convert.ToBoolean(value, CultureInfo.InvariantCulture);
        }

        internal static int IntProperty(string name, int fallback)
        {
            object value = Property(name);
            return value == null ? fallback : Convert.ToInt32(value, CultureInfo.InvariantCulture);
        }

        internal static void SetProperty(string name, object value)
        {
            EnsureAvailable();
            PropertyInfo property = _utilityType.GetProperty(name, AllStatic);
            if (property == null || !property.CanWrite)
                throw new MissingMemberException(_utilityType.FullName, name);
            property.SetValue(null, value, null);
        }

        internal static Array Events()
        {
            return Invoke("GetFrameEvents") as Array ?? new object[0];
        }

        internal static object EventData(int index)
        {
            EnsureAvailable();
            object data = Activator.CreateInstance(_eventDataType, true);
            object valid = Invoke("GetFrameEventData", index, data);
            if (valid is bool && !(bool)valid)
                throw new InvalidOperationException("Unity did not return Frame Debugger data for event " + index + ".");
            return data;
        }

        internal static object Field(object value, string name)
        {
            if (value == null)
                return null;
            FieldInfo field = value.GetType().GetField(name, AllInstance);
            return field == null ? null : field.GetValue(value);
        }

        internal static int ConnectedProfilerId()
        {
            Type type = FindType("UnityEditorInternal.ProfilerDriver");
            if (type == null)
                return 0;
            PropertyInfo property = type.GetProperty("connectedProfiler", AllStatic);
            if (property != null)
                return Convert.ToInt32(property.GetValue(null, null), CultureInfo.InvariantCulture);
            FieldInfo field = type.GetField("connectedProfiler", AllStatic);
            return field == null ? 0 : Convert.ToInt32(field.GetValue(null), CultureInfo.InvariantCulture);
        }

        internal static bool TryEnableWithBuiltInWindow()
        {
            Type type = FindType("UnityEditor.FrameDebuggerWindow");
            if (type == null)
                return false;
            MethodInfo method = type.GetMethod("EnableFrameDebugger", AllInstance);
            if (method == null)
                return false;
            EditorWindow window = EditorWindow.GetWindow(type, false, "Frame Debugger", true);
            if (window == null)
                return false;
            try
            {
                method.Invoke(window, null);
                return UnityEngine.FrameDebugger.enabled;
            }
            catch (TargetInvocationException exception)
            {
                throw new InvalidOperationException(
                    "Unity Frame Debugger window failed to enable capture: "
                    + (exception.InnerException == null ? exception.Message : exception.InnerException.Message),
                    exception.InnerException ?? exception);
            }
        }

        internal static bool TryDisableWithBuiltInWindow()
        {
            Type type = FindType("UnityEditor.FrameDebuggerWindow");
            if (type == null)
                return false;
            MethodInfo method = type.GetMethod("DisableFrameDebugger", AllInstance);
            if (method == null)
                return false;
            UnityEngine.Object[] windows = Resources.FindObjectsOfTypeAll(type);
            if (windows == null || windows.Length == 0)
                return false;
            try
            {
                method.Invoke(windows[0], null);
                return !UnityEngine.FrameDebugger.enabled;
            }
            catch (TargetInvocationException exception)
            {
                throw new InvalidOperationException(
                    "Unity Frame Debugger window failed to disable capture: "
                    + (exception.InnerException == null ? exception.Message : exception.InnerException.Message),
                    exception.InnerException ?? exception);
            }
        }

        internal static void BlitRenderTarget(
            RenderTexture source,
            RenderTexture destination,
            int width,
            int height,
            Vector4 channels,
            Vector4 levels,
            bool flipY,
            bool undoOutputSrgb)
        {
            EnsureAvailable();
            if (_helperType != null)
            {
                MethodInfo[] methods = _helperType.GetMethods(AllStatic);
                for (int i = 0; i < methods.Length; i++)
                {
                    MethodInfo method = methods[i];
                    ParameterInfo[] parameters = method.GetParameters();
                    if (method.Name != "BlitToRenderTexture" || parameters.Length != 8)
                        continue;
                    Type first = parameters[0].ParameterType;
                    if (first.IsByRef)
                        first = first.GetElementType();
                    if (first != typeof(RenderTexture))
                        continue;
                    object[] args = { source, destination, width, height, channels, levels, flipY, undoOutputSrgb };
                    try
                    {
                        method.Invoke(null, args);
                        return;
                    }
                    catch (TargetInvocationException)
                    {
                        break;
                    }
                }
            }
            Graphics.Blit(source, destination);
        }

        private static object Property(string name)
        {
            EnsureAvailable();
            PropertyInfo property = _utilityType.GetProperty(name, AllStatic);
            return property == null ? null : property.GetValue(null, null);
        }

        private static MethodInfo FindMethod(Type type, string name, int argumentCount)
        {
            MethodInfo[] methods = type.GetMethods(AllStatic);
            for (int i = 0; i < methods.Length; i++)
            {
                if (methods[i].Name == name && methods[i].GetParameters().Length == argumentCount)
                    return methods[i];
            }
            return null;
        }
    }

    internal static class FrameDebuggerJson
    {
        internal static string Serialize(object value)
        {
            StringBuilder output = new StringBuilder(512);
            HashSet<object> path = new HashSet<object>(ReferenceComparer.Instance);
            Write(output, value, path, 0);
            return output.ToString();
        }

        private static void Write(StringBuilder output, object value, HashSet<object> path, int depth)
        {
            if (value == null) { output.Append("null"); return; }
            if (depth > 12) { Quote(output, "<max-depth>"); return; }
            if (value is string || value is char) { Quote(output, Convert.ToString(value, CultureInfo.InvariantCulture)); return; }
            if (value is bool) { output.Append((bool)value ? "true" : "false"); return; }
            if (IsNumber(value)) { output.Append(Convert.ToString(value, CultureInfo.InvariantCulture)); return; }
            if (value is Enum) { Quote(output, value.ToString()); return; }
            if (value is UnityEngine.Object)
            {
                UnityEngine.Object unityObject = (UnityEngine.Object)value;
                Quote(output, unityObject == null ? null : unityObject.name);
                return;
            }
            bool track = !value.GetType().IsValueType;
            if (track && !path.Add(value)) { Quote(output, "<cycle>"); return; }
            try
            {
                IDictionary dictionary = value as IDictionary;
                if (dictionary != null) { WriteDictionary(output, dictionary, path, depth); return; }
                IEnumerable enumerable = value as IEnumerable;
                if (enumerable != null) { WriteList(output, enumerable, path, depth); return; }
                WriteObject(output, value, path, depth);
            }
            finally { if (track) path.Remove(value); }
        }

        private static void WriteDictionary(StringBuilder output, IDictionary value, HashSet<object> path, int depth)
        {
            output.Append('{');
            bool first = true;
            foreach (DictionaryEntry item in value)
            {
                if (Omit(item.Value)) continue;
                if (!first) output.Append(',');
                first = false;
                Quote(output, Convert.ToString(item.Key, CultureInfo.InvariantCulture));
                output.Append(':');
                Write(output, item.Value, path, depth + 1);
            }
            output.Append('}');
        }

        private static void WriteList(StringBuilder output, IEnumerable value, HashSet<object> path, int depth)
        {
            output.Append('[');
            bool first = true;
            foreach (object item in value)
            {
                if (!first) output.Append(',');
                first = false;
                Write(output, item, path, depth + 1);
            }
            output.Append(']');
        }

        private static void WriteObject(StringBuilder output, object value, HashSet<object> path, int depth)
        {
            output.Append('{');
            bool first = true;
            Type type = value.GetType();
            PropertyInfo[] properties = type.GetProperties(BindingFlags.Instance | BindingFlags.Public);
            for (int i = 0; i < properties.Length; i++)
            {
                PropertyInfo property = properties[i];
                if (!property.CanRead || property.GetIndexParameters().Length != 0) continue;
                object item;
                try { item = property.GetValue(value, null); } catch { continue; }
                if (Omit(item)) continue;
                if (!first) output.Append(',');
                first = false;
                Quote(output, char.ToLowerInvariant(property.Name[0]) + property.Name.Substring(1));
                output.Append(':');
                Write(output, item, path, depth + 1);
            }
            FieldInfo[] fields = type.GetFields(BindingFlags.Instance | BindingFlags.Public);
            for (int i = 0; i < fields.Length; i++)
            {
                object item = fields[i].GetValue(value);
                if (Omit(item)) continue;
                if (!first) output.Append(',');
                first = false;
                Quote(output, fields[i].Name);
                output.Append(':');
                Write(output, item, path, depth + 1);
            }
            output.Append('}');
        }

        private static bool Omit(object value)
        {
            if (value == null) return true;
            string text = value as string;
            if (text != null) return text.Length == 0;
            ICollection collection = value as ICollection;
            return collection != null && collection.Count == 0;
        }

        private static bool IsNumber(object value)
        {
            return value is byte || value is sbyte || value is short || value is ushort
                || value is int || value is uint || value is long || value is ulong
                || value is float || value is double || value is decimal;
        }

        private static void Quote(StringBuilder output, string value)
        {
            if (value == null) { output.Append("null"); return; }
            output.Append('"');
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                switch (ch)
                {
                    case '"': output.Append("\\\""); break;
                    case '\\': output.Append("\\\\"); break;
                    case '\b': output.Append("\\b"); break;
                    case '\f': output.Append("\\f"); break;
                    case '\n': output.Append("\\n"); break;
                    case '\r': output.Append("\\r"); break;
                    case '\t': output.Append("\\t"); break;
                    default:
                        if (ch < 32) output.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                        else output.Append(ch);
                        break;
                }
            }
            output.Append('"');
        }

        private sealed class ReferenceComparer : IEqualityComparer<object>
        {
            internal static readonly ReferenceComparer Instance = new ReferenceComparer();
            public new bool Equals(object left, object right) { return ReferenceEquals(left, right); }
            public int GetHashCode(object value) { return System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(value); }
        }
    }
}
