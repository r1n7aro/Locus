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
import type { WorkspaceRef } from "./project";
import { subscribeWorkspaceFileChanges } from "./workspaceExplorer";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
} from "./project";

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
  getWorkspaceRef: () => WorkspaceRef | null,
  cb: (evt: DiffProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<RoutedWorkspaceEvent<DiffProgressEvent>>(
    WORKSPACE_EVENT_NAME,
    ({ payload }) => {
      if (payload.eventName !== "diff-progress") return;
      const workspaceRef = getWorkspaceRef();
      if (!workspaceRef || payload.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && payload.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      cb(payload.payload);
    },
  );
}

// ── Request key computation (for cache + dedup) ──

function diffScopeKey(req: FileDiffRequest, workspaceRef?: WorkspaceRef | null): string {
  if (workspaceRef?.checkoutId.trim()) {
    const generation = Number.isSafeInteger(workspaceRef.expectedGeneration)
      ? String(workspaceRef.expectedGeneration)
      : "current";
    return `w=${workspaceRef.checkoutId.trim()}@${generation}`;
  }
  if (req.sessionId?.trim()) return `s=${req.sessionId.trim()}`;
  return "u";
}

function computeRequestKey(
  req: FileDiffRequest,
  workspaceRef?: WorkspaceRef | null,
): string {
  return [
    req.source,
    req.filePath,
    req.oldPath ?? "",
    req.commitHash ?? "",
    req.sessionId ?? "",
    req.assistantMessageId ?? "",
    req.detail,
    req.fullContext ? "fc" : "",
    diffScopeKey(req, workspaceRef),
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

export function parseDiffWorkspaceRefFromKey(key: string): WorkspaceRef | undefined {
  const scope = key.split(":")[8] ?? "";
  if (!scope.startsWith("w=")) return undefined;
  const separator = scope.lastIndexOf("@");
  if (separator <= 2) return undefined;
  const checkoutId = scope.slice(2, separator).trim();
  const generationRaw = scope.slice(separator + 1);
  if (!checkoutId) return undefined;
  const expectedGeneration = /^\d+$/.test(generationRaw)
    ? Number(generationRaw)
    : undefined;
  return Number.isSafeInteger(expectedGeneration)
    ? { checkoutId, expectedGeneration }
    : { checkoutId };
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
let workspaceChangeSubscriptionStarted = false;

function ensureDiffWorkspaceChangeSubscription(): void {
  if (workspaceChangeSubscriptionStarted) return;
  workspaceChangeSubscriptionStarted = true;
  void subscribeWorkspaceFileChanges((event) => {
    invalidateDiffCacheForFiles([event.payload.path], {
      checkoutId: event.checkoutId,
      expectedGeneration: event.workspaceGeneration,
    });
  });
}

// ── Public API ──

export async function diffSingleFile(
  request: FileDiffRequest,
  workspaceRef?: WorkspaceRef | null,
): Promise<FileDiffPayload> {
  ensureDiffWorkspaceChangeSubscription();
  const key = computeRequestKey(request, workspaceRef);

  // Check cache
  const cached = lruGet(key);
  if (cached) return cached;

  // Dedup in-flight
  const existing = inflight.get(key);
  if (existing) return existing;

  console.log("[diff] IPC call start, key=", key);
  const promise = ipcInvoke<FileDiffPayload>("diff_single_file", {
    request,
    workspaceRef: workspaceRef ?? null,
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
  workspaceRef?: WorkspaceRef | null,
): Promise<TextDiff> {
  return ipcInvoke<TextDiff>("diff_text_for_large", {
    request,
    workspaceRef: workspaceRef ?? null,
  });
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
 * Drop every cached/in-flight diff whose filePath or oldPath matches one of
 * the given workspace-relative paths (e.g. after a per-file revert changed
 * the worktree side of the diff).
 */
export function invalidateDiffCacheForFiles(
  paths: readonly string[],
  workspaceRef?: WorkspaceRef | null,
) {
  if (paths.length === 0) return;
  const targets = new Set(paths);
  const matches = (key: string) => {
    const request = parseDiffRequestKey(key);
    if (!request) return false;
    if (workspaceRef) {
      const keyWorkspaceRef = parseDiffWorkspaceRefFromKey(key);
      if (!keyWorkspaceRef || keyWorkspaceRef.checkoutId !== workspaceRef.checkoutId) return false;
      if (
        workspaceRef.expectedGeneration != null
        && keyWorkspaceRef.expectedGeneration !== workspaceRef.expectedGeneration
      ) return false;
    }
    return targets.has(request.filePath) || (!!request.oldPath && targets.has(request.oldPath));
  };
  for (const key of [...cache.keys()]) {
    if (matches(key)) cache.delete(key);
  }
  for (const key of [...inflight.keys()]) {
    if (matches(key)) inflight.delete(key);
  }
}

/**
 * Re-fetch a diff by its cache key (invalidates cache first).
 * Returns the new payload or null if the key cannot be parsed.
 */
export async function refetchDiffByKey(key: string): Promise<FileDiffPayload | null> {
  const request = parseDiffRequestKey(key);
  if (!request) return null;
  const workspaceRef = parseDiffWorkspaceRefFromKey(key);
  invalidateDiffCache(key);
  return diffSingleFile(request, workspaceRef);
}

export { computeRequestKey };
