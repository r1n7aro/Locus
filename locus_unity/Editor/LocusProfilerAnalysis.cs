using System;
using System.Collections.Generic;

namespace Locus
{
    /// <summary>Statistics over finite observations, using nearest-rank percentiles.</summary>
    public sealed class ProfilerStatistics
    {
        public int SampleCount;
        public int MissingCount;
        public double? Min, Max, Average, Median, P90, P95, P99, StandardDeviation, First, Last, Delta;

        public static ProfilerStatistics Calculate(IEnumerable<double> observations)
        {
            if (observations == null) throw new ArgumentNullException(nameof(observations));
            var values = new List<double>();
            var result = new ProfilerStatistics();
            double mean = 0, m2 = 0;
            foreach (double value in observations)
            {
                if (double.IsNaN(value) || double.IsInfinity(value)) { result.MissingCount++; continue; }
                if (values.Count == 0) result.First = value;
                result.Last = value;
                values.Add(value);
                double delta = value - mean;
                mean += delta / values.Count;
                m2 += delta * (value - mean);
            }
            result.SampleCount = values.Count;
            if (values.Count == 0) return result;
            values.Sort();
            result.Min = values[0]; result.Max = values[values.Count - 1];
            result.Average = mean;
            result.StandardDeviation = Math.Sqrt(Math.Max(0, m2 / values.Count));
            result.Median = Rank(values, 0.5); result.P90 = Rank(values, 0.90);
            result.P95 = Rank(values, 0.95); result.P99 = Rank(values, 0.99);
            result.Delta = result.Last - result.First;
            return result;
        }

        private static double Rank(List<double> sorted, double percentile)
        {
            return sorted[Math.Max(0, (int)Math.Ceiling(sorted.Count * percentile) - 1)];
        }
    }
}
