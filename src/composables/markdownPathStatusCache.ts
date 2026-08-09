import { statWorkspaceEntries, type WorkspaceEntryStat } from "../services/project";
import type { MarkdownPathStatus } from "./markdownInject";

interface PendingRequest {
  workingDir: string;
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

function cacheKey(workingDir: string, path: string) {
  return `${workingDir}\u0000${path}`;
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

function cachedStatus(workingDir: string, path: string): MarkdownPathStatus | undefined {
  const key = cacheKey(workingDir, path);
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
  const latestWorkingDir = requests[requests.length - 1]?.workingDir ?? "";
  const activeRequests = requests.filter((request) => request.workingDir === latestWorkingDir);
  const paths = [...new Set(activeRequests.flatMap((request) => request.candidates))]
    .filter((path) => !cachedStatus(latestWorkingDir, path));

  if (paths.length > 0) {
    try {
      const batches = [];
      for (let index = 0; index < paths.length; index += STAT_BATCH_SIZE) {
        batches.push(statWorkspaceEntries(paths.slice(index, index + STAT_BATCH_SIZE)));
      }
      const entries = (await Promise.all(batches)).flat();
      const expiresAt = Date.now() + CACHE_TTL_MS;
      for (const entry of entries) {
        statusCache.set(cacheKey(latestWorkingDir, entry.path), {
          value: statusFromEntry(entry),
          expiresAt,
        });
      }
    } catch {
      // A missing status keeps the inline token in its neutral presentation.
    }
  }

  for (const request of requests) {
    if (request.workingDir !== latestWorkingDir) {
      request.resolve(new Map());
      continue;
    }
    const statuses = new Map<string, MarkdownPathStatus>();
    for (const candidate of request.candidates) {
      const status = cachedStatus(request.workingDir, candidate);
      if (status) statuses.set(candidate, status);
    }
    request.resolve(statuses);
  }
}

export function loadCachedMarkdownPathStatuses(
  workingDir: string,
  candidates: string[],
): Promise<Map<string, MarkdownPathStatus>> {
  if (candidates.length === 0) return Promise.resolve(new Map());
  const cached = new Map<string, MarkdownPathStatus>();
  let complete = true;
  for (const candidate of candidates) {
    const status = cachedStatus(workingDir, candidate);
    if (status) cached.set(candidate, status);
    else complete = false;
  }
  if (complete) return Promise.resolve(cached);

  return new Promise((resolve) => {
    pendingRequests.push({ workingDir, candidates, resolve });
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
