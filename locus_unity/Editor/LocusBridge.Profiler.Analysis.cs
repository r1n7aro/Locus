using System;
using System.Collections.Generic;

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed partial class ProfilerApi
        {
            public bool IsProfilerStopped(string name) { return RequireProfiler(name).IsStopped; }

            public ProfilerObservation[] GetProfilerSamples(string name, string metricName, int offset = 0, int count = 600)
            {
                if (offset < 0 || count < 0 || count > 60000) throw new ArgumentOutOfRangeException("offset/count");
                var points = RequireProfiler(name).FindSample(metricName).Points();
                var result = new List<ProfilerObservation>();
                for (int i = offset; i < points.Length && result.Count < count; i++)
                {
                    var p = points[i];
                    result.Add(new ProfilerObservation { SessionFrame = p.SessionFrame, UnityTimeFrameCount = p.UnityTimeFrameCount,
                        ProfilerFrameIndex = p.ProfilerFrameIndex, ElapsedMs = p.ElapsedMs,
                        Value = double.IsNaN(p.Value) || double.IsInfinity(p.Value) ? (double?)null : p.Value });
                }
                return result.ToArray();
            }

            public ProfilerBudget GetProfilerBudget(string name, string metricName, double threshold)
            {
                if (double.IsNaN(threshold) || double.IsInfinity(threshold)) throw new ArgumentOutOfRangeException(nameof(threshold));
                var sample = RequireProfiler(name).FindSample(metricName);
                var result = new ProfilerBudget { Metric = metricName, Unit = sample.Metric.Unit, Threshold = threshold };
                foreach (var point in sample.Points())
                {
                    if (double.IsNaN(point.Value) || double.IsInfinity(point.Value)) continue;
                    result.SampleCount++;
                    if (point.Value > threshold) result.ExceededCount++;
                }
                if (result.SampleCount > 0) result.ExceededPercent = 100.0 * result.ExceededCount / result.SampleCount;
                return result;
            }

            public ProfilerComparison CompareProfilers(string baseline, string candidate, string metricName)
            {
                var baselineCapture = RequireProfiler(baseline);
                var candidateCapture = RequireProfiler(candidate);
                var a = baselineCapture.FindSample(metricName);
                var b = candidateCapture.FindSample(metricName);
                var result = new ProfilerComparison { Metric = metricName, Unit = a.Metric.Unit, Error = "" };
                if (!baselineCapture.CanComparePolicy(candidateCapture))
                { result.Error = "Sampling clock, interval, capture overhead or Editor/Play Mode differ."; return result; }
                if (a.Metric.MarkerName != b.Metric.MarkerName || a.Metric.Category != b.Metric.Category
                    || a.Metric.Unit != b.Metric.Unit || a.Metric.Scale != b.Metric.Scale || a.Metric.Options != b.Metric.Options
                    || a.Metric.ZeroIsUnavailable != b.Metric.ZeroIsUnavailable
                    || (a.Metric.Reader == null) != (b.Metric.Reader == null))
                { result.Error = "Metric source, category, unit, scale or recorder options differ."; return result; }
                var left = a.GetSummary(); var right = b.GetSummary();
                result.BaselineSamples = left.SampleCount; result.CandidateSamples = right.SampleCount;
                if (!left.Available || !right.Available)
                { result.Error = "Both captures require valid samples."; return result; }
                result.AverageDelta = right.Average - left.Average;
                result.P95Delta = right.P95 - left.P95;
                result.MaxDelta = right.Max - left.Max;
                // A zero baseline has no meaningful percentage increase.
                if (left.Average != 0) result.AveragePercent = 100 * result.AverageDelta / Math.Abs(left.Average);
                if (left.P95 != 0) result.P95Percent = 100 * result.P95Delta / Math.Abs(left.P95);
                return result;
            }
        }
    }
}
