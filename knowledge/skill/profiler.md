---
id: kd_skill_builtin_profiler
injectMode: excerpt
summary: >-
  Profile live Unity Editor / Play Mode CPU, GPU, memory, GC, rendering, physics, audio, animation, UI, loading, 2D and custom metrics. Discover counters, capture samples, compare baselines, inspect hierarchies and save memory snapshots through ctx.Profiler in unity_execute or unity_run_states. Not for static code review.
aiMaintained: false
skillEnabled: true
skillSurface: both
commandTrigger: /profile
tools:
  - unity_execute
  - unity_run_states
---

# Unity Profiler Runtime Sampling

Use for slow frames, stutters, allocations, memory growth, rendering load, simulation cost, or performance before/after an interaction. Prefer state inspection for deterministic logic bugs and static inspection for questions that need no measurements.

## Workflow

1. Reproduce the relevant scene, interaction and load. Prefer representative Play Mode conditions.
2. Discover metrics if their names/categories are uncertain. Capture a small overview first, then narrow to the relevant subsystem.
3. Allow warmup. Collect 120–600 observations or a bounded time window. Keep baseline/candidate conditions and sampling policies comparable.
4. Check availability and missing counts before interpreting statistics. Use frame exports for call attribution and snapshots for memory ownership.
5. Print a short finding with evidence, limitations and saved paths. Save raw samples in CSV and hierarchies in JSON.

All helpers are available through **`ctx.Profiler` in both `unity_execute` and `unity_run_states`**. Captures belong to the current call and are cleaned up on success, failure or cancellation. Save before returning. Use `readonly: false` when starting captures or writing artifacts; discovery and reading buffered frames can be read-only. Standalone/remote Player capture is outside this helper's scope.

## unity_execute

For a bounded observation window, use `request_editor_status: "playing"`:

```csharp
var p = ctx.Profiler;
p.StartProfiler("baseline", p.ProfilerMetrics("overview", "memory"),
    new Locus.LocusBridge.ProfilerCaptureOptions { WarmupFrames = 30, MaxSamples = 300 });
await ctx.WaitSeconds(8);
p.StopProfiler("baseline");
p.PrintProfilerSummary("baseline");
printJson(p.GetProfilerSummary("baseline", "main_thread_ms").Statistics);
printJson(p.GetProfilerBudget("baseline", "main_thread_ms", 16.67));
p.SaveProfiler("baseline");
```

The duration does not promise 300 rendered frames; inspect sample counts. To wait for the sample cap, use `await ctx.WaitUntil(() => p.IsProfilerStopped("baseline"))` while the scene keeps rendering. Stop explicitly if the scenario pauses or finishes earlier.

## unity_run_states

```csharp
// start
ctx.Profiler.StartProfiler("baseline", ctx.Profiler.ProfilerMetrics("overview"),
    new Locus.LocusBridge.ProfilerCaptureOptions { WarmupFrames = 30, MaxSamples = 300 });

// update
if (!ctx.Profiler.IsProfilerStopped("baseline")) return;
ctx.Profiler.PrintProfilerSummary("baseline");
ctx.Profiler.SaveProfiler("baseline");
ctx.Done("profile captured");
```

Sampling continues while a state sleeps. Existing `ctx.StartProfiler`, `StopProfiler`, `PrintProfilerSummary`, `SaveProfiler`, `GetProfilerSummary`, last-value, spike and frame helpers remain supported. Legacy `ctx.StartProfiler` enables hierarchy capture; prefer `ctx.Profiler.StartProfiler` when only counters are needed.

## Metric discovery and domains

```csharp
var p = ctx.Profiler;
var page = p.DiscoverMetrics(nameContains: "Texture", categoryContains: "Memory", limit: 40, offset: 0);
printJson(page);
// Items have Name, Category, Unit, DataType and Flags; paginate using TotalMatches/HasMore.
// Use a discovered item in this call: p.ProfilerMetric(page.Items[0], "texture_metric").
```

Filters are case-insensitive substrings. Limit is 1–512. `ProfilerMetric(info, name, options)` converts nanoseconds to ms and preserves other native units. Explicit definitions use `ProfilerMetric(name, category, markerName, scale, unit, options)`.

`ProfilerMetrics(params string[] groups)` composes these domains:

| Group | Data |
| --- | --- |
| `overview` | CPU threads, allocation, reserved/system memory, rendering and GPU frame time |
| `cpu`, `gpu` | Main/render thread time; GPU frame time where supported |
| `memory` | Used/reserved/managed/system memory, allocation, textures, meshes, graphics memory |
| `rendering` | Draw calls, batches, SetPass, triangles, vertices, render textures |
| `physics`, `physics2d` | Registered metrics in the corresponding physics categories |
| `audio`, `animation`, `ui`, `loading` | Registered subsystem metrics |
| `2d` | Registered 2D/sprite metrics, including those added by newer Unity versions |

The last three rows use runtime discovery. An empty domain or one with more than 64 metrics asks you to narrow discovery instead of silently substituting data. Registration depends on Unity version, packages and whether the subsystem has run. At most 128 metrics and 1,000,000 stored values are allowed per capture.

`GC Allocated In Frame` is the allocation counter for a frame. `GC.Alloc` is useful for allocation attribution; do not assume a marker's duration is a byte count. Verify discovered units before selecting a scale.

For a custom marker and main-thread-only collection:

```csharp
var metric = ctx.Profiler.ProfilerMetric("combat_ms", Unity.Profiling.ProfilerCategory.Scripts,
    "Combat.SpawnWave", 0.000001, "ms", Unity.Profiling.ProfilerRecorderOptions.Default
    | Unity.Profiling.ProfilerRecorderOptions.CollectOnlyOnCurrentThread);
```

Defaults sum samples across threads in a frame. `GpuRecorder` requests GPU timing for an actual GPU-capable marker; it does not turn C# work into GPU work. Keep default sum/wrap flags unless individual-sample semantics are intended. For project state, use `ProfilerMetric("enemy_count", () => (double?)enemyManager.ActiveCount, "count")`. A reader returns `null` for missing data and should avoid expensive enumeration, allocations or logs on the measured main thread. Add stable project `ProfilerMarker` scopes only when built-in attribution is too coarse.

## Capture policy and analysis

`ProfilerCaptureOptions` fields:

| Field | Default | Meaning |
| --- | --- | --- |
| `WarmupFrames` | `0` | Skip initial observed frames/ticks |
| `SampleEveryFrames` | `1` | Sample latest values every N observed frames/ticks; not an N-frame sum |
| `MaxSamples` | `3600` | Stop at this row count; range 1–60000 |
| `Clock` | `"auto"` | Distinct `Time.frameCount` in Play Mode, editor ticks in Edit Mode; explicit `"unity_frame"` / `"editor_update"` also supported |
| `CaptureHierarchy` | `false` | Temporarily enable Profiler recording/CPU data for frame export |
| `CaptureGpu` | `false` | Also request GPU Profiler data, subject to hardware/API support |

The API observes latest recorder values; it cannot reconstruct frames missed between editor callbacks. Edit Mode ticks can read the same completed value again. Recorder buffers are not reset every tick: Unity `Reset()` stops collection. No completed data is missing, not zero. The row cap still applies if metrics are missing. Non-positive GPU frame timing is treated as unavailable.

Methods on `ctx.Profiler`:

- `GetProfilerSummary(name, metric)` retains `SampleCount`, `Average`, `P95`, `Max`, `Last`, `Available`, `Error`. `Statistics` adds missing count, min, median, p90, p99, population standard deviation, first and last values, and `Delta = last - first`.
- `GetProfilerSamples(name, metric, offset, count)` returns nullable observations with frame/timing metadata; default count 600, cap 60000.
- `GetProfilerBudget(name, metric, threshold)` reports observations strictly above the budget and their percentage among valid samples.
- `CompareProfilers(baseline, candidate, metric)` reports candidate minus baseline average/p95/max and percentage changes. Incompatible definitions or missing data produce errors; zero baselines have no percentage change.
- `IsProfilerStopped`, `StopProfiler`, `TryGetProfilerLastValue(name, metric, out value)` support scenario control.

Percentiles use nearest rank and exclude missing/non-finite observations. No-data statistics are `null` in JSON. Memory `Delta` is net change, not proof of a leak. CPU waits may be pacing, synchronization or GPU backpressure; do not add overlapping main/render/job durations or equate a rendering wait with GPU duration.

## Spikes and frame hierarchy

Start with `CaptureHierarchy = true`. Between awaits or in state updates:

```csharp
var p = ctx.Profiler;
if (p.RecordProfilerSpikeTop("baseline", "main_thread_ms", 30, "slow", 5)) {
    int frame = p.GetProfilerLastSpikeFrame("baseline", "main_thread_ms");
    if (frame >= 0) p.SaveProfilerFrame("slow_" + frame, frame,
        new Locus.LocusBridge.ProfilerFrameOptions {
            ThreadName = "Main Thread", SortBy = "self_ms", TopCount = 80
        }, inlineRows: 0);
}
```

Spikes retain observation metadata; repeated checks do not duplicate the observation. Top-N retains the highest values for each metric/label. `GetProfilerSpikes` returns retained records. The legacy spelling `ctx.RecordProfilerSpikeTop` remains available.

`GetProfilerThreads(frame)` lists names/groups/indices/IDs. `GetProfilerFrame(frame, options)` returns CPU rows in memory. Options select `ThreadName` or concrete `ThreadIndex`, `SortBy` (`total_ms`, `self_ms`, `gc_bytes`, `calls`), `NameContains` (path substring), and `TopCount` (0–512). Missing threads produce errors, not another thread's rows. Frame metadata includes GPU duration when available; row durations remain **CPU** durations.

Legacy exports remain supported: `ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount)` and `ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount, inlineRows)`. Overloads without the index use the latest buffered frame.

Unity frames, tool ticks and Profiler indices are different clocks. A sampled row records the latest available Profiler frame as a **candidate**, not guaranteed exact attribution; GPU data can arrive later. Inspect adjacent frames and verify the marker before attributing a spike. Buffers roll over: save promptly. Use the local Editor target; switching targets invalidates frame association. Do not sum hierarchy total-time rows because parent values include children.

## GPU timing and memory snapshots

Combine render counters, the GPU frame counter, GPU-capable marker recorders and frame metadata. Use the graphics debugger skill for render-pass/resource/overdraw investigation beyond timing counters.

For CPU/GPU/present-wait records and dynamic resolution scale:

```csharp
if (ctx.Profiler.CaptureFrameTimings()) {
    await ctx.WaitFrames(8);
    printJson(ctx.Profiler.GetFrameTimings(4));
} else print("Frame Timing Stats is disabled or unsupported.");
```

Frame Timing Manager requires platform support and Frame Timing Stats configuration. The helper does not change Player settings. Records preserve timestamps for deduplication. GPU data may remain missing/delayed when CPU data is available.

For retained/native/managed memory investigation, take snapshots separately from timing measurements:

```csharp
string snapshot = await ctx.Profiler.SaveMemorySnapshotAsync("after_load", cancellationToken);
print(snapshot);
```

This saves `.snap` for Unity Memory Profiler. Select the local Editor target in the Profiler; the helper rejects a non-Editor target instead of taking a Player snapshot. Use `includeNativeAllocations: true` for allocation detail. Snapshot capture is intrusive; stop timing capture first. Cancellation ends the wait, but Unity's native operation may still finish writing. Snapshot requests are serialized. The helper does not automatically analyze retained-object graphs.

## Saved data

`SaveProfiler(name)` stops the capture, saves full-precision `locus.profiler.samples_csv.v1` CSV and `locus.profiler.summary.v1` JSON under `Library/Locus/RunStates`, prints `profiler_file` / `profiler_summary_file`, and returns the CSV path.

```csv
sample_index,session_frame,unity_time_frame_count,profiler_frame_index,elapsed_ms,main_thread_ms,gpu_frame_ms
0,31,546,190,517,11.2,
1,32,547,191,534,12.4,8.6
```

Empty cells mean missing, not zero. Summary JSON includes definitions, units, source/options, availability/statistics/spikes, Unity version, graphics API, Editor/Play Mode, sample policy and stop reason. `sample_rows` counts rows; spans are boundary distances and need not equal row count. Frame JSON retains `locus.profiler.frame_hierarchy.v1` with added filter/sort/GPU metadata.

## Unity 6.5

Unity 6.5 adds a 2D Profiler module for sprites/atlas usage, an experimental USS Stats Profiler, and Profiler AI integration. UI features do not imply public recorder APIs for every detail: discover exposed 2D metrics and use Unity's specific tools for atlas/USS detail absent from the catalog. Ordinary counter sampling does not require Unity Assistant or the Memory Profiler package.

Sources: [Unity 6.5 changes](https://docs.unity3d.com/6000.5/Documentation/Manual/WhatsNewUnity65.html), [ProfilerRecorder](https://docs.unity3d.com/6000.5/Documentation/ScriptReference/Unity.Profiling.ProfilerRecorder.html), [metric discovery](https://docs.unity3d.com/6000.5/Documentation/ScriptReference/Unity.Profiling.LowLevel.Unsafe.ProfilerRecorderHandle.GetAvailable.html), [Frame Timing Manager](https://docs.unity3d.com/6000.5/Documentation/ScriptReference/FrameTimingManager.html).
