import { statWorkspaceEntries, type WorkspaceEntryStat } from "../services/project";
import type { WorkspaceRef } from "../services/project";
import type { MarkdownPathStatus } from "./markdownInject";

interface PendingRequest {
  workingDir: string;
  workspaceRef: WorkspaceRef;
  scopeKey: string;
  candidates: string[];
  resolve: (statuses: Map<string, MarkdownPathStatus>) => void;
}

interface CachedStatus {
  value: MarkdownPathStatus;
  checkedAt: number;
}

export interface CachedMarkdownPathStatuses {
  /** Every candidate with a known status, including ones due for revalidation. */
  statuses: Map<string, MarkdownPathStatus>;
  /** Candidates that have never been resolved in this scope. */
  missing: string[];
  /** Whether a known status is older than the revalidation interval. */
  stale: boolean;
}

// Statuses are revalidated after this interval, but an older status keeps
// rendering until the new one arrives: switching back to a transcript must not
// replace file references with code spans and back again.
const REVALIDATE_AFTER_MS = 2_000;
const MAX_CACHED_STATUSES = 20_000;
const STAT_BATCH_SIZE = 300;
const statusCache = new Map<string, CachedStatus>();
// Cache key → the stat that is resolving it. A request for a path already in
// flight (e.g. a streaming prefetch followed by the renderer) waits for that
// stat instead of sending the path again.
const inflightStats = new Map<string, Promise<void>>();
let pendingRequests: PendingRequest[] = [];
let flushScheduled = false;

function workspaceScopeKey(workingDir: string, workspaceRef: WorkspaceRef) {
  return `${workspaceRef.checkoutId}@${workspaceRef.expectedGeneration ?? "current"}\u0000${workingDir}`;
}

function cacheKey(scopeKey: string, path: string) {
  return `${scopeKey}\u0000${path}`;
}

function statusFromEntry(entry: WorkspaceEntryStat): MarkdownPathStatus {
  const entryKind = entry.entryKind === "folder" || entry.entryKind === "file"
    ? entry.entryKind
    : null;
  return {
    path: entry.path,
    exists: entry.exists && !!entryKind,
    entryKind,
  };
}

function cachedEntry(scopeKey: string, path: string): CachedStatus | undefined {
  return statusCache.get(cacheKey(scopeKey, path));
}

function isFresh(entry: CachedStatus): boolean {
  return Date.now() - entry.checkedAt < REVALIDATE_AFTER_MS;
}

function storeStatus(key: string, value: MarkdownPathStatus, checkedAt: number) {
  // Re-insertion keeps the map in least-recently-checked order for eviction.
  statusCache.delete(key);
  statusCache.set(key, { value, checkedAt });
  while (statusCache.size > MAX_CACHED_STATUSES) {
    const oldest = statusCache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    statusCache.delete(oldest);
  }
}

async function statPaths(scopeKey: string, paths: string[], workspaceRef: WorkspaceRef): Promise<void> {
  const keys = paths.map((path) => cacheKey(scopeKey, path));
  let settle!: () => void;
  const done = new Promise<void>((resolve) => {
    settle = resolve;
  });
  for (const key of keys) inflightStats.set(key, done);
  try {
    const batches = [];
    for (let index = 0; index < paths.length; index += STAT_BATCH_SIZE) {
      batches.push(statWorkspaceEntries(paths.slice(index, index + STAT_BATCH_SIZE), workspaceRef));
    }
    const entries = (await Promise.all(batches)).flat();
    const checkedAt = Date.now();
    for (const entry of entries) {
      storeStatus(cacheKey(scopeKey, entry.path), statusFromEntry(entry), checkedAt);
    }
  } catch {
    // A failed stat keeps the last known statuses; unknown tokens stay neutral.
  } finally {
    for (const key of keys) {
      if (inflightStats.get(key) === done) inflightStats.delete(key);
    }
    settle();
  }
}

async function flushPendingRequests() {
  flushScheduled = false;
  const requests = pendingRequests;
  pendingRequests = [];
  const latestRequest = requests[requests.length - 1];
  if (!latestRequest) return;
  const latestScopeKey = latestRequest.scopeKey;
  const activeRequests = requests.filter((request) => request.scopeKey === latestScopeKey);
  const waits = new Set<Promise<void>>();
  const paths: string[] = [];
  for (const path of new Set(activeRequests.flatMap((request) => request.candidates))) {
    const entry = cachedEntry(latestScopeKey, path);
    if (entry && isFresh(entry)) continue;
    const inflight = inflightStats.get(cacheKey(latestScopeKey, path));
    if (inflight) waits.add(inflight);
    else paths.push(path);
  }

  if (paths.length > 0) waits.add(statPaths(latestScopeKey, paths, latestRequest.workspaceRef));
  if (waits.size > 0) await Promise.all(waits);

  for (const request of requests) {
    if (request.scopeKey !== latestScopeKey) {
      request.resolve(new Map());
      continue;
    }
    const statuses = new Map<string, MarkdownPathStatus>();
    for (const candidate of request.candidates) {
      const entry = cachedEntry(request.scopeKey, candidate);
      if (entry) statuses.set(candidate, entry.value);
    }
    request.resolve(statuses);
  }
}

/** Reads cached statuses synchronously, so a renderer can paint them in its first frame. */
export function peekCachedMarkdownPathStatuses(
  workingDir: string,
  candidates: string[],
  workspaceRef: WorkspaceRef,
): CachedMarkdownPathStatuses {
  const scopeKey = workspaceScopeKey(workingDir, workspaceRef);
  const statuses = new Map<string, MarkdownPathStatus>();
  const missing: string[] = [];
  let stale = false;
  for (const candidate of candidates) {
    const entry = cachedEntry(scopeKey, candidate);
    if (!entry) {
      missing.push(candidate);
      continue;
    }
    statuses.set(candidate, entry.value);
    if (!isFresh(entry)) stale = true;
  }
  return { statuses, missing, stale };
}

export function loadCachedMarkdownPathStatuses(
  workingDir: string,
  candidates: string[],
  workspaceRef: WorkspaceRef,
): Promise<Map<string, MarkdownPathStatus>> {
  const scopeKey = workspaceScopeKey(workingDir, workspaceRef);
  if (candidates.length === 0) return Promise.resolve(new Map());
  const cached = new Map<string, MarkdownPathStatus>();
  let complete = true;
  for (const candidate of candidates) {
    const entry = cachedEntry(scopeKey, candidate);
    if (entry && isFresh(entry)) cached.set(candidate, entry.value);
    else complete = false;
  }
  if (complete) return Promise.resolve(cached);

  return new Promise((resolve) => {
    pendingRequests.push({ workingDir, workspaceRef, scopeKey, candidates, resolve });
    if (flushScheduled) return;
    flushScheduled = true;
    queueMicrotask(() => {
      void flushPendingRequests();
    });
  });
}

/**
 * Resolves the statuses of paths a transcript is about to render. Returns null
 * when every candidate already has a known status, so the caller can render in
 * the same task; otherwise the promise settles after one batched stat.
 */
export function prefetchMarkdownPathStatuses(
  workingDir: string,
  candidates: string[],
  workspaceRef: WorkspaceRef,
): Promise<void> | null {
  const { missing } = peekCachedMarkdownPathStatuses(workingDir, candidates, workspaceRef);
  if (missing.length === 0) return null;
  return loadCachedMarkdownPathStatuses(workingDir, missing, workspaceRef).then(() => undefined);
}

export function clearMarkdownPathStatusCache() {
  statusCache.clear();
  inflightStats.clear();
}
