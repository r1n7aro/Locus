// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  isRuntimePerformanceDiagnosticsRunning,
  snapshotLongAnimationFrame,
  startRuntimePerformanceDiagnostics,
  stopRuntimePerformanceDiagnostics,
  type RuntimePerformanceIncident,
} from "../services/runtimePerformanceDiagnostics";

class MockPerformanceObserver {
  static supportedEntryTypes = ["long-animation-frame"];
  static instances: MockPerformanceObserver[] = [];

  readonly observe = vi.fn();
  readonly disconnect = vi.fn();

  constructor(private readonly callback: PerformanceObserverCallback) {
    MockPerformanceObserver.instances.push(this);
  }

  emit(entries: PerformanceEntry[]) {
    this.callback({ getEntries: () => entries } as PerformanceObserverEntryList, this as unknown as PerformanceObserver);
  }
}

afterEach(() => {
  stopRuntimePerformanceDiagnostics();
  MockPerformanceObserver.instances = [];
  MockPerformanceObserver.supportedEntryTypes = ["long-animation-frame"];
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("runtime performance diagnostics", () => {
  it("does not install runtime work merely by loading the module", () => {
    vi.useFakeTimers();

    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("extracts bounded long-frame attribution without URL query data", () => {
    const snapshot = snapshotLongAnimationFrame({
      name: "long-animation-frame",
      entryType: "long-animation-frame",
      startTime: 100,
      duration: 600,
      blockingDuration: 470,
      renderStart: 620,
      styleAndLayoutStart: 660,
      firstUIEventTimestamp: 105,
      scripts: [{
        duration: 520,
        executionStart: 110,
        forcedStyleAndLayoutDuration: 45,
        pauseDuration: 0,
        invoker: "DOMWindow.onclick",
        invokerType: "event-listener",
        sourceURL: "http://localhost:14901/src/App.vue?cache=secret#fragment",
        sourceFunctionName: "handleClick",
        sourceCharPosition: 42,
      }],
      toJSON: () => ({}),
    });

    expect(snapshot).toMatchObject({
      startTimeMs: 100,
      durationMs: 600,
      blockingDurationMs: 470,
      renderDurationMs: 80,
      styleAndLayoutDurationMs: 40,
      firstUiEventTimestampMs: 105,
    });
    expect(snapshot.scripts[0]).toMatchObject({
      durationMs: 520,
      forcedStyleAndLayoutDurationMs: 45,
      invoker: "DOMWindow.onclick",
      sourceUrl: "http://localhost:14901/src/App.vue",
      sourceFunctionName: "handleClick",
      sourceCharPosition: 42,
    });
  });

  it("reports long frames only after diagnostics explicitly start", () => {
    vi.useFakeTimers();
    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.spyOn(console, "info").mockImplementation(() => {});
    const incidents: RuntimePerformanceIncident[] = [];

    startRuntimePerformanceDiagnostics({
      now: () => 1_000,
      warmupMs: 0,
      coalesceDelayMs: 0,
      reportIncident: (incident) => incidents.push(incident),
      getContext: () => ({ pendingInvokes: [{ command: "slow_command", ageMs: 800 }] }),
    });

    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(true);
    expect(MockPerformanceObserver.instances).toHaveLength(1);
    expect(MockPerformanceObserver.instances[0]?.observe).toHaveBeenCalledWith({
      type: "long-animation-frame",
    });

    MockPerformanceObserver.instances[0]?.emit([{
      name: "long-animation-frame",
      entryType: "long-animation-frame",
      startTime: 400,
      duration: 500,
      blockingDuration: 420,
      scripts: [],
      toJSON: () => ({}),
    } as PerformanceEntry]);
    vi.advanceTimersByTime(0);

    expect(incidents).toHaveLength(1);
    expect(incidents[0]).toMatchObject({
      durationMs: 500,
      triggers: ["long-animation-frame"],
      context: {
        pendingInvokes: [{ command: "slow_command", ageMs: 800 }],
      },
    });
  });

  it("reports a two-second burst of medium long frames as one jank incident", () => {
    vi.useFakeTimers();
    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.spyOn(console, "info").mockImplementation(() => {});
    const incidents: RuntimePerformanceIncident[] = [];

    startRuntimePerformanceDiagnostics({
      now: () => 2_500,
      warmupMs: 0,
      coalesceDelayMs: 0,
      reportIncident: (incident) => incidents.push(incident),
    });

    MockPerformanceObserver.instances[0]?.emit(
      [100, 400, 700, 1_000, 1_300].map((startTime) => ({
        name: "long-animation-frame",
        entryType: "long-animation-frame",
        startTime,
        duration: 100,
        blockingDuration: 20,
        scripts: [],
        toJSON: () => ({}),
      } as PerformanceEntry)),
    );
    vi.advanceTimersByTime(0);

    expect(incidents).toHaveLength(1);
    expect(incidents[0]).toMatchObject({
      durationMs: 1_300,
      triggers: ["jank-burst"],
      jankBurst: {
        windowStartTimeMs: 100,
        windowEndTimeMs: 1_400,
        windowDurationMs: 1_300,
        frameCount: 5,
        totalFrameDurationMs: 500,
        totalBlockingDurationMs: 100,
        maxFrameDurationMs: 100,
      },
    });
    expect(incidents[0]?.jankBurst?.frames).toHaveLength(5);
  });

  it("keeps isolated medium frames below the burst threshold", () => {
    vi.useFakeTimers();
    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.spyOn(console, "info").mockImplementation(() => {});
    const incidents: RuntimePerformanceIncident[] = [];

    startRuntimePerformanceDiagnostics({
      now: () => 4_000,
      warmupMs: 0,
      coalesceDelayMs: 0,
      reportIncident: (incident) => incidents.push(incident),
    });

    MockPerformanceObserver.instances[0]?.emit(
      [100, 700, 1_300, 3_500].map((startTime) => ({
        name: "long-animation-frame",
        entryType: "long-animation-frame",
        startTime,
        duration: 100,
        blockingDuration: 0,
        scripts: [],
        toJSON: () => ({}),
      } as PerformanceEntry)),
    );
    vi.advanceTimersByTime(0);

    expect(incidents).toHaveLength(0);
  });

  it("detects event-loop stalls with the foreground watchdog", () => {
    vi.useFakeTimers();
    MockPerformanceObserver.supportedEntryTypes = [];
    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.spyOn(console, "info").mockImplementation(() => {});
    let clockMs = 0;
    const incidents: RuntimePerformanceIncident[] = [];

    startRuntimePerformanceDiagnostics({
      now: () => clockMs,
      warmupMs: 0,
      watchdogIntervalMs: 100,
      stallThresholdMs: 200,
      coalesceDelayMs: 0,
      reportIncident: (incident) => incidents.push(incident),
    });

    clockMs = 700;
    vi.advanceTimersByTime(100);
    vi.advanceTimersByTime(1);

    expect(incidents).toHaveLength(1);
    expect(incidents[0]).toMatchObject({
      durationMs: 600,
      triggers: ["watchdog"],
      watchdogDriftMs: 600,
    });
  });

  it("removes observers, timers, and interaction listeners when stopped", () => {
    vi.useFakeTimers();
    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.spyOn(console, "info").mockImplementation(() => {});

    startRuntimePerformanceDiagnostics({ warmupMs: 0 });
    const observer = MockPerformanceObserver.instances[0];
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    stopRuntimePerformanceDiagnostics();

    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(false);
    expect(observer?.disconnect).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
  });
});
