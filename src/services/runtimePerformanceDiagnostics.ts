const DEFAULT_STALL_THRESHOLD_MS = 250;
const DEFAULT_WATCHDOG_INTERVAL_MS = 250;
const DEFAULT_WARMUP_MS = 2_000;
const DEFAULT_COALESCE_DELAY_MS = 75;
const DEFAULT_JANK_WINDOW_MS = 2_000;
const DEFAULT_JANK_FRAME_THRESHOLD_MS = 50;
const DEFAULT_JANK_MIN_FRAME_COUNT = 5;
const DEFAULT_JANK_MIN_TOTAL_DURATION_MS = 300;
const INCIDENT_MERGE_GAP_MS = 250;
const MAX_REPORTABLE_STALL_MS = 30_000;
const MAX_SCRIPT_ATTRIBUTIONS = 8;
const MAX_JANK_FRAME_SAMPLES = 16;
const MAX_STRING_CHARS = 512;
const TRACE_MEASURE_NAME = "locus.runtime-stall";

type StallTrigger = "jank-burst" | "long-animation-frame" | "longtask" | "watchdog";

export interface RuntimePerformanceContext {
  [key: string]: unknown;
}

export interface RuntimePerformanceScriptAttribution {
  durationMs: number;
  executionStartMs: number | null;
  forcedStyleAndLayoutDurationMs: number;
  pauseDurationMs: number;
  invoker: string;
  invokerType: string;
  sourceUrl: string;
  sourceFunctionName: string;
  sourceCharPosition: number | null;
}

export interface RuntimeLongAnimationFrameSnapshot {
  startTimeMs: number;
  durationMs: number;
  blockingDurationMs: number;
  renderDurationMs: number;
  styleAndLayoutDurationMs: number;
  firstUiEventTimestampMs: number | null;
  scripts: RuntimePerformanceScriptAttribution[];
}

export interface RuntimeJankBurstSnapshot {
  windowStartTimeMs: number;
  windowEndTimeMs: number;
  windowDurationMs: number;
  frameCount: number;
  totalFrameDurationMs: number;
  totalBlockingDurationMs: number;
  maxFrameDurationMs: number;
  frames: Array<{
    startTimeMs: number;
    durationMs: number;
    blockingDurationMs: number;
  }>;
}

export interface RuntimePerformanceInteractionSnapshot {
  type: "pointer" | "keyboard";
  atMs: number;
  target: {
    tag: string;
    id: string;
    role: string;
    classes: string;
  } | null;
  key?: string;
  modifiers?: string[];
}

export interface RuntimePerformanceIncident {
  id: string;
  detectedAtMs: number;
  approximateStartedAtMs: number;
  performanceStartMs: number;
  durationMs: number;
  triggers: StallTrigger[];
  visibilityState: string;
  href: string;
  activeElement: RuntimePerformanceInteractionSnapshot["target"];
  lastInteraction: RuntimePerformanceInteractionSnapshot | null;
  watchdogDriftMs: number | null;
  longAnimationFrame: RuntimeLongAnimationFrameSnapshot | null;
  jankBurst: RuntimeJankBurstSnapshot | null;
  longTask: {
    startTimeMs: number;
    durationMs: number;
    name: string;
  } | null;
  document: {
    elementCount: number;
  };
  memory: {
    usedJsHeapBytes: number;
    totalJsHeapBytes: number;
    jsHeapLimitBytes: number;
  } | null;
  context: RuntimePerformanceContext;
}

export interface RuntimePerformanceDiagnosticsOptions {
  getContext?: () => RuntimePerformanceContext;
  reportIncident?: (incident: RuntimePerformanceIncident) => void;
  now?: () => number;
  stallThresholdMs?: number;
  watchdogIntervalMs?: number;
  warmupMs?: number;
  coalesceDelayMs?: number;
  jankWindowMs?: number;
  jankFrameThresholdMs?: number;
  jankMinFrameCount?: number;
  jankMinTotalDurationMs?: number;
}

interface LongAnimationFrameEntryLike extends PerformanceEntry {
  blockingDuration?: number;
  renderStart?: number;
  styleAndLayoutStart?: number;
  firstUIEventTimestamp?: number;
  scripts?: Array<{
    duration?: number;
    executionStart?: number;
    forcedStyleAndLayoutDuration?: number;
    pauseDuration?: number;
    invoker?: string;
    invokerType?: string;
    name?: string;
    sourceURL?: string;
    sourceFunctionName?: string;
    sourceCharPosition?: number;
  }>;
}

interface StallCandidate {
  trigger: StallTrigger;
  startTimeMs: number;
  durationMs: number;
  watchdogDriftMs?: number;
  longAnimationFrame?: RuntimeLongAnimationFrameSnapshot;
  jankBurst?: RuntimeJankBurstSnapshot;
  longTask?: RuntimePerformanceIncident["longTask"];
}

interface PendingIncident {
  startTimeMs: number;
  endTimeMs: number;
  triggers: Set<StallTrigger>;
  watchdogDriftMs: number | null;
  longAnimationFrame: RuntimeLongAnimationFrameSnapshot | null;
  jankBurst: RuntimeJankBurstSnapshot | null;
  longTask: RuntimePerformanceIncident["longTask"];
}

interface RunningDiagnostics {
  options: Required<Pick<
    RuntimePerformanceDiagnosticsOptions,
    | "now"
    | "stallThresholdMs"
    | "watchdogIntervalMs"
    | "warmupMs"
    | "coalesceDelayMs"
    | "jankWindowMs"
    | "jankFrameThresholdMs"
    | "jankMinFrameCount"
    | "jankMinTotalDurationMs"
  >> & Pick<RuntimePerformanceDiagnosticsOptions, "getContext" | "reportIncident">;
  observer: PerformanceObserver | null;
  observerEntryType: "long-animation-frame" | "longtask" | null;
  watchdogTimer: ReturnType<typeof setTimeout> | null;
  pendingTimer: ReturnType<typeof setTimeout> | null;
  pendingIncident: PendingIncident | null;
  warmupUntilMs: number;
  suppressUntilMs: number;
  jankFrames: RuntimeLongAnimationFrameSnapshot[];
  jankBurstReported: boolean;
  lastInteraction: RuntimePerformanceInteractionSnapshot | null;
  onPointerDown: (event: PointerEvent) => void;
  onKeyDown: (event: KeyboardEvent) => void;
}

let runningDiagnostics: RunningDiagnostics | null = null;
let nextIncidentSequence = 1;

function roundMs(value: number): number {
  return Math.round(value * 100) / 100;
}

function finiteNumber(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function optionalFiniteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function boundedString(value: unknown, maxChars = MAX_STRING_CHARS): string {
  const text = typeof value === "string" ? value : "";
  return text.length > maxChars ? `${text.slice(0, maxChars)} …(truncated)` : text;
}

function normalizedSourceUrl(value: unknown): string {
  const raw = boundedString(value);
  if (!raw) return "";
  try {
    const parsed = new URL(raw, window.location.href);
    parsed.search = "";
    parsed.hash = "";
    return boundedString(parsed.toString());
  } catch {
    return raw;
  }
}

function elementSnapshot(target: EventTarget | Element | null): RuntimePerformanceInteractionSnapshot["target"] {
  if (!(target instanceof Element)) return null;
  return {
    tag: target.tagName.toLocaleLowerCase(),
    id: boundedString(target.id, 120),
    role: boundedString(target.getAttribute("role"), 120),
    classes: boundedString(target.getAttribute("class"), 240),
  };
}

function keyboardLabel(event: KeyboardEvent): string {
  if (event.key.length === 1) return "printable";
  return boundedString(event.key, 80);
}

function keyboardModifiers(event: KeyboardEvent): string[] {
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("ctrl");
  if (event.altKey) modifiers.push("alt");
  if (event.shiftKey) modifiers.push("shift");
  if (event.metaKey) modifiers.push("meta");
  return modifiers;
}

export function snapshotLongAnimationFrame(
  entry: LongAnimationFrameEntryLike,
): RuntimeLongAnimationFrameSnapshot {
  const startTimeMs = finiteNumber(entry.startTime);
  const durationMs = Math.max(0, finiteNumber(entry.duration));
  const endTimeMs = startTimeMs + durationMs;
  const renderStartMs = finiteNumber(entry.renderStart);
  const styleAndLayoutStartMs = finiteNumber(entry.styleAndLayoutStart);
  const firstUiEventTimestampMs = optionalFiniteNumber(entry.firstUIEventTimestamp);
  const scripts = Array.isArray(entry.scripts) ? entry.scripts : [];

  return {
    startTimeMs: roundMs(startTimeMs),
    durationMs: roundMs(durationMs),
    blockingDurationMs: roundMs(Math.max(0, finiteNumber(entry.blockingDuration))),
    renderDurationMs: roundMs(renderStartMs > 0 ? Math.max(0, endTimeMs - renderStartMs) : 0),
    styleAndLayoutDurationMs: roundMs(
      styleAndLayoutStartMs > 0 ? Math.max(0, endTimeMs - styleAndLayoutStartMs) : 0,
    ),
    firstUiEventTimestampMs: firstUiEventTimestampMs !== null && firstUiEventTimestampMs > 0
      ? firstUiEventTimestampMs
      : null,
    scripts: scripts
      .map((script) => ({
        durationMs: roundMs(Math.max(0, finiteNumber(script.duration))),
        executionStartMs: optionalFiniteNumber(script.executionStart),
        forcedStyleAndLayoutDurationMs: roundMs(
          Math.max(0, finiteNumber(script.forcedStyleAndLayoutDuration)),
        ),
        pauseDurationMs: roundMs(Math.max(0, finiteNumber(script.pauseDuration))),
        invoker: boundedString(script.invoker ?? script.name, 240),
        invokerType: boundedString(script.invokerType, 120),
        sourceUrl: normalizedSourceUrl(script.sourceURL),
        sourceFunctionName: boundedString(script.sourceFunctionName, 240),
        sourceCharPosition: finiteNumber(script.sourceCharPosition, -1) >= 0
          ? finiteNumber(script.sourceCharPosition)
          : null,
      }))
      .sort((left, right) => right.durationMs - left.durationMs)
      .slice(0, MAX_SCRIPT_ATTRIBUTIONS),
  };
}

function readMemorySnapshot(): RuntimePerformanceIncident["memory"] {
  const memory = (performance as Performance & {
    memory?: {
      usedJSHeapSize?: number;
      totalJSHeapSize?: number;
      jsHeapSizeLimit?: number;
    };
  }).memory;
  if (!memory) return null;
  return {
    usedJsHeapBytes: Math.round(finiteNumber(memory.usedJSHeapSize)),
    totalJsHeapBytes: Math.round(finiteNumber(memory.totalJSHeapSize)),
    jsHeapLimitBytes: Math.round(finiteNumber(memory.jsHeapSizeLimit)),
  };
}

function safelyReadContext(provider: RuntimePerformanceDiagnosticsOptions["getContext"]): RuntimePerformanceContext {
  if (!provider) return {};
  try {
    const context = provider();
    return context && typeof context === "object" ? context : {};
  } catch (error) {
    return { contextError: error instanceof Error ? error.message : String(error) };
  }
}

function defaultReportIncident(incident: RuntimePerformanceIncident): void {
  console.warn("[RuntimePerformance]", "stall detected", incident);
}

function markIncidentInPerformanceTimeline(incident: RuntimePerformanceIncident): void {
  try {
    performance.clearMeasures(TRACE_MEASURE_NAME);
    performance.measure(TRACE_MEASURE_NAME, {
      start: incident.performanceStartMs,
      duration: incident.durationMs,
      detail: {
        id: incident.id,
        triggers: incident.triggers,
      },
    });
  } catch {
    // User Timing detail and numeric ranges vary across WebView2 versions.
  }
}

function reportPendingIncident(state: RunningDiagnostics): void {
  if (runningDiagnostics !== state) return;
  if (state.pendingTimer !== null) {
    clearTimeout(state.pendingTimer);
    state.pendingTimer = null;
  }
  const pending = state.pendingIncident;
  state.pendingIncident = null;
  if (!pending) return;

  const detectedPerformanceMs = state.options.now();
  const detectedAtMs = Date.now();
  const durationMs = Math.max(0, pending.endTimeMs - pending.startTimeMs);
  const incident: RuntimePerformanceIncident = {
    id: `${detectedAtMs}-${nextIncidentSequence++}`,
    detectedAtMs,
    approximateStartedAtMs: Math.round(
      detectedAtMs - Math.max(0, detectedPerformanceMs - pending.startTimeMs),
    ),
    performanceStartMs: roundMs(pending.startTimeMs),
    durationMs: roundMs(durationMs),
    triggers: Array.from(pending.triggers).sort(),
    visibilityState: document.visibilityState,
    href: boundedString(window.location.href, 1_000),
    activeElement: elementSnapshot(document.activeElement),
    lastInteraction: state.lastInteraction,
    watchdogDriftMs: pending.watchdogDriftMs === null
      ? null
      : roundMs(pending.watchdogDriftMs),
    longAnimationFrame: pending.longAnimationFrame,
    jankBurst: pending.jankBurst,
    longTask: pending.longTask,
    document: {
      elementCount: document.getElementsByTagName("*").length,
    },
    memory: readMemorySnapshot(),
    context: safelyReadContext(state.options.getContext),
  };

  markIncidentInPerformanceTimeline(incident);
  (window as unknown as {
    __LOCUS_RUNTIME_PERFORMANCE_INCIDENT__?: RuntimePerformanceIncident;
  }).__LOCUS_RUNTIME_PERFORMANCE_INCIDENT__ = incident;
  try {
    (state.options.reportIncident ?? defaultReportIncident)(incident);
  } catch {
    // Diagnostics remain best-effort and never affect the application path.
  } finally {
    state.suppressUntilMs = Math.max(
      state.suppressUntilMs,
      state.options.now() + state.options.watchdogIntervalMs,
    );
  }
}

function schedulePendingReport(state: RunningDiagnostics): void {
  if (state.pendingTimer !== null) return;
  state.pendingTimer = setTimeout(
    () => reportPendingIncident(state),
    state.options.coalesceDelayMs,
  );
}

function mergeCandidate(state: RunningDiagnostics, candidate: StallCandidate): void {
  const candidateEndMs = candidate.startTimeMs + candidate.durationMs;
  const pending = state.pendingIncident;
  const overlapsPending = pending
    && candidate.startTimeMs <= pending.endTimeMs + INCIDENT_MERGE_GAP_MS
    && candidateEndMs >= pending.startTimeMs - INCIDENT_MERGE_GAP_MS;

  if (pending && !overlapsPending) {
    reportPendingIncident(state);
  }

  const target = state.pendingIncident ?? {
    startTimeMs: candidate.startTimeMs,
    endTimeMs: candidateEndMs,
    triggers: new Set<StallTrigger>(),
    watchdogDriftMs: null,
    longAnimationFrame: null,
    jankBurst: null,
    longTask: null,
  };
  target.startTimeMs = Math.min(target.startTimeMs, candidate.startTimeMs);
  target.endTimeMs = Math.max(target.endTimeMs, candidateEndMs);
  target.triggers.add(candidate.trigger);
  if (candidate.watchdogDriftMs !== undefined) {
    target.watchdogDriftMs = Math.max(target.watchdogDriftMs ?? 0, candidate.watchdogDriftMs);
  }
  if (
    candidate.longAnimationFrame
    && (!target.longAnimationFrame
      || candidate.longAnimationFrame.durationMs > target.longAnimationFrame.durationMs)
  ) {
    target.longAnimationFrame = candidate.longAnimationFrame;
  }
  if (
    candidate.jankBurst
    && (!target.jankBurst
      || candidate.jankBurst.totalFrameDurationMs > target.jankBurst.totalFrameDurationMs)
  ) {
    target.jankBurst = candidate.jankBurst;
  }
  if (candidate.longTask && (!target.longTask || candidate.longTask.durationMs > target.longTask.durationMs)) {
    target.longTask = candidate.longTask;
  }
  state.pendingIncident = target;
  schedulePendingReport(state);
}

function recordCandidate(state: RunningDiagnostics, candidate: StallCandidate): void {
  if (runningDiagnostics !== state) return;
  if (document.visibilityState === "hidden") return;
  if (state.options.now() < state.warmupUntilMs) return;
  if (
    candidate.startTimeMs <= state.suppressUntilMs
    && candidate.startTimeMs + candidate.durationMs <= state.suppressUntilMs
  ) return;
  if (candidate.durationMs < state.options.stallThresholdMs) return;
  if (candidate.durationMs > MAX_REPORTABLE_STALL_MS) return;
  mergeCandidate(state, candidate);
}

function trackMediumFrameBurst(
  state: RunningDiagnostics,
  snapshot: RuntimeLongAnimationFrameSnapshot,
): void {
  if (runningDiagnostics !== state) return;
  if (document.visibilityState === "hidden" || state.options.now() < state.warmupUntilMs) {
    state.jankFrames = [];
    state.jankBurstReported = false;
    return;
  }

  const durationMs = snapshot.durationMs;
  if (
    durationMs < state.options.jankFrameThresholdMs
    || durationMs >= state.options.stallThresholdMs
  ) {
    if (durationMs >= state.options.stallThresholdMs) {
      state.jankFrames = [];
      state.jankBurstReported = true;
    }
    return;
  }

  const previous = state.jankFrames[state.jankFrames.length - 1];
  const previousEndMs = previous
    ? previous.startTimeMs + previous.durationMs
    : Number.NEGATIVE_INFINITY;
  if (snapshot.startTimeMs - previousEndMs > state.options.jankWindowMs) {
    state.jankFrames = [];
    state.jankBurstReported = false;
  }

  const frameEndMs = snapshot.startTimeMs + snapshot.durationMs;
  const cutoffMs = frameEndMs - state.options.jankWindowMs;
  state.jankFrames = state.jankFrames.filter((frame) => frame.startTimeMs >= cutoffMs);
  state.jankFrames.push(snapshot);
  if (state.jankBurstReported) return;

  const totalFrameDurationMs = state.jankFrames.reduce(
    (total, frame) => total + frame.durationMs,
    0,
  );
  if (
    state.jankFrames.length < state.options.jankMinFrameCount
    || totalFrameDurationMs < state.options.jankMinTotalDurationMs
  ) return;

  const firstFrame = state.jankFrames[0]!;
  const lastFrame = state.jankFrames[state.jankFrames.length - 1]!;
  const windowStartTimeMs = firstFrame.startTimeMs;
  const windowEndTimeMs = lastFrame.startTimeMs + lastFrame.durationMs;
  const largestFrame = state.jankFrames.reduce((largest, frame) => (
    frame.durationMs > largest.durationMs ? frame : largest
  ));
  const jankBurst: RuntimeJankBurstSnapshot = {
    windowStartTimeMs: roundMs(windowStartTimeMs),
    windowEndTimeMs: roundMs(windowEndTimeMs),
    windowDurationMs: roundMs(Math.max(0, windowEndTimeMs - windowStartTimeMs)),
    frameCount: state.jankFrames.length,
    totalFrameDurationMs: roundMs(totalFrameDurationMs),
    totalBlockingDurationMs: roundMs(state.jankFrames.reduce(
      (total, frame) => total + frame.blockingDurationMs,
      0,
    )),
    maxFrameDurationMs: roundMs(largestFrame.durationMs),
    frames: state.jankFrames.slice(-MAX_JANK_FRAME_SAMPLES).map((frame) => ({
      startTimeMs: roundMs(frame.startTimeMs),
      durationMs: roundMs(frame.durationMs),
      blockingDurationMs: roundMs(frame.blockingDurationMs),
    })),
  };

  state.jankBurstReported = true;
  recordCandidate(state, {
    trigger: "jank-burst",
    startTimeMs: windowStartTimeMs,
    durationMs: Math.max(0, windowEndTimeMs - windowStartTimeMs),
    longAnimationFrame: largestFrame,
    jankBurst,
  });
}

function installPerformanceObserver(state: RunningDiagnostics): void {
  if (typeof PerformanceObserver === "undefined") return;
  const supported = PerformanceObserver.supportedEntryTypes ?? [];
  const entryType = supported.includes("long-animation-frame")
    ? "long-animation-frame"
    : supported.includes("longtask")
      ? "longtask"
      : null;
  if (!entryType) return;

  try {
    state.observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entryType === "long-animation-frame") {
          const snapshot = snapshotLongAnimationFrame(entry as LongAnimationFrameEntryLike);
          trackMediumFrameBurst(state, snapshot);
          recordCandidate(state, {
            trigger: "long-animation-frame",
            startTimeMs: snapshot.startTimeMs,
            durationMs: snapshot.durationMs,
            longAnimationFrame: snapshot,
          });
        } else {
          recordCandidate(state, {
            trigger: "longtask",
            startTimeMs: finiteNumber(entry.startTime),
            durationMs: finiteNumber(entry.duration),
            longTask: {
              startTimeMs: roundMs(finiteNumber(entry.startTime)),
              durationMs: roundMs(Math.max(0, finiteNumber(entry.duration))),
              name: boundedString(entry.name, 120),
            },
          });
        }
      }
    });
    state.observer.observe({ type: entryType });
    state.observerEntryType = entryType;
  } catch {
    state.observer?.disconnect();
    state.observer = null;
    state.observerEntryType = null;
  }
}

function scheduleWatchdog(state: RunningDiagnostics): void {
  const expectedAtMs = state.options.now() + state.options.watchdogIntervalMs;
  state.watchdogTimer = setTimeout(() => {
    state.watchdogTimer = null;
    if (runningDiagnostics !== state) return;
    const currentMs = state.options.now();
    const driftMs = Math.max(0, currentMs - expectedAtMs);
    recordCandidate(state, {
      trigger: "watchdog",
      startTimeMs: expectedAtMs,
      durationMs: driftMs,
      watchdogDriftMs: driftMs,
    });
    scheduleWatchdog(state);
  }, state.options.watchdogIntervalMs);
}

export function startRuntimePerformanceDiagnostics(
  options: RuntimePerformanceDiagnosticsOptions = {},
): void {
  if (runningDiagnostics || typeof window === "undefined" || typeof document === "undefined") return;
  const resolvedOptions: RunningDiagnostics["options"] = {
    getContext: options.getContext,
    reportIncident: options.reportIncident,
    now: options.now ?? (() => performance.now()),
    stallThresholdMs: Math.max(50, options.stallThresholdMs ?? DEFAULT_STALL_THRESHOLD_MS),
    watchdogIntervalMs: Math.max(50, options.watchdogIntervalMs ?? DEFAULT_WATCHDOG_INTERVAL_MS),
    warmupMs: Math.max(0, options.warmupMs ?? DEFAULT_WARMUP_MS),
    coalesceDelayMs: Math.max(0, options.coalesceDelayMs ?? DEFAULT_COALESCE_DELAY_MS),
    jankWindowMs: Math.max(250, options.jankWindowMs ?? DEFAULT_JANK_WINDOW_MS),
    jankFrameThresholdMs: Math.max(
      50,
      options.jankFrameThresholdMs ?? DEFAULT_JANK_FRAME_THRESHOLD_MS,
    ),
    jankMinFrameCount: Math.max(
      2,
      Math.trunc(options.jankMinFrameCount ?? DEFAULT_JANK_MIN_FRAME_COUNT),
    ),
    jankMinTotalDurationMs: Math.max(
      50,
      options.jankMinTotalDurationMs ?? DEFAULT_JANK_MIN_TOTAL_DURATION_MS,
    ),
  };
  const state: RunningDiagnostics = {
    options: resolvedOptions,
    observer: null,
    observerEntryType: null,
    watchdogTimer: null,
    pendingTimer: null,
    pendingIncident: null,
    warmupUntilMs: resolvedOptions.now() + resolvedOptions.warmupMs,
    suppressUntilMs: Number.NEGATIVE_INFINITY,
    jankFrames: [],
    jankBurstReported: false,
    lastInteraction: null,
    onPointerDown: () => {},
    onKeyDown: () => {},
  };
  state.onPointerDown = (event) => {
    state.lastInteraction = {
      type: "pointer",
      atMs: Date.now(),
      target: elementSnapshot(event.target),
    };
  };
  state.onKeyDown = (event) => {
    state.lastInteraction = {
      type: "keyboard",
      atMs: Date.now(),
      target: elementSnapshot(event.target),
      key: keyboardLabel(event),
      modifiers: keyboardModifiers(event),
    };
  };

  runningDiagnostics = state;
  window.addEventListener("pointerdown", state.onPointerDown, true);
  window.addEventListener("keydown", state.onKeyDown, true);
  installPerformanceObserver(state);
  scheduleWatchdog(state);
  console.info("[RuntimePerformance]", "diagnostics started", {
    observer: state.observerEntryType ?? "watchdog-only",
    stallThresholdMs: resolvedOptions.stallThresholdMs,
    watchdogIntervalMs: resolvedOptions.watchdogIntervalMs,
    jankWindowMs: resolvedOptions.jankWindowMs,
    jankMinFrameCount: resolvedOptions.jankMinFrameCount,
    jankMinTotalDurationMs: resolvedOptions.jankMinTotalDurationMs,
  });
}

export function stopRuntimePerformanceDiagnostics(): void {
  const state = runningDiagnostics;
  if (!state) return;
  runningDiagnostics = null;
  state.observer?.disconnect();
  if (state.watchdogTimer !== null) clearTimeout(state.watchdogTimer);
  if (state.pendingTimer !== null) clearTimeout(state.pendingTimer);
  window.removeEventListener("pointerdown", state.onPointerDown, true);
  window.removeEventListener("keydown", state.onKeyDown, true);
  state.pendingIncident = null;
  delete (window as unknown as {
    __LOCUS_RUNTIME_PERFORMANCE_INCIDENT__?: RuntimePerformanceIncident;
  }).__LOCUS_RUNTIME_PERFORMANCE_INCIDENT__;
  try {
    performance.clearMeasures(TRACE_MEASURE_NAME);
  } catch {
    // Ignore WebView2 variants without User Timing cleanup support.
  }
  console.info("[RuntimePerformance]", "diagnostics stopped");
}

export function isRuntimePerformanceDiagnosticsRunning(): boolean {
  return runningDiagnostics !== null;
}
