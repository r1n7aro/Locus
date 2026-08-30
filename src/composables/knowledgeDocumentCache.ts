import type {
  KnowledgeDocument,
  KnowledgeDocumentType,
} from "../types";
import type { WorkspaceRef } from "../services/project";

const MAX_CACHED_DOCUMENTS = 48;
const MAX_CACHED_CHARACTERS = 8 * 1024 * 1024;
const MAX_ENRICHMENT_METADATA_KEYS = MAX_CACHED_DOCUMENTS * 4;

interface CachedKnowledgeDocument {
  document: KnowledgeDocument;
  characters: number;
  touchedAt: number;
}

interface KnowledgeDocumentTarget {
  type: KnowledgeDocumentType;
  path: string;
}

const documentCache = new Map<string, CachedKnowledgeDocument>();
const pendingReads = new Map<string, Promise<KnowledgeDocument>>();
const pendingEnrichments = new Map<string, Promise<void>>();
const enrichedAt = new Map<string, number>();
const cacheEpochs = new Map<string, number>();
let cachedCharacters = 0;
const ENRICHMENT_TTL_MS = 5 * 60 * 1000;

function normalizePath(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/\/+/g, "/");
}

function workspaceScopeKey(
  workingDir: string,
  workspaceRef: WorkspaceRef,
): string {
  return [
    normalizePath(workingDir).toLocaleLowerCase(),
    workspaceRef.checkoutId,
    workspaceRef.expectedGeneration ?? "",
  ].join("|");
}

function targetKey(target: KnowledgeDocumentTarget): string {
  return [target.type, normalizePath(target.path)].join(":");
}

function cacheKey(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target: KnowledgeDocumentTarget,
): string {
  return [workspaceScopeKey(workingDir, workspaceRef), targetKey(target)].join("|");
}

function pathMatchesSubtree(candidate: string, root: string): boolean {
  const normalizedCandidate = normalizePath(candidate).replace(/^\/+|\/+$/g, "");
  const normalizedRoot = normalizePath(root).replace(/^\/+|\/+$/g, "");
  if (!normalizedRoot) return true;
  return normalizedCandidate === normalizedRoot
    || normalizedCandidate.startsWith(`${normalizedRoot}/`);
}

function documentCharacters(document: KnowledgeDocument): number {
  return (document.summary?.length ?? 0)
    + (document.maintenanceRules?.length ?? 0)
    + document.body.length;
}

function touchEntry(key: string, entry: CachedKnowledgeDocument): void {
  documentCache.delete(key);
  entry.touchedAt = Date.now();
  documentCache.set(key, entry);
}

function releaseEpochIfUnused(key: string): void {
  if (
    documentCache.has(key)
    || pendingReads.has(key)
    || pendingEnrichments.has(key)
    || enrichedAt.has(key)
  ) return;
  cacheEpochs.delete(key);
}

function trimEnrichmentMetadata(): void {
  if (enrichedAt.size <= MAX_ENRICHMENT_METADATA_KEYS) return;
  for (const key of enrichedAt.keys()) {
    if (pendingEnrichments.has(key)) continue;
    enrichedAt.delete(key);
    releaseEpochIfUnused(key);
    if (enrichedAt.size <= MAX_ENRICHMENT_METADATA_KEYS) break;
  }
}

function trimCache(): void {
  if (
    documentCache.size <= MAX_CACHED_DOCUMENTS
    && cachedCharacters <= MAX_CACHED_CHARACTERS
  ) return;

  for (const [key, entry] of documentCache) {
    documentCache.delete(key);
    cachedCharacters = Math.max(0, cachedCharacters - entry.characters);
    enrichedAt.delete(key);
    releaseEpochIfUnused(key);
    if (
      documentCache.size <= MAX_CACHED_DOCUMENTS
      && cachedCharacters <= MAX_CACHED_CHARACTERS
    ) break;
  }
}

export function getCachedKnowledgeDocument(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target: KnowledgeDocumentTarget,
): KnowledgeDocument | null {
  const key = cacheKey(workingDir, workspaceRef, target);
  const entry = documentCache.get(key);
  if (!entry) return null;
  touchEntry(key, entry);
  return entry.document;
}

export function cacheKnowledgeDocument(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  document: KnowledgeDocument,
): void {
  const key = cacheKey(workingDir, workspaceRef, document);
  const previous = documentCache.get(key);
  if (previous) cachedCharacters = Math.max(0, cachedCharacters - previous.characters);
  const characters = documentCharacters(document);
  documentCache.delete(key);
  documentCache.set(key, {
    document,
    characters,
    touchedAt: Date.now(),
  });
  cachedCharacters += characters;
  trimCache();
}

export async function readKnowledgeDocumentCached(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target: KnowledgeDocumentTarget,
  load: () => Promise<KnowledgeDocument>,
  options?: { force?: boolean },
): Promise<KnowledgeDocument> {
  const key = cacheKey(workingDir, workspaceRef, target);
  if (!options?.force) {
    const cached = getCachedKnowledgeDocument(workingDir, workspaceRef, target);
    if (cached) return cached;
  }

  // A forced background revalidation still shares an in-flight filesystem
  // read. This is what keeps several visible panes from multiplying the same
  // work after they all hit the shared document cache.
  const existing = pendingReads.get(key);
  if (existing) return existing;

  const loadCurrentEpoch = async (epoch: number): Promise<KnowledgeDocument> => {
    const document = await load();
    const currentEpoch = cacheEpochs.get(key) ?? 0;
    if (currentEpoch !== epoch) {
      // The resource changed while the read was running. Keep every waiter on
      // this same Promise, but retry before exposing or caching stale content.
      return loadCurrentEpoch(currentEpoch);
    }
    cacheKnowledgeDocument(workingDir, workspaceRef, document);
    return document;
  };
  const pending = loadCurrentEpoch(cacheEpochs.get(key) ?? 0);
  pendingReads.set(key, pending);
  try {
    return await pending;
  } finally {
    if (pendingReads.get(key) === pending) pendingReads.delete(key);
    releaseEpochIfUnused(key);
  }
}

export function runKnowledgeDocumentEnrichment<T = void>(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target: KnowledgeDocumentTarget,
  work: () => Promise<T>,
  commit?: (value: T) => void | Promise<void>,
): Promise<void> {
  const key = cacheKey(workingDir, workspaceRef, target);
  const completedAt = enrichedAt.get(key) ?? 0;
  if (Date.now() - completedAt < ENRICHMENT_TTL_MS) return Promise.resolve();
  const existing = pendingEnrichments.get(key);
  if (existing) return existing;
  const epoch = cacheEpochs.get(key) ?? 0;
  const pending = work().then(async (value) => {
    if ((cacheEpochs.get(key) ?? 0) !== epoch) return;
    await commit?.(value);
    if ((cacheEpochs.get(key) ?? 0) === epoch) {
      // Refresh insertion order as well as the timestamp so the metadata map
      // has a real LRU boundary independent of the document-body budget.
      enrichedAt.delete(key);
      enrichedAt.set(key, Date.now());
      trimEnrichmentMetadata();
    }
  });
  pendingEnrichments.set(key, pending);
  return pending.finally(() => {
    if (pendingEnrichments.get(key) === pending) pendingEnrichments.delete(key);
    releaseEpochIfUnused(key);
  });
}

export function invalidateKnowledgeDocumentCache(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target?: KnowledgeDocumentTarget | null,
): void {
  const scope = workspaceScopeKey(workingDir, workspaceRef) + "|";
  if (target) {
    const key = scope + targetKey(target);
    if (pendingReads.has(key) || pendingEnrichments.has(key)) {
      cacheEpochs.set(key, (cacheEpochs.get(key) ?? 0) + 1);
    } else {
      cacheEpochs.delete(key);
    }
    const entry = documentCache.get(key);
    if (entry) cachedCharacters = Math.max(0, cachedCharacters - entry.characters);
    documentCache.delete(key);
    enrichedAt.delete(key);
    return;
  }

  const invalidatedKeys = new Set<string>();
  for (const [key, entry] of documentCache) {
    if (!key.startsWith(scope)) continue;
    invalidatedKeys.add(key);
    documentCache.delete(key);
    cachedCharacters = Math.max(0, cachedCharacters - entry.characters);
  }
  for (const key of pendingReads.keys()) {
    if (key.startsWith(scope)) invalidatedKeys.add(key);
  }
  for (const key of pendingEnrichments.keys()) {
    if (key.startsWith(scope)) invalidatedKeys.add(key);
  }
  for (const key of enrichedAt.keys()) {
    if (!key.startsWith(scope)) continue;
    invalidatedKeys.add(key);
    enrichedAt.delete(key);
  }
  for (const key of cacheEpochs.keys()) {
    if (key.startsWith(scope)) invalidatedKeys.add(key);
  }
  for (const key of invalidatedKeys) {
    if (pendingReads.has(key) || pendingEnrichments.has(key)) {
      cacheEpochs.set(key, (cacheEpochs.get(key) ?? 0) + 1);
    } else {
      cacheEpochs.delete(key);
    }
  }
}

/**
 * Invalidates every cached document beneath a directory/config target while
 * keeping unrelated documents in the same knowledge type warm.
 */
export function invalidateKnowledgeDocumentCacheSubtree(
  workingDir: string,
  workspaceRef: WorkspaceRef,
  target: KnowledgeDocumentTarget,
): void {
  const scope = workspaceScopeKey(workingDir, workspaceRef) + "|";
  const typedPrefix = `${scope}${target.type}:`;
  const matches = (key: string) => key.startsWith(typedPrefix)
    && pathMatchesSubtree(key.slice(typedPrefix.length), target.path);
  const invalidatedKeys = new Set<string>();

  for (const [key, entry] of documentCache) {
    if (!matches(key)) continue;
    invalidatedKeys.add(key);
    documentCache.delete(key);
    cachedCharacters = Math.max(0, cachedCharacters - entry.characters);
  }
  for (const key of pendingReads.keys()) {
    if (matches(key)) invalidatedKeys.add(key);
  }
  for (const key of pendingEnrichments.keys()) {
    if (matches(key)) invalidatedKeys.add(key);
  }
  for (const key of enrichedAt.keys()) {
    if (!matches(key)) continue;
    invalidatedKeys.add(key);
    enrichedAt.delete(key);
  }
  for (const key of cacheEpochs.keys()) {
    if (matches(key)) invalidatedKeys.add(key);
  }

  for (const key of invalidatedKeys) {
    if (pendingReads.has(key) || pendingEnrichments.has(key)) {
      cacheEpochs.set(key, (cacheEpochs.get(key) ?? 0) + 1);
    } else {
      cacheEpochs.delete(key);
    }
  }
}

export function clearKnowledgeDocumentCacheForTests(): void {
  documentCache.clear();
  pendingReads.clear();
  pendingEnrichments.clear();
  enrichedAt.clear();
  cacheEpochs.clear();
  cachedCharacters = 0;
}

export function knowledgeDocumentCacheStats(): {
  documents: number;
  characters: number;
  pendingReads: number;
  pendingEnrichments: number;
  enrichmentMetadata: number;
  epochs: number;
} {
  return {
    documents: documentCache.size,
    characters: cachedCharacters,
    pendingReads: pendingReads.size,
    pendingEnrichments: pendingEnrichments.size,
    enrichmentMetadata: enrichedAt.size,
    epochs: cacheEpochs.size,
  };
}
