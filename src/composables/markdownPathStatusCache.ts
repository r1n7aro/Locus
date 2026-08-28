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
  expiresAt: number;
}

const CACHE_TTL_MS = 2_000;
const STAT_BATCH_SIZE = 300;
const statusCache = new Map<string, CachedStatus>();
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

function cachedStatus(scopeKey: string, path: string): MarkdownPathStatus | undefined {
  const key = cacheKey(scopeKey, path);
  const cached = statusCache.get(key);
  if (!cached) return undefined;
  if (cached.expiresAt <= Date.now()) {
    statusCache.delete(key);
    return undefined;
  }
  return cached.value;
}

async function flushPendingRequests() {
  flushScheduled = false;
  const requests = pendingRequests;
  pendingRequests = [];
  const latestRequest = requests[requests.length - 1];
  if (!latestRequest) return;
  const latestScopeKey = latestRequest.scopeKey;
  const activeRequests = requests.filter((request) => request.scopeKey === latestScopeKey);
  const paths = [...new Set(activeRequests.flatMap((request) => request.candidates))]
    .filter((path) => !cachedStatus(latestScopeKey, path));

  if (paths.length > 0) {
    try {
      const batches = [];
      for (let index = 0; index < paths.length; index += STAT_BATCH_SIZE) {
        batches.push(statWorkspaceEntries(
          paths.slice(index, index + STAT_BATCH_SIZE),
          latestRequest.workspaceRef,
        ));
      }
      const entries = (await Promise.all(batches)).flat();
      const expiresAt = Date.now() + CACHE_TTL_MS;
      for (const entry of entries) {
        statusCache.set(cacheKey(latestScopeKey, entry.path), {
          value: statusFromEntry(entry),
          expiresAt,
        });
      }
    } catch {
      // A missing status keeps the inline token in its neutral presentation.
    }
  }

  for (const request of requests) {
    if (request.scopeKey !== latestScopeKey) {
      request.resolve(new Map());
      continue;
    }
    const statuses = new Map<string, MarkdownPathStatus>();
    for (const candidate of request.candidates) {
      const status = cachedStatus(request.scopeKey, candidate);
      if (status) statuses.set(candidate, status);
    }
    request.resolve(statuses);
  }
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
    const status = cachedStatus(scopeKey, candidate);
    if (status) cached.set(candidate, status);
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

export function clearMarkdownPathStatusCache() {
  statusCache.clear();
}
