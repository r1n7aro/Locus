import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DebugConsoleEntry, DebugConsoleLevel } from "../types";
import { invokeLocusRuntime } from "./locusRuntime";
import { hasTauriWindowRuntime } from "./tauriRuntime";

const MAX_ENTRIES = 2_000;
const MAX_TEXT_CHARS = 1024 * 1024;
const MAX_MESSAGE_CHARS = 16_000;
const MAX_FIELD_CHARS = 512;
const BACKEND_EVENT_NAME = "app-log-batch";
const DISPLAY_FLUSH_DELAY_MS = 16;
const DISPLAY_MAX_PENDING = 4_096;
const FORWARD_FLUSH_DELAY_MS = 400;
const FORWARD_MAX_BATCH = 128;
const FORWARD_MAX_BATCH_CHARS = 128 * 1024;

const listeners = new Set<() => void>();
const entryIds = new Set<string>();
const entries: DebugConsoleEntry[] = [];
let entriesChars = 0;

const originalConsole = {
  log: console.log.bind(console),
  info: console.info.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  debug: console.debug.bind(console),
};

let consoleInstalled = false;
let backendReady = false;
let backendUnlisten: UnlistenFn | null = null;
let backendBridgeRequest: Promise<void> | null = null;
let backendSnapshotRequest: Promise<void> | null = null;
let backendSnapshotLoaded = false;
let backendGeneration = 0;
let snapshotRevision = 0;
let nextFrontendId = 1;
let nextDroppedMarkerId = 1;

interface BackendLogBatchEvent {
  entries: DebugConsoleEntry[];
  droppedCount: number;
}

type ConsoleMethod = keyof typeof originalConsole;

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

function trimEntries() {
  while (entries.length > MAX_ENTRIES || entriesChars > MAX_TEXT_CHARS) {
    const removed = entries.shift();
    if (removed) {
      entryIds.delete(removed.id);
      entriesChars -= entryTextChars(removed);
    }
  }
}

function entryTextChars(entry: DebugConsoleEntry): number {
  return entry.id.length + entry.level.length + entry.source.length
    + entry.module.length + entry.target.length + entry.message.length;
}

function boundConsoleText(value: string, limit: number): string {
  if (value.length <= limit) return value;
  const suffix = " …(truncated)";
  let end = limit - suffix.length;
  // Do not split a UTF-16 surrogate pair at the preview boundary.
  const last = value.charCodeAt(end - 1);
  if (last >= 0xd800 && last <= 0xdbff) end -= 1;
  // Copy only the bounded prefix: V8 sliced strings can otherwise retain the
  // entire original log allocation, even after it has left the queues.
  const prefix: string = JSON.parse(JSON.stringify(value.slice(0, end)));
  return prefix + suffix;
}

function boundConsoleEntry(entry: DebugConsoleEntry): DebugConsoleEntry {
  return {
    ...entry,
    message: boundConsoleText(entry.message, MAX_MESSAGE_CHARS),
    module: boundConsoleText(entry.module, MAX_FIELD_CHARS),
    target: boundConsoleText(entry.target, MAX_FIELD_CHARS),
  };
}

export async function saveDebugConsoleLogExport(
  filePath: string,
): Promise<string> {
  // Export the durable source, including records no longer in the UI buffer.
  while (forwardInFlight || forwardQueue.length > 0) await flushForwardQueue();
  return invokeLocusRuntime<string>("save_log_export", { filePath });
}

export interface ForwardedLogLine {
  timestampMs: number;
  level: string;
  module: string;
  message: string;
}

export function buildForwardPayload(
  batch: readonly DebugConsoleEntry[],
): ForwardedLogLine[] {
  return batch.map((entry) => ({
    timestampMs: entry.timestampMs,
    level: entry.level,
    module: entry.module,
    message: entry.message,
  }));
}

// Frontend console lines are mirrored into the backend's persistent log file
// (best-effort, batched) so crash/freeze reports include the JS side too.
const forwardQueue: DebugConsoleEntry[] = [];
let forwardTimer: ReturnType<typeof setTimeout> | null = null;
let forwardInFlight: Promise<void> | null = null;

function scheduleForwardFlush(delayMs = FORWARD_FLUSH_DELAY_MS) {
  if (forwardTimer !== null) return;
  forwardTimer = setTimeout(() => {
    forwardTimer = null;
    void flushForwardQueue().catch(() => {
      // Keep logging failures out of the console capture pipeline.
    });
  }, delayMs);
}

function queueForwardToFile(entry: DebugConsoleEntry) {
  if (!hasTauriWindowRuntime()) return;
  // This is a write queue, not the display ring. Keep records until the
  // backend accepts them, including records evicted from the visible console.
  forwardQueue.push(entry);
  scheduleForwardFlush();
}

function flushForwardQueue(): Promise<void> {
  if (forwardInFlight) return forwardInFlight;
  if (forwardQueue.length === 0) return Promise.resolve();
  if (forwardTimer !== null) {
    clearTimeout(forwardTimer);
    forwardTimer = null;
  }
  let count = 0;
  let chars = 0;
  for (const entry of forwardQueue) {
    if (count >= FORWARD_MAX_BATCH || (count > 0 && chars + entryTextChars(entry) > FORWARD_MAX_BATCH_CHARS)) break;
    chars += entryTextChars(entry);
    count += 1;
  }
  const batch = forwardQueue.splice(0, count);
  let failed = false;
  const request = invokeLocusRuntime("append_frontend_logs", {
    entries: buildForwardPayload(batch),
  }).then(() => undefined).catch((error: unknown) => {
    failed = true;
    forwardQueue.unshift(...batch);
    throw error;
  }).finally(() => {
    forwardInFlight = null;
    if (forwardQueue.length > 0) scheduleForwardFlush(failed ? FORWARD_FLUSH_DELAY_MS : 0);
  });
  forwardInFlight = request;
  return request;
}

function pushEntries(batch: DebugConsoleEntry[]) {
  let changed = false;
  for (const incoming of batch) {
    if (entryIds.has(incoming.id)) continue;
    const entry = boundConsoleEntry(incoming);
    entryIds.add(entry.id);
    entries.push(entry);
    entriesChars += entryTextChars(entry);
    changed = true;
  }
  if (!changed) return;
  entries.sort((left, right) => left.timestampMs - right.timestampMs);
  trimEntries();
  notify();
}

const displayQueue: DebugConsoleEntry[] = [];
let displayQueueChars = 0;
let displayFlushTimer: ReturnType<typeof setTimeout> | null = null;
let displayDropped = 0;

function clearDisplayFlushTimer() {
  if (displayFlushTimer === null) return;
  clearTimeout(displayFlushTimer);
  displayFlushTimer = null;
}

export function buildLiveLogDroppedEntry(
  droppedCount: number,
  timestampMs = Date.now(),
  markerId = nextDroppedMarkerId++,
): DebugConsoleEntry {
  return {
    id: `backend-live-dropped-${markerId}`,
    timestampMs,
    level: "warn",
    source: "backend",
    module: "logging",
    target: "logging",
    message: `Live console dropped ${droppedCount} entries during a log burst. Refresh to load the latest backend snapshot.`,
  };
}

function flushDisplayQueue() {
  clearDisplayFlushTimer();
  if (displayQueue.length === 0 && displayDropped === 0) return;
  const batch = displayQueue.splice(0, displayQueue.length);
  displayQueueChars = 0;
  const droppedCount = displayDropped;
  displayDropped = 0;
  if (droppedCount > 0) {
    batch.push(buildLiveLogDroppedEntry(droppedCount));
  }
  pushEntries(batch);
}

function scheduleDisplayFlush() {
  if (displayFlushTimer !== null) return;
  displayFlushTimer = setTimeout(flushDisplayQueue, DISPLAY_FLUSH_DELAY_MS);
}

function queueDisplayEntries(batch: readonly DebugConsoleEntry[], droppedCount = 0) {
  if (Number.isFinite(droppedCount)) {
    displayDropped += Math.max(0, Math.trunc(droppedCount));
  }
  for (const incoming of batch) {
    const entry = boundConsoleEntry(incoming);
    displayQueue.push(entry);
    displayQueueChars += entryTextChars(entry);
    while (displayQueue.length > DISPLAY_MAX_PENDING || displayQueueChars > MAX_TEXT_CHARS) {
      const removed = displayQueue.shift()!;
      displayQueueChars -= entryTextChars(removed);
      displayDropped += 1;
    }
  }
  scheduleDisplayFlush();
}

function parseBracketPrefix(input: string): { module: string; message: string } | null {
  const trimmed = input.trimStart();
  if (!trimmed.startsWith("[")) return null;
  const end = trimmed.indexOf("]");
  if (end <= 1) return null;
  const firstModule = trimmed.slice(1, end).trim();
  if (!firstModule) return null;
  let module = firstModule;
  let message = trimmed.slice(end + 1).trimStart();
  if (["DEBUG", "TRACE", "INFO", "WARN", "ERROR"].includes(firstModule) && message.startsWith("[")) {
    const secondEnd = message.indexOf("]");
    if (secondEnd > 1) {
      const secondModule = message.slice(1, secondEnd).trim();
      if (secondModule) {
        module = secondModule;
        message = message.slice(secondEnd + 1).trimStart();
      }
    }
  }
  return {
    module,
    message,
  };
}

function formatArg(value: unknown): string {
  if (value instanceof Error) {
    return value.stack || value.message || value.name;
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean" || value == null) {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function inferCallerModule(): string {
  const stack = new Error().stack ?? "";
  const lines = stack.split("\n");
  for (const line of lines) {
    const normalized = line.replace(/\\/g, "/");
    const match = normalized.match(/\/src\/([^():?]+?\.(?:ts|vue))/);
    if (!match?.[1]) continue;
    const relative = match[1];
    if (relative.startsWith("services/debugConsole")) continue;
    return relative.replace(/\.(ts|vue)$/, "");
  }
  return "frontend";
}

function normalizeMessage(
  args: unknown[],
  fallbackModule: string,
): { module: string; message: string; hasExplicitModule: boolean } {
  const serialized = args.map((arg) => formatArg(arg));
  let module = fallbackModule;
  let hasExplicitModule = false;

  if (typeof args[0] === "string") {
    const parsed = parseBracketPrefix(args[0]);
    if (parsed) {
      module = parsed.module;
      serialized[0] = parsed.message;
      hasExplicitModule = true;
    }
  }

  const message = serialized.filter((part) => part.length > 0).join(" ").trim();
  return {
    module,
    message: message || "(empty log)",
    hasExplicitModule,
  };
}

function mapConsoleMethodToLevel(method: ConsoleMethod): DebugConsoleLevel {
  switch (method) {
    case "debug":
      return "debug";
    case "info":
      return "info";
    case "warn":
      return "warn";
    case "error":
      return "error";
    default:
      return "info";
  }
}

function captureConsole(method: ConsoleMethod, args: unknown[]) {
  const inferredModule = inferCallerModule();
  const normalized = normalizeMessage(args, inferredModule);
  const displayArgs = normalized.hasExplicitModule ? args : [`[${normalized.module}]`, ...args];
  originalConsole[method](...displayArgs);

  const entry: DebugConsoleEntry = {
    id: `frontend-${nextFrontendId++}`,
    timestampMs: Date.now(),
    level: mapConsoleMethodToLevel(method),
    source: "frontend",
    module: normalized.module,
    target: normalized.module,
    message: normalized.message,
  };
  queueDisplayEntries([entry]);
  queueForwardToFile(entry);
}

function installConsoleCapture() {
  if (consoleInstalled) return;
  consoleInstalled = true;

  console.log = (...args: unknown[]) => captureConsole("log", args);
  console.info = (...args: unknown[]) => captureConsole("info", args);
  console.warn = (...args: unknown[]) => captureConsole("warn", args);
  console.error = (...args: unknown[]) => captureConsole("error", args);
  console.debug = (...args: unknown[]) => captureConsole("debug", args);

  window.addEventListener("error", (event) => {
    captureConsole("error", [event.error ?? event.message]);
  });

  window.addEventListener("unhandledrejection", (event) => {
    captureConsole("error", ["Unhandled promise rejection", event.reason]);
  });
}

function fetchBackendSnapshot(): Promise<void> {
  if (backendSnapshotRequest) return backendSnapshotRequest;
  const generation = backendGeneration;
  const revision = snapshotRevision;
  const request = (async () => {
    const snapshot = await invokeLocusRuntime<DebugConsoleEntry[]>("get_log_entries", {
      limit: MAX_ENTRIES,
    });
    if (generation !== backendGeneration || revision !== snapshotRevision) return;
    pushEntries(snapshot);
    backendSnapshotLoaded = true;
  })().finally(() => {
    if (backendSnapshotRequest === request) backendSnapshotRequest = null;
  });
  backendSnapshotRequest = request;
  return request;
}

async function ensureBackendBridge() {
  if (!hasTauriWindowRuntime() || backendReady) return;
  if (backendBridgeRequest) return backendBridgeRequest;
  const generation = backendGeneration;
  const request = (async () => {
    const unlisten = await listen<BackendLogBatchEvent>(BACKEND_EVENT_NAME, (event) => {
      if (generation === backendGeneration) {
        queueDisplayEntries(event.payload.entries, event.payload.droppedCount);
      }
    });
    if (generation !== backendGeneration) {
      unlisten();
      return;
    }
    backendUnlisten = unlisten;
    backendReady = true;
  })().finally(() => {
    if (backendBridgeRequest === request) backendBridgeRequest = null;
  });
  backendBridgeRequest = request;
  return request;
}

export async function initDebugConsole() {
  installConsoleCapture();
  try {
    await ensureBackendBridge();
    if (backendReady && !backendSnapshotLoaded) await fetchBackendSnapshot();
  } catch (error) {
    originalConsole.warn("[debugConsole] failed to initialize backend log bridge", error);
  }
}

export async function refreshDebugConsole() {
  installConsoleCapture();
  await ensureBackendBridge();
  if (backendReady) await fetchBackendSnapshot();
}

export async function clearDebugConsole() {
  snapshotRevision += 1;
  backendSnapshotRequest = null;
  clearDisplayFlushTimer();
  displayQueue.splice(0, displayQueue.length);
  displayQueueChars = 0;
  displayDropped = 0;
  entries.splice(0, entries.length);
  entriesChars = 0;
  entryIds.clear();
  notify();
  await invokeLocusRuntime("clear_log_entries");
}

export function getDebugConsoleSnapshot(): DebugConsoleEntry[] {
  flushDisplayQueue();
  return entries.slice();
}

export function subscribeDebugConsole(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function teardownDebugConsole() {
  backendGeneration += 1;
  backendBridgeRequest = null;
  backendSnapshotRequest = null;
  backendSnapshotLoaded = false;
  backendUnlisten?.();
  backendUnlisten = null;
  backendReady = false;
  clearDisplayFlushTimer();
  displayQueue.splice(0, displayQueue.length);
  displayQueueChars = 0;
  displayDropped = 0;
  if (forwardTimer !== null) {
    clearTimeout(forwardTimer);
    forwardTimer = null;
  }
}

export async function revealLogFile(): Promise<string> {
  return invokeLocusRuntime<string>("reveal_log_file");
}
