import {
  getCachedDebugMode,
  subscribeDebugMode,
} from "./permissions";
import {
  invokeLocusRuntime,
  subscribeLocusRuntimeInvokeActivity,
  type LocusRuntimeInvokeActivity,
} from "./locusRuntime";
import { hasTauriWindowRuntime } from "./tauriRuntime";

const HEARTBEAT_COMMAND = "debug_webview_bridge_heartbeat";
const HEARTBEAT_INTERVAL_MS = 5_000;
const HEARTBEAT_TIMEOUT_MS = 4_000;
const MAX_PENDING_INVOKES = 24;
const MAX_LIFECYCLE_EVENTS = 16;
const DEBUG_MODE_STORAGE_KEY = "locus:webview-bridge:debug-enabled:v1";
const LIFECYCLE_STORAGE_KEY = "locus:webview-bridge:lifecycle:v1";
const STALL_STORAGE_KEY = "locus:webview-bridge:stall:v1";

interface TauriInternals {
  callbacks?: Map<unknown, unknown> | Record<string, unknown>;
}

interface PendingInvoke {
  id: number;
  command: string;
  startedAtMs: number;
}

export interface PendingInvokeSnapshot {
  command: string;
  ageMs: number;
}

export interface FrontendLifecycleEvent {
  event: string;
  timestampMs: number;
  sessionId: string;
  href: string;
  detail?: string;
}

export interface FrontendBridgeStallSnapshot {
  id: string;
  detectedAtMs: number;
  reason: string;
  heartbeat: FrontendBridgeHeartbeat;
}

export interface FrontendBridgeHeartbeat {
  sequence: number;
  sentAtMs: number;
  sessionId: string;
  href: string;
  readyState: string;
  visibilityState: string;
  navigationType: string | null;
  performanceNowMs: number;
  eventLoopLagMs: number;
  callbackCount: number | null;
  pendingInvokes: PendingInvokeSnapshot[];
  lifecycle: FrontendLifecycleEvent[];
  recoveredStall: FrontendBridgeStallSnapshot | null;
}

let initialized = false;
let running = false;
let debugModeUnsubscribe: (() => void) | null = null;
let invokeActivityUnsubscribe: (() => void) | null = null;
let heartbeatTimer: ReturnType<typeof setTimeout> | null = null;
let heartbeatRequest: Promise<void> | null = null;
let heartbeatSequence = 0;
let expectedHeartbeatAt = 0;
let lastEventLoopLagMs = 0;
let stallReportedForRequest = false;
let lifecycleEvents: FrontendLifecycleEvent[] = [];
const pendingInvokes = new Map<number, PendingInvoke>();
const sessionId = createSessionId();
type RuntimePerformanceDiagnosticsModule = typeof import("./runtimePerformanceDiagnostics");
let performanceDiagnosticsModulePromise: Promise<RuntimePerformanceDiagnosticsModule> | null = null;
let performanceDiagnosticsActivation = 0;

function createSessionId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `frontend-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }
}

function getTauriInternals(): TauriInternals | null {
  if (typeof window === "undefined") return null;
  return ((window as unknown as { __TAURI_INTERNALS__?: TauriInternals })
    .__TAURI_INTERNALS__ ?? null);
}

function debugEnabledAtStartup(): boolean {
  if (getCachedDebugMode() === true) return true;
  if ((window as unknown as { __LOCUS_DEBUG_ENABLED__?: boolean })
    .__LOCUS_DEBUG_ENABLED__ === true) return true;
  try {
    return window.localStorage.getItem(DEBUG_MODE_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function readStoredJson<T>(key: string): T | null {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : null;
  } catch {
    return null;
  }
}

function writeStoredJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Diagnostics must remain best-effort when storage is unavailable.
  }
}

function removeStoredValue(key: string): void {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // Diagnostics must remain best-effort when storage is unavailable.
  }
}

function loadLifecycleEvents(): FrontendLifecycleEvent[] {
  const stored = readStoredJson<FrontendLifecycleEvent[]>(LIFECYCLE_STORAGE_KEY);
  return Array.isArray(stored) ? stored.slice(-MAX_LIFECYCLE_EVENTS) : [];
}

function appendLifecycleEvent(event: string, detail?: string): void {
  lifecycleEvents.push({
    event,
    timestampMs: Date.now(),
    sessionId,
    href: window.location.href,
    ...(detail ? { detail } : {}),
  });
  lifecycleEvents = lifecycleEvents.slice(-MAX_LIFECYCLE_EVENTS);
  writeStoredJson(LIFECYCLE_STORAGE_KEY, lifecycleEvents);
}

function callbackCount(): number | null {
  const callbacks = getTauriInternals()?.callbacks;
  if (callbacks instanceof Map) return callbacks.size;
  if (callbacks && typeof callbacks === "object") return Object.keys(callbacks).length;
  return null;
}

function navigationType(): string | null {
  const entry = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
  return entry?.type ?? null;
}

export function snapshotPendingInvokes(
  pending: Iterable<PendingInvoke>,
  now = Date.now(),
): PendingInvokeSnapshot[] {
  return Array.from(pending)
    .sort((left, right) => left.startedAtMs - right.startedAtMs)
    .slice(0, MAX_PENDING_INVOKES)
    .map((entry) => ({
      command: entry.command.slice(0, 160),
      ageMs: Math.max(0, now - entry.startedAtMs),
    }));
}

function buildHeartbeat(): FrontendBridgeHeartbeat {
  const now = Date.now();
  return {
    sequence: ++heartbeatSequence,
    sentAtMs: now,
    sessionId,
    href: window.location.href,
    readyState: document.readyState,
    visibilityState: document.visibilityState,
    navigationType: navigationType(),
    performanceNowMs: performance.now(),
    eventLoopLagMs: lastEventLoopLagMs,
    callbackCount: callbackCount(),
    pendingInvokes: snapshotPendingInvokes(pendingInvokes.values(), now),
    lifecycle: lifecycleEvents.slice(-MAX_LIFECYCLE_EVENTS),
    recoveredStall: readStoredJson<FrontendBridgeStallSnapshot>(STALL_STORAGE_KEY),
  };
}

function runtimePerformanceContext(): Record<string, unknown> {
  const now = Date.now();
  return {
    sessionId,
    eventLoopLagMs: lastEventLoopLagMs,
    callbackCount: callbackCount(),
    pendingInvokes: snapshotPendingInvokes(pendingInvokes.values(), now),
    lifecycle: lifecycleEvents.slice(-MAX_LIFECYCLE_EVENTS),
  };
}

function startPerformanceDiagnostics(): void {
  const activation = ++performanceDiagnosticsActivation;
  performanceDiagnosticsModulePromise ??= import("./runtimePerformanceDiagnostics");
  void performanceDiagnosticsModulePromise
    .then((module) => {
      if (!running || performanceDiagnosticsActivation !== activation) return;
      module.startRuntimePerformanceDiagnostics({
        getContext: runtimePerformanceContext,
      });
    })
    .catch((error) => {
      if (performanceDiagnosticsActivation !== activation) return;
      performanceDiagnosticsModulePromise = null;
      console.warn("[RuntimePerformance]", "diagnostics module failed to load", error);
    });
}

function stopPerformanceDiagnostics(): void {
  performanceDiagnosticsActivation += 1;
  if (!performanceDiagnosticsModulePromise) return;
  void performanceDiagnosticsModulePromise
    .then((module) => module.stopRuntimePerformanceDiagnostics())
    .catch(() => {
      performanceDiagnosticsModulePromise = null;
    });
}

function installInvokeTracking(): void {
  if (invokeActivityUnsubscribe) return;
  invokeActivityUnsubscribe = subscribeLocusRuntimeInvokeActivity(
    (activity: LocusRuntimeInvokeActivity) => {
      if (activity.phase === "started") {
        pendingInvokes.set(activity.id, {
          id: activity.id,
          command: activity.command,
          startedAtMs: activity.startedAtMs,
        });
      } else {
        pendingInvokes.delete(activity.id);
      }
    },
  );
}

function restoreInvokeTracking(): void {
  invokeActivityUnsubscribe?.();
  invokeActivityUnsubscribe = null;
  pendingInvokes.clear();
}

function persistHeartbeatTimeout(heartbeat: FrontendBridgeHeartbeat): void {
  const reason = `heartbeat invoke exceeded ${HEARTBEAT_TIMEOUT_MS}ms`;
  appendLifecycleEvent("heartbeat-timeout", reason);
  const snapshot: FrontendBridgeStallSnapshot = {
    id: `${sessionId}-${heartbeat.sequence}-${Date.now()}`,
    detectedAtMs: Date.now(),
    reason,
    heartbeat: {
      ...heartbeat,
      pendingInvokes: snapshotPendingInvokes(pendingInvokes.values()),
      lifecycle: lifecycleEvents.slice(-MAX_LIFECYCLE_EVENTS),
      recoveredStall: null,
    },
  };
  writeStoredJson(STALL_STORAGE_KEY, snapshot);
  if (!stallReportedForRequest) {
    stallReportedForRequest = true;
    console.error("[WebViewBridge] frontend heartbeat timed out", snapshot);
  }
}

async function sendHeartbeat(): Promise<void> {
  if (!running || heartbeatRequest) return;
  const heartbeat = buildHeartbeat();
  let timeoutHandle: ReturnType<typeof setTimeout> | null = null;

  const request = invokeLocusRuntime<void>(HEARTBEAT_COMMAND, { heartbeat });
  heartbeatRequest = request;
  stallReportedForRequest = false;
  void request.finally(() => {
    if (heartbeatRequest === request) heartbeatRequest = null;
  }).catch(() => {
    // The raced request reports failures below; keep this branch handled.
  });

  try {
    await Promise.race([
      request,
      new Promise<never>((_, reject) => {
        timeoutHandle = setTimeout(
          () => reject(new Error("WebView bridge heartbeat timed out")),
          HEARTBEAT_TIMEOUT_MS,
        );
      }),
    ]);
    if (heartbeat.recoveredStall) removeStoredValue(STALL_STORAGE_KEY);
  } catch {
    if (heartbeatRequest === request) persistHeartbeatTimeout(heartbeat);
  } finally {
    if (timeoutHandle !== null) clearTimeout(timeoutHandle);
  }
}

function scheduleHeartbeat(delayMs = HEARTBEAT_INTERVAL_MS): void {
  if (!running) return;
  if (heartbeatTimer !== null) clearTimeout(heartbeatTimer);
  expectedHeartbeatAt = performance.now() + delayMs;
  heartbeatTimer = setTimeout(() => {
    heartbeatTimer = null;
    lastEventLoopLagMs = Math.max(0, performance.now() - expectedHeartbeatAt);
    void sendHeartbeat().finally(() => scheduleHeartbeat());
  }, delayMs);
}

function handlePageShow(event: PageTransitionEvent): void {
  appendLifecycleEvent("pageshow", `persisted=${event.persisted}`);
}

function handlePageHide(event: PageTransitionEvent): void {
  appendLifecycleEvent("pagehide", `persisted=${event.persisted}`);
}

function handleBeforeUnload(): void {
  appendLifecycleEvent("beforeunload");
}

function handleVisibilityChange(): void {
  appendLifecycleEvent("visibilitychange", document.visibilityState);
}

function startDiagnostics(): void {
  if (running || !hasTauriWindowRuntime()) return;
  running = true;
  try {
    try {
      window.localStorage.setItem(DEBUG_MODE_STORAGE_KEY, "1");
    } catch {
      // The native watchdog still captures the host side without local storage.
    }
    lifecycleEvents = loadLifecycleEvents();
    appendLifecycleEvent("diagnostics-start", `navigation=${navigationType() ?? "unknown"}`);
    window.addEventListener("pageshow", handlePageShow);
    window.addEventListener("pagehide", handlePageHide);
    window.addEventListener("beforeunload", handleBeforeUnload);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    installInvokeTracking();
    startPerformanceDiagnostics();
    scheduleHeartbeat(250);
  } catch (error) {
    running = false;
    if (heartbeatTimer !== null) {
      clearTimeout(heartbeatTimer);
      heartbeatTimer = null;
    }
    window.removeEventListener("pageshow", handlePageShow);
    window.removeEventListener("pagehide", handlePageHide);
    window.removeEventListener("beforeunload", handleBeforeUnload);
    document.removeEventListener("visibilitychange", handleVisibilityChange);
    restoreInvokeTracking();
    stopPerformanceDiagnostics();
    console.warn("[WebViewBridge] diagnostics initialization failed", error);
  }
}

function stopDiagnostics(): void {
  if (!running) return;
  appendLifecycleEvent("diagnostics-stop");
  running = false;
  removeStoredValue(DEBUG_MODE_STORAGE_KEY);
  if (heartbeatTimer !== null) {
    clearTimeout(heartbeatTimer);
    heartbeatTimer = null;
  }
  window.removeEventListener("pageshow", handlePageShow);
  window.removeEventListener("pagehide", handlePageHide);
  window.removeEventListener("beforeunload", handleBeforeUnload);
  document.removeEventListener("visibilitychange", handleVisibilityChange);
  restoreInvokeTracking();
  stopPerformanceDiagnostics();
}

export function initWebviewBridgeDiagnostics(): void {
  if (initialized || typeof window === "undefined") return;
  initialized = true;
  try {
    debugModeUnsubscribe = subscribeDebugMode((enabled) => {
      try {
        if (enabled) startDiagnostics();
        else stopDiagnostics();
      } catch (error) {
        console.warn("[WebViewBridge] diagnostics state update failed", error);
      }
    });
    if (debugEnabledAtStartup()) startDiagnostics();
  } catch (error) {
    debugModeUnsubscribe?.();
    debugModeUnsubscribe = null;
    initialized = false;
    console.warn("[WebViewBridge] diagnostics setup failed", error);
  }
}

export function teardownWebviewBridgeDiagnostics(): void {
  stopDiagnostics();
  debugModeUnsubscribe?.();
  debugModeUnsubscribe = null;
  initialized = false;
}
