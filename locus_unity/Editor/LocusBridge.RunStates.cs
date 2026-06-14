using UnityEngine;
using UnityEditor;
using UnityEditor.Profiling;
using UnityEditorInternal;
using Unity.Profiling;

using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Reflection;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int RunStatesMaxFrames = 3600;
        private const int RunStatesPrintTokenByteRatio = 4;
        private const int RunStatesPrintHardLimitTokens = 1000000;
        private const long RunStatesPrintHardLimitBytes =
            (long)RunStatesPrintHardLimitTokens * RunStatesPrintTokenByteRatio;
        private const int DefaultProfilerFrameInlineRows = 8;
        private const int MaxProfilerFrameSavedRows = 512;

        private static RuntimeStateMachineSession _activeRunStatesSession;

        [Serializable]
        private sealed class RunStatesRequest
        {
            public string request_editor_status;
            public string initial_state;
            public RunStatesStateRequest[] states;
            public string[] auto_usings;
        }

        [Serializable]
        private sealed class RunStatesStateRequest
        {
            public string name;
            public string variables;
            public string start;
            public string update;
            public string end;
        }

        private sealed class CompiledRunStates
        {
            public readonly Func<RuntimeStateMachineDefinition> Builder;

            public CompiledRunStates(Func<RuntimeStateMachineDefinition> builder)
            {
                Builder = builder;
            }
        }

        internal sealed class RunStatesCompletion
        {
            public readonly bool Ok;
            public readonly string Message;

            public RunStatesCompletion(bool ok, string message)
            {
                Ok = ok;
                Message = message ?? "";
            }
        }

        internal enum RunStatesControlKind
        {
            Sleep,
            Goto,
            Done,
            Fail
        }

        internal sealed class RunStatesControlException : Exception
        {
            public RunStatesControlKind Kind;
            public string Target;
            public string MessageText;
            public int SleepFrames;

            public RunStatesControlException()
                : base("run_states_control")
            {
            }

            public void Reset(RunStatesControlKind kind, string target, string message, int sleepFrames)
            {
                Kind = kind;
                Target = target;
                MessageText = message;
                SleepFrames = sleepFrames;
            }
        }

        public sealed class RuntimeStateMachineDefinition
        {
            private readonly Dictionary<string, RuntimeStateDefinition> _states =
                new Dictionary<string, RuntimeStateDefinition>(StringComparer.Ordinal);

            public void AddState(string name, Action<RuntimeCtx> start, Action<RuntimeCtx> update, Action<RuntimeCtx> end)
            {
                string normalizedName = (name ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedName))
                    throw new ArgumentException("State name is required.");
                if (update == null)
                    throw new ArgumentException("State '" + normalizedName + "' requires an update handler.");
                if (_states.ContainsKey(normalizedName))
                    throw new ArgumentException("Duplicate state name: " + normalizedName);

                _states.Add(normalizedName, new RuntimeStateDefinition(normalizedName, start, update, end));
            }

            internal bool ContainsState(string name)
            {
                return !string.IsNullOrEmpty(name) && _states.ContainsKey(name);
            }

            internal RuntimeStateDefinition GetState(string name)
            {
                RuntimeStateDefinition state;
                if (!_states.TryGetValue(name, out state))
                    throw new InvalidOperationException("Unknown state: " + name);
                return state;
            }
        }

        internal sealed class RuntimeStateDefinition
        {
            public readonly string Name;
            public readonly Action<RuntimeCtx> Start;
            public readonly Action<RuntimeCtx> Update;
            public readonly Action<RuntimeCtx> End;

            public RuntimeStateDefinition(string name, Action<RuntimeCtx> start, Action<RuntimeCtx> update, Action<RuntimeCtx> end)
            {
                Name = name;
                Start = start;
                Update = update;
                End = end;
            }
        }

        public sealed class RuntimeVar<T>
        {
            private readonly RuntimeStateMachineSession _session;
            private readonly string _key;

            internal RuntimeVar(RuntimeStateMachineSession session, string key)
            {
                _session = session;
                _key = key;
            }

            public string Key { get { return _key; } }

            public T Value
            {
                get { return _session.GetMemory<T>(_key); }
                set { _session.SetMemory(_key, value); }
            }

            public override string ToString()
            {
                object value = Value;
                return value != null ? value.ToString() : "null";
            }
        }

        public sealed class RuntimeProfilerMetric
        {
            public readonly string Name;
            public readonly ProfilerCategory Category;
            public readonly string MarkerName;
            public readonly double Scale;
            public readonly string Unit;

            internal RuntimeProfilerMetric(string name, ProfilerCategory category, string markerName, double scale, string unit)
            {
                Name = name;
                Category = category;
                MarkerName = markerName;
                Scale = scale;
                Unit = unit;
            }
        }

        public sealed class RuntimeProfilerMetricSummary
        {
            public readonly string Name;
            public readonly string MarkerName;
            public readonly string Unit;
            public readonly bool Available;
            public readonly string Error;
            public readonly int SampleCount;
            public readonly double Average;
            public readonly double P95;
            public readonly double Max;
            public readonly double Last;

            internal RuntimeProfilerMetricSummary(
                string name,
                string markerName,
                string unit,
                bool available,
                string error,
                int sampleCount,
                double average,
                double p95,
                double max,
                double last)
            {
                Name = name ?? "";
                MarkerName = markerName ?? "";
                Unit = unit ?? "";
                Available = available;
                Error = error ?? "";
                SampleCount = sampleCount;
                Average = average;
                P95 = p95;
                Max = max;
                Last = last;
            }
        }

        public sealed class RuntimeProfilerSpike
        {
            public readonly string Label;
            public readonly string MetricName;
            public readonly double Threshold;
            public readonly double Value;
            public readonly int SessionFrame;
            public readonly int UnityTimeFrameCount;
            public readonly int ProfilerFrameIndex;

            internal RuntimeProfilerSpike(
                string label,
                string metricName,
                double threshold,
                double value,
                int sessionFrame,
                int unityTimeFrameCount,
                int profilerFrameIndex)
            {
                Label = label ?? "";
                MetricName = metricName ?? "";
                Threshold = threshold;
                Value = value;
                SessionFrame = sessionFrame;
                UnityTimeFrameCount = unityTimeFrameCount;
                ProfilerFrameIndex = profilerFrameIndex;
            }

            internal void AppendJson(StringBuilder sb)
            {
                sb.Append("{\"label\":").Append(ToCSharpStringLiteral(Label))
                    .Append(",\"metric\":").Append(ToCSharpStringLiteral(MetricName))
                    .Append(",\"threshold\":");
                AppendProfilerJsonNumber(sb, Threshold);
                sb.Append(",\"value\":");
                AppendProfilerJsonNumber(sb, Value);
                sb.Append(",\"session_frame\":").Append(SessionFrame)
                    .Append(",\"unity_time_frame_count\":").Append(UnityTimeFrameCount)
                    .Append(",\"profiler_frame_index\":").Append(ProfilerFrameIndex)
                    .Append('}');
            }
        }

        private sealed class RuntimeProfilerSaveResult
        {
            public readonly string SamplesPath;
            public readonly string SummaryPath;

            public RuntimeProfilerSaveResult(string samplesPath, string summaryPath)
            {
                SamplesPath = samplesPath ?? "";
                SummaryPath = summaryPath ?? "";
            }
        }

        private struct RuntimeProfilerPoint
        {
            public int SessionFrame;
            public int UnityTimeFrameCount;
            public int ProfilerFrameIndex;
            public long ElapsedMs;
            public double Value;
        }

        private sealed class RuntimeProfilerSample : IDisposable
        {
            private readonly RuntimeProfilerMetric _metric;
            private readonly List<RuntimeProfilerPoint> _points = new List<RuntimeProfilerPoint>(512);
            private ProfilerRecorder _recorder;
            private string _error;
            private bool _disposed;

            public RuntimeProfilerSample(RuntimeProfilerMetric metric)
            {
                _metric = metric;
                try
                {
                    _recorder = ProfilerRecorder.StartNew(metric.Category, metric.MarkerName);
                    if (!_recorder.Valid)
                        _error = "recorder is invalid";
                }
                catch (Exception ex)
                {
                    _error = ex.Message;
                }
            }

            public string Name { get { return _metric.Name; } }
            public int PointCount { get { return _points.Count; } }

            public void Sample(
                int sessionFrame,
                int unityTimeFrameCount,
                int profilerFrameIndex,
                long elapsedMs)
            {
                if (_disposed || !string.IsNullOrEmpty(_error))
                    return;

                try
                {
                    if (!_recorder.Valid)
                    {
                        _error = "recorder became invalid";
                        return;
                    }

                    _points.Add(new RuntimeProfilerPoint
                    {
                        SessionFrame = sessionFrame,
                        UnityTimeFrameCount = unityTimeFrameCount,
                        ProfilerFrameIndex = profilerFrameIndex,
                        ElapsedMs = elapsedMs,
                        Value = _recorder.LastValue * _metric.Scale
                    });
                }
                catch (Exception ex)
                {
                    _error = ex.Message;
                }
            }

            public bool TryGetLastValue(out double value)
            {
                if (_points.Count == 0)
                {
                    value = 0;
                    return false;
                }

                value = _points[_points.Count - 1].Value;
                return true;
            }

            public bool TryGetPoint(
                int index,
                out int sessionFrame,
                out int unityTimeFrameCount,
                out int profilerFrameIndex,
                out long elapsedMs,
                out double value)
            {
                if (index < 0 || index >= _points.Count)
                {
                    sessionFrame = 0;
                    unityTimeFrameCount = 0;
                    profilerFrameIndex = -1;
                    elapsedMs = 0;
                    value = 0;
                    return false;
                }

                RuntimeProfilerPoint point = _points[index];
                sessionFrame = point.SessionFrame;
                unityTimeFrameCount = point.UnityTimeFrameCount;
                profilerFrameIndex = point.ProfilerFrameIndex;
                elapsedMs = point.ElapsedMs;
                value = point.Value;
                return true;
            }

            public RuntimeProfilerMetricSummary GetSummary()
            {
                if (_points.Count == 0)
                {
                    return new RuntimeProfilerMetricSummary(
                        _metric.Name,
                        _metric.MarkerName,
                        _metric.Unit,
                        string.IsNullOrEmpty(_error),
                        _error,
                        _points.Count,
                        0,
                        0,
                        0,
                        0
                    );
                }

                return new RuntimeProfilerMetricSummary(
                    _metric.Name,
                    _metric.MarkerName,
                    _metric.Unit,
                    true,
                    _error,
                    _points.Count,
                    Average(),
                    Percentile(0.95),
                    Max(),
                    _points[_points.Count - 1].Value
                );
            }

            public void Stop()
            {
                Dispose();
            }

            public void Dispose()
            {
                if (_disposed)
                    return;

                _disposed = true;
                try
                {
                    _recorder.Dispose();
                }
                catch
                {
                }
            }

            public void AppendSummary(StringBuilder sb)
            {
                RuntimeProfilerMetricSummary summary = GetSummary();
                if (!summary.Available)
                {
                    sb.Append(_metric.Name)
                        .Append(" unavailable error=")
                        .AppendLine(ToCSharpStringLiteral(summary.Error));
                    return;
                }

                if (summary.SampleCount == 0)
                {
                    sb.Append(_metric.Name).AppendLine(" samples=0");
                    return;
                }

                sb.Append(_metric.Name)
                    .Append(" samples=").Append(summary.SampleCount)
                    .Append(" avg=").Append(FormatProfilerDouble(summary.Average))
                    .Append(" p95=").Append(FormatProfilerDouble(summary.P95))
                    .Append(" max=").Append(FormatProfilerDouble(summary.Max))
                    .Append(" last=").Append(FormatProfilerDouble(summary.Last));

                if (!string.IsNullOrEmpty(_metric.Unit))
                    sb.Append(" unit=").Append(_metric.Unit);

                if (!string.IsNullOrEmpty(summary.Error))
                    sb.Append(" error=").Append(ToCSharpStringLiteral(summary.Error));

                sb.AppendLine();
            }

            public void AppendSummaryJson(StringBuilder sb)
            {
                RuntimeProfilerMetricSummary summary = GetSummary();
                sb.Append("{\"name\":").Append(ToCSharpStringLiteral(_metric.Name))
                    .Append(",\"category\":").Append(ToCSharpStringLiteral(_metric.Category.ToString()))
                    .Append(",\"marker\":").Append(ToCSharpStringLiteral(_metric.MarkerName))
                    .Append(",\"scale\":");
                AppendProfilerJsonNumber(sb, _metric.Scale);
                sb.Append(",\"unit\":").Append(ToCSharpStringLiteral(_metric.Unit ?? ""))
                    .Append(",\"available\":").Append(summary.Available ? "true" : "false")
                    .Append(",\"error\":").Append(ToCSharpStringLiteral(summary.Error))
                    .Append(",\"summary\":{\"sample_count\":").Append(summary.SampleCount)
                    .Append(",\"avg\":");
                AppendProfilerJsonNumber(sb, summary.Average);
                sb.Append(",\"p95\":");
                AppendProfilerJsonNumber(sb, summary.P95);
                sb.Append(",\"max\":");
                AppendProfilerJsonNumber(sb, summary.Max);
                sb.Append(",\"last\":");
                AppendProfilerJsonNumber(sb, summary.Last);
                sb.Append("}}");
            }

            private double Average()
            {
                double total = 0;
                for (int i = 0; i < _points.Count; i++)
                    total += _points[i].Value;
                return total / Math.Max(1, _points.Count);
            }

            private double Max()
            {
                double max = _points[0].Value;
                for (int i = 1; i < _points.Count; i++)
                    if (_points[i].Value > max)
                        max = _points[i].Value;
                return max;
            }

            private double Percentile(double percentile)
            {
                if (_points.Count == 0)
                    return 0;

                double[] sorted = new double[_points.Count];
                for (int i = 0; i < _points.Count; i++)
                    sorted[i] = _points[i].Value;

                Array.Sort(sorted);
                int index = (int)Math.Ceiling(percentile * sorted.Length) - 1;
                index = Math.Max(0, Math.Min(sorted.Length - 1, index));
                return sorted[index];
            }
        }

        private sealed class RuntimeProfilerSession : IDisposable
        {
            private readonly string _name;
            private readonly List<RuntimeProfilerSample> _samples = new List<RuntimeProfilerSample>();
            private readonly List<RuntimeProfilerSpike> _spikes = new List<RuntimeProfilerSpike>();
            private readonly Stopwatch _stopwatch = Stopwatch.StartNew();
            private readonly int _startFrame;
            private readonly int _startUnityFrame;
            private bool _stopped;
            private int _endFrame;
            private int _endUnityFrame;

            public RuntimeProfilerSession(
                string name,
                IEnumerable<RuntimeProfilerMetric> metrics,
                int startFrame,
                int startUnityFrame)
            {
                _name = name;
                _startFrame = startFrame;
                _startUnityFrame = startUnityFrame;
                _endFrame = startFrame;
                _endUnityFrame = startUnityFrame;

                if (metrics != null)
                {
                    foreach (RuntimeProfilerMetric metric in metrics)
                    {
                        if (metric != null)
                            _samples.Add(new RuntimeProfilerSample(metric));
                    }
                }
            }

            public string Name { get { return _name; } }

            public void Sample(
                int sessionFrame,
                int unityTimeFrameCount,
                int profilerFrameIndex,
                long elapsedMs)
            {
                if (_stopped)
                    return;

                for (int i = 0; i < _samples.Count; i++)
                    _samples[i].Sample(sessionFrame, unityTimeFrameCount, profilerFrameIndex, elapsedMs);
            }

            public void Stop(int endFrame, int endUnityFrame)
            {
                if (_stopped)
                    return;

                _stopped = true;
                _endFrame = endFrame;
                _endUnityFrame = endUnityFrame;
                _stopwatch.Stop();
                for (int i = 0; i < _samples.Count; i++)
                    _samples[i].Stop();
            }

            public bool TryGetLastValue(string metricName, out double value)
            {
                return FindSample(metricName).TryGetLastValue(out value);
            }

            public double GetLastValue(string metricName)
            {
                double value;
                if (!TryGetLastValue(metricName, out value))
                    throw new InvalidOperationException("Profiler metric has no sampled value: " + metricName);
                return value;
            }

            public RuntimeProfilerMetricSummary GetSummary(string metricName)
            {
                return FindSample(metricName).GetSummary();
            }

            public bool RecordSpike(
                string metricName,
                double threshold,
                string label,
                int sessionFrame,
                int unityTimeFrameCount,
                int profilerFrameIndex)
            {
                return RecordSpike(metricName, threshold, label, sessionFrame, unityTimeFrameCount, profilerFrameIndex, 0);
            }

            public bool RecordSpike(
                string metricName,
                double threshold,
                string label,
                int sessionFrame,
                int unityTimeFrameCount,
                int profilerFrameIndex,
                int maxSpikes)
            {
                string normalizedMetricName = NormalizeMetricName(metricName);
                double value;
                if (!TryGetLastValue(normalizedMetricName, out value) || value <= threshold)
                    return false;

                string normalizedLabel = (label ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedLabel))
                    normalizedLabel = normalizedMetricName + "_spike";

                RuntimeProfilerSpike spike = new RuntimeProfilerSpike(
                    normalizedLabel,
                    normalizedMetricName,
                    threshold,
                    value,
                    sessionFrame,
                    unityTimeFrameCount,
                    profilerFrameIndex
                );

                int normalizedMaxSpikes = Math.Max(0, maxSpikes);
                if (normalizedMaxSpikes <= 0)
                {
                    _spikes.Add(spike);
                    return true;
                }

                int matchingCount = 0;
                int weakestIndex = -1;
                double weakestValue = double.MaxValue;
                for (int i = 0; i < _spikes.Count; i++)
                {
                    RuntimeProfilerSpike existing = _spikes[i];
                    if (!string.Equals(existing.MetricName, normalizedMetricName, StringComparison.Ordinal)
                        || !string.Equals(existing.Label, normalizedLabel, StringComparison.Ordinal))
                        continue;

                    matchingCount++;
                    if (existing.Value < weakestValue)
                    {
                        weakestValue = existing.Value;
                        weakestIndex = i;
                    }
                }

                if (matchingCount < normalizedMaxSpikes)
                {
                    _spikes.Add(spike);
                    return true;
                }

                if (weakestIndex >= 0 && value > weakestValue)
                {
                    _spikes[weakestIndex] = spike;
                    return true;
                }

                return false;
            }

            public int GetLastSpikeProfilerFrame(string metricName)
            {
                string normalizedMetricName = NormalizeMetricName(metricName);
                for (int i = _spikes.Count - 1; i >= 0; i--)
                {
                    if (string.Equals(_spikes[i].MetricName, normalizedMetricName, StringComparison.Ordinal))
                        return _spikes[i].ProfilerFrameIndex;
                }

                throw new InvalidOperationException("Profiler spike not found for metric: " + normalizedMetricName);
            }

            public RuntimeProfilerSpike[] GetSpikes()
            {
                return _spikes.ToArray();
            }

            public string BuildSummary()
            {
                var sb = new StringBuilder(512);
                sb.Append("profiler ").AppendLine(_name);
                sb.Append("sample_rows=").Append(SampleRowCount().ToString(CultureInfo.InvariantCulture))
                    .Append(" frame_span=").Append(SessionFrameSpan().ToString(CultureInfo.InvariantCulture))
                    .Append(" unity_frame_span=").Append(UnityFrameSpan().ToString(CultureInfo.InvariantCulture))
                    .Append(" duration_ms=").AppendLine(_stopwatch.ElapsedMilliseconds.ToString(CultureInfo.InvariantCulture));

                if (_samples.Count == 0)
                    sb.AppendLine("metrics=0");

                for (int i = 0; i < _samples.Count; i++)
                    _samples[i].AppendSummary(sb);

                if (_spikes.Count > 0)
                    sb.Append("spikes=").AppendLine(_spikes.Count.ToString(CultureInfo.InvariantCulture));

                return sb.ToString().TrimEnd();
            }

            public RuntimeProfilerSaveResult Save()
            {
                string directory = RunStatesResultDirectory();
                string filePrefix = "profiler-"
                    + SanitizeRunStatesFileName(_name)
                    + "-"
                    + DateTime.UtcNow.ToString("yyyyMMdd-HHmmss-fff", CultureInfo.InvariantCulture);
                string samplesPath = Path.Combine(directory, filePrefix + ".csv");
                string summaryPath = Path.Combine(directory, filePrefix + "-summary.json");

                File.WriteAllText(samplesPath, BuildSamplesCsv(), Utf8NoBom);

                var sb = new StringBuilder(4096);
                sb.Append("{\n");
                sb.Append("  \"schema\": \"locus.profiler.summary.v1\",\n");
                sb.Append("  \"name\": ").Append(ToCSharpStringLiteral(_name)).Append(",\n");
                sb.Append("  \"start\": {\"session_frame\": ").Append(_startFrame)
                    .Append(", \"unity_time_frame_count\": ").Append(_startUnityFrame).Append("},\n");
                sb.Append("  \"end\": {\"session_frame\": ").Append(_endFrame)
                    .Append(", \"unity_time_frame_count\": ").Append(_endUnityFrame).Append("},\n");
                sb.Append("  \"duration_ms\": ").Append(_stopwatch.ElapsedMilliseconds.ToString(CultureInfo.InvariantCulture)).Append(",\n");
                sb.Append("  \"sample_policy\": {\"clock\": \"unity_run_states_tick\", \"sample_rows\": ")
                    .Append(SampleRowCount().ToString(CultureInfo.InvariantCulture))
                    .Append(", \"session_frame_span\": ")
                    .Append(SessionFrameSpan().ToString(CultureInfo.InvariantCulture))
                    .Append(", \"unity_frame_span\": ")
                    .Append(UnityFrameSpan().ToString(CultureInfo.InvariantCulture))
                    .Append(", \"distinct_unity_frames\": ")
                    .Append(CountDistinctUnityFrames().ToString(CultureInfo.InvariantCulture))
                    .Append(", \"distinct_profiler_frames\": ")
                    .Append(CountDistinctProfilerFrames().ToString(CultureInfo.InvariantCulture))
                    .Append("},\n");
                sb.Append("  \"samples_csv\": {\"schema\": \"locus.profiler.samples_csv.v1\", \"path\": ")
                    .Append(ToCSharpStringLiteral(samplesPath))
                    .Append("},\n");
                sb.Append("  \"metrics\": [");
                for (int i = 0; i < _samples.Count; i++)
                {
                    if (i > 0)
                        sb.Append(',');
                    sb.Append("\n    ");
                    _samples[i].AppendSummaryJson(sb);
                }
                if (_samples.Count > 0)
                    sb.Append('\n');
                sb.Append("  ],\n");
                sb.Append("  \"spikes\": [");
                for (int i = 0; i < _spikes.Count; i++)
                {
                    if (i > 0)
                        sb.Append(',');
                    sb.Append("\n    ");
                    _spikes[i].AppendJson(sb);
                }
                if (_spikes.Count > 0)
                    sb.Append('\n');
                sb.Append("  ]\n");
                sb.Append("}\n");

                File.WriteAllText(summaryPath, sb.ToString(), Utf8NoBom);
                return new RuntimeProfilerSaveResult(samplesPath, summaryPath);
            }

            private string BuildSamplesCsv()
            {
                var sb = new StringBuilder(4096);
                AppendCsvField(sb, "sample_index");
                sb.Append(',');
                AppendCsvField(sb, "session_frame");
                sb.Append(',');
                AppendCsvField(sb, "unity_time_frame_count");
                sb.Append(',');
                AppendCsvField(sb, "profiler_frame_index");
                sb.Append(',');
                AppendCsvField(sb, "elapsed_ms");
                for (int i = 0; i < _samples.Count; i++)
                {
                    sb.Append(',');
                    AppendCsvField(sb, _samples[i].Name);
                }
                sb.AppendLine();

                int rowCount = MaxSampleCount();
                for (int row = 0; row < rowCount; row++)
                {
                    int sessionFrame = 0;
                    int unityTimeFrameCount = 0;
                    int profilerFrameIndex = -1;
                    long elapsedMs = 0;
                    bool hasFrame = TryGetFrameForSampleIndex(
                        row,
                        out sessionFrame,
                        out unityTimeFrameCount,
                        out profilerFrameIndex,
                        out elapsedMs);
                    sb.Append(row.ToString(CultureInfo.InvariantCulture));
                    sb.Append(',');
                    if (hasFrame)
                    {
                        sb.Append(sessionFrame.ToString(CultureInfo.InvariantCulture));
                        sb.Append(',');
                        sb.Append(unityTimeFrameCount.ToString(CultureInfo.InvariantCulture));
                        sb.Append(',');
                        sb.Append(profilerFrameIndex.ToString(CultureInfo.InvariantCulture));
                        sb.Append(',');
                        sb.Append(elapsedMs.ToString(CultureInfo.InvariantCulture));
                    }
                    else
                    {
                        sb.Append(",,,,");
                    }

                    for (int i = 0; i < _samples.Count; i++)
                    {
                        sb.Append(',');
                        int unusedSessionFrame;
                        int unusedUnityFrame;
                        int unusedProfilerFrameIndex;
                        long unusedElapsedMs;
                        double value;
                        if (_samples[i].TryGetPoint(
                            row,
                            out unusedSessionFrame,
                            out unusedUnityFrame,
                            out unusedProfilerFrameIndex,
                            out unusedElapsedMs,
                            out value))
                            AppendProfilerCsvNumber(sb, value);
                    }
                    sb.AppendLine();
                }

                return sb.ToString();
            }

            private int MaxSampleCount()
            {
                int max = 0;
                for (int i = 0; i < _samples.Count; i++)
                    max = Math.Max(max, _samples[i].PointCount);
                return max;
            }

            private int SampleRowCount()
            {
                return MaxSampleCount();
            }

            private int SessionFrameSpan()
            {
                return Math.Max(0, _endFrame - _startFrame);
            }

            private int UnityFrameSpan()
            {
                return Math.Max(0, _endUnityFrame - _startUnityFrame);
            }

            private bool TryGetFrameForSampleIndex(
                int index,
                out int sessionFrame,
                out int unityTimeFrameCount,
                out int profilerFrameIndex,
                out long elapsedMs)
            {
                for (int i = 0; i < _samples.Count; i++)
                {
                    double unusedValue;
                    if (_samples[i].TryGetPoint(
                        index,
                        out sessionFrame,
                        out unityTimeFrameCount,
                        out profilerFrameIndex,
                        out elapsedMs,
                        out unusedValue))
                        return true;
                }

                sessionFrame = 0;
                unityTimeFrameCount = 0;
                profilerFrameIndex = -1;
                elapsedMs = 0;
                return false;
            }

            private int CountDistinctUnityFrames()
            {
                var values = new HashSet<int>();
                for (int row = 0; row < MaxSampleCount(); row++)
                {
                    int sessionFrame;
                    int unityTimeFrameCount;
                    int profilerFrameIndex;
                    long elapsedMs;
                    if (TryGetFrameForSampleIndex(
                        row,
                        out sessionFrame,
                        out unityTimeFrameCount,
                        out profilerFrameIndex,
                        out elapsedMs))
                        values.Add(unityTimeFrameCount);
                }
                return values.Count;
            }

            private int CountDistinctProfilerFrames()
            {
                var values = new HashSet<int>();
                for (int row = 0; row < MaxSampleCount(); row++)
                {
                    int sessionFrame;
                    int unityTimeFrameCount;
                    int profilerFrameIndex;
                    long elapsedMs;
                    if (TryGetFrameForSampleIndex(
                        row,
                        out sessionFrame,
                        out unityTimeFrameCount,
                        out profilerFrameIndex,
                        out elapsedMs)
                        && profilerFrameIndex >= 0)
                        values.Add(profilerFrameIndex);
                }
                return values.Count;
            }

            public void Dispose()
            {
                Stop(_endFrame, _endUnityFrame);
            }

            private RuntimeProfilerSample FindSample(string metricName)
            {
                string normalizedMetricName = NormalizeMetricName(metricName);
                for (int i = 0; i < _samples.Count; i++)
                {
                    if (string.Equals(_samples[i].Name, normalizedMetricName, StringComparison.Ordinal))
                        return _samples[i];
                }

                throw new KeyNotFoundException("Profiler metric not found: " + normalizedMetricName);
            }

            private static string NormalizeMetricName(string metricName)
            {
                string normalizedMetricName = (metricName ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedMetricName))
                    throw new ArgumentException("Profiler metric name is required.");
                return normalizedMetricName;
            }
        }

        private sealed class RuntimeProfilerFrameRow
        {
            public int Depth;
            public string Name;
            public string Path;
            public double TotalMs;
            public double SelfMs;
            public double TotalPercent;
            public double SelfPercent;
            public int Calls;
            public double GcBytes;
            public int WarningCount;

            public void AppendJson(StringBuilder sb)
            {
                sb.Append("{\"depth\":").Append(Depth)
                    .Append(",\"name\":").Append(ToCSharpStringLiteral(Name ?? ""))
                    .Append(",\"path\":").Append(ToCSharpStringLiteral(Path ?? ""))
                    .Append(",\"total_ms\":");
                AppendProfilerJsonNumber(sb, TotalMs);
                sb.Append(",\"self_ms\":");
                AppendProfilerJsonNumber(sb, SelfMs);
                sb.Append(",\"total_pct\":");
                AppendProfilerJsonNumber(sb, TotalPercent);
                sb.Append(",\"self_pct\":");
                AppendProfilerJsonNumber(sb, SelfPercent);
                sb.Append(",\"calls\":").Append(Calls)
                    .Append(",\"gc_bytes\":");
                AppendProfilerJsonNumber(sb, GcBytes);
                sb.Append(",\"warning_count\":").Append(WarningCount)
                    .Append('}');
            }
        }

        private sealed class RuntimeProfilerFrameExport
        {
            public string Name;
            public int ProfilerFrameIndex;
            public int SessionFrame;
            public int ExportedAtUnityTimeFrameCount;
            public string RequestedThreadName;
            public string ThreadName;
            public string ThreadGroupName;
            public int ThreadIndex;
            public long ThreadId;
            public bool ThreadMatched;
            public double FrameTimeMs;
            public double FrameFps;
            public int TopCount;
            public string Error;
            public readonly List<RuntimeProfilerFrameRow> Rows = new List<RuntimeProfilerFrameRow>(64);

            public string BuildSummary(int inlineRowCount)
            {
                var sb = new StringBuilder(1024);
                int normalizedInlineRows = Math.Max(0, Math.Min(inlineRowCount, Rows.Count));
                sb.Append("profiler_frame ").AppendLine(Name ?? "");
                sb.Append("frame=").Append(ProfilerFrameIndex)
                    .Append(" session_frame=").Append(SessionFrame)
                    .Append(" unity_frame=").Append(ExportedAtUnityTimeFrameCount)
                    .Append(" thread=").Append(ToCSharpStringLiteral(ThreadName ?? ""))
                    .Append(" thread_matched=").Append(ThreadMatched ? "true" : "false")
                    .Append(" cpu_ms=").Append(FormatProfilerDouble(FrameTimeMs))
                    .Append(" rows=").Append(Rows.Count)
                    .Append(" inline_rows=").Append(normalizedInlineRows)
                    .AppendLine();

                if (!string.IsNullOrEmpty(Error))
                {
                    sb.Append("error=").AppendLine(ToCSharpStringLiteral(Error));
                    return sb.ToString().TrimEnd();
                }

                for (int i = 0; i < normalizedInlineRows; i++)
                {
                    RuntimeProfilerFrameRow row = Rows[i];
                    sb.Append("depth=").Append(row.Depth)
                        .Append(" name=").Append(ToCSharpStringLiteral(row.Name ?? ""))
                        .Append(" total_ms=").Append(FormatProfilerDouble(row.TotalMs))
                        .Append(" self_ms=").Append(FormatProfilerDouble(row.SelfMs))
                        .Append(" calls=").Append(row.Calls)
                        .Append(" gc_bytes=").Append(FormatProfilerDouble(row.GcBytes))
                        .Append(" pct=").Append(FormatProfilerDouble(row.TotalPercent))
                        .AppendLine();
                }

                if (Rows.Count > normalizedInlineRows)
                    sb.Append("rows_truncated=").Append(Rows.Count - normalizedInlineRows).AppendLine();

                return sb.ToString().TrimEnd();
            }

            public void AppendJson(StringBuilder sb)
            {
                sb.Append("{\n");
                sb.Append("  \"schema\": \"locus.profiler.frame_hierarchy.v1\",\n");
                sb.Append("  \"name\": ").Append(ToCSharpStringLiteral(Name ?? "")).Append(",\n");
                sb.Append("  \"frame\": {\"profiler_frame_index\": ").Append(ProfilerFrameIndex)
                    .Append(", \"session_frame\": ").Append(SessionFrame)
                    .Append(", \"exported_at_unity_time_frame_count\": ").Append(ExportedAtUnityTimeFrameCount)
                    .Append(", \"frame_time_ms\": ");
                AppendProfilerJsonNumber(sb, FrameTimeMs);
                sb.Append(", \"frame_fps\": ");
                AppendProfilerJsonNumber(sb, FrameFps);
                sb.Append("},\n");
                sb.Append("  \"thread\": {\"requested\": ").Append(ToCSharpStringLiteral(RequestedThreadName ?? ""))
                    .Append(", \"name\": ").Append(ToCSharpStringLiteral(ThreadName ?? ""))
                    .Append(", \"group\": ").Append(ToCSharpStringLiteral(ThreadGroupName ?? ""))
                    .Append(", \"index\": ").Append(ThreadIndex)
                    .Append(", \"id\": ").Append(ThreadId)
                    .Append(", \"matched\": ").Append(ThreadMatched ? "true" : "false")
                    .Append("},\n");
                sb.Append("  \"top_count\": ").Append(TopCount).Append(",\n");
                sb.Append("  \"sort\": {\"column\": \"total_ms\", \"descending\": true},\n");
                sb.Append("  \"error\": ").Append(ToCSharpStringLiteral(Error ?? "")).Append(",\n");
                sb.Append("  \"rows\": [");
                for (int i = 0; i < Rows.Count; i++)
                {
                    if (i > 0)
                        sb.Append(',');
                    sb.Append("\n    ");
                    Rows[i].AppendJson(sb);
                }
                if (Rows.Count > 0)
                    sb.Append('\n');
                sb.Append("  ]\n");
                sb.Append("}\n");
            }
        }

        private static string FormatProfilerDouble(double value)
        {
            return value.ToString("0.###", CultureInfo.InvariantCulture);
        }

        private static void AppendProfilerJsonNumber(StringBuilder sb, double value)
        {
            if (double.IsNaN(value) || double.IsInfinity(value))
                sb.Append("null");
            else
                sb.Append(value.ToString("G17", CultureInfo.InvariantCulture));
        }

        private static void AppendProfilerCsvNumber(StringBuilder sb, double value)
        {
            if (double.IsNaN(value) || double.IsInfinity(value))
                return;

            sb.Append(FormatProfilerDouble(value));
        }

        private static void AppendCsvField(StringBuilder sb, string value)
        {
            string text = value ?? "";
            bool quote = text.IndexOfAny(new[] { ',', '"', '\r', '\n' }) >= 0;
            if (!quote)
            {
                sb.Append(text);
                return;
            }

            sb.Append('"');
            for (int i = 0; i < text.Length; i++)
            {
                char ch = text[i];
                if (ch == '"')
                    sb.Append("\"\"");
                else
                    sb.Append(ch);
            }
            sb.Append('"');
        }

        private static void EnsureProfilerRecording()
        {
            try
            {
                PropertyInfo enabledProperty = typeof(ProfilerDriver).GetProperty(
                    "enabled",
                    BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic
                );
                if (enabledProperty != null && enabledProperty.CanWrite)
                    enabledProperty.SetValue(null, true, null);
            }
            catch
            {
            }
        }

        private static int CurrentProfilerFrameIndex()
        {
            return ReadProfilerDriverIntProperty("lastFrameIndex", -1);
        }

        private static int ReadProfilerDriverIntProperty(string propertyName, int fallback)
        {
            try
            {
                PropertyInfo property = typeof(ProfilerDriver).GetProperty(
                    propertyName,
                    BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic
                );
                if (property == null || !property.CanRead)
                    return fallback;

                object value = property.GetValue(null, null);
                if (value == null)
                    return fallback;

                return Convert.ToInt32(value, CultureInfo.InvariantCulture);
            }
            catch
            {
                return fallback;
            }
        }

        private static RuntimeProfilerFrameExport BuildProfilerFrameExport(
            string name,
            int profilerFrameIndex,
            int sessionFrame,
            int unityTimeFrameCount,
            string threadName,
            int topCount)
        {
            EnsureProfilerRecording();

            int normalizedTopCount = Math.Max(0, Math.Min(topCount, MaxProfilerFrameSavedRows));
            string requestedThreadName = (threadName ?? "").Trim();
            var export = new RuntimeProfilerFrameExport
            {
                Name = name ?? "",
                ProfilerFrameIndex = profilerFrameIndex,
                SessionFrame = sessionFrame,
                ExportedAtUnityTimeFrameCount = unityTimeFrameCount,
                RequestedThreadName = requestedThreadName,
                ThreadName = "",
                ThreadGroupName = "",
                ThreadIndex = -1,
                ThreadId = 0,
                ThreadMatched = string.IsNullOrEmpty(requestedThreadName),
                FrameTimeMs = 0,
                FrameFps = 0,
                TopCount = normalizedTopCount,
                Error = ""
            };

            if (export.ProfilerFrameIndex < 0)
                export.ProfilerFrameIndex = CurrentProfilerFrameIndex();

            if (export.ProfilerFrameIndex < 0)
            {
                export.Error = "Unity Profiler has no available frame.";
                return export;
            }

            int firstFrameIndex = -1;
            int lastFrameIndex = -1;
            firstFrameIndex = ReadProfilerDriverIntProperty("firstFrameIndex", -1);
            lastFrameIndex = ReadProfilerDriverIntProperty("lastFrameIndex", -1);

            if (firstFrameIndex >= 0 && lastFrameIndex >= firstFrameIndex)
            {
                if (export.ProfilerFrameIndex < firstFrameIndex || export.ProfilerFrameIndex > lastFrameIndex)
                {
                    export.Error = "Profiler frame is outside the available buffer: requested="
                        + export.ProfilerFrameIndex
                        + " available="
                        + firstFrameIndex
                        + ".."
                        + lastFrameIndex;
                    return export;
                }
            }

            int selectedThreadIndex = -1;
            int fallbackThreadIndex = -1;
            for (int threadIndex = 0; ; threadIndex++)
            {
                using (HierarchyFrameDataView frameData = ProfilerDriver.GetHierarchyFrameDataView(
                    export.ProfilerFrameIndex,
                    threadIndex,
                    HierarchyFrameDataView.ViewModes.Default,
                    HierarchyFrameDataView.columnTotalTime,
                    false))
                {
                    if (!frameData.valid)
                        break;

                    if (fallbackThreadIndex < 0)
                        fallbackThreadIndex = threadIndex;

                    if (string.IsNullOrEmpty(requestedThreadName)
                        || string.Equals(frameData.threadName, requestedThreadName, StringComparison.OrdinalIgnoreCase)
                        || string.Equals(frameData.threadGroupName, requestedThreadName, StringComparison.OrdinalIgnoreCase))
                    {
                        selectedThreadIndex = threadIndex;
                        export.ThreadMatched = true;
                        break;
                    }
                }
            }

            if (selectedThreadIndex < 0)
            {
                selectedThreadIndex = fallbackThreadIndex;
                export.ThreadMatched = false;
            }

            if (selectedThreadIndex < 0)
            {
                export.Error = "Profiler frame has no valid thread data.";
                return export;
            }

            using (HierarchyFrameDataView frameData = ProfilerDriver.GetHierarchyFrameDataView(
                export.ProfilerFrameIndex,
                selectedThreadIndex,
                HierarchyFrameDataView.ViewModes.Default,
                HierarchyFrameDataView.columnTotalTime,
                false))
            {
                if (!frameData.valid)
                {
                    export.Error = "Profiler frame data is unavailable for selected thread.";
                    return export;
                }

                export.ProfilerFrameIndex = frameData.frameIndex;
                export.ThreadName = frameData.threadName ?? "";
                export.ThreadGroupName = frameData.threadGroupName ?? "";
                export.ThreadIndex = frameData.threadIndex;
                try
                {
                    export.ThreadId = Convert.ToInt64(frameData.threadId, CultureInfo.InvariantCulture);
                }
                catch
                {
                    export.ThreadId = 0;
                }
                export.FrameTimeMs = frameData.frameTimeMs;
                export.FrameFps = frameData.frameFps;

                var rows = new List<RuntimeProfilerFrameRow>(256);
                CollectProfilerHierarchyRows(frameData, rows);
                rows.Sort(delegate (RuntimeProfilerFrameRow left, RuntimeProfilerFrameRow right)
                {
                    return right.TotalMs.CompareTo(left.TotalMs);
                });

                int rowCount = normalizedTopCount > 0 ? Math.Min(normalizedTopCount, rows.Count) : 0;
                for (int i = 0; i < rowCount; i++)
                    export.Rows.Add(rows[i]);

                if (rows.Count == 0)
                    export.Error = "Profiler hierarchy has no rows for selected thread.";
            }

            return export;
        }

        private static void CollectProfilerHierarchyRows(
            HierarchyFrameDataView frameData,
            List<RuntimeProfilerFrameRow> rows)
        {
            int rootId = frameData.GetRootItemID();
            var stack = new Stack<int>();
            var children = new List<int>(64);

            frameData.GetItemChildren(rootId, children);
            for (int i = children.Count - 1; i >= 0; i--)
                stack.Push(children[i]);

            while (stack.Count > 0)
            {
                int itemId = stack.Pop();
                rows.Add(BuildProfilerHierarchyRow(frameData, itemId));

                children.Clear();
                frameData.GetItemChildren(itemId, children);
                for (int i = children.Count - 1; i >= 0; i--)
                    stack.Push(children[i]);
            }
        }

        private static RuntimeProfilerFrameRow BuildProfilerHierarchyRow(
            HierarchyFrameDataView frameData,
            int itemId)
        {
            string path = "";
            try
            {
                path = GetProfilerHierarchyItemPath(frameData, itemId);
            }
            catch
            {
            }

            return new RuntimeProfilerFrameRow
            {
                Depth = frameData.GetItemDepth(itemId),
                Name = frameData.GetItemName(itemId) ?? "",
                Path = path,
                TotalMs = GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnTotalTime),
                SelfMs = GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnSelfTime),
                TotalPercent = GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnTotalPercent),
                SelfPercent = GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnSelfPercent),
                Calls = (int)Math.Round(GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnCalls)),
                GcBytes = GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnGcMemory),
                WarningCount = (int)Math.Round(GetProfilerHierarchyColumn(frameData, itemId, HierarchyFrameDataView.columnWarningCount))
            };
        }

        private static double GetProfilerHierarchyColumn(
            HierarchyFrameDataView frameData,
            int itemId,
            int column)
        {
            try
            {
                return frameData.GetItemColumnDataAsSingle(itemId, column);
            }
            catch
            {
                return 0;
            }
        }

        private static string GetProfilerHierarchyItemPath(HierarchyFrameDataView frameData, int itemId)
        {
            MethodInfo method = typeof(HierarchyFrameDataView).GetMethod(
                "GetItemPath",
                BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic,
                null,
                new[] { typeof(int) },
                null
            );
            if (method == null)
                return "";

            object value = method.Invoke(frameData, new object[] { itemId });
            return value != null ? value.ToString() : "";
        }

        private static string RunStatesResultDirectory()
        {
            string dataPath = Application.dataPath;
            string projectRoot = Directory.GetParent(dataPath).FullName;
            string directory = Path.Combine(projectRoot, "Library", "Locus", "RunStates");
            Directory.CreateDirectory(directory);
            return directory;
        }

        private static string SanitizeRunStatesFileName(string value)
        {
            string normalized = (value ?? "").Trim();
            if (string.IsNullOrEmpty(normalized))
                normalized = "unnamed";

            char[] invalid = Path.GetInvalidFileNameChars();
            var sb = new StringBuilder(normalized.Length);
            for (int i = 0; i < normalized.Length; i++)
            {
                char ch = normalized[i];
                bool isInvalid = false;
                for (int j = 0; j < invalid.Length; j++)
                {
                    if (ch == invalid[j])
                    {
                        isInvalid = true;
                        break;
                    }
                }

                sb.Append(isInvalid || char.IsWhiteSpace(ch) ? '-' : ch);
            }
            return sb.ToString();
        }

        public sealed class RuntimeCtx
        {
            private readonly RuntimeStateMachineSession _session;

            internal RuntimeCtx(RuntimeStateMachineSession session)
            {
                _session = session;
            }

            public string StateName { get { return _session.CurrentStateName; } }
            public int TotalFrames { get { return _session.TotalFrames; } }
            public int ElapsedFramesInState { get { return _session.ElapsedFramesInState; } }
            public float TotalSeconds { get { return (float)_session.TotalSeconds; } }
            public float ElapsedSecondsInState { get { return (float)_session.ElapsedSecondsInState; } }

            public void Print(object value)
            {
                _session.Print(value);
            }

            public void Sleep(int frames)
            {
                int normalized = Math.Max(0, frames);
                if (normalized <= 0)
                    return;
                throw _session.Control(RunStatesControlKind.Sleep, null, null, normalized);
            }

            public void Goto(string stateName)
            {
                throw _session.Control(RunStatesControlKind.Goto, stateName, null, 0);
            }

            public void Done(string message)
            {
                throw _session.Control(RunStatesControlKind.Done, null, message, 0);
            }

            public void Done()
            {
                Done(null);
            }

            public void Fail(string message)
            {
                throw _session.Control(RunStatesControlKind.Fail, null, message, 0);
            }

            public void PromptUser(string token, string message)
            {
                _session.PromptUser(token, message);
            }

            public void ClearPrompt(string token)
            {
                _session.ClearPrompt(token);
            }

            public RuntimeProfilerMetric ProfilerMetric(string name, ProfilerCategory category, string markerName, double scale, string unit)
            {
                return _session.ProfilerMetric(name, category, markerName, scale, unit);
            }

            public RuntimeProfilerMetric[] DefaultProfilerMetrics()
            {
                return _session.DefaultProfilerMetrics();
            }

            public void StartProfiler(string name)
            {
                _session.StartProfiler(name, DefaultProfilerMetrics());
            }

            public void StartProfiler(string name, IEnumerable<RuntimeProfilerMetric> metrics)
            {
                _session.StartProfiler(name, metrics);
            }

            public void StopProfiler(string name)
            {
                _session.StopProfiler(name);
            }

            public void PrintProfilerSummary(string name)
            {
                _session.PrintProfilerSummary(name);
            }

            public bool TryGetProfilerLastValue(string profilerName, string metricName, out double value)
            {
                return _session.TryGetProfilerLastValue(profilerName, metricName, out value);
            }

            public double GetProfilerLastValue(string profilerName, string metricName)
            {
                return _session.GetProfilerLastValue(profilerName, metricName);
            }

            public RuntimeProfilerMetricSummary GetProfilerSummary(string profilerName, string metricName)
            {
                return _session.GetProfilerSummary(profilerName, metricName);
            }

            public bool RecordProfilerSpike(string profilerName, string metricName, double threshold)
            {
                return _session.RecordProfilerSpike(profilerName, metricName, threshold, "");
            }

            public bool RecordProfilerSpike(string profilerName, string metricName, double threshold, string label)
            {
                return _session.RecordProfilerSpike(profilerName, metricName, threshold, label);
            }

            public bool RecordProfilerSpikeTop(string profilerName, string metricName, double threshold, string label, int maxSpikes)
            {
                return _session.RecordProfilerSpikeTop(profilerName, metricName, threshold, label, maxSpikes);
            }

            public int GetProfilerLastSpikeFrame(string profilerName, string metricName)
            {
                return _session.GetProfilerLastSpikeFrame(profilerName, metricName);
            }

            public RuntimeProfilerSpike[] GetProfilerSpikes(string profilerName)
            {
                return _session.GetProfilerSpikes(profilerName);
            }

            public int LatestProfilerFrameIndex()
            {
                return CurrentProfilerFrameIndex();
            }

            public string SaveProfiler(string name)
            {
                return _session.SaveProfiler(name);
            }

            public string SaveProfilerFrame(string name, string threadName, int topCount)
            {
                return _session.SaveProfilerFrame(name, threadName, topCount);
            }

            public string SaveProfilerFrame(string name, string threadName, int topCount, int inlineRows)
            {
                return _session.SaveProfilerFrame(name, threadName, topCount, inlineRows);
            }

            public string SaveProfilerFrame(string name, int profilerFrameIndex, string threadName, int topCount)
            {
                return _session.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount);
            }

            public string SaveProfilerFrame(string name, int profilerFrameIndex, string threadName, int topCount, int inlineRows)
            {
                return _session.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount, inlineRows);
            }

            public void Set(string key, object value)
            {
                _session.SetMemory(key, value);
            }

            public void SetGlobal(string key, object value)
            {
                _session.SetMemory(key, value);
            }

            public bool Has(string key)
            {
                return _session.HasMemory(key);
            }

            public bool HasGlobal(string key)
            {
                return _session.HasMemory(key);
            }

            public T Get<T>(string key)
            {
                return _session.GetMemory<T>(key);
            }

            public T GetGlobal<T>(string key)
            {
                return _session.GetMemory<T>(key);
            }

            public RuntimeVar<T> Global<T>(string key)
            {
                return _session.GetGlobal<T>(key, default(T));
            }

            public RuntimeVar<T> Global<T>(string key, T initialValue)
            {
                return _session.GetGlobal<T>(key, initialValue);
            }

            public bool Remove(string key)
            {
                return _session.RemoveMemory(key);
            }

            public bool RemoveGlobal(string key)
            {
                return _session.RemoveMemory(key);
            }
        }

        internal sealed class RuntimeStateMachineSession
        {
            private readonly RuntimeStateMachineDefinition _definition;
            private readonly TaskCompletionSource<RunStatesCompletion> _completion;
            private readonly List<string> _prints = new List<string>(64);
            private readonly Dictionary<string, object> _memory = new Dictionary<string, object>(StringComparer.Ordinal);
            private readonly Dictionary<string, RuntimeProfilerSession> _profilers =
                new Dictionary<string, RuntimeProfilerSession>(StringComparer.Ordinal);
            private readonly Stopwatch _stopwatch = Stopwatch.StartNew();
            private readonly RuntimeCtx _runtimeCtx;
            private readonly RunStatesControlException _control = new RunStatesControlException();

            private string _currentStateName;
            private bool _needsStart = true;
            private bool _completed;
            private bool _runningEnd;
            private int _sleepFrames;
            private int _stateStartFrame;
            private double _stateStartSeconds;
            private int _printLineCount;
            private long _printBytes;
            private bool _printHardLimitReached;

            internal RuntimeStateMachineSession(
                RuntimeStateMachineDefinition definition,
                string initialState,
                TaskCompletionSource<RunStatesCompletion> completion)
            {
                _definition = definition;
                _currentStateName = initialState;
                _completion = completion;
                _runtimeCtx = new RuntimeCtx(this);
                _stateStartFrame = 0;
                _stateStartSeconds = 0;
            }

            public string CurrentStateName { get { return _currentStateName; } }
            public bool IsCompleted { get { return _completed; } }
            public int TotalFrames { get; private set; }
            public int ElapsedFramesInState { get { return Math.Max(0, TotalFrames - _stateStartFrame); } }
            public double TotalSeconds { get { return _stopwatch.Elapsed.TotalSeconds; } }
            public double ElapsedSecondsInState { get { return Math.Max(0, TotalSeconds - _stateStartSeconds); } }

            internal RunStatesControlException Control(RunStatesControlKind kind, string target, string message, int sleepFrames)
            {
                _control.Reset(kind, target, message, sleepFrames);
                return _control;
            }

            public void Tick()
            {
                if (_completed)
                    return;

                TotalFrames++;

                if (TotalFrames > RunStatesMaxFrames)
                {
                    Complete(false, BuildResult("error", "max frame limit reached: " + RunStatesMaxFrames));
                    return;
                }

                if (_sleepFrames > 0)
                {
                    _sleepFrames--;
                    return;
                }

                RuntimeStateDefinition state;
                try
                {
                    state = _definition.GetState(_currentStateName);
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", ex.Message));
                    return;
                }

                if (_needsStart)
                {
                    _needsStart = false;
                    if (!RunHandler(state.Start, "start"))
                        return;
                }

                SampleProfilers();
                RunHandler(state.Update, "update");
            }

            public void Print(object value)
            {
                string text = value != null ? (value.ToString() ?? "null") : "null";
                long nextBytes = _printBytes + EstimatePrintBytes(text);
                _printLineCount += CountPrintLines(text);

                if (nextBytes > RunStatesPrintHardLimitBytes)
                {
                    _printBytes = nextBytes;
                    _printHardLimitReached = true;
                    Complete(false, BuildResult(
                        "error",
                        "too large: print output exceeded hard limit of "
                            + RunStatesPrintHardLimitTokens
                            + " estimated tokens; result was not saved."));
                    throw Control(RunStatesControlKind.Fail, null, "too large", 0);
                }

                _printBytes = nextBytes;
                _prints.Add(text);
            }

            public RuntimeProfilerMetric ProfilerMetric(string name, ProfilerCategory category, string markerName, double scale, string unit)
            {
                string normalizedName = (name ?? "").Trim();
                string normalizedMarker = (markerName ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedName))
                    throw new ArgumentException("Profiler metric name is required.");
                if (string.IsNullOrEmpty(normalizedMarker))
                    throw new ArgumentException("Profiler marker name is required.");

                return new RuntimeProfilerMetric(
                    normalizedName,
                    category,
                    normalizedMarker,
                    scale,
                    (unit ?? "").Trim()
                );
            }

            public RuntimeProfilerMetric[] DefaultProfilerMetrics()
            {
                return new[]
                {
                    ProfilerMetric("main_thread_ms", ProfilerCategory.Internal, "Main Thread", 0.000001, "ms"),
                    ProfilerMetric("render_thread_ms", ProfilerCategory.Internal, "Render Thread", 0.000001, "ms"),
                    ProfilerMetric("gc_alloc_bytes", ProfilerCategory.Memory, "GC.Alloc", 1.0, "bytes"),
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
                string normalizedName = NormalizeProfilerName(name);
                if (_profilers.ContainsKey(normalizedName))
                    throw new InvalidOperationException("Profiler already started: " + normalizedName);

                var metricList = new List<RuntimeProfilerMetric>();
                if (metrics != null)
                {
                    foreach (RuntimeProfilerMetric metric in metrics)
                    {
                        if (metric != null)
                            metricList.Add(metric);
                    }
                }

                EnsureProfilerRecording();
                _profilers.Add(normalizedName, new RuntimeProfilerSession(
                    normalizedName,
                    metricList,
                    TotalFrames,
                    Time.frameCount
                ));
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

            public void PromptUser(string token, string message)
            {
                string normalizedToken = (token ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedToken))
                    return;

                string normalizedMessage = (message ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedMessage))
                    return;

                UpdateRunStatesPrompt(normalizedToken, normalizedMessage, _currentStateName, TotalFrames);
            }

            public void ClearPrompt(string token)
            {
                string normalizedToken = (token ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedToken))
                    return;

                ClearRunStatesPrompt(normalizedToken);
            }

            public void SetMemory(string key, object value)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                _memory[normalizedKey] = value;
            }

            public bool HasMemory(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                return _memory.ContainsKey(normalizedKey);
            }

            public T GetMemory<T>(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                object value;
                if (!_memory.TryGetValue(normalizedKey, out value))
                    throw new KeyNotFoundException("Runtime memory key not found: " + normalizedKey);

                if (value == null)
                    return default(T);

                if (!(value is T))
                    throw new InvalidCastException("Runtime memory key '" + normalizedKey + "' contains " + value.GetType().FullName);

                return (T)value;
            }

            public RuntimeVar<T> GetGlobal<T>(string key, T initialValue)
            {
                string normalizedKey = EnsureMemory(key, initialValue);
                return new RuntimeVar<T>(this, normalizedKey);
            }

            public bool RemoveMemory(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                return _memory.Remove(normalizedKey);
            }

            private string EnsureMemory<T>(string key, T initialValue)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                object value;
                if (_memory.TryGetValue(normalizedKey, out value))
                {
                    if (value != null && !(value is T))
                        throw new InvalidCastException("Runtime memory key '" + normalizedKey + "' contains " + value.GetType().FullName);
                    return normalizedKey;
                }

                _memory[normalizedKey] = initialValue;
                return normalizedKey;
            }

            private string NormalizeMemoryKey(string key)
            {
                string normalizedKey = (key ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedKey))
                    throw new ArgumentException("Runtime memory key is required.");
                return normalizedKey;
            }

            private void SampleProfilers()
            {
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

            private void DisposeProfilers()
            {
                foreach (RuntimeProfilerSession profiler in _profilers.Values)
                    profiler.Stop(TotalFrames, Time.frameCount);
                _profilers.Clear();
            }

            private bool RunHandler(Action<RuntimeCtx> handler, string phase)
            {
                if (handler == null || _completed)
                    return !_completed;

                try
                {
                    handler(_runtimeCtx);
                    return !_completed;
                }
                catch (RunStatesControlException control)
                {
                    HandleControl(control, phase);
                    return false;
                }
                catch (Exception ex)
                {
                    CompleteWithEnd(false, "runtime error in state '" + _currentStateName + "' " + phase + ": " + ex);
                    return false;
                }
            }

            private void HandleControl(RunStatesControlException control, string phase)
            {
                if (_completed)
                    return;

                if (_runningEnd && control.Kind != RunStatesControlKind.Fail)
                {
                    Complete(false, BuildResult("error", "State end handler cannot call " + control.Kind + "."));
                    return;
                }

                switch (control.Kind)
                {
                    case RunStatesControlKind.Sleep:
                        if (string.Equals(phase, "end", StringComparison.Ordinal))
                        {
                            Complete(false, BuildResult("error", "State end handler cannot call Sleep."));
                            return;
                        }
                        _sleepFrames = Math.Max(0, control.SleepFrames);
                        break;

                    case RunStatesControlKind.Goto:
                        TransitionTo(control.Target);
                        break;

                    case RunStatesControlKind.Done:
                        CompleteWithEnd(true, control.MessageText);
                        break;

                    case RunStatesControlKind.Fail:
                        CompleteWithEnd(false, string.IsNullOrEmpty(control.MessageText) ? "state machine failed" : control.MessageText);
                        break;
                }
            }

            private void TransitionTo(string targetState)
            {
                string normalizedTarget = (targetState ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedTarget))
                {
                    CompleteWithEnd(false, "Goto requires a state name.");
                    return;
                }

                if (!_definition.ContainsState(normalizedTarget))
                {
                    CompleteWithEnd(false, "Unknown target state: " + normalizedTarget);
                    return;
                }

                string previousState = _currentStateName;
                RunEnd(previousState);
                if (_completed)
                    return;

                ClearPromptsForState(previousState);
                _currentStateName = normalizedTarget;
                _needsStart = true;
                _sleepFrames = 0;
                _stateStartFrame = TotalFrames;
                _stateStartSeconds = TotalSeconds;
            }

            private void CompleteWithEnd(bool ok, string message)
            {
                string stateName = _currentStateName;
                RunEnd(stateName);
                if (_completed)
                    return;

                ClearPromptsForState(stateName);
                string status = ok ? "ok" : "error";
                Complete(ok, BuildResult(status, message));
            }

            private void RunEnd(string stateName)
            {
                if (_runningEnd || _completed)
                    return;

                RuntimeStateDefinition state;
                try
                {
                    state = _definition.GetState(stateName);
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", ex.Message));
                    return;
                }

                if (state.End == null)
                    return;

                _runningEnd = true;
                try
                {
                    state.End(_runtimeCtx);
                }
                catch (RunStatesControlException control)
                {
                    if (control.Kind == RunStatesControlKind.Fail)
                        Complete(false, BuildResult("error", string.IsNullOrEmpty(control.MessageText) ? "state end failed" : control.MessageText));
                    else
                        Complete(false, BuildResult("error", "State end handler cannot call " + control.Kind + "."));
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", "runtime error in state '" + stateName + "' end: " + ex));
                }
                finally
                {
                    _runningEnd = false;
                }
            }

            private string BuildResult(string status, string message)
            {
                var sb = new StringBuilder(1024);
                sb.Append("status: ").AppendLine(status);
                sb.Append("final_state: ").AppendLine(_currentStateName ?? "");
                sb.Append("frames: ").AppendLine(TotalFrames.ToString());
                sb.Append("duration_ms: ").AppendLine(_stopwatch.ElapsedMilliseconds.ToString());
                if (!string.IsNullOrEmpty(message))
                    sb.Append("message: ").AppendLine(message);
                sb.Append("print_lines: ").AppendLine(_printLineCount.ToString());
                sb.Append("print_tokens_estimate: ").AppendLine(EstimatePrintTokens(_printBytes).ToString());
                if (_printHardLimitReached)
                {
                    sb.AppendLine("print_output: too large");
                    return sb.ToString();
                }
                sb.AppendLine("prints:");
                for (int i = 0; i < _prints.Count; i++)
                    sb.AppendLine(_prints[i]);
                return sb.ToString();
            }

            private static long EstimatePrintBytes(string text)
            {
                long bytes = Utf8NoBom.GetByteCount(text ?? "");
                return bytes + 1;
            }

            private static int CountPrintLines(string text)
            {
                if (string.IsNullOrEmpty(text))
                    return 1;

                int lines = 1;
                for (int i = 0; i < text.Length; i++)
                {
                    if (text[i] == '\n')
                        lines++;
                }
                return lines;
            }

            private static long EstimatePrintTokens(long byteCount)
            {
                if (byteCount <= 0)
                    return 0;
                return (byteCount + RunStatesPrintTokenByteRatio - 1) / RunStatesPrintTokenByteRatio;
            }

            private void Complete(bool ok, string message)
            {
                if (_completed)
                    return;

                _completed = true;
                ClearAllRunStatesPrompts();
                DisposeProfilers();
                _completion.TrySetResult(new RunStatesCompletion(ok, message));
            }
        }

        private static PipeEnvelope HandleSetEditorStatus(string requestId, string desiredStatus)
        {
            string normalized = (desiredStatus ?? "").Trim();
            if (string.IsNullOrEmpty(normalized))
                return ErrorResponse(requestId, "empty requested editor status");

            switch (normalized)
            {
                case "editing":
                    _isPaused = false;
                    _isPlaying = false;
                    NativePublishEditorStatusNow();
                    PostToMainThread(delegate
                    {
                        try
                        {
                            EditorApplication.isPaused = false;
                            EditorApplication.isPlaying = false;
                        }
                        catch (Exception ex)
                        {
                            UnityEngine.Debug.LogError("[Locus] set_editor_status(editing) failed: " + ex);
                        }
                    });
                    return OkResponse(requestId, "editing_requested");

                case "playing":
                    _isPaused = false;
                    _isPlaying = true;
                    NativePublishEditorStatusNow();
                    PostToMainThread(delegate
                    {
                        try
                        {
                            EditorApplication.isPaused = false;
                            EditorApplication.isPlaying = true;
                        }
                        catch (Exception ex)
                        {
                            UnityEngine.Debug.LogError("[Locus] set_editor_status(playing) failed: " + ex);
                        }
                    });
                    return OkResponse(requestId, "playing_requested");

                case "playing_paused":
                    _isPaused = true;
                    _isPlaying = true;
                    NativePublishEditorStatusNow();
                    PostToMainThread(delegate
                    {
                        try
                        {
                            if (!EditorApplication.isPlaying)
                                EditorApplication.isPlaying = true;
                            EditorApplication.isPaused = true;
                        }
                        catch (Exception ex)
                        {
                            UnityEngine.Debug.LogError("[Locus] set_editor_status(playing_paused) failed: " + ex);
                        }
                    });
                    return OkResponse(requestId, "playing_paused_requested");

                default:
                    return ErrorResponse(requestId, "unsupported editor status: " + normalized);
            }
        }

        private static async Task<PipeEnvelope> HandleCompileRunStates(string requestId, string requestJson)
        {
            if (string.IsNullOrWhiteSpace(requestJson))
                return ErrorResponse(requestId, "empty run_states request");

            await _runStatesLock.WaitAsync();
            try
            {
                string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
                if (!string.IsNullOrEmpty(prepareError))
                    return ErrorResponse(requestId, prepareError);

                RunStatesRequest request;
                try
                {
                    request = JsonUtility.FromJson<RunStatesRequest>(requestJson);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states request parse failed: " + ex.Message);
                }

                string validationError = ValidateRunStatesRequest(request);
                if (!string.IsNullOrEmpty(validationError))
                    return ErrorResponse(requestId, validationError);

                try
                {
                    CompileRunStates(request);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states compilation exception: " + ex.Message);
                }

                return OkResponse(requestId, "run_states compilation ok");
            }
            finally
            {
                _runStatesLock.Release();
            }
        }

        private static bool IsRunStatesEditorStatus(string status)
        {
            switch (status)
            {
                case "editing":
                case "playing":
                case "playing_paused":
                    return true;
                default:
                    return false;
            }
        }

        private static string CurrentRunStatesEditorStatus()
        {
            return _isPlaying
                ? (_isPaused ? "playing_paused" : "playing")
                : "editing";
        }

        private static string ValidateRunStatesEditorStatus(string requestedStatus)
        {
            string normalized = (requestedStatus ?? "").Trim();
            if (!IsRunStatesEditorStatus(normalized))
                return "unsupported request_editor_status: " + normalized;

            string actual = CurrentRunStatesEditorStatus();
            if (actual != normalized)
            {
                return "Unity Editor status check failed before run_states: requested \""
                    + normalized
                    + "\", actual \""
                    + actual
                    + "\".";
            }

            return null;
        }

        private static async Task<PipeEnvelope> HandleRunStates(string requestId, string requestJson)
        {
            if (string.IsNullOrWhiteSpace(requestJson))
                return ErrorResponse(requestId, "empty run_states request");

            await _runStatesLock.WaitAsync();
            try
            {
                string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
                if (!string.IsNullOrEmpty(prepareError))
                    return ErrorResponse(requestId, prepareError);

                RunStatesRequest request;
                try
                {
                    request = JsonUtility.FromJson<RunStatesRequest>(requestJson);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states request parse failed: " + ex.Message);
                }

                string validationError = ValidateRunStatesRequest(request);
                if (!string.IsNullOrEmpty(validationError))
                    return ErrorResponse(requestId, validationError);

                string initialState = request.initial_state.Trim();

                CompiledRunStates compiled;
                try
                {
                    compiled = CompileRunStates(request);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states compilation exception: " + ex.Message);
                }

                return await RunCompiledRunStatesAsync(
                    requestId,
                    compiled,
                    request.request_editor_status,
                    initialState);
            }
            finally
            {
                _runStatesLock.Release();
            }
        }

        // ───────────────── run_states_loaded (compile-server sidecar) ─────────────────

        [Serializable]
        private sealed class RunStatesLoadedRequest
        {
            public string assembly_b64;
            public string assembly_path;
            public string entry_type;
            public string request_editor_status;
            public string initial_state;
        }

        /// <summary>
        /// Sidecar variant of run_states: the state machine was already
        /// compiled by the Locus compile server; load the assembly bytes and
        /// run the same session pipeline as the in-Unity compile path.
        /// </summary>
        private static async Task<PipeEnvelope> HandleRunStatesLoaded(string requestId, string requestJson)
        {
            if (string.IsNullOrWhiteSpace(requestJson))
                return ErrorResponse(requestId, "empty run_states_loaded request");

            RunStatesLoadedRequest request;
            try
            {
                request = JsonUtility.FromJson<RunStatesLoadedRequest>(requestJson);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "run_states_loaded request parse failed: " + ex.Message);
            }

            if (request == null ||
                (string.IsNullOrEmpty(request.assembly_b64) &&
                 string.IsNullOrEmpty(request.assembly_path)))
                return ErrorResponse(requestId, "run_states_loaded request missing assembly bytes");
            if (string.IsNullOrWhiteSpace(request.initial_state))
                return ErrorResponse(requestId, "initial_state is required");

            byte[] assemblyBytes;
            try
            {
                assemblyBytes = ReadAssemblyPayload(request.assembly_b64, request.assembly_path);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "run_states_loaded assembly load failed: " + ex.Message);
            }

            string entryTypeName = string.IsNullOrEmpty(request.entry_type)
                ? "Locus.RuntimeStateMachines.__LocusRunStatesHost"
                : request.entry_type;

            await _runStatesLock.WaitAsync();
            try
            {
                CompiledRunStates compiled;
                try
                {
                    compiled = LoadCompiledRunStates(assemblyBytes, entryTypeName);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states compilation exception: " + ex.Message);
                }

                return await RunCompiledRunStatesAsync(
                    requestId,
                    compiled,
                    request.request_editor_status,
                    request.initial_state.Trim());
            }
            finally
            {
                _runStatesLock.Release();
            }
        }

        /// <summary>
        /// Load a sidecar-compiled run_states assembly and bind its Build
        /// entry point. Mirrors the load section of CompileRunStates,
        /// including error wording.
        /// </summary>
        private static CompiledRunStates LoadCompiledRunStates(byte[] assemblyBytes, string entryTypeName)
        {
            try
            {
                Assembly assembly = Assembly.Load(assemblyBytes);

                Type hostType = assembly.GetType(entryTypeName, true);
                MethodInfo buildMethod = hostType.GetMethod(
                    "Build",
                    BindingFlags.Public | BindingFlags.Static
                );

                if (buildMethod == null)
                    throw new Exception("compiled state machine missing Build method");

                Func<RuntimeStateMachineDefinition> builder =
                    (Func<RuntimeStateMachineDefinition>)Delegate.CreateDelegate(
                        typeof(Func<RuntimeStateMachineDefinition>),
                        buildMethod
                    );

                return new CompiledRunStates(builder);
            }
            catch (Exception ex)
            {
                throw new Exception("assembly load/bootstrap failed: " + ex);
            }
        }

        /// <summary>
        /// Shared run_states / run_states_loaded session bootstrap: validate
        /// editor status on the main thread, build the definition, and hand
        /// it to a RuntimeStateMachineSession.
        /// </summary>
        private static async Task<PipeEnvelope> RunCompiledRunStatesAsync(
            string requestId,
            CompiledRunStates compiled,
            string requestedEditorStatus,
            string initialState)
        {
            var completion = new TaskCompletionSource<RunStatesCompletion>();
            PostToMainThread(delegate
            {
                try
                {
                    if (_activeRunStatesSession != null)
                    {
                        completion.TrySetResult(new RunStatesCompletion(false, "A unity_run_states session is already running."));
                        return;
                    }

                    string statusError = ValidateRunStatesEditorStatus(requestedEditorStatus);
                    if (!string.IsNullOrEmpty(statusError))
                    {
                        completion.TrySetResult(new RunStatesCompletion(false, statusError));
                        return;
                    }

                    RuntimeStateMachineDefinition definition = compiled.Builder();
                    if (!definition.ContainsState(initialState))
                    {
                        completion.TrySetResult(new RunStatesCompletion(false, "Initial state not found: " + initialState));
                        return;
                    }

                    _activeRunStatesSession = new RuntimeStateMachineSession(definition, initialState, completion);
                }
                catch (Exception ex)
                {
                    completion.TrySetResult(new RunStatesCompletion(false, "run_states bootstrap failed: " + ex));
                }
            });

            RunStatesCompletion result = await completion.Task;
            if (result.Ok)
                return OkResponse(requestId, result.Message);
            return ErrorResponse(requestId, result.Message);
        }

        private static string ValidateRunStatesRequest(RunStatesRequest request)
        {
            if (request == null)
                return "run_states request is empty";
            if (string.IsNullOrWhiteSpace(request.request_editor_status))
                return "request_editor_status is required";
            if (!IsRunStatesEditorStatus(request.request_editor_status.Trim()))
                return "unsupported request_editor_status: " + request.request_editor_status.Trim();
            if (string.IsNullOrWhiteSpace(request.initial_state))
                return "initial_state is required";
            if (request.states == null || request.states.Length == 0)
                return "states must contain at least one state";

            var names = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < request.states.Length; i++)
            {
                RunStatesStateRequest state = request.states[i];
                if (state == null)
                    return "states[" + i + "] is empty";

                string name = (state.name ?? "").Trim();
                if (string.IsNullOrEmpty(name))
                    return "states[" + i + "].name is required";
                if (!names.Add(name))
                    return "duplicate state name: " + name;
                if (string.IsNullOrWhiteSpace(state.update))
                    return "state '" + name + "' requires update code";
            }

            if (!names.Contains(request.initial_state.Trim()))
                return "initial_state not found in states: " + request.initial_state;

            return null;
        }

        private static CompiledRunStates CompileRunStates(RunStatesRequest request)
        {
            string source = BuildRunStatesSource(request);

            SyntaxTree syntaxTree;
            try
            {
                syntaxTree = CSharpSyntaxTree.ParseText(
                    source,
                    SnippetParseOptions,
                    path: "LocusRunStates.cs",
                    encoding: Utf8NoBom
                );
            }
            catch (Exception ex)
            {
                throw new Exception("parse failed: " + ex);
            }

            string assemblyName =
                "__LocusRunStates_" + Interlocked.Increment(ref _snippetAssemblyCounter).ToString("X8");

            CSharpCompilation compilation = CSharpCompilation.Create(
                assemblyName: assemblyName,
                syntaxTrees: new[] { syntaxTree },
                references: EnsureMetadataReferences(),
                options: SnippetCompilationOptions
            );

            using (var peStream = new MemoryStream(16 * 1024))
            {
                EmitResult emitResult;
                try
                {
                    emitResult = compilation.Emit(peStream);
                }
                catch (Exception ex)
                {
                    throw new Exception("emit failed: " + ex);
                }

                if (!emitResult.Success)
                    throw new Exception(BuildDiagnosticErrorText(emitResult.Diagnostics));

                try
                {
                    byte[] assemblyBytes = peStream.ToArray();
                    Assembly assembly = Assembly.Load(assemblyBytes);

                    Type hostType = assembly.GetType("Locus.RuntimeStateMachines.__LocusRunStatesHost", true);
                    MethodInfo buildMethod = hostType.GetMethod(
                        "Build",
                        BindingFlags.Public | BindingFlags.Static
                    );

                    if (buildMethod == null)
                        throw new Exception("compiled state machine missing Build method");

                    Func<RuntimeStateMachineDefinition> builder =
                        (Func<RuntimeStateMachineDefinition>)Delegate.CreateDelegate(
                            typeof(Func<RuntimeStateMachineDefinition>),
                            buildMethod
                        );

                    return new CompiledRunStates(builder);
                }
                catch (Exception ex)
                {
                    throw new Exception("assembly load/bootstrap failed: " + ex);
                }
            }
        }

        private static string BuildRunStatesSource(RunStatesRequest request)
        {
            var sb = new StringBuilder(8192);
            sb.AppendLine("using System;");
            sb.AppendLine("using System.IO;");
            sb.AppendLine("using System.Text;");
            sb.AppendLine("using System.Linq;");
            sb.AppendLine("using System.Reflection;");
            sb.AppendLine("using System.Threading;");
            sb.AppendLine("using System.Threading.Tasks;");
            sb.AppendLine("using System.Collections;");
            sb.AppendLine("using System.Collections.Generic;");
            sb.AppendLine("using UnityEngine;");
            sb.AppendLine("using UnityEngine.SceneManagement;");
            sb.AppendLine("using UnityEngine.UI;");
            sb.AppendLine("using Unity.Profiling;");
            sb.AppendLine("using UnityEditor;");
            sb.AppendLine("using UnityEditor.Profiling;");
            sb.AppendLine("using UnityEditorInternal;");
            sb.AppendLine("using UnityEditor.SceneManagement;");
            sb.AppendLine("using UnityEditor.Animations;");
            AppendRunStatesAutoUsings(sb, request.auto_usings);
            sb.AppendLine("using static UnityEngine.Object;");
            sb.AppendLine("using Object = UnityEngine.Object;");
            sb.AppendLine();
            sb.AppendLine("namespace Locus.RuntimeStateMachines");
            sb.AppendLine("{");
            sb.AppendLine("    public static class __LocusRunStatesHost");
            sb.AppendLine("    {");
            sb.AppendLine("        public static global::Locus.LocusBridge.RuntimeStateMachineDefinition Build()");
            sb.AppendLine("        {");
            sb.AppendLine("            var machine = new global::Locus.LocusBridge.RuntimeStateMachineDefinition();");

            for (int i = 0; i < request.states.Length; i++)
            {
                RunStatesStateRequest state = request.states[i];
                string name = (state.name ?? "").Trim();
                sb.AppendLine("            {");
                AppendRunStatesVariables(sb, name, state.variables, "                ");
                sb.Append("                machine.AddState(").Append(ToCSharpStringLiteral(name)).AppendLine(",");
                AppendRunStatesHandler(sb, name, "start", state.start, "                    ");
                sb.AppendLine(",");
                AppendRunStatesHandler(sb, name, "update", state.update, "                    ");
                sb.AppendLine(",");
                AppendRunStatesHandler(sb, name, "end", state.end, "                    ");
                sb.AppendLine("                );");
                sb.AppendLine("            }");
            }

            sb.AppendLine("            return machine;");
            sb.AppendLine("        }");
            sb.AppendLine("    }");
            sb.AppendLine("}");
            return sb.ToString();
        }

        private static void AppendRunStatesAutoUsings(StringBuilder sb, string[] namespaces)
        {
            if (namespaces == null || namespaces.Length == 0)
                return;

            var seen = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < namespaces.Length; i++)
            {
                string ns = (namespaces[i] ?? "").Trim();
                if (string.IsNullOrEmpty(ns) || !seen.Add(ns) || !IsValidUsingNamespace(ns))
                    continue;

                sb.Append("using ").Append(ns).AppendLine(";");
            }
        }

        private static bool IsValidUsingNamespace(string ns)
        {
            if (string.IsNullOrEmpty(ns))
                return false;

            for (int i = 0; i < ns.Length; i++)
            {
                char ch = ns[i];
                bool ok = ch == '_' || ch == '.' || char.IsLetterOrDigit(ch);
                if (!ok)
                    return false;
            }

            return true;
        }

        private static void AppendRunStatesVariables(StringBuilder sb, string stateName, string code, string indent)
        {
            if (string.IsNullOrWhiteSpace(code))
                return;

            sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":variables"));
            sb.AppendLine(code);
            sb.Append(indent).AppendLine("    #line default");
        }

        private static void AppendRunStatesHandler(StringBuilder sb, string stateName, string phase, string code, string indent)
        {
            if (string.IsNullOrWhiteSpace(code))
            {
                sb.Append(indent).Append("null");
                return;
            }

            sb.Append(indent).AppendLine("new global::System.Action<global::Locus.LocusBridge.RuntimeCtx>(ctx =>");
            sb.Append(indent).AppendLine("{");
            sb.Append(indent).AppendLine("    var print = new global::System.Action<object>(ctx.Print);");
            sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":" + phase));
            sb.AppendLine(code);
            sb.Append(indent).AppendLine("    #line default");
            sb.Append(indent).Append("})");
        }

        private static string ToCSharpStringLiteral(string value)
        {
            if (value == null)
                return "null";

            var sb = new StringBuilder(value.Length + 2);
            sb.Append('"');
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                switch (ch)
                {
                    case '\\': sb.Append("\\\\"); break;
                    case '"': sb.Append("\\\""); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\t': sb.Append("\\t"); break;
                    default: sb.Append(ch); break;
                }
            }
            sb.Append('"');
            return sb.ToString();
        }

        private static void PumpRunStates()
        {
            RuntimeStateMachineSession session = _activeRunStatesSession;
            if (session == null)
                return;

            session.Tick();

            if (session == _activeRunStatesSession && session.IsCompleted)
                _activeRunStatesSession = null;
        }

        private static void UpdateRunStatesPrompt(string token, string message, string stateName, int frame)
        {
        }

        private static void ClearRunStatesPrompt(string token)
        {
        }

        private static void ClearPromptsForState(string stateName)
        {
        }

        private static void ClearAllRunStatesPrompts()
        {
        }
    }
}
