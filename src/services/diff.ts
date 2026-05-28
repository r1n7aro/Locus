import { ipcInvoke } from "./ipc";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  FileDiffRequest,
  FileDiffPayload,
  DiffHunk,
  TextDiff,
  SemanticTargetInspector,
  SemanticTargetRequest,
} from "../types";

// ── Diff progress events ──

export interface DiffProgressEvent {
  requestKey: string;
  phase: "fetchContent" | "textDiff" | "parseYaml" | "buildSemantic" | "done" | "error";
  current: number;
  total: number;
  elapsedMs: number;
  error?: string;
  /** Per-phase durations in ms. Present only when phase === "done". */
  phaseDurations?: Record<string, number>;
}

export function listenDiffProgress(
  cb: (evt: DiffProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<DiffProgressEvent>("diff-progress", (e) => cb(e.payload));
}

// ── Request key computation (for cache + dedup) ──

function computeRequestKey(req: FileDiffRequest): string {
  return [
    req.source,
    req.filePath,
    req.oldPath ?? "",
    req.commitHash ?? "",
    req.sessionId ?? "",
    req.assistantMessageId ?? "",
    req.detail,
    req.fullContext ? "fc" : "",
  ].join(":");
}

export function parseDiffRequestKey(key: string): FileDiffRequest | null {
  const parts = key.split(":");
  if (parts.length < 8) return null;
  const [source, filePath, oldPath, commitHash, sessionId, assistantMessageId, detail, fc] = parts;
  return {
    source: source as FileDiffRequest["source"],
    filePath,
    oldPath: oldPath || undefined,
    commitHash: commitHash || undefined,
    sessionId: sessionId || undefined,
    assistantMessageId: assistantMessageId || undefined,
    detail: detail as FileDiffRequest["detail"],
    fullContext: fc === "fc",
  };
}

// ── LRU cache ──

const LRU_CAPACITY = 50;
const cache = new Map<string, FileDiffPayload>();

function lruSet(key: string, value: FileDiffPayload) {
  if (cache.has(key)) cache.delete(key);
  cache.set(key, value);
  if (cache.size > LRU_CAPACITY) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
}

function lruGet(key: string): FileDiffPayload | undefined {
  const val = cache.get(key);
  if (val !== undefined) {
    // Move to end (most recently used)
    cache.delete(key);
    cache.set(key, val);
  }
  return val;
}

// ── In-flight dedup ──

const inflight = new Map<string, Promise<FileDiffPayload>>();

// ── Public API ──

export async function diffSingleFile(
  request: FileDiffRequest,
): Promise<FileDiffPayload> {
  const key = computeRequestKey(request);

  // Check cache
  const cached = lruGet(key);
  if (cached) return cached;

  // Dedup in-flight
  const existing = inflight.get(key);
  if (existing) return existing;

  console.log("[diff] IPC call start, key=", key);
  const promise = ipcInvoke<FileDiffPayload>("diff_single_file", {
    request,
  }).then((payload) => {
    console.log("[diff] IPC resolved, payload=", !!payload);
    lruSet(key, payload);
    inflight.delete(key);
    return payload;
  }).catch((err) => {
    console.error("[diff] IPC error:", err);
    inflight.delete(key);
    throw err;
  });

  inflight.set(key, promise);
  return promise;
}

export async function diffStrings(
  oldText: string,
  newText: string,
  contextLines?: number,
): Promise<DiffHunk[]> {
  return ipcInvoke<DiffHunk[]>("diff_strings", {
    oldText,
    newText,
    contextLines: contextLines ?? null,
  });
}

export async function diffTextForLarge(
  request: FileDiffRequest,
): Promise<TextDiff> {
  return ipcInvoke<TextDiff>("diff_text_for_large", { request });
}

export async function diffSemanticTarget(
  request: SemanticTargetRequest,
): Promise<SemanticTargetInspector> {
  return ipcInvoke<SemanticTargetInspector>("diff_semantic_target", {
    request,
  });
}

// ── Request token for stale response discard ──

let tokenCounter = 0;

export function createRequestToken(): number {
  return ++tokenCounter;
}

export function isTokenStale(token: number): boolean {
  return token < tokenCounter;
}

export function invalidateDiffCache(key: string) {
  cache.delete(key);
  inflight.delete(key);
}

/**
 * Re-fetch a diff by its cache key (invalidates cache first).
 * Returns the new payload or null if the key cannot be parsed.
 */
export async function refetchDiffByKey(key: string): Promise<FileDiffPayload | null> {
  const request = parseDiffRequestKey(key);
  if (!request) return null;
  invalidateDiffCache(key);
  return diffSingleFile(request);
}

export { computeRequestKey };
