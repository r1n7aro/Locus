---
id: kd_skill_builtin_debugger
injectMode: excerpt
summary: >-
  Use when a live Unity logic bug needs precise PlayerLoop positioning, a project-defined Tick System, cross-frame conditions, thread switching, cooperative pause, frame stepping, paused inspection, or native Windows thread stacks from an installed CDB or WinDbg debugger. Read this skill before writing advanced debugger code.
aiMaintained: false
skillEnabled: true
skillSurface: both
commandTrigger: /debug
tools:
  - unity_execute
  - unity_run_states
  - bash
---

# Unity PlayerLoop Debugger

This skill describes Locus's cooperative runtime debugger. It can list the effective `PlayerLoop`, await before or after any anchorable node, pause when a condition matches, inspect the paused game in a later `unity_execute`, step one frame, and resume. It works through the existing `unity_execute` and `unity_run_states` tools. When a Windows-native thread stack is required, use `bash` to discover and invoke an available CDB or WinDbg command-line debugger.

## Debugging model

- PlayerLoop breakpoints are cooperative conditions evaluated on Unity's main thread at a selected loop boundary.
- When `await ctx.BreakWhen(...)` matches, Locus requests `EditorApplication.isPaused = true`, confirms the paused state on an Editor update, returns `status: breakpoint`, and ends that `unity_execute` invocation. Statements after `BreakWhen` in the same invocation do not run.
- Inspect, step, and resume with later independent `unity_execute` calls. Use `request_editor_status: "playing_paused"` while inspecting or stepping, and `request_editor_status: "playing"` when the call must begin with a running game.
- This protocol targets runtime state and PlayerLoop positions. Managed source-line breakpoints, call stacks, local-variable scopes, and arbitrary instruction stepping require an external Mono/DAP debugger.

## Native Windows thread stacks

Escalate to a native debugger when cooperative inspection cannot explain a Unity main-thread hang, render-thread or worker-thread wait, native plug-in failure, access violation, or engine-level deadlock.

- Confirm that the host is Windows and resolve the Unity process from the current project. Verify the PID still belongs to that project immediately before attaching; never select a process by executable name alone.
- Read the injected `windows-native-debuggers` runtime context first. It reports the `cdb`, `windbg`, and `windbgx` executables Locus found in PATH, Windows Kits, or WinDbg app execution aliases. Refresh after `refreshAfterSeconds`, treat `signatureStatus: not_checked` as unverified provenance, and revalidate the selected executable with `bash` immediately before use.
- When the runtime context is absent or stale, use `bash` to probe `cdb`, `windbg`, and `windbgx`. Never assume an executable path, version, architecture, symbol path, or installation state, and never embed machine-specific discovery results in this Skill.
- Prefer CDB for bounded, non-interactive stack capture because its command output can be returned to the Agent. Use WinDbg when an available installation supports the required command-line or interactive workflow.
- Use a non-invasive attach for inspection. For CDB, keep the target alive with `-pd`, use `-pv` when a coherent snapshot is required, finish the command script with `qd`, and apply a bounded timeout. Tell the user that `-pv` briefly suspends the target threads while the snapshot is collected.
- Capture only the threads and frame depth needed to answer the question. Treat ordinary debugger output as a native stack; Mono-managed frames may still require a Mono/DAP debugger or mixed-stack resolver.
- If none of the supported debuggers is available and native stacks are necessary, explain the diagnostic need and recommend installing Microsoft WinDbg or Debugging Tools for Windows. Leave installation to the user and continue only after their direction.

Revalidate PATH commands without assuming a local path:

```powershell
Get-Command cdb, windbg, windbgx -ErrorAction SilentlyContinue
```

Build the debugger invocation from the executable returned by that probe and the verified Unity PID. Keep every debugger command read-only and always detach while leaving Unity running.

## Thread APIs

Async `unity_execute` starts on Unity's main thread.

```csharp
printJson(ctx.Thread);                 // current UnityThreadInfo
print(ctx.IsMainThread);

await ctx.SwitchToThreadPool();
// Pure CPU or non-Unity work can run here.
printJson(ctx.GetCurrentThread());

await ctx.SwitchToMainThread();
// UnityEngine and UnityEditor APIs are safe here.
```

`UnityThreadInfo` includes `ManagedThreadId`, `Name`, `IsMainThread`, `IsThreadPoolThread`, and `SynchronizationContext`. Treat Unity objects and editor APIs as main-thread-only unless their own documentation explicitly says otherwise.

## List and select Tick Systems

Call the list APIs on the main thread:

```csharp
var snapshot = ctx.ListTickSystems();
print($"fingerprint={snapshot.Fingerprint} nodes={snapshot.Count}");
foreach (var node in snapshot.Nodes)
    print($"{node.Id} can_anchor={node.CanAnchor} method={node.ManagedMethod}");
```

Each `UnityTickSystemInfo` includes a stable path-like `Id`, type and assembly names, hierarchy depth, sibling index, managed/native callback metadata, `CanAnchor`, and the snapshot fingerprint. Duplicate node types receive occurrence indices in their IDs.

Use exact type matching when the project has custom systems:

```csharp
var all = ctx.FindTickSystems("Game.Runtime.CombatTickSystem");
var combat = ctx.FindTickSystem("Game.Runtime.CombatTickSystem", occurrence: 0);
```

A listed node carries its snapshot fingerprint. Waiting with that object fails clearly when another package replaces the PlayerLoop before the wait is registered. List again and resolve a fresh node after such a change.

## Await precise loop positions

Built-in points:

```csharp
await ctx.Next(UnityLoopPoint.BeforeFixedUpdate);
await ctx.Next(UnityLoopPoint.AfterFixedUpdate);
await ctx.Next(UnityLoopPoint.BeforeUpdate);
await ctx.Next(UnityLoopPoint.AfterUpdate);
await ctx.Next(UnityLoopPoint.BeforeLateUpdate);
await ctx.Next(UnityLoopPoint.AfterLateUpdate);
await ctx.Next(UnityLoopPoint.EndOfFrame);
```

Dynamic nodes:

```csharp
var tick = ctx.FindTickSystem("Game.Runtime.CombatTickSystem");
UnityTickStamp before = await ctx.WaitBefore(tick);
UnityTickStamp after = await ctx.WaitAfter(tick);
UnityTickStamp exact = await ctx.WaitAt(tick, UnityTickBoundary.After);
printJson(exact);
```

`UnityTickStamp` reports node ID/type, boundary, anchor sequence, `Time.frameCount`, rendered frame count, editor time, and managed thread ID.

`UnityLoopPoint.EditorUpdate`, `ctx.wait`, `WaitFrame`, `WaitFrames`, `WaitSeconds`, and the original `WaitUntil` use `EditorApplication.update`. They continue to work in Edit Mode and paused Play Mode. PlayerLoop node waits require running, unpaused Play Mode.

## Wait for a condition

Supply a human-readable `condition` so an asynchronous task-status query can explain the wait:

```csharp
var tick = ctx.FindTickSystem("Game.Runtime.CombatTickSystem");
await ctx.WaitUntil(
    tick,
    UnityTickBoundary.After,
    () => player.Health <= 20,
    condition: "player.Health <= 20");
```

For Editor-update polling:

```csharp
await ctx.WaitUntil(
    () => EditorApplication.isCompiling == false,
    condition: "EditorApplication.isCompiling == false");
```

While the call is awaiting, the existing asynchronous task status reports:

- wait kind and target;
- caller source line and source text;
- condition text;
- elapsed wait time.

The line number is captured from the generated snippet's `#line 1` mapping. `ctx.wait` remains a compact compatibility property and may report line `0`; prefer `ctx.WaitFrame()` when diagnostic line reporting matters. When `condition` is omitted, Locus reports the generated predicate method name.

## Break, inspect, step, resume

Set a cooperative breakpoint:

```csharp
var player = UnityEngine.Object.FindObjectOfType<PlayerController>();
await ctx.BreakWhen(
    UnityLoopPoint.AfterUpdate,
    () => player.Health <= 0,
    label: "player-death",
    condition: "player.Health <= 0");

// This line does not run after the breakpoint matches.
print("unreachable after breakpoint");
```

The completed result includes the label, tick node/type/boundary, hit frame, sequence, hit thread, and pause-confirmation frame/time.

Inspect with a new paused call:

```csharp
var player = UnityEngine.Object.FindObjectOfType<PlayerController>();
printJson(new {
    frame = Time.frameCount,
    thread = ctx.Thread,
    health = player != null ? player.Health : -1,
    position = player != null ? player.transform.position : Vector3.zero
});
```

Step exactly one Unity frame while remaining paused:

```csharp
UnityTickStamp observed = await ctx.StepFrame();
printJson(observed);
```

Resume Play Mode:

```csharp
await ctx.ResumeGame();
printJson(ctx.Thread);
```

`BreakWhen`, `StepFrame`, and `ResumeGame` are recognized as editor-status-changing behavior and use the existing status-change permission flow.

## `unity_run_states` Tick scheduling

`RuntimeCtx` exposes the same PlayerLoop listing metadata and current-thread information. State handlers are synchronous and remain on Unity's main thread.

Configure all later state-machine ticks at a built-in point:

```csharp
// state start
ctx.SetTickPoint(UnityLoopPoint.AfterUpdate);
```

Configure a project Tick System:

```csharp
// state start
var combat = ctx.FindTickSystem("Game.Runtime.CombatTickSystem");
ctx.SetTickSystem(combat, UnityTickBoundary.After);
```

Return to the default editor-update clock with `ctx.UseEditorUpdate()`. `ctx.Sleep(frames)` counts future ticks of the currently selected clock.

## Concurrency and cancellation

- Locus serializes request preparation and snippet compilation, then releases the project operation lock before awaiting Unity frames or conditions.
- Concurrent async `unity_execute` calls use independent execution IDs, progress snapshots, heartbeats, cancellation tokens, and result channels.
- Cancelling an async task targets its execution ID. It does not cancel unrelated `unity_execute` calls.
- Breakpoint completion releases the original request; paused inspection and recovery never depend on keeping that request alive.

## Safe usage rules

- Switch back to the main thread before touching Unity objects or editor APIs.
- Keep breakpoint predicates fast, deterministic, allocation-light, and side-effect-free.
- Resolve scene objects before a long wait only when their lifetime is guaranteed; otherwise re-resolve inside a safe predicate and handle destroyed Unity objects.
- Use explicit `condition` and `label` strings. They make pending task output and breakpoint results understandable to another model turn.
- Use `unity_run_states` for long structured observation; use `unity_execute` for precise awaits, cooperative breaks, paused inspection, stepping, and recovery.
