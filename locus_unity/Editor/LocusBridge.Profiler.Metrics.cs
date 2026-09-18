using System;
using System.Collections.Generic;
using System.Linq;
using Unity.Profiling;
using Unity.Profiling.LowLevel.Unsafe;

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed class ProfilerMetricInfo
        {
            public readonly string Name, Category, Unit, DataType, Flags;
            internal readonly ProfilerCategory RecorderCategory;
            internal ProfilerMetricInfo(ProfilerRecorderDescription description)
            {
                Name = description.Name; Category = description.Category.Name; Unit = description.UnitType.ToString();
                DataType = description.DataType.ToString(); Flags = description.Flags.ToString(); RecorderCategory = description.Category;
            }
        }

        public sealed class ProfilerMetricCatalog
        {
            public ProfilerMetricInfo[] Items;
            public int TotalMatches, Offset;
            public bool HasMore;
        }

        public sealed partial class ProfilerApi
        {
            /// <summary>Discover registered metrics. Name and category filters are case-insensitive substrings.</summary>
            public ProfilerMetricCatalog DiscoverMetrics(string nameContains = "", string categoryContains = "", int limit = 100, int offset = 0)
            {
                if (limit < 1 || limit > 512 || offset < 0) throw new ArgumentOutOfRangeException("limit/offset");
                var handles = new List<ProfilerRecorderHandle>();
                ProfilerRecorderHandle.GetAvailable(handles);
                var matches = new List<ProfilerMetricInfo>();
                foreach (var handle in handles)
                {
                    if (!handle.Valid) continue;
                    var d = ProfilerRecorderHandle.GetDescription(handle);
                    if (!Contains(d.Name, nameContains) || !Contains(d.Category.Name, categoryContains)) continue;
                    matches.Add(new ProfilerMetricInfo(d));
                }
                matches.Sort((a, b) => { int c = string.CompareOrdinal(a.Category, b.Category); return c != 0 ? c : string.CompareOrdinal(a.Name, b.Name); });
                return new ProfilerMetricCatalog { Items = matches.Skip(offset).Take(limit).ToArray(),
                    Offset = offset, TotalMatches = matches.Count, HasMore = (long)offset + limit < matches.Count };
            }

            public RuntimeProfilerMetric ProfilerMetric(ProfilerMetricInfo info, string name = null,
                ProfilerRecorderOptions options = ProfilerRecorderOptions.Default)
            {
                if (info == null) throw new ArgumentNullException(nameof(info));
                double scale = info.Unit == "TimeNanoseconds" ? 0.000001 : 1;
                string unit = info.Unit == "TimeNanoseconds" ? "ms" : info.Unit;
                return ProfilerMetric(name ?? (info.Category + "/" + info.Name), info.RecorderCategory, info.Name, scale, unit, options);
            }

            /// <summary>A lightweight observation of a project-specific numeric value. null means unavailable.</summary>
            public RuntimeProfilerMetric ProfilerMetric(string name, Func<double?> reader, string unit)
            {
                if (reader == null) throw new ArgumentNullException(nameof(reader));
                if (string.IsNullOrWhiteSpace(name)) throw new ArgumentException("Metric name is required.");
                return new RuntimeProfilerMetric(name.Trim(), ProfilerCategory.Scripts, name.Trim(), 1, unit ?? "",
                    ProfilerRecorderOptions.Default, reader);
            }

            /// <summary>Composable domains. Non-core domains are discovered from the current Unity/package runtime.</summary>
            public RuntimeProfilerMetric[] ProfilerMetrics(params string[] groups)
            {
                var result = new Dictionary<string, RuntimeProfilerMetric>(StringComparer.Ordinal);
                foreach (string group in groups ?? new[] { "overview" })
                {
                    if (group == null) throw new ArgumentException("Metric group is required.");
                    string key = group.ToLowerInvariant();
                    var metrics = new List<RuntimeProfilerMetric>();
                    switch (key)
                    {
                        case "overview": metrics.AddRange(DefaultProfilerMetrics()); metrics.AddRange(ProfilerMetrics("gpu")); break;
                        case "cpu":
                            metrics.Add(ProfilerMetric("main_thread_ms", ProfilerCategory.Internal, "Main Thread", 1e-6, "ms"));
                            metrics.Add(ProfilerMetric("render_thread_ms", ProfilerCategory.Internal, "Render Thread", 1e-6, "ms")); break;
                        case "gpu":
                            metrics.Add(ProfilerMetric("gpu_frame_ms", ProfilerCategory.Render, "GPU Frame Time", 1e-6, "ms",
                                ProfilerRecorderOptions.Default, zeroIsUnavailable: true)); break;
                        case "memory":
                            metrics.Add(ProfilerMetric("gc_alloc_bytes", ProfilerCategory.Memory, "GC Allocated In Frame", 1, "bytes"));
                            foreach (string marker in new[] { "Total Used Memory", "Total Reserved Memory", "System Used Memory", "GC Used Memory", "GC Reserved Memory", "Texture Memory", "Mesh Memory", "Gfx Used Memory" })
                                metrics.Add(ProfilerMetric("Memory/" + marker, ProfilerCategory.Memory, marker, 1, "bytes"));
                            break;
                        case "rendering":
                            foreach (string marker in new[] { "Draw Calls Count", "Batches Count", "SetPass Calls Count", "Triangles Count", "Vertices Count", "Render Textures Count", "Render Textures Bytes" })
                                metrics.Add(ProfilerMetric("Render/" + marker, ProfilerCategory.Render, marker, 1, marker.EndsWith("Bytes") ? "bytes" : "count"));
                            break;
                        case "physics": case "physics2d": case "audio": case "animation": case "ui": case "loading": case "2d":
                            // Version/package-dependent names must come from discovery, not guessed constants.
                            int offset = 0;
                            ProfilerMetricCatalog page;
                            do
                            {
                                page = DiscoverMetrics(limit: 512, offset: offset);
                                foreach (var info in page.Items)
                                    if (MatchesDomain(key, info)) metrics.Add(ProfilerMetric(info));
                                offset += page.Items.Length;
                            } while (page.HasMore);
                            if (metrics.Count == 0) throw new InvalidOperationException("No registered metrics for " + group + ". Exercise the subsystem and discover again.");
                            if (metrics.Count > 64) throw new InvalidOperationException("More than 64 metrics in " + group + "; use DiscoverMetrics with a narrower filter.");
                            break;
                        default: throw new ArgumentException("Unknown profiler domain: " + group);
                    }
                    foreach (var metric in metrics)
                    {
                        if (result.TryGetValue(metric.Name, out var previous)
                            && (previous.MarkerName != metric.MarkerName || previous.Unit != metric.Unit || previous.Scale != metric.Scale))
                            throw new ArgumentException("Conflicting metric name: " + metric.Name);
                        result[metric.Name] = metric;
                    }
                }
                return result.Values.ToArray();
            }

            private static bool Contains(string value, string filter)
            {
                return string.IsNullOrEmpty(filter) || (value ?? "").IndexOf(filter, StringComparison.OrdinalIgnoreCase) >= 0;
            }

            private static bool MatchesDomain(string domain, ProfilerMetricInfo info)
            {
                switch (domain)
                {
                    case "physics": return Contains(info.Category, "Physics") && !Contains(info.Category, "2D");
                    case "physics2d": return Contains(info.Category, "Physics") && Contains(info.Category, "2D");
                    case "2d": return Contains(info.Category, "2D") && !Contains(info.Category, "Physics") || Contains(info.Name, "Sprite");
                    case "ui": return Contains(info.Category, "UI") || Contains(info.Category, "GUI");
                    default: return Contains(info.Category, domain);
                }
            }
        }
    }
}
