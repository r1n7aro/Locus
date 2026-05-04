import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  assetDbOverview,
  assetDbScan,
  searchWorkspaceAssets,
  previewWorkspaceAsset,
  previewWorkspaceAssetTarget,
  getWatcherTuning,
  setWatcherTuning,
} from "../services/asset";
import { listDirEntriesPage } from "../services/project";
import { normalizeAppError } from "../services/errors";
import { getWarmup } from "./warmupCache";
import { acquireSelectionLock } from "./useSelectionLock";
import type {
  AssetDbOverview,
  AssetSearchResult,
  AssetPreviewPayload,
  AssetDbScanEvent,
  SemanticTargetInspector,
  WatcherTuning,
} from "../types";

interface AssetProps {
  workingDir: string;
}

// ── Explorer node ──────────────────────────────────────────
export type AssetExplorerNode =
  | {
      kind: "folder";
      name: string;
      path: string; // workspace-relative, forward slashes
      depth: number;
      isRoot: boolean;
      loaded: boolean;
      loading: boolean;
      hasMore: boolean;
      nextOffset: number;
      totalCount: number;
      hasChildFoldersKnown: boolean;
      hasChildFolders: boolean;
      branchProbeLoading: boolean;
      children: AssetExplorerNode[];
    }
  | {
      kind: "file";
      name: string;
      path: string;
      depth: number;
    };

type AssetPreviewFileNode = Extract<AssetExplorerNode, { kind: "file" }>;
type AssetFolderNode = Extract<AssetExplorerNode, { kind: "folder" }>;

const FIXED_ROOTS = ["Assets", "Packages", "ProjectSettings"] as const;
const ASSET_EXPLORER_PAGE_SIZE = 200;
const ASSET_EXPLORER_BRANCH_PROBE_PAGE_SIZE = 1;
const ASSET_EXPLORER_BRANCH_PROBE_CONCURRENCY = 8;
type ViewMode = "stats" | "preview";
type AssetSearchScope = "folder" | "global";
type FolderRevealMode = "none" | "ancestors" | "self";

export function useAssetState(props: AssetProps) {
  // ── Reactive state ────────────────────────────────────────
  const loading = ref(false);
  const error = ref("");
  const sidebarWidth = ref(240);
  const directoryPaneWidth = ref(320);

  const explorerTree = ref<AssetExplorerNode[]>([]);
  const expandedPaths = ref<Set<string>>(new Set());
  const selectedFolderPath = ref<string | null>(null);
  const selectedNode = ref<AssetExplorerNode | null>(null);

  const viewMode = ref<ViewMode>("stats");

  // search
  const searchQuery = ref("");
  const searchScope = ref<AssetSearchScope>("folder");
  const searchResults = ref<AssetSearchResult[]>([]);
  const searchTruncated = ref(false);
  const searchHasFallback = ref(false);
  const searching = ref(false);

  // preview
  const previewPayload = ref<AssetPreviewPayload | null>(null);
  const previewNode = ref<AssetPreviewFileNode | null>(null);
  const previewLoading = ref(false);
  const previewError = ref("");
  const activeTargetId = ref<string | null>(null);
  const targetCache = ref<Map<string, SemanticTargetInspector>>(new Map());
  const targetLoading = ref(false);
  let previewSession = 0;
  let targetRequestGeneration = 0;

  function invalidatePreviewSession(): number {
    previewSession += 1;
    targetRequestGeneration += 1;
    return previewSession;
  }

  function clearPreviewState() {
    previewPayload.value = null;
    previewNode.value = null;
    previewLoading.value = false;
    previewError.value = "";
    activeTargetId.value = null;
    targetCache.value = new Map();
    targetLoading.value = false;
  }

  function toPreviewFileNode(file: string | AssetPreviewFileNode): AssetPreviewFileNode {
    if (typeof file !== "string") return file;
    const segments = file.split("/").filter(Boolean);
    return {
      kind: "file",
      name: segments[segments.length - 1] ?? file,
      path: file,
      depth: Math.max(0, segments.length - 1),
    };
  }

  function isPrefabPath(path: string): boolean {
    return path.toLowerCase().endsWith(".prefab");
  }

  function defaultPrefabRootTargetId(payload: AssetPreviewPayload, assetPath: string): string | null {
    if (!isPrefabPath(assetPath) || payload.kind !== "structured") return null;
    const knownIds = new Set(payload.tree.map((node) => node.id));
    const root = payload.tree.find((node) =>
      node.hasInspector && (!node.parentId || !knownIds.has(node.parentId)),
    );
    return root?.id ?? null;
  }

  // db overview
  const dbOverview = ref<AssetDbOverview | null>(null);
  const dbLoading = ref(false);

  // watcher tuning
  const watcherTuning = ref<WatcherTuning | null>(null);
  const watcherTuningSaving = ref(false);
  const hasWorkspace = computed(() => !!props.workingDir.trim());

  function resetWorkspaceState() {
    invalidatePreviewSession();
    explorerTree.value = [];
    expandedPaths.value = new Set();
    selectedFolderPath.value = null;
    selectedNode.value = null;
    viewMode.value = "stats";
    searchQuery.value = "";
    searchScope.value = "folder";
    directoryPaneWidth.value = 320;
    searchResults.value = [];
    searchTruncated.value = false;
    searchHasFallback.value = false;
    searching.value = false;
    clearPreviewState();
    dbOverview.value = null;
    dbLoading.value = false;
    watcherTuning.value = null;
    watcherTuningSaving.value = false;
    error.value = "";
  }

  // ── Explorer ──────────────────────────────────────────────
  function initRoots() {
    explorerTree.value = FIXED_ROOTS.map((name) => ({
      kind: "folder",
      name,
      path: name,
      depth: 0,
      isRoot: true,
      loaded: false,
      loading: false,
      hasMore: false,
      nextOffset: 0,
      totalCount: 0,
      hasChildFoldersKnown: false,
      hasChildFolders: false,
      branchProbeLoading: false,
      children: [],
    }));
    selectedFolderPath.value = FIXED_ROOTS[0];
  }

  function isPathExpanded(path: string): boolean {
    return expandedPaths.value.has(path);
  }

  function findNodeByPath(path: string): AssetExplorerNode | null {
    function walk(nodes: AssetExplorerNode[]): AssetExplorerNode | null {
      for (const n of nodes) {
        if (n.path === path) return n;
        if (n.kind === "folder") {
          const found = walk(n.children);
          if (found) return found;
        }
      }
      return null;
    }
    return walk(explorerTree.value);
  }

  function parentFolderPath(path: string): string | null {
    const segments = path.split("/").filter(Boolean);
    if (segments.length <= 1) return segments[0] ?? null;
    return segments.slice(0, -1).join("/");
  }

  function collapseExpandedBranch(path: string) {
    const prefix = `${path}/`;
    let changed = false;
    const next = new Set<string>();
    for (const expandedPath of expandedPaths.value) {
      if (expandedPath === path || expandedPath.startsWith(prefix)) {
        changed = true;
        continue;
      }
      next.add(expandedPath);
    }
    if (changed) {
      expandedPaths.value = next;
    }
  }

  function createFolderNode(
    name: string,
    path: string,
    depth: number,
    isRoot: boolean,
  ): AssetFolderNode {
    return {
      kind: "folder",
      name,
      path,
      depth,
      isRoot,
      loaded: false,
      loading: false,
      hasMore: false,
      nextOffset: 0,
      totalCount: 0,
      hasChildFoldersKnown: false,
      hasChildFolders: false,
      branchProbeLoading: false,
      children: [],
    };
  }

  function createFileNode(
    name: string,
    path: string,
    depth: number,
  ): AssetExplorerNode {
    return {
      kind: "file",
      name,
      path,
      depth,
    };
  }

  function assignFolderPage(
    folder: AssetFolderNode,
    page: Awaited<ReturnType<typeof listDirEntriesPage>>,
    append: boolean,
  ) {
    const children = page.entries.map((entry) =>
      entry.isDir
        ? createFolderNode(entry.name, entry.relPath, folder.depth + 1, false)
        : createFileNode(entry.name, entry.relPath, folder.depth + 1),
    );

    folder.children = append ? [...folder.children, ...children] : children;
    folder.loaded = true;
    folder.hasMore = page.hasMore;
    folder.nextOffset = page.nextOffset;
    folder.totalCount = page.totalCount;
    folder.hasChildFoldersKnown = true;
    folder.hasChildFolders = folder.children.some((child) => child.kind === "folder");
    if (!folder.hasChildFolders) {
      collapseExpandedBranch(folder.path);
    }
  }

  async function loadFolderChildren(
    folder: AssetFolderNode,
    options: { append?: boolean } = {},
  ) {
    if (!hasWorkspace.value) return;
    if (folder.loading) return;
    if (!options.append && folder.loaded) return;
    if (options.append && !folder.hasMore) return;
    folder.loading = true;
    try {
      const page = await listDirEntriesPage(
        folder.path,
        options.append ? folder.nextOffset : 0,
        ASSET_EXPLORER_PAGE_SIZE,
        true,
      );
      assignFolderPage(folder, page, !!options.append);
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      folder.loading = false;
    }
  }

  async function probeFolderBranchState(folder: AssetFolderNode) {
    if (!hasWorkspace.value) return;
    if (folder.loaded) {
      folder.hasChildFoldersKnown = true;
      folder.hasChildFolders = folder.children.some((child) => child.kind === "folder");
      return;
    }
    if (folder.hasChildFoldersKnown || folder.branchProbeLoading) return;

    folder.branchProbeLoading = true;
    try {
      const page = await listDirEntriesPage(
        folder.path,
        0,
        ASSET_EXPLORER_BRANCH_PROBE_PAGE_SIZE,
        true,
      );
      folder.hasChildFoldersKnown = true;
      folder.hasChildFolders = page.entries[0]?.isDir === true;
      if (!folder.hasChildFolders) {
        collapseExpandedBranch(folder.path);
      }
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      folder.branchProbeLoading = false;
    }
  }

  async function prefetchChildFolderBranchState(folder: AssetFolderNode) {
    if (!hasWorkspace.value) return;
    const childFolders = folder.children.filter(
      (child): child is AssetFolderNode => child.kind === "folder",
    );
    if (!childFolders.length) return;

    let cursor = 0;
    const workerCount = Math.min(ASSET_EXPLORER_BRANCH_PROBE_CONCURRENCY, childFolders.length);
    const workers = Array.from({ length: workerCount }, async () => {
      while (cursor < childFolders.length) {
        const nextIndex = cursor;
        cursor += 1;
        await probeFolderBranchState(childFolders[nextIndex]);
      }
    });

    await Promise.all(workers);
    explorerTree.value = [...explorerTree.value];
  }

  async function prefetchRootFolderBranchState() {
    if (!hasWorkspace.value) return;
    const rootFolders = explorerTree.value.filter(
      (node): node is AssetFolderNode => node.kind === "folder",
    );
    if (!rootFolders.length) return;
    await Promise.all(rootFolders.map((folder) => probeFolderBranchState(folder)));
    explorerTree.value = [...explorerTree.value];
  }

  async function probeFolderPath(path: string) {
    if (!hasWorkspace.value) return;
    const node = findNodeByPath(path);
    if (!node || node.kind !== "folder") return;
    await probeFolderBranchState(node);
    explorerTree.value = [...explorerTree.value];
  }

  async function togglePath(path: string) {
    if (!hasWorkspace.value) return;
    const node = findNodeByPath(path);
    if (!node || node.kind !== "folder") return;
    if (expandedPaths.value.has(path)) {
      const set = new Set(expandedPaths.value);
      set.delete(path);
      expandedPaths.value = set;
      explorerTree.value = [...explorerTree.value];
      return;
    }

    const set = new Set(expandedPaths.value);
    set.add(path);
    expandedPaths.value = set;
    if (!node.loaded) await loadFolderChildren(node);
    if (!node.hasChildFoldersKnown || node.hasChildFolders) {
      void prefetchChildFolderBranchState(node);
    }
    // trigger reactivity for the children mutation
    explorerTree.value = [...explorerTree.value];
  }

  async function loadMoreFolder(path: string) {
    if (!hasWorkspace.value) return;
    const node = findNodeByPath(path);
    if (!node || node.kind !== "folder") return;
    await loadFolderChildren(node, { append: true });
    if (isPathExpanded(path)) {
      void prefetchChildFolderBranchState(node);
    }
    explorerTree.value = [...explorerTree.value];
  }

  async function loadCurrentFolderMore() {
    if (!selectedFolderPath.value) return;
    await loadMoreFolder(selectedFolderPath.value);
  }

  async function expandToPath(path: string) {
    if (!hasWorkspace.value) return;
    // Expand each ancestor and ensure children are loaded.
    const segments = path.split("/").filter(Boolean);
    let current = "";
    for (let i = 0; i < segments.length - 1; i++) {
      current = current ? `${current}/${segments[i]}` : segments[i];
      const node = findNodeByPath(current);
      if (!node || node.kind !== "folder") continue;
      if (!node.loaded) await loadFolderChildren(node);
      const set = new Set(expandedPaths.value);
      set.add(current);
      expandedPaths.value = set;
    }
    explorerTree.value = [...explorerTree.value];
  }

  async function expandFolderPath(path: string, includeSelf = true) {
    if (!hasWorkspace.value) return;
    const segments = path.split("/").filter(Boolean);
    if (!segments.length) return;
    let current = "";
    const lastIndex = includeSelf ? segments.length - 1 : segments.length - 2;
    for (let i = 0; i <= lastIndex; i++) {
      if (i < 0) continue;
      current = current ? `${current}/${segments[i]}` : segments[i];
      const node = findNodeByPath(current);
      if (!node || node.kind !== "folder") continue;
      if (!node.loaded) await loadFolderChildren(node);
      const set = new Set(expandedPaths.value);
      set.add(current);
      expandedPaths.value = set;
    }
    explorerTree.value = [...explorerTree.value];
  }

  async function materializePath(path: string) {
    const segments = path.split("/").filter(Boolean);
    if (segments.length <= 1) return;
    const parentPath = segments.slice(0, -1).join("/");
    const filePath = segments.join("/");
    const parentNode = findNodeByPath(parentPath);
    if (!parentNode || parentNode.kind !== "folder") return;

    while (
      parentNode.hasMore
      && !parentNode.children.some((child) => child.path === filePath)
    ) {
      await loadFolderChildren(parentNode, { append: true });
    }
    explorerTree.value = [...explorerTree.value];
  }

  async function selectFolder(
    path: string,
    options: { preservePreview?: boolean; revealInTree?: FolderRevealMode } = {},
  ) {
    if (!hasWorkspace.value) return;
    const revealMode = options.revealInTree ?? "none";
    if (revealMode === "self") {
      await expandFolderPath(path, true);
    } else if (revealMode === "ancestors") {
      await expandFolderPath(path, false);
    }
    const node = findNodeByPath(path);
    if (!node || node.kind !== "folder") return;
    if (!node.loaded) {
      await loadFolderChildren(node);
    }
    selectedFolderPath.value = node.path;
    if (!options.preservePreview) {
      closePreview();
    }
    explorerTree.value = [...explorerTree.value];
  }

  async function selectNode(node: AssetExplorerNode) {
    if (!hasWorkspace.value) return;
    if (node.kind === "folder") {
      await selectFolder(node.path, { revealInTree: "ancestors" });
      return;
    }
    const parentPath = parentFolderPath(node.path);
    if (parentPath) {
      selectedFolderPath.value = parentPath;
    }
    selectedNode.value = node;
    viewMode.value = "preview";
    await loadPreview(node);
  }

  function closePreview() {
    invalidatePreviewSession();
    selectedNode.value = null;
    clearPreviewState();
    viewMode.value = "stats";
  }

  // ── Search ───────────────────────────────────────────────
  let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  function runFilenameSearch(query: string) {
    searchQuery.value = query;
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    if (!hasWorkspace.value) {
      searchResults.value = [];
      searchTruncated.value = false;
      searchHasFallback.value = false;
      searching.value = false;
      return;
    }
    if (!query.trim()) {
      searchResults.value = [];
      searchTruncated.value = false;
      searchHasFallback.value = false;
      searching.value = false;
      return;
    }
    if (searchScope.value !== "global") {
      searchResults.value = [];
      searchTruncated.value = false;
      searchHasFallback.value = false;
      searching.value = false;
      return;
    }
    searchDebounceTimer = setTimeout(async () => {
      if (!hasWorkspace.value) {
        searching.value = false;
        searchResults.value = [];
        return;
      }
      searching.value = true;
      try {
        const results = await searchWorkspaceAssets(query, [
          "Assets",
          "Packages",
          "ProjectSettings",
        ]);
        searchResults.value = results;
        searchTruncated.value = results.length === 200;
        searchHasFallback.value = results.some((r) => r.source === "filesystem");
      } catch (e) {
        const err = normalizeAppError(e);
        error.value = err.message;
        searchResults.value = [];
      } finally {
        searching.value = false;
      }
    }, 200);
  }

  function updateSearchScope(scope: AssetSearchScope) {
    searchScope.value = scope;
    if (searchDebounceTimer) {
      clearTimeout(searchDebounceTimer);
      searchDebounceTimer = null;
    }
    if (scope !== "global") {
      searching.value = false;
      searchResults.value = [];
      searchTruncated.value = false;
      searchHasFallback.value = false;
      return;
    }
    if (searchQuery.value.trim()) {
      runFilenameSearch(searchQuery.value);
    }
  }

  async function selectFromSearchResult(result: AssetSearchResult) {
    if (!hasWorkspace.value) return;
      await expandToPath(result.path);
      await materializePath(result.path);
      const parentPath = parentFolderPath(result.path);
      if (parentPath) {
      await selectFolder(parentPath, {
        preservePreview: true,
        revealInTree: "ancestors",
      });
      }
    // Find or fabricate a leaf node entry to feed selectNode.
    let node = findNodeByPath(result.path);
    if (!node) {
      node = {
        kind: "file",
        name: result.name,
        path: result.path,
        depth: result.path.split("/").length - 1,
      };
    }
    if (node.kind === "file") {
      selectedNode.value = node;
      viewMode.value = "preview";
      await loadPreview(node);
    }
  }

  // ── Preview ──────────────────────────────────────────────
  async function loadPreview(file: string | AssetPreviewFileNode) {
    const nextNode = toPreviewFileNode(file);
    if (!hasWorkspace.value) {
      invalidatePreviewSession();
      clearPreviewState();
      return;
    }
    const session = invalidatePreviewSession();
    const keepCurrentPreview = previewPayload.value !== null;
    previewLoading.value = true;
    previewError.value = "";
    targetLoading.value = false;
    if (!keepCurrentPreview) {
      previewNode.value = nextNode;
      previewPayload.value = null;
      activeTargetId.value = null;
      targetCache.value = new Map();
    }
    try {
      const payload = await previewWorkspaceAsset(nextNode.path);
      if (session !== previewSession) return;
      previewPayload.value = payload;
      previewNode.value = nextNode;
      activeTargetId.value = null;
      targetCache.value = new Map();
      const defaultTargetId = defaultPrefabRootTargetId(payload, nextNode.path);
      if (payload.kind === "structured" && defaultTargetId) {
        await loadTarget(payload.previewKey, defaultTargetId);
      }
    } catch (e) {
      if (session !== previewSession) return;
      const err = normalizeAppError(e);
      previewPayload.value = null;
      previewNode.value = nextNode;
      previewError.value = err.message;
    } finally {
      if (session === previewSession) {
        previewLoading.value = false;
        targetLoading.value = false;
      }
    }
  }

  async function loadTarget(previewKey: string, targetId: string) {
    if (!hasWorkspace.value) return null;
    const session = previewSession;
    const generation = ++targetRequestGeneration;
    activeTargetId.value = targetId;
    const cached = targetCache.value.get(targetId);
    if (cached) {
      targetLoading.value = false;
      return cached;
    }
    targetLoading.value = true;
    try {
      const inspector = await previewWorkspaceAssetTarget(previewKey, targetId);
      if (session !== previewSession) return null;
      const payload = previewPayload.value;
      if (!payload || payload.kind !== "structured" || payload.previewKey !== previewKey) {
        return null;
      }
      const next = new Map(targetCache.value);
      next.set(targetId, inspector);
      targetCache.value = next;
      if (generation === targetRequestGeneration) {
        activeTargetId.value = targetId;
      }
      return inspector;
    } catch (e) {
      if (session !== previewSession) return null;
      if (generation !== targetRequestGeneration) return null;
      const err = normalizeAppError(e);
      // Cache eviction recovery: rebuild session and retry once.
      if (
        err.code === "asset.preview.cache_miss"
        && err.retryable
        && selectedNode.value
        && selectedNode.value.kind === "file"
      ) {
        await loadPreview(selectedNode.value);
        const newPayload = previewPayload.value;
        if (newPayload && newPayload.kind === "structured") {
          return loadTarget(newPayload.previewKey, targetId);
        }
      } else {
        error.value = err.message;
      }
      return null;
    } finally {
      if (session === previewSession && generation === targetRequestGeneration) {
        targetLoading.value = false;
      }
    }
  }

  // ── DB Overview ──────────────────────────────────────────
  async function refreshDbOverview() {
    if (!hasWorkspace.value) {
      dbOverview.value = null;
      dbLoading.value = false;
      return;
    }
    dbLoading.value = true;
    try {
      dbOverview.value = await assetDbOverview();
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      dbLoading.value = false;
    }
  }

  async function refreshWatcherTuning() {
    if (!hasWorkspace.value) {
      watcherTuning.value = null;
      return;
    }
    try {
      watcherTuning.value = await getWatcherTuning();
    } catch (e) {
      const err = normalizeAppError(e);
      console.warn("[useAssetState] getWatcherTuning failed", err.message);
    }
  }

  async function updateWatcherTuning(debounceMs: number, workerCount: number) {
    if (!hasWorkspace.value) return;
    watcherTuningSaving.value = true;
    try {
      watcherTuning.value = await setWatcherTuning(debounceMs, workerCount);
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      watcherTuningSaving.value = false;
    }
  }

  async function triggerRescan() {
    if (!hasWorkspace.value) return;
    try {
      await assetDbScan();
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      await refreshDbOverview();
    }
  }

  // ── Lifecycle ────────────────────────────────────────────
  let unlisten: UnlistenFn | null = null;
  let watcherPollTimer: ReturnType<typeof setInterval> | null = null;

  // Lightweight polling so the watcher card can show queue depth + current
  // file in near real-time without requiring a dedicated event channel.
  // Skips while a full scan is running (the scan-event subscription drives
  // updates in that case) and while the page is hidden.
  function startWatcherPoll() {
    if (!hasWorkspace.value) return;
    if (watcherPollTimer) return;
    watcherPollTimer = setInterval(() => {
      if (!hasWorkspace.value) return;
      if (typeof document !== "undefined" && document.hidden) return;
      if (dbOverview.value?.status === "scanning") return;
      refreshDbOverview();
    }, 1500);
  }
  function stopWatcherPoll() {
    if (watcherPollTimer) {
      clearInterval(watcherPollTimer);
      watcherPollTimer = null;
    }
  }

  onMounted(async () => {
    if (hasWorkspace.value) {
      initRoots();
      void prefetchRootFolderBranchState();
      await selectFolder(FIXED_ROOTS[0], {
        preservePreview: true,
        revealInTree: "none",
      });
      const cachedDbOverview = getWarmup<AssetDbOverview>("asset:dbOverview");
      const cachedWatcherTuning = getWarmup<WatcherTuning>("asset:watcherTuning");

      if (cachedDbOverview) {
        dbOverview.value = cachedDbOverview;
        dbLoading.value = false;
      } else {
        await refreshDbOverview();
      }

      if (cachedWatcherTuning) {
        watcherTuning.value = cachedWatcherTuning;
      } else {
        refreshWatcherTuning();
      }
    } else {
      resetWorkspaceState();
    }
    try {
      unlisten = await listen<AssetDbScanEvent>("ref-graph-scan", async (e) => {
        if (!hasWorkspace.value) return;
        const phase = e.payload;
        if (!dbOverview.value) {
          await refreshDbOverview();
          return;
        }
        // Update the sticky phase + status from the live event.
        if (phase.phase === "done") {
          dbOverview.value = {
            ...dbOverview.value,
            currentScanPhase: undefined,
            status: "indexed",
          };
          await refreshDbOverview();
        } else if (phase.phase === "error") {
          dbOverview.value = {
            ...dbOverview.value,
            currentScanPhase: phase,
            status: "error",
          };
          await refreshDbOverview();
        } else {
          dbOverview.value = {
            ...dbOverview.value,
            currentScanPhase: phase,
            status: "scanning",
          };
        }
      });
    } catch (e) {
      // listen failure shouldn't break the page
      console.warn("[useAssetState] failed to listen ref-graph-scan", e);
    }
    if (hasWorkspace.value) startWatcherPoll();
  });

  onUnmounted(() => {
    unlisten?.();
    unlisten = null;
    stopWatcherPoll();
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    releaseSelectionLock?.();
    releaseSelectionLock = null;
  });

  // Re-init when workingDir changes (workspace switch).
  watch(
    () => props.workingDir,
    (workingDir) => {
      stopWatcherPoll();
      resetWorkspaceState();
      if (!workingDir.trim()) return;
      initRoots();
      void prefetchRootFolderBranchState();
      void selectFolder(FIXED_ROOTS[0], {
        preservePreview: true,
        revealInTree: "none",
      });
      const cachedDbOverview = getWarmup<AssetDbOverview>("asset:dbOverview");
      const cachedWatcherTuning = getWarmup<WatcherTuning>("asset:watcherTuning");

      if (cachedDbOverview) {
        dbOverview.value = cachedDbOverview;
        dbLoading.value = false;
      } else {
        refreshDbOverview();
      }

      if (cachedWatcherTuning) {
        watcherTuning.value = cachedWatcherTuning;
      } else {
        refreshWatcherTuning();
      }
      startWatcherPoll();
    },
  );

  // ── Resize handle ────────────────────────────────────────
  let resizing = false;
  let resizeTarget: "sidebar" | "directory" | null = null;
  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let releaseSelectionLock: (() => void) | null = null;

  function beginResize(
    target: "sidebar" | "directory",
    startWidth: number,
    e: MouseEvent,
  ) {
    resizing = true;
    resizeTarget = target;
    resizeStartX = e.clientX;
    resizeStartWidth = startWidth;
    document.addEventListener("mousemove", onResizeMove);
    document.addEventListener("mouseup", onResizeEnd);
    document.body.style.cursor = "col-resize";
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
  }

  function onResizeStart(e: MouseEvent) {
    beginResize("sidebar", sidebarWidth.value, e);
  }

  function onDirectoryResizeStart(e: MouseEvent) {
    beginResize("directory", directoryPaneWidth.value, e);
  }

  function onResizeMove(e: MouseEvent) {
    if (!resizing) return;
    const delta = e.clientX - resizeStartX;
    if (resizeTarget === "sidebar") {
      sidebarWidth.value = Math.min(480, Math.max(220, resizeStartWidth + delta));
      return;
    }
    if (resizeTarget === "directory") {
      directoryPaneWidth.value = Math.min(520, Math.max(260, resizeStartWidth + delta));
    }
  }
  function onResizeEnd() {
    resizing = false;
    resizeTarget = null;
    document.removeEventListener("mousemove", onResizeMove);
    document.removeEventListener("mouseup", onResizeEnd);
    document.body.style.cursor = "";
    releaseSelectionLock?.();
    releaseSelectionLock = null;
  }

  function compareExplorerNodes(a: AssetExplorerNode, b: AssetExplorerNode): number {
    if (a.kind !== b.kind) {
      return a.kind === "folder" ? -1 : 1;
    }
    return a.name.localeCompare(b.name, undefined, {
      numeric: true,
      sensitivity: "base",
    });
  }

  const currentFolder = computed<AssetFolderNode | null>(() => {
    const path = selectedFolderPath.value;
    if (!path) return null;
    const node = findNodeByPath(path);
    return node && node.kind === "folder" ? node : null;
  });

  const currentFolderLabel = computed(() =>
    selectedFolderPath.value
      ? selectedFolderPath.value.split("/").filter(Boolean).join(" / ")
      : "",
  );

  const currentFolderEntries = computed<AssetExplorerNode[]>(() => {
    const folder = currentFolder.value;
    if (!folder) return [];
    return [...folder.children].sort(compareExplorerNodes);
  });

  const visibleDirectoryEntries = computed<AssetExplorerNode[]>(() => {
    const rawQuery = searchQuery.value.trim().toLowerCase();
    const entries = currentFolderEntries.value;
    if (!rawQuery || searchScope.value !== "folder") return entries;
    return entries.filter((entry) =>
      entry.name.toLowerCase().includes(rawQuery)
      || entry.path.toLowerCase().includes(rawQuery),
    );
  });

  const currentFolderLoading = computed(() => currentFolder.value?.loading ?? false);
  const currentFolderLoaded = computed(() => currentFolder.value?.loaded ?? false);
  const currentFolderHasMore = computed(() => currentFolder.value?.hasMore ?? false);

  return {
    // state
    loading,
    error,
    sidebarWidth,
    directoryPaneWidth,
    explorerTree,
    expandedPaths,
    selectedFolderPath,
    selectedNode,
    viewMode,
    searchQuery,
    searchScope,
    searchResults,
    searchTruncated,
    searchHasFallback,
    searching,
    currentFolderLabel,
    visibleDirectoryEntries,
    currentFolderLoading,
    currentFolderLoaded,
    currentFolderHasMore,
    previewPayload,
    previewNode,
    previewLoading,
    previewError,
    activeTargetId,
    targetCache,
    targetLoading,
    dbOverview,
    dbLoading,
    watcherTuning,
    watcherTuningSaving,
    // actions
    initRoots,
    isPathExpanded,
    selectFolder,
    togglePath,
    probeFolderPath,
    loadMoreFolder,
    loadCurrentFolderMore,
    selectNode,
    closePreview,
    runFilenameSearch,
    updateSearchScope,
    selectFromSearchResult,
    loadPreview,
    loadTarget,
    refreshDbOverview,
    triggerRescan,
    refreshWatcherTuning,
    updateWatcherTuning,
    onResizeStart,
    onDirectoryResizeStart,
  };
}
