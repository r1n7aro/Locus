import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  assetDbOverview,
  assetDbScanStart,
  previewWorkspaceAsset,
  previewWorkspaceAssetTarget,
  getWatcherTuning,
  setWatcherTuning,
} from "../services/asset";
import {
  listDirEntriesPage,
  searchWorkspaceEntries,
  statWorkspaceEntries,
} from "../services/project";
import { WORKSPACE_EVENT_NAME } from "../services/project";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "../services/project";
import {
  subscribeWorkspaceFileChanges,
  type WorkspaceFileChangedPayload,
} from "../services/workspaceExplorer";
import type { RuntimeUnsubscribe } from "../services/locusRuntime";
import { t } from "../i18n";
import { isUnityConnectionError, normalizeAppError } from "../services/errors";
import { getWarmup } from "./warmupCache";
import { acquireSelectionLock } from "./useSelectionLock";
import { useDisplaySettings } from "./useDisplaySettings";
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
  workspaceRef?: WorkspaceRef | null;
  active?: boolean;
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

const WORKSPACE_ROOT_PATH = ".";
const ASSET_EXPLORER_PAGE_SIZE = 200;
const ASSET_EXPLORER_BRANCH_PROBE_PAGE_SIZE = 1;
const ASSET_EXPLORER_BRANCH_PROBE_CONCURRENCY = 8;
type ViewMode = "stats" | "preview";
type AssetSearchScope = "folder" | "global";
type FolderRevealMode = "none" | "ancestors" | "self";

function assetPreviewErrorMessage(error: unknown): string {
  const err = normalizeAppError(error);
  return isUnityConnectionError(err) ? t("asset.preview.unityConnectionRequired") : err.message;
}

export function useAssetState(props: AssetProps) {
  const { state: displaySettings } = useDisplaySettings();
  // ── Reactive state ────────────────────────────────────────
  const loading = ref(false);
  const error = ref("");
  const sidebarWidth = ref(240);
  const directoryPaneWidth = ref(320);

  const explorerTree = ref<AssetExplorerNode[]>([]);
  const expandedPaths = ref<Set<string>>(new Set());
  const selectedFolderPath = ref<string | null>(null);
  const selectedNode = ref<AssetExplorerNode | null>(null);
  const isUnityWorkspace = ref(false);

  const viewMode = ref<ViewMode>("stats");

  // search
  const searchQuery = ref("");
  const searchScope = ref<AssetSearchScope>("folder");
  const searchResults = ref<AssetSearchResult[]>([]);
  const searchTruncated = ref(false);
  const searchHasFallback = ref(false);
  const searching = ref(false);
  const selectedSearchKey = ref<string | null>(null);

  // preview
  const previewPayload = ref<AssetPreviewPayload | null>(null);
  const previewNode = ref<AssetPreviewFileNode | null>(null);
  const previewLoading = ref(false);
  const previewError = ref("");
  const previewFocusLine = ref<number | null>(null);
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
    previewFocusLine.value = null;
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
  let explorerInitialization = 0;
  let explorerInitializing = false;
  const pendingExplorerFileChanges = new Map<string, WorkspaceFileChangedPayload["kind"]>();
  let explorerFileChangeTimer: ReturnType<typeof setTimeout> | null = null;
  const hasWorkspace = computed(() => !!props.workingDir.trim() && !!props.workspaceRef);
  const hiddenDirectories = computed(() => {
    const names = [
      ...displaySettings.fileExplorerHiddenDirectories,
      ...(isUnityWorkspace.value ? displaySettings.unityFileExplorerHiddenDirectories : []),
    ];
    const seen = new Set<string>();
    return names.filter((name) => {
      const key = name.toLocaleLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  });

  function requireAssetWorkspaceRef(): WorkspaceRef {
    const workspaceRef = props.workspaceRef;
    if (!workspaceRef) throw new Error("A workspace checkout is required for asset operations.");
    return workspaceRef;
  }

  function explorerParentPath(path: string): string {
    const normalized = path.trim().replace(/\\/g, "/").replace(/^\.\//, "").replace(/\/+$/, "");
    const separator = normalized.lastIndexOf("/");
    return separator > 0 ? normalized.slice(0, separator) : WORKSPACE_ROOT_PATH;
  }

  function explorerPathHidden(path: string): boolean {
    const normalized = path.trim().replace(/\\/g, "/").replace(/^\.\//, "");
    if (isUnityWorkspace.value && normalized.toLocaleLowerCase().endsWith(".meta")) return true;
    const hidden = new Set(hiddenDirectories.value.map((name) => name.toLocaleLowerCase()));
    return normalized
      .split("/")
      .slice(0, -1)
      .some((segment) => hidden.has(segment.toLocaleLowerCase()));
  }

  async function applyExplorerFileChange(
    path: string,
    kind: WorkspaceFileChangedPayload["kind"],
  ): Promise<void> {
    if (!hasWorkspace.value || explorerPathHidden(path)) return;
    const parent = findNodeByPath(explorerParentPath(path));
    if (!parent || parent.kind !== "folder" || !parent.loaded) return;
    const normalizedPath = path.replace(/\\/g, "/");
    const childIndex = parent.children.findIndex((child) => (
      child.path.toLocaleLowerCase() === normalizedPath.toLocaleLowerCase()
    ));

    if (kind === "delete") {
      if (childIndex < 0) return;
      parent.children.splice(childIndex, 1);
      parent.totalCount = Math.max(0, parent.totalCount - 1);
      parent.nextOffset = Math.max(0, parent.nextOffset - 1);
      parent.hasChildFolders = parent.children.some((child) => child.kind === "folder");
      if (selectedNode.value?.path.toLocaleLowerCase() === normalizedPath.toLocaleLowerCase()) {
        closePreview();
        selectedNode.value = null;
      }
      explorerTree.value = [...explorerTree.value];
      return;
    }

    if (childIndex >= 0) return;
    const workspaceRef = props.workspaceRef;
    if (!workspaceRef) return;
    const [entry] = await statWorkspaceEntries([normalizedPath], workspaceRef);
    if (!entry?.exists || (entry.entryKind !== "file" && entry.entryKind !== "folder")) return;
    if (parent.children.some((child) => (
      child.path.toLocaleLowerCase() === normalizedPath.toLocaleLowerCase()
    ))) return;
    const name = normalizedPath.split("/").filter(Boolean).pop() ?? normalizedPath;
    parent.children.push(entry.entryKind === "folder"
      ? createFolderNode(name, normalizedPath, parent.depth + 1, false)
      : createFileNode(name, normalizedPath, parent.depth + 1));
    parent.totalCount += 1;
    parent.nextOffset += 1;
    parent.hasChildFoldersKnown = true;
    parent.hasChildFolders = parent.children.some((child) => child.kind === "folder");
    explorerTree.value = [...explorerTree.value];
  }

  function armExplorerFileChangeFlush(): void {
    if (props.active === false || pendingExplorerFileChanges.size === 0) return;
    if (explorerFileChangeTimer) clearTimeout(explorerFileChangeTimer);
    explorerFileChangeTimer = setTimeout(() => {
      explorerFileChangeTimer = null;
      const changes = [...pendingExplorerFileChanges.entries()];
      pendingExplorerFileChanges.clear();
      void (async () => {
        for (const [changedPath, changedKind] of changes) {
          await applyExplorerFileChange(changedPath, changedKind);
        }
      })();
    }, 120);
  }

  function scheduleExplorerFileChange(
    path: string,
    kind: WorkspaceFileChangedPayload["kind"],
  ): void {
    pendingExplorerFileChanges.set(path.replace(/\\/g, "/"), kind);
    armExplorerFileChangeFlush();
  }

  function resetWorkspaceState() {
    explorerInitialization += 1;
    explorerInitializing = false;
    if (explorerFileChangeTimer) clearTimeout(explorerFileChangeTimer);
    explorerFileChangeTimer = null;
    pendingExplorerFileChanges.clear();
    invalidatePreviewSession();
    explorerTree.value = [];
    expandedPaths.value = new Set();
    selectedFolderPath.value = null;
    selectedNode.value = null;
    isUnityWorkspace.value = false;
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
  function workspaceRootName(): string {
    const normalized = props.workingDir.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
    return normalized.split("/").filter(Boolean).pop() || t("asset.explorer.workspaceRoot");
  }

  function initRoots() {
    explorerTree.value = [createFolderNode(
      workspaceRootName(),
      WORKSPACE_ROOT_PATH,
      0,
      true,
    )];
    expandedPaths.value = new Set([WORKSPACE_ROOT_PATH]);
    selectedFolderPath.value = WORKSPACE_ROOT_PATH;
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
    if (segments.length <= 1) return segments.length ? WORKSPACE_ROOT_PATH : null;
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
    const workspaceRef = props.workspaceRef;
    if (!workspaceRef) return;
    if (folder.loading) return;
    if (!options.append && folder.loaded) return;
    if (options.append && !folder.hasMore) return;
    folder.loading = true;
    try {
      const page = await listDirEntriesPage(
        folder.path,
        workspaceRef,
        options.append ? folder.nextOffset : 0,
        ASSET_EXPLORER_PAGE_SIZE,
        isUnityWorkspace.value,
        hiddenDirectories.value,
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
    const workspaceRef = props.workspaceRef;
    if (!workspaceRef) return;
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
        workspaceRef,
        0,
        ASSET_EXPLORER_BRANCH_PROBE_PAGE_SIZE,
        isUnityWorkspace.value,
        hiddenDirectories.value,
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
    const workspaceRoot = findNodeByPath(WORKSPACE_ROOT_PATH);
    if (workspaceRoot?.kind === "folder" && !workspaceRoot.loaded) {
      await loadFolderChildren(workspaceRoot);
    }
    expandedPaths.value = new Set(expandedPaths.value).add(WORKSPACE_ROOT_PATH);
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
    if (path === WORKSPACE_ROOT_PATH) {
      const workspaceRoot = findNodeByPath(WORKSPACE_ROOT_PATH);
      if (workspaceRoot?.kind === "folder" && !workspaceRoot.loaded) {
        await loadFolderChildren(workspaceRoot);
      }
      expandedPaths.value = new Set(expandedPaths.value).add(WORKSPACE_ROOT_PATH);
      explorerTree.value = [...explorerTree.value];
      return;
    }
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
    if (!segments.length) return;
    const parentPath = segments.length === 1
      ? WORKSPACE_ROOT_PATH
      : segments.slice(0, -1).join("/");
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
    selectedSearchKey.value = null;
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
    selectedSearchKey.value = null;
    clearPreviewState();
    viewMode.value = "stats";
  }

  // ── Search ───────────────────────────────────────────────
  let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  function searchResultKey(result: AssetSearchResult): string {
    if (result.objectKey) return result.objectKey;
    if (result.isSubAsset && result.name.trim()) return `${result.path}/${result.name.trim()}`;
    return result.path;
  }

  function workspaceSearchResultKind(name: string, isDirectory: boolean): string {
    if (isDirectory) return t("asset.search.kind.folder");
    const extension = name.toLocaleLowerCase().split(".").pop();
    return extension && extension !== name.toLocaleLowerCase()
      ? extension
      : t("asset.search.kind.file");
  }

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
        const entries = await searchWorkspaceEntries(
          query,
          requireAssetWorkspaceRef(),
          200,
          hiddenDirectories.value,
        );
        searchResults.value = entries.map((entry) => ({
          path: entry.relPath,
          name: entry.name,
          root: "workspace",
          kind: workspaceSearchResultKind(entry.name, entry.isDir),
          isDirectory: entry.isDir,
          matchScore: entry.matchScore,
          source: "filesystem",
        }));
        searchTruncated.value = entries.length === 200;
        searchHasFallback.value = false;
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
    selectedSearchKey.value = searchResultKey(result);
    if (result.isDirectory) {
      await expandFolderPath(result.path, false);
      await materializePath(result.path);
      await selectFolder(result.path, { revealInTree: "ancestors" });
      return;
    }
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
      if (result.targetId && previewPayload.value?.kind === "structured") {
        await loadTarget(previewPayload.value.previewKey, result.targetId);
      }
    }
  }

  function normalizeOpenAssetPath(path: string): string | null {
    const normalized = path.trim().replace(/\\/g, "/");
    if (!normalized) return null;
    const workspace = props.workingDir.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
    const isAbsolute = /^[a-z]:\//i.test(normalized) || normalized.startsWith("/");
    const isWithinWorkspace = normalized.toLowerCase().startsWith(`${workspace.toLowerCase()}/`);
    if (isAbsolute && !isWithinWorkspace) return null;
    const relative = isWithinWorkspace
      ? normalized.slice(workspace.length + 1)
      : normalized.replace(/^\/+/, "");
    const segments = relative.split("/").filter(Boolean);
    if (!segments.length || segments.some((segment) => segment === "." || segment === "..")) {
      return null;
    }
    return segments.join("/");
  }

  async function openAssetPath(path: string, focusLine?: number): Promise<boolean> {
    if (!hasWorkspace.value) return false;
    const assetPath = normalizeOpenAssetPath(path);
    if (!assetPath) return false;

    selectedSearchKey.value = null;
    await expandToPath(assetPath);
    await materializePath(assetPath);
    const parentPath = parentFolderPath(assetPath);
    if (parentPath) {
      await selectFolder(parentPath, {
        preservePreview: true,
        revealInTree: "ancestors",
      });
    }
    let node = findNodeByPath(assetPath);
    if (!node) {
      const segments = assetPath.split("/").filter(Boolean);
      node = {
        kind: "file",
        name: segments[segments.length - 1] ?? assetPath,
        path: assetPath,
        depth: Math.max(0, segments.length - 1),
      };
    }
    if (node.kind !== "file") return false;

    selectedNode.value = node;
    viewMode.value = "preview";
    await loadPreview(node, focusLine);
    return true;
  }

  // ── Preview ──────────────────────────────────────────────
  async function loadPreview(file: string | AssetPreviewFileNode, focusLine?: number) {
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
    previewFocusLine.value = focusLine != null && Number.isFinite(focusLine)
      ? Math.max(1, Math.floor(focusLine))
      : null;
    targetLoading.value = false;
    if (!keepCurrentPreview) {
      previewNode.value = nextNode;
      previewPayload.value = null;
      activeTargetId.value = null;
      targetCache.value = new Map();
    }
    try {
      const payload = await previewWorkspaceAsset(
        nextNode.path,
        previewFocusLine.value ?? undefined,
        requireAssetWorkspaceRef(),
      );
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
      previewPayload.value = null;
      previewNode.value = nextNode;
      previewError.value = assetPreviewErrorMessage(e);
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
      const inspector = await previewWorkspaceAssetTarget(
        previewKey,
        targetId,
        requireAssetWorkspaceRef(),
      );
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
        error.value = assetPreviewErrorMessage(err);
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
    if (!hasWorkspace.value || !isUnityWorkspace.value) {
      dbOverview.value = null;
      dbLoading.value = false;
      return;
    }
    dbLoading.value = true;
    try {
      dbOverview.value = await assetDbOverview(requireAssetWorkspaceRef());
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    } finally {
      dbLoading.value = false;
    }
  }

  async function refreshWatcherTuning() {
    if (!hasWorkspace.value || !isUnityWorkspace.value) {
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
    if (!hasWorkspace.value || !isUnityWorkspace.value) return;
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
    if (!hasWorkspace.value || !isUnityWorkspace.value) return;
    try {
      const result = await assetDbScanStart(requireAssetWorkspaceRef());
      if ((result.started || result.alreadyRunning) && dbOverview.value) {
        dbOverview.value = {
          ...dbOverview.value,
          currentScanPhase: { phase: "dirScan" },
          status: "scanning",
        };
      }
    } catch (e) {
      const err = normalizeAppError(e);
      error.value = err.message;
    }
  }

  async function detectUnityWorkspace(): Promise<boolean> {
    if (!hasWorkspace.value) return false;
    try {
      const entries = await statWorkspaceEntries(
        ["Assets", "ProjectSettings/ProjectVersion.txt"],
        requireAssetWorkspaceRef(),
      );
      const assets = entries.find((entry) => entry.path === "Assets");
      const projectVersion = entries.find(
        (entry) => entry.path === "ProjectSettings/ProjectVersion.txt",
      );
      return assets?.entryKind === "folder" && projectVersion?.entryKind === "file";
    } catch {
      return false;
    }
  }

  async function initializeExplorer(options: { detectUnity?: boolean } = {}) {
    if (!hasWorkspace.value) return false;
    const initialization = ++explorerInitialization;
    explorerInitializing = true;
    try {
      if (options.detectUnity !== false) {
        const detected = await detectUnityWorkspace();
        if (initialization !== explorerInitialization) return false;
        isUnityWorkspace.value = detected;
      }
      initRoots();
      closePreview();
      await selectFolder(WORKSPACE_ROOT_PATH, {
        preservePreview: true,
        revealInTree: "none",
      });
      if (initialization !== explorerInitialization) return false;
      const root = findNodeByPath(WORKSPACE_ROOT_PATH);
      if (root?.kind === "folder") {
        void prefetchChildFolderBranchState(root);
      }
      return true;
    } finally {
      if (initialization === explorerInitialization) {
        explorerInitializing = false;
      }
    }
  }

  async function initializeUnityAssetState() {
    if (!isUnityWorkspace.value) {
      dbOverview.value = null;
      dbLoading.value = false;
      watcherTuning.value = null;
      return;
    }
    const workspaceRef = requireAssetWorkspaceRef();
    const cachedDbOverview = getWarmup<AssetDbOverview>(
      `asset:dbOverview:${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}`,
    );
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
      void refreshWatcherTuning();
    }
  }

  // ── Lifecycle ────────────────────────────────────────────
  let unlistenScoped: UnlistenFn | null = null;
  let releaseWorkspaceFileChanges: RuntimeUnsubscribe | null = null;
  let assetStateUnmounted = false;
  let watcherPollTimer: ReturnType<typeof setInterval> | null = null;

  // Lightweight polling so the watcher card can show queue depth + current
  // file in near real-time without requiring a dedicated event channel.
  // Skips while a full scan is running (the scan-event subscription drives
  // updates in that case) and while the page is hidden.
  function startWatcherPoll() {
    if (!hasWorkspace.value || !isUnityWorkspace.value) return;
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
      const initialized = await initializeExplorer();
      if (initialized) await initializeUnityAssetState();
    } else {
      resetWorkspaceState();
    }
    const releaseFileChanges = await subscribeWorkspaceFileChanges((event) => {
      const workspaceRef = props.workspaceRef;
      if (!workspaceRef || event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      scheduleExplorerFileChange(event.payload.path, event.payload.kind);
    });
    if (assetStateUnmounted) releaseFileChanges();
    else releaseWorkspaceFileChanges = releaseFileChanges;
    try {
      const applyScanPhase = async (phase: AssetDbScanEvent) => {
        if (!hasWorkspace.value) return;
        if (!dbOverview.value) {
          await refreshDbOverview();
          return;
        }
        // Update the sticky phase + status from the live event.
        if (phase.phase === "done" || phase.phase === "reconcileDone") {
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
      };
      unlistenScoped = await listen<RoutedWorkspaceEvent<AssetDbScanEvent>>(
        WORKSPACE_EVENT_NAME,
        async (event) => {
          const workspaceRef = props.workspaceRef;
          if (!workspaceRef) return;
          const routed = event.payload;
          if (routed.eventName !== "ref-graph-scan") return;
          if (routed.checkoutId !== workspaceRef.checkoutId) return;
          if (
            workspaceRef.expectedGeneration != null
            && routed.workspaceGeneration !== workspaceRef.expectedGeneration
          ) return;
          await applyScanPhase(routed.payload);
        },
      );
    } catch (e) {
      // listen failure shouldn't break the page
      console.warn("[useAssetState] failed to listen ref-graph-scan", e);
    }
    if (hasWorkspace.value && isUnityWorkspace.value) startWatcherPoll();
  });

  onUnmounted(() => {
    assetStateUnmounted = true;
    unlistenScoped?.();
    unlistenScoped = null;
    releaseWorkspaceFileChanges?.();
    releaseWorkspaceFileChanges = null;
    if (explorerFileChangeTimer) clearTimeout(explorerFileChangeTimer);
    explorerFileChangeTimer = null;
    pendingExplorerFileChanges.clear();
    stopWatcherPoll();
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    releaseSelectionLock?.();
    releaseSelectionLock = null;
  });

  // Re-init when workingDir changes (workspace switch).
  watch(
    () => [
      props.workingDir,
      props.workspaceRef?.checkoutId ?? "",
      props.workspaceRef?.expectedGeneration ?? null,
    ] as const,
    async ([workingDir]) => {
      stopWatcherPoll();
      resetWorkspaceState();
      if (!workingDir.trim()) return;
      const initialized = await initializeExplorer();
      if (!initialized) return;
      await initializeUnityAssetState();
      startWatcherPoll();
    },
  );

  watch(() => props.active, (active) => {
    if (active !== false) armExplorerFileChangeFlush();
  });

  watch(
    () => hiddenDirectories.value.join("\u0000"),
    () => {
      if (!hasWorkspace.value || explorerInitializing) return;
      void initializeExplorer({ detectUnity: false });
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
    selectedFolderPath.value === WORKSPACE_ROOT_PATH
      ? workspaceRootName()
      : selectedFolderPath.value
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
    isUnityWorkspace,
    viewMode,
    searchQuery,
    searchScope,
    searchResults,
    searchTruncated,
    searchHasFallback,
    searching,
    selectedSearchKey,
    currentFolderLabel,
    visibleDirectoryEntries,
    currentFolderLoading,
    currentFolderLoaded,
    currentFolderHasMore,
    previewPayload,
    previewNode,
    previewLoading,
    previewError,
    previewFocusLine,
    activeTargetId,
    targetCache,
    targetLoading,
    dbOverview,
    dbLoading,
    watcherTuning,
    watcherTuningSaving,
    // actions
    initRoots,
    initializeExplorer,
    isPathExpanded,
    selectFolder,
    togglePath,
    probeFolderPath,
    loadMoreFolder,
    loadCurrentFolderMore,
    applyExplorerFileChange,
    selectNode,
    closePreview,
    runFilenameSearch,
    updateSearchScope,
    selectFromSearchResult,
    openAssetPath,
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
