import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DebugConsoleEntry, DebugConsoleLevel } from "../types";
import { hasTauriWindowRuntime } from "./tauriRuntime";

const MAX_ENTRIES = 2_000;
const BACKEND_EVENT_NAME = "app-log";

const listeners = new Set<() => void>();
const entryIds = new Set<string>();
const entries: DebugConsoleEntry[] = [];

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
let nextFrontendId = 1;

type ConsoleMethod = keyof typeof originalConsole;

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

function trimEntries() {
  while (entries.length > MAX_ENTRIES) {
    const removed = entries.shift();
    if (removed) {
      entryIds.delete(removed.id);
    }
  }
}

function normalizeExportMessage(message: string): string {
  return message.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

function formatExportEntry(entry: DebugConsoleEntry): string {
  const timestamp = new Date(entry.timestampMs).toISOString();
  const prefix = `[${timestamp}] [${entry.level.toUpperCase()}] [${entry.source}] [${entry.module}]`;
  const messageLines = normalizeExportMessage(entry.message).split("\n");
  const [firstLine = "", ...restLines] = messageLines;
  const continuation = restLines.map((line) => `    ${line}`);
  return [`${prefix} ${firstLine}`, ...continuation].join("\n");
}

export function formatDebugConsoleEntriesForLogExport(
  logEntries: readonly DebugConsoleEntry[],
  exportedAt = new Date(),
): string {
  const sortedEntries = logEntries
    .map((entry, index) => ({ entry, index }))
    .sort((left, right) =>
      left.entry.timestampMs - right.entry.timestampMs || left.index - right.index,
    )
    .map(({ entry }) => entry);
  const header = [
    "# Locus Console Log Export",
    `# Exported At: ${exportedAt.toISOString()}`,
    `# Entries: ${sortedEntries.length}`,
  ].join("\n");
  const body = sortedEntries.map(formatExportEntry).join("\n");
  return body ? `${header}\n\n${body}\n` : `${header}\n`;
}

export async function saveDebugConsoleLogExport(
  filePath: string,
  logEntries: readonly DebugConsoleEntry[],
): Promise<string> {
  const content = formatDebugConsoleEntriesForLogExport(logEntries);
  return invoke<string>("save_log_export", { filePath, content });
}

function pushEntries(batch: DebugConsoleEntry[]) {
  let changed = false;
  for (const entry of batch) {
    if (entryIds.has(entry.id)) continue;
    entryIds.add(entry.id);
    entries.push(entry);
    changed = true;
  }
  if (!changed) return;
  entries.sort((left, right) => left.timestampMs - right.timestampMs);
  trimEntries();
  notify();
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

  pushEntries([
    {
      id: `frontend-${nextFrontendId++}`,
      timestampMs: Date.now(),
      level: mapConsoleMethodToLevel(method),
      source: "frontend",
      module: normalized.module,
      target: normalized.module,
      message: normalized.message,
    },
  ]);
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

async function fetchBackendSnapshot() {
  const snapshot = await invoke<DebugConsoleEntry[]>("get_log_entries", { limit: MAX_ENTRIES });
  pushEntries(snapshot);
}

async function ensureBackendBridge() {
  if (!hasTauriWindowRuntime()) return;
  if (!backendReady) {
    backendUnlisten = await listen<DebugConsoleEntry>(BACKEND_EVENT_NAME, (event) => {
      pushEntries([event.payload]);
    });
    backendReady = true;
    await fetchBackendSnapshot();
    return;
  }

  await fetchBackendSnapshot();
}

export async function initDebugConsole() {
  installConsoleCapture();
  try {
    await ensureBackendBridge();
  } catch (error) {
    originalConsole.warn("[debugConsole] failed to initialize backend log bridge", error);
  }
}

export async function refreshDebugConsole() {
  await ensureBackendBridge();
}

export async function clearDebugConsole() {
  entries.splice(0, entries.length);
  entryIds.clear();
  notify();
  await invoke("clear_log_entries");
}

export function getDebugConsoleSnapshot(): DebugConsoleEntry[] {
  return entries.slice();
}

export function subscribeDebugConsole(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function teardownDebugConsole() {
  backendUnlisten?.();
  backendUnlisten = null;
  backendReady = false;
}
