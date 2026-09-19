using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using UnityEditorInternal;
using UnityEngine;
#if UNITY_2022_2_OR_NEWER
using MemoryProfilerApi = Unity.Profiling.Memory.MemoryProfiler;
using MemoryCaptureFlags = Unity.Profiling.Memory.CaptureFlags;
#else
using MemoryProfilerApi = UnityEngine.Profiling.Memory.Experimental.MemoryProfiler;
using MemoryCaptureFlags = UnityEngine.Profiling.Memory.Experimental.CaptureFlags;
#endif

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed class ProfilerFrameTiming
        {
            public ulong Timestamp;
            public double CpuFrameMs, CpuMainThreadMs, CpuRenderThreadMs, CpuPresentWaitMs;
            public double? GpuFrameMs;
            public float WidthScale, HeightScale;
        }

        public sealed class ProfilerFrameTimings
        {
            public bool Enabled;
            public string Error;
            public ProfilerFrameTiming[] Frames;
        }

        public sealed class ProfilerThreadInfo
        {
            public int Index;
            public ulong Id;
            public string Name, Group;
        }

        public sealed class ProfilerFrameOptions
        {
            public string ThreadName = "Main Thread";
            public int ThreadIndex = -1;
            public int TopCount = 80;
            public string SortBy = "total_ms";
            public string NameContains = "";
        }

        public sealed partial class ProfilerApi
        {
            // The native snapshot operation cannot be canceled. Cancellation ends the wait only.
            private static bool _memorySnapshotInProgress;

            public async Task<string> SaveMemorySnapshotAsync(string name, CancellationToken cancellationToken = default,
                bool includeNativeAllocations = false)
            {
                if (_disposed) throw new ObjectDisposedException(nameof(ProfilerApi));
                _lifetime.Token.ThrowIfCancellationRequested();
                cancellationToken.ThrowIfCancellationRequested();
                RequireProfilerEditorTarget();
                if (_memorySnapshotInProgress) throw new InvalidOperationException("A memory snapshot is already in progress.");
                string path = Path.Combine(RunStatesResultDirectory(), "memory-" + SanitizeRunStatesFileName(NormalizeProfilerName(name))
                    + "-" + Guid.NewGuid().ToString("N") + ".snap");
                var completion = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
                var flags = MemoryCaptureFlags.ManagedObjects | MemoryCaptureFlags.NativeObjects;
                if (includeNativeAllocations) flags |= MemoryCaptureFlags.NativeAllocations | MemoryCaptureFlags.NativeAllocationSites;
                _memorySnapshotInProgress = true;
                try
                {
                    MemoryProfilerApi.TakeSnapshot(path, (savedPath, success) =>
                    {
                        _memorySnapshotInProgress = false;
                        if (success) completion.TrySetResult(savedPath);
                        else completion.TrySetException(new InvalidOperationException("Memory snapshot failed: " + path));
                    }, flags);
                }
                catch { _memorySnapshotInProgress = false; throw; }
                using (var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, _lifetime.Token))
                using (linked.Token.Register(() => completion.TrySetCanceled(linked.Token)))
                {
                    string result = await completion.Task;
                    Print("profiler_memory_file: " + result);
                    return result;
                }
            }

            /// <summary>Request timings; Unity usually makes the data available several frames later.</summary>
            public bool CaptureFrameTimings()
            {
                if (!FrameTimingManager.IsFeatureEnabled()) return false;
                FrameTimingManager.CaptureFrameTimings();
                return true;
            }

            public ProfilerFrameTimings GetFrameTimings(int count = 1)
            {
                if (count < 1 || count > 128) throw new ArgumentOutOfRangeException(nameof(count));
                var result = new ProfilerFrameTimings { Enabled = FrameTimingManager.IsFeatureEnabled(), Error = "",
                    Frames = Array.Empty<ProfilerFrameTiming>() };
                if (!result.Enabled) { result.Error = "Frame Timing Stats is disabled or unsupported on this platform."; return result; }
                var raw = new FrameTiming[count];
                uint returned = FrameTimingManager.GetLatestTimings((uint)count, raw);
                var frames = new List<ProfilerFrameTiming>();
                for (int i = 0; i < returned; i++)
                {
                    var f = raw[i];
                    frames.Add(new ProfilerFrameTiming { Timestamp = f.frameStartTimestamp, CpuFrameMs = f.cpuFrameTime,
                        CpuMainThreadMs = f.cpuMainThreadFrameTime, CpuRenderThreadMs = f.cpuRenderThreadFrameTime,
                        CpuPresentWaitMs = f.cpuMainThreadPresentWaitTime, GpuFrameMs = f.gpuFrameTime > 0 ? f.gpuFrameTime : (double?)null,
                        WidthScale = f.widthScale, HeightScale = f.heightScale });
                }
                result.Frames = frames.ToArray();
                if (frames.Count == 0) result.Error = "No completed frame timings; request a capture and wait several rendered frames.";
                return result;
            }

            public ProfilerThreadInfo[] GetProfilerThreads(int profilerFrameIndex)
            {
                var result = new List<ProfilerThreadInfo>();
                for (int i = 0; ; i++)
                {
                    using (var frame = ProfilerDriver.GetRawFrameDataView(profilerFrameIndex, i))
                    {
                        if (!frame.valid) break;
                        result.Add(new ProfilerThreadInfo { Index = i, Id = frame.threadId, Name = frame.threadName, Group = frame.threadGroupName });
                    }
                }
                return result.ToArray();
            }

            public RuntimeProfilerFrameExport GetProfilerFrame(int profilerFrameIndex, ProfilerFrameOptions options = null)
            {
                options = options ?? new ProfilerFrameOptions();
                if (options.ThreadIndex < -1 || options.TopCount < 0 || options.TopCount > MaxProfilerFrameSavedRows)
                    throw new ArgumentOutOfRangeException("ThreadIndex/TopCount");
                if (options.SortBy != "total_ms" && options.SortBy != "self_ms" && options.SortBy != "gc_bytes" && options.SortBy != "calls")
                    throw new ArgumentException("SortBy must be total_ms, self_ms, gc_bytes, or calls.");
                return BuildProfilerFrameExport("frame", profilerFrameIndex, TotalFrames, Time.frameCount,
                    options.ThreadName, options.TopCount, options);
            }

            public string SaveProfilerFrame(string name, int profilerFrameIndex, ProfilerFrameOptions options, int inlineRows = 8)
            {
                var export = GetProfilerFrame(profilerFrameIndex, options);
                export.Name = NormalizeProfilerName(name);
                string path = Path.Combine(RunStatesResultDirectory(), "profiler-frame-" + SanitizeRunStatesFileName(export.Name)
                    + "-" + Guid.NewGuid().ToString("N") + ".json");
                var json = new System.Text.StringBuilder();
                export.AppendJson(json);
                File.WriteAllText(path, json.ToString(), Utf8NoBom);
                Print(export.BuildSummary(inlineRows));
                Print("profiler_frame_file: " + path);
                return path;
            }
        }
    }
}
