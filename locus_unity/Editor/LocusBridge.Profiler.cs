using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;
using System.Threading;
using Unity.Profiling;
using UnityEditor;
using UnityEngine;

namespace Locus
{
    public static partial class LocusBridge
    {
        /// <summary>Per-call profiling shared by unity_execute and unity_run_states. Main thread only.</summary>
        public sealed partial class ProfilerApi : IDisposable
        {
            private readonly Dictionary<string, RuntimeProfilerSession> _profilers =
                new Dictionary<string, RuntimeProfilerSession>(StringComparer.Ordinal);
            private readonly Func<int> _frame;
            private readonly Action<object> _print;
            private readonly string _clock;
            private readonly bool _autoTick;
            private bool _disposed;
            private readonly CancellationTokenSource _lifetime;
            private readonly Stopwatch _stopwatch = Stopwatch.StartNew();
            private int TotalFrames => _frame();

            internal ProfilerApi(Func<int> frame, Action<object> print, string clock, bool autoTick, CancellationToken cancellationToken = default)
            {
                _frame = frame; _print = print; _clock = clock; _autoTick = autoTick;
                _lifetime = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                AssemblyReloadEvents.beforeAssemblyReload += Dispose;
                EditorApplication.quitting += Dispose;
            }

            private void Print(object value) { _print(value); }
            public int LatestProfilerFrameIndex() { return CurrentProfilerFrameIndex(); }
            public void StartProfiler(string name) { StartProfiler(name, DefaultProfilerMetrics()); }

            public RuntimeProfilerMetric ProfilerMetric(string name, ProfilerCategory category, string markerName, double scale, string unit)
            {
                return ProfilerMetric(name, category, markerName, scale, unit, ProfilerRecorderOptions.Default);
            }

            public RuntimeProfilerMetric ProfilerMetric(string name, ProfilerCategory category, string markerName, double scale, string unit,
                ProfilerRecorderOptions options, bool zeroIsUnavailable = false)
            {
                string normalizedName = (name ?? "").Trim();
                string normalizedMarker = (markerName ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedName))
                    throw new ArgumentException("Profiler metric name is required.");
                if (string.IsNullOrEmpty(normalizedMarker))
                    throw new ArgumentException("Profiler marker name is required.");
                if (double.IsNaN(scale) || double.IsInfinity(scale) || scale == 0)
                    throw new ArgumentOutOfRangeException(nameof(scale));
                if ((options & ProfilerRecorderOptions.KeepAliveDuringDomainReload) != 0)
                    throw new ArgumentException("Per-call recorders cannot survive domain reload.");

                return new RuntimeProfilerMetric(
                    normalizedName,
                    category,
                    normalizedMarker,
                    scale,
                    (unit ?? "").Trim(), options, zeroIsUnavailable: zeroIsUnavailable
                );
            }

            public RuntimeProfilerMetric[] DefaultProfilerMetrics()
            {
                return new[]
                {
                    ProfilerMetric("main_thread_ms", ProfilerCategory.Internal, "Main Thread", 0.000001, "ms"),
                    ProfilerMetric("render_thread_ms", ProfilerCategory.Internal, "Render Thread", 0.000001, "ms"),
                    ProfilerMetric("gc_alloc_bytes", ProfilerCategory.Memory, "GC Allocated In Frame", 1.0, "bytes"),
                    ProfilerMetric("gc_reserved_mb", ProfilerCategory.Memory, "GC Reserved Memory", 0.000001, "MB"),
                    ProfilerMetric("system_used_memory_mb", ProfilerCategory.Memory, "System Used Memory", 0.000001, "MB"),
                    ProfilerMetric("batches_count", ProfilerCategory.Render, "Batches Count", 1.0, "count"),
                    ProfilerMetric("setpass_calls_count", ProfilerCategory.Render, "SetPass Calls Count", 1.0, "count"),
                    ProfilerMetric("triangles_count", ProfilerCategory.Render, "Triangles Count", 1.0, "count"),
                    ProfilerMetric("vertices_count", ProfilerCategory.Render, "Vertices Count", 1.0, "count")
                };
            }

            public void StartProfiler(string name, IEnumerable<RuntimeProfilerMetric> metrics)
            {
                StartProfiler(name, metrics, new ProfilerCaptureOptions());
            }

            public void StartProfiler(string name, IEnumerable<RuntimeProfilerMetric> metrics, ProfilerCaptureOptions options)
            {
                if (_disposed) throw new ObjectDisposedException(nameof(ProfilerApi));
                _lifetime.Token.ThrowIfCancellationRequested();
                if (options == null) throw new ArgumentNullException(nameof(options));
                options = options.CopyValidated();
                string normalizedName = NormalizeProfilerName(name);
                if (_profilers.ContainsKey(normalizedName))
                    throw new InvalidOperationException("Profiler already started: " + normalizedName);

                var metricList = new List<RuntimeProfilerMetric>();
                var names = new HashSet<string>(StringComparer.Ordinal);
                if (metrics != null)
                {
                    foreach (RuntimeProfilerMetric metric in metrics)
                    {
                        if (metric != null)
                        {
                            if (!names.Add(metric.Name)) throw new ArgumentException("Duplicate profiler metric: " + metric.Name);
                            metricList.Add(metric);
                            if (metricList.Count > 128) throw new ArgumentException("At most 128 metrics per capture.");
                        }
                    }
                }

                if (metricList.Count == 0) throw new ArgumentException("At least one metric is required.");
                if ((long)metricList.Count * options.MaxSamples > 1000000)
                    throw new ArgumentException("Reduce metrics or MaxSamples to at most 1,000,000 values per capture.");
                if (_profilers.Count >= 16) throw new InvalidOperationException("At most 16 captures per call.");
                _profilers.Add(normalizedName, new RuntimeProfilerSession(
                    normalizedName,
                    metricList,
                    TotalFrames,
                    Time.frameCount, options, _clock
                ));
                if (_autoTick)
                {
                    EditorApplication.update -= SampleProfilers;
                    EditorApplication.update += SampleProfilers;
                }
            }

            public void StopProfiler(string name)
            {
                RequireProfiler(name).Stop(TotalFrames, Time.frameCount);
            }

            public void PrintProfilerSummary(string name)
            {
                Print(RequireProfiler(name).BuildSummary());
            }

            public bool TryGetProfilerLastValue(string profilerName, string metricName, out double value)
            {
                return RequireProfiler(profilerName).TryGetLastValue(metricName, out value);
            }

            public double GetProfilerLastValue(string profilerName, string metricName)
            {
                return RequireProfiler(profilerName).GetLastValue(metricName);
            }

            public RuntimeProfilerMetricSummary GetProfilerSummary(string profilerName, string metricName)
            {
                return RequireProfiler(profilerName).GetSummary(metricName);
            }

            public bool RecordProfilerSpike(string profilerName, string metricName, double threshold, string label)
            {
                return RequireProfiler(profilerName).RecordSpike(
                    metricName,
                    threshold,
                    label,
                    TotalFrames,
                    Time.frameCount,
                    CurrentProfilerFrameIndex()
                );
            }

            public bool RecordProfilerSpikeTop(string profilerName, string metricName, double threshold, string label, int maxSpikes)
            {
                if (maxSpikes < 1 || maxSpikes > 512) throw new ArgumentOutOfRangeException(nameof(maxSpikes));
                return RequireProfiler(profilerName).RecordSpike(
                    metricName,
                    threshold,
                    label,
                    TotalFrames,
                    Time.frameCount,
                    CurrentProfilerFrameIndex(),
                    maxSpikes
                );
            }

            public int GetProfilerLastSpikeFrame(string profilerName, string metricName)
            {
                return RequireProfiler(profilerName).GetLastSpikeProfilerFrame(metricName);
            }

            public RuntimeProfilerSpike[] GetProfilerSpikes(string profilerName)
            {
                return RequireProfiler(profilerName).GetSpikes();
            }

            public string SaveProfiler(string name)
            {
                RuntimeProfilerSession profiler = RequireProfiler(name);
                profiler.Stop(TotalFrames, Time.frameCount);
                RuntimeProfilerSaveResult result = profiler.Save();
                Print("profiler_file: " + result.SamplesPath);
                Print("profiler_summary_file: " + result.SummaryPath);
                return result.SamplesPath;
            }

            public string SaveProfilerFrame(string name, string threadName, int topCount)
            {
                return SaveProfilerFrame(name, CurrentProfilerFrameIndex(), threadName, topCount, DefaultProfilerFrameInlineRows);
            }

            public string SaveProfilerFrame(string name, string threadName, int topCount, int inlineRows)
            {
                return SaveProfilerFrame(name, CurrentProfilerFrameIndex(), threadName, topCount, inlineRows);
            }

            public string SaveProfilerFrame(string name, int profilerFrameIndex, string threadName, int topCount)
            {
                return SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount, DefaultProfilerFrameInlineRows);
            }

            public string SaveProfilerFrame(string name, int profilerFrameIndex, string threadName, int topCount, int inlineRows)
            {
                string normalizedName = NormalizeProfilerName(name);
                string directory = RunStatesResultDirectory();
                string path = Path.Combine(
                    directory,
                    "profiler-frame-" + SanitizeRunStatesFileName(normalizedName) + "-" + DateTime.UtcNow.ToString("yyyyMMdd-HHmmss-fff", CultureInfo.InvariantCulture) + ".json"
                );

                RuntimeProfilerFrameExport export;
                try
                {
                    export = BuildProfilerFrameExport(
                        normalizedName,
                        profilerFrameIndex,
                        TotalFrames,
                        Time.frameCount,
                        threadName,
                        topCount
                    );
                }
                catch (Exception ex)
                {
                    export = new RuntimeProfilerFrameExport
                    {
                        Name = normalizedName,
                        ProfilerFrameIndex = profilerFrameIndex,
                        SessionFrame = TotalFrames,
                        ExportedAtUnityTimeFrameCount = Time.frameCount,
                        RequestedThreadName = (threadName ?? "").Trim(),
                        ThreadName = "",
                        ThreadGroupName = "",
                        ThreadIndex = -1,
                        ThreadId = 0,
                        ThreadMatched = false,
                        FrameTimeMs = 0,
                        FrameFps = 0,
                        TopCount = Math.Max(0, Math.Min(topCount, MaxProfilerFrameSavedRows)),
                        Error = "Frame hierarchy export failed: " + ex.Message
                    };
                }

                var sb = new StringBuilder(4096);
                export.AppendJson(sb);
                File.WriteAllText(path, sb.ToString(), Utf8NoBom);
                Print(export.BuildSummary(inlineRows));
                Print("profiler_frame_file: " + path);
                return path;
            }

            internal void SampleProfilers()
            {
                if (_disposed) return;
                if (_lifetime.IsCancellationRequested) { Dispose(); return; }
                int profilerFrameIndex = CurrentProfilerFrameIndex();
                long elapsedMs = _stopwatch.ElapsedMilliseconds;
                foreach (RuntimeProfilerSession profiler in _profilers.Values)
                    profiler.Sample(TotalFrames, Time.frameCount, profilerFrameIndex, elapsedMs);
            }

            private RuntimeProfilerSession RequireProfiler(string name)
            {
                string normalizedName = NormalizeProfilerName(name);
                RuntimeProfilerSession profiler;
                if (!_profilers.TryGetValue(normalizedName, out profiler))
                    throw new KeyNotFoundException("Profiler not found: " + normalizedName);
                return profiler;
            }

            private string NormalizeProfilerName(string name)
            {
                string normalizedName = (name ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedName))
                    throw new ArgumentException("Profiler name is required.");
                return normalizedName;
            }

            public void Dispose()
            {
                if (_disposed) return;
                _disposed = true;
                AssemblyReloadEvents.beforeAssemblyReload -= Dispose;
                EditorApplication.quitting -= Dispose;
                _lifetime.Cancel();
                _lifetime.Dispose();
                if (_autoTick) EditorApplication.update -= SampleProfilers;
                foreach (RuntimeProfilerSession profiler in _profilers.Values)
                    profiler.Stop(TotalFrames, Time.frameCount);
                _profilers.Clear();
            }

        }
    }
}
