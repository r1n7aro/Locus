import type { WorkspaceRef } from "../services/project";

const CATALOG_TTL_MS = 30_000;
const MAX_CATALOG_ENTRIES = 64;

interface CatalogEntry {
  value: unknown;
  cachedAt: number;
}

const catalogEntries = new Map<string, CatalogEntry>();
const pendingCatalogReads = new Map<string, Promise<unknown>>();
const catalogEpochs = new Map<string, number>();

function normalizeWorkspacePath(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/\/+/g, "/").toLocaleLowerCase();
}

function scopeKey(workingDir: string, workspaceRef: WorkspaceRef): string {
  return [
    normalizeWorkspacePath(workingDir),
    workspaceRef.checkoutId,
    workspaceRef.expectedGeneration ?? "",
  ].join("|");
}

function requestKey(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  segment: string,
): string {
  return [scopeKey(workingDir, workspaceRef), segment].join("|");
}

function trimCatalogEntries(): void {
  while (catalogEntries.size > MAX_CATALOG_ENTRIES) {
    const oldest = catalogEntries.keys().next().value as string | undefined;
    if (!oldest) break;
    catalogEntries.delete(oldest);
  }
}

export async function readKnowledgeCatalogCached<T>(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  segment: string,
  load: () => Promise<T>,
  options?: { force?: boolean },
): Promise<T> {
  const key = requestKey(workingDir, workspaceRef, segment);
  if (!options?.force) {
    const cached = catalogEntries.get(key);
    if (cached && Date.now() - cached.cachedAt < CATALOG_TTL_MS) {
      catalogEntries.delete(key);
      catalogEntries.set(key, cached);
      return cached.value as T;
    }
  }

  const existing = pendingCatalogReads.get(key);
  if (existing) return existing as Promise<T>;

  const loadCurrentEpoch = async (epoch: number): Promise<T> => {
    const value = await load();
    const currentEpoch = catalogEpochs.get(key) ?? 0;
    if (currentEpoch !== epoch) return loadCurrentEpoch(currentEpoch);
    catalogEntries.delete(key);
    catalogEntries.set(key, { value, cachedAt: Date.now() });
    trimCatalogEntries();
    return value;
  };
  const pending = loadCurrentEpoch(catalogEpochs.get(key) ?? 0);
  pendingCatalogReads.set(key, pending);
  try {
    return await pending;
  } finally {
    if (pendingCatalogReads.get(key) === pending) pendingCatalogReads.delete(key);
  }
}

export function invalidateKnowledgeCatalogCache(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  segmentPrefix?: string,
): void {
  const prefix = scopeKey(workingDir, workspaceRef) + "|";
  const invalidatedKeys = new Set<string>();
  for (const key of catalogEntries.keys()) {
    if (!key.startsWith(prefix)) continue;
    if (segmentPrefix && !key.slice(prefix.length).startsWith(segmentPrefix)) continue;
    invalidatedKeys.add(key);
    catalogEntries.delete(key);
  }
  for (const key of pendingCatalogReads.keys()) {
    if (!key.startsWith(prefix)) continue;
    if (segmentPrefix && !key.slice(prefix.length).startsWith(segmentPrefix)) continue;
    invalidatedKeys.add(key);
  }
  for (const key of invalidatedKeys) {
    catalogEpochs.set(key, (catalogEpochs.get(key) ?? 0) + 1);
  }
}

export function clearKnowledgeCatalogCacheForTests(): void {
  catalogEntries.clear();
  pendingCatalogReads.clear();
  catalogEpochs.clear();
}
