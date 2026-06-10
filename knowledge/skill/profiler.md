---
id: kd_skill_builtin_profiler
type: skill
path: profiler.md
title: Unity Profiler Runtime Sampling
injectMode: none
summaryEnabled: true
commandEnabled: false
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: auto
commandTrigger:
argumentHint:
tools:
  - unity_run_states
createdAt: 1777332556825
updatedAt: 1781049600000
---

# Unity Profiler Runtime Sampling

## Summary
Use when a runtime debugging task needs Unity performance data: frame time, GC allocation, memory, rendering, physics, script cost, spike frames, or a specific Unity Profiler marker. Ignore static code-quality questions that need no live runtime data.

## Content
Use this skill when a runtime debugging task asks for frame time, GC allocation, memory, rendering, physics, script update cost, spike analysis, or a specific Unity Profiler marker.

## When to use

- The user asks why Play Mode is slow, stutters, spikes, allocates memory, or drops FPS.
- The task needs runtime profile data around a specific interaction, scene state, user action, or state transition.
- The agent needs to compare baseline performance before and after an operation.
- The agent needs a specific marker, counter, frame hierarchy, or spike-frame breakdown.

## When NOT to use

- The question is about static code quality and does not need live runtime data.
- The problem is a deterministic logic bug that can be diagnosed from state inspection alone.
- The task only needs file-level asset, scene, or prefab inspection.

## Core model

Unity Profiler data is organized around markers and counters.

- A marker is a named sample scope. Built-in examples include `PlayerLoop`, `BehaviourUpdate`, `Update.ScriptRunBehaviourUpdate`, `GC.Alloc`, and `Physics.Processing`.
- A counter is a named numeric value. Common examples include memory usage, object count, draw calls, and triangle count.
- CPU markers form a hierarchy. `Total` time includes children; `Self` time excludes children.
- Business C# methods do not automatically appear as useful named samples unless Unity already marks them, Deep Profile is active, or the project adds custom `ProfilerMarker` / `Profiler.BeginSample` scopes.

Use `ProfilerRecorder` style sampling for trends and known metrics. Use frame hierarchy export when the user needs one frame's function or marker time breakdown.

## Runtime workflow

1. Choose the runtime window to measure.
   - Wait for the scene, object, or user action that matters.
   - Start recording immediately before the suspicious behavior.
   - Stop recording after enough frames to include steady-state or the spike.

2. Record default metrics first.
   - Main thread time.
   - Render thread time when available.
   - GC allocated in frame.
   - GC reserved memory.
   - System or total used memory.
   - Rendering counters when the issue is visual load.
   - Physics counters when the issue is simulation load.

3. Watch for spike frames.
   - Keep a threshold such as `main_thread_ms > 30`.
   - Use `ctx.RecordProfilerSpike(...)` after the profiler has at least one sample.
   - Read the saved Unity Profiler frame index with `ctx.GetProfilerLastSpikeFrame(...)`.
   - Export hierarchy rows for that frame with `ctx.SaveProfilerFrame(...)`.

4. Narrow to specific markers.
   - Enumerate available marker/counter names when the exact name is unknown.
   - Record the specific marker with its category and unit conversion.
   - Prefer stable custom marker names for gameplay systems.

5. Return a short summary and save full data.
   - Print averages, p95, max, last, sample count, and frame range.
   - Save raw samples, spike records, and frame hierarchy data under `Library/Locus/RunStates`.
   - Include saved file paths in the `unity_run_states` result.

## Expected ctx profiler helpers

Start the profiler in the state's `start` snippet and stop it in `update`:

```csharp
// start
ctx.StartProfiler("baseline", ctx.DefaultProfilerMetrics());

// update
if (ctx.ElapsedFramesInState < 300) return;

ctx.StopProfiler("baseline");
ctx.PrintProfilerSummary("baseline");
ctx.SaveProfiler("baseline");
ctx.Done("profile captured");
```

Use explicit metrics in `start` when the task needs a targeted marker:

```csharp
ctx.StartProfiler("gc_spike", new[] {
    ctx.ProfilerMetric("gc_alloc", Unity.Profiling.ProfilerCategory.Memory, "GC.Alloc", 1, "bytes"),
    ctx.ProfilerMetric("gc_reserved_mb", Unity.Profiling.ProfilerCategory.Memory, "GC Reserved Memory", 0.000001, "MB"),
    ctx.ProfilerMetric("main_thread_ms", Unity.Profiling.ProfilerCategory.Internal, "Main Thread", 0.000001, "ms"),
});
```

Use last-value reads and spike records after the profiler has sampled at least one frame:

```csharp
double mainThreadMs;
if (ctx.TryGetProfilerLastValue("baseline", "main_thread_ms", out mainThreadMs)
    && ctx.RecordProfilerSpikeTop("baseline", "main_thread_ms", 30.0, "main_thread_spike", 5))
{
    int profilerFrame = ctx.GetProfilerLastSpikeFrame("baseline", "main_thread_ms");
    ctx.SaveProfilerFrame("main_thread_spike_" + profilerFrame, profilerFrame, "Main Thread", 80, 0);
}
```

`RecordProfilerSpikeTop` keeps the highest spike records for a metric/label pair and returns `true` when a new saved record was added or replaced. Use the final JSON files for full rows; use `inlineRows=0` or a small value when saving many spike frames.

Available helper surface:

- `ctx.StartProfiler(name)` and `ctx.StartProfiler(name, metrics)`.
- `ctx.StopProfiler(name)`.
- `ctx.PrintProfilerSummary(name)`.
- `ctx.SaveProfiler(name)`.
- `ctx.TryGetProfilerLastValue(profilerName, metricName, out value)`.
- `ctx.GetProfilerLastValue(profilerName, metricName)`.
- `ctx.GetProfilerSummary(profilerName, metricName)`.
- `ctx.RecordProfilerSpike(profilerName, metricName, threshold, label)`.
- `ctx.RecordProfilerSpikeTop(profilerName, metricName, threshold, label, maxSpikes)` to keep only the strongest records for that metric and label.
- `ctx.GetProfilerLastSpikeFrame(profilerName, metricName)`.
- `ctx.GetProfilerSpikes(profilerName)`.
- `ctx.LatestProfilerFrameIndex()`.
- `ctx.SaveProfilerFrame(name, threadName, topCount)` for the latest profiler frame.
- `ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount)` for a specific profiler frame.
- `ctx.SaveProfilerFrame(name, threadName, topCount, inlineRows)` and `ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount, inlineRows)` to separate saved hierarchy rows from printed rows.

If a metric is unavailable in the current Unity version or scene state, the profiler summary will mark that metric unavailable. Use direct Unity profiling APIs in `unity_run_states` or `unity_execute` only when the helper output is too coarse.

## Overall profile data

For an initial profile pass, capture a compact default set:

- `Main Thread` time in ms.
- `Render Thread` time in ms when available.
- `GC.Alloc` bytes per frame.
- `GC Reserved Memory` in MB.
- `System Used Memory` or total used memory in MB.
- `Batches Count`, `SetPass Calls Count`, `Triangles Count`, and `Vertices Count` for rendering issues.
- Physics processing markers for simulation issues.

Prefer recording 120 to 600 frames depending on the symptom. Short bursts are useful for spike capture; longer windows are useful for p95 and trend analysis.

## Specific marker data

To capture a specific marker:

1. Find the exact marker or counter name.
2. Confirm the category when possible.
3. Start a recording immediately before the relevant behavior.
4. Stop after the behavior ends.
5. Print summary and save raw samples.

Examples of useful marker groups:

- Script update: `BehaviourUpdate`, `Update.ScriptRunBehaviourUpdate`, `PreLateUpdate.ScriptRunBehaviourLateUpdate`.
- GC: `GC.Alloc`, `GC.Collect`, GC reserved or used memory counters.
- Rendering waits: `WaitForTargetFPS`, `Gfx.WaitForPresentOnGfxThread`.
- Physics: `Physics.Processing`, `Physics.Simulate`.
- UI: Canvas rebuild, layout, and render markers when available.

Marker names can vary by Unity version, render pipeline, package, and whether the marker has appeared in the current session. Enumerate available metrics when a name fails.

## One-frame hierarchy data

Use one-frame hierarchy export when the user asks for the kind of data shown in Unity Profiler's Hierarchy table:

- Total time.
- Self time.
- Calls.
- GC allocation.
- Percent of the selected frame.
- Thread name.
- Parent/child depth.

This is the right path for questions like "which function took most of frame 191" or "what occupied the spike frame".

`SaveProfilerFrame` reads Unity `HierarchyFrameDataView` and saves sorted top rows with depth and path. `topCount` controls saved JSON rows and is capped at 512. `inlineRows` controls printed rows only and defaults to 8. `Time.frameCount`, the `unity_run_states` session frame, and the Unity Profiler frame index are separate values. The helpers store all three where available so saved samples, spike records, and hierarchy exports can be matched.

## When to modify project C# scripts

Modify project C# scripts only when the profiler data is too coarse to identify a business-system cause.

Add custom markers when:

- The hot row is only `BehaviourUpdate` or `ScriptRunBehaviourUpdate`.
- Several systems run inside the same `Update`, coroutine, async continuation, or callback.
- The analysis needs business context such as wave id, inventory item count, enemy count, scene phase, or asset id.
- The same performance question is likely to be repeated.

Use stable marker names:

```csharp
using Unity.Profiling;

static readonly ProfilerMarker SpawnWaveMarker =
    new ProfilerMarker("Combat.SpawnWave");

void SpawnWave()
{
    using (SpawnWaveMarker.Auto())
    {
        // spawn logic
    }
}
```

Prefer names like `Combat.SpawnWave`, `AI.Navigation.Tick`, `Inventory.Rebuild`, or `UI.Hud.Refresh`. Keep marker scopes around meaningful work, not around every tiny line.

## Output expectations

A profiler summary should be concise:

```text
profiler baseline
sample_rows=300 frame_span=299 unity_frame_span=299 duration_ms=5120
main_thread_ms samples=300 avg=12.4 p95=18.9 max=31.6 last=11.8 unit=ms
gc_alloc_bytes samples=300 avg=384 p95=2048 max=8192 last=0 unit=bytes
spikes=1
profiler_file: F:\Project\Library\Locus\RunStates\profiler-baseline.csv
profiler_summary_file: F:\Project\Library\Locus\RunStates\profiler-baseline-summary.json
```

`sample_rows` is the number of CSV sample rows. `frame_span` and `unity_frame_span` are the distance between the saved start and end frame numbers, so they can be one lower than `sample_rows` when sampling includes both boundary ticks.

A frame hierarchy export should be similarly compact:

```text
profiler_frame main_thread_spike
frame=191 session_frame=126 unity_frame=845 thread="Main Thread" thread_matched=true cpu_ms=33.42 rows=80 inline_rows=2
depth=1 name="PlayerLoop" total_ms=32.99 self_ms=0.05 calls=3 gc_bytes=2867 pct=98.7
depth=3 name="Update.ScriptRunBehaviourUpdate" total_ms=18.1 self_ms=0.2 calls=1 gc_bytes=2048 pct=54.2
rows_truncated=78
profiler_frame_file: F:\Project\Library\Locus\RunStates\profiler-frame-main-thread-spike.json
```

`SaveProfiler` writes per-frame samples as `locus.profiler.samples_csv.v1` CSV:

```csv
sample_index,session_frame,unity_time_frame_count,profiler_frame_index,elapsed_ms,main_thread_ms,gc_alloc_bytes
0,1,546,190,16,11.2,0
1,2,547,191,33,12.4,384
```

The CSV is the primary `profiler_file` output. Each row is one `unity_run_states` sampling tick. `unity_time_frame_count` and `profiler_frame_index` let readers map sampled metric rows back to Unity runtime frames and Profiler hierarchy frames. Metric units and availability are stored in the summary JSON.

`SaveProfiler` also writes `locus.profiler.summary.v1` JSON:

```json
{
  "schema": "locus.profiler.summary.v1",
  "name": "baseline",
  "start": { "session_frame": 1, "unity_time_frame_count": 546 },
  "end": { "session_frame": 300, "unity_time_frame_count": 845 },
  "duration_ms": 5120,
  "sample_policy": {
    "clock": "unity_run_states_tick",
    "sample_rows": 300,
    "session_frame_span": 299,
    "unity_frame_span": 299,
    "distinct_unity_frames": 300,
    "distinct_profiler_frames": 300
  },
  "samples_csv": {
    "schema": "locus.profiler.samples_csv.v1",
    "path": "F:\\Project\\Library\\Locus\\RunStates\\profiler-baseline.csv"
  },
  "metrics": [
    {
      "name": "main_thread_ms",
      "category": "Internal",
      "marker": "Main Thread",
      "scale": 0.000001,
      "unit": "ms",
      "available": true,
      "error": "",
      "summary": { "sample_count": 300, "avg": 12.4, "p95": 18.9, "max": 31.6, "last": 11.8 }
    }
  ],
  "spikes": [
    {
      "label": "main_thread_spike",
      "metric": "main_thread_ms",
      "threshold": 30,
      "value": 31.6,
      "session_frame": 126,
      "unity_time_frame_count": 671,
      "profiler_frame_index": 191
    }
  ]
}
```

`SaveProfilerFrame` writes `locus.profiler.frame_hierarchy.v1` JSON:

```json
{
  "schema": "locus.profiler.frame_hierarchy.v1",
  "name": "main_thread_spike",
  "frame": {
    "profiler_frame_index": 191,
    "session_frame": 126,
    "exported_at_unity_time_frame_count": 672,
    "frame_time_ms": 33.42,
    "frame_fps": 29.9
  },
  "thread": {
    "requested": "Main Thread",
    "name": "Main Thread",
    "group": "Main Thread",
    "index": 0,
    "id": 1,
    "matched": true
  },
  "top_count": 40,
  "sort": { "column": "total_ms", "descending": true },
  "error": "",
  "rows": [
    {
      "depth": 1,
      "name": "PlayerLoop",
      "path": "PlayerLoop",
      "total_ms": 32.99,
      "self_ms": 0.05,
      "total_pct": 98.7,
      "self_pct": 0.1,
      "calls": 3,
      "gc_bytes": 2867,
      "warning_count": 0
    }
  ]
}
```

Save per-frame sample data to CSV and hierarchy data to JSON when sample count or hierarchy depth is high.

## Pitfalls

- Deep Profile can significantly distort timings. Use it only for short, targeted diagnosis.
- Editor overhead appears in Play Mode profiling. Compare similar conditions and avoid over-reading single-frame noise.
- Some counters are unavailable until the marker appears or the relevant module is active.
- Unity Profiler keeps a rolling frame buffer. Save hierarchy JSON soon after a spike is detected, and keep `inlineRows` small to avoid oversized tool output.
- `LastValue` style sampling is good for trends; hierarchy export is better for a single frame's time distribution.
- GC allocation spikes need both size and call source. Use call stacks or custom markers when the allocation source matters.
