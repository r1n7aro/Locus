<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import {
  BadgeInfo,
  Check,
  ChevronRight,
  ChevronsDownUp,
  FilePlus,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  ListTree,
  LocateFixed,
  Lock,
  Package,
  PackagePlus,
  X,
} from "lucide";
import { t } from "../../i18n";
import {
  isSkillPackageRootDocument,
  type ExplorerNode,
} from "../../composables/useKnowledgeState";
import type {
  KnowledgeDirectoryConfigRecord,
  KnowledgeDocumentType,
  KnowledgeExternalSource,
  KnowledgeFolderDisplayStats,
  KnowledgeSearchResult,
} from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import FileTreeList from "../explorer/FileTreeList.vue";
import {
  buildFolderListTags,
  buildExternalFolderTag,
  buildKnowledgeLegendEntries,
  buildKnowledgeListTags,
  buildKnowledgeSearchMatchTags,
  type KnowledgeListTag,
} from "./knowledgeMetaLabels";
import { buildFolderDisplayStats } from "./knowledgeExplorerFolderCounts";
import {
  pruneKnowledgeDragNodes,
  resolveKnowledgeContextSelection,
  resolveKnowledgeExplorerSelection,
} from "./knowledgeExplorerSelection";
import {
  resolveKnowledgeTreeKeyboardAction,
  type KnowledgeTreeKeyboardAction,
  type KnowledgeTreeKeyboardRow,
} from "./knowledgeExplorerKeyboard";
import { buildKnowledgeSnippetSegments } from "./knowledgeSearchSnippet";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
} from "../icons/unityAssetIcons";

type FolderNode = Extract<ExplorerNode, { kind: "folder" }>;
type PackageNode = Extract<ExplorerNode, { kind: "package" }>;
type DocumentNode = Extract<ExplorerNode, { kind: "document" }>;
type BranchNode = FolderNode | PackageNode;

const props = defineProps<{
  tree: ExplorerNode[];
  activeType: KnowledgeDocumentType;
  rootDirectoryConfigs: Record<string, KnowledgeDirectoryConfigRecord>;
  externalDirectorySources: Record<string, KnowledgeExternalSource[]>;
  folderStats: Record<string, KnowledgeFolderDisplayStats>;
  selectedPath: string | null;
  isPathExpanded: (path: string) => boolean;
  hasMoreRootDocuments: boolean;
  rootDocumentsLoading: boolean;
  hasMoreFolderDocuments: (path: string) => boolean;
  folderDocumentsLoaded: (path: string) => boolean;
  folderDocumentsLoading: (path: string) => boolean;
  loading: boolean;
  searchQuery: string;
  searchResults: KnowledgeSearchResult[];
  searching: boolean;
}>();

const emit = defineEmits<{
  (e: "selectDocument", document: DocumentNode["document"]): void;
  (e: "selectPackage", document: PackageNode["document"]): void;
  (e: "selectSearchResult", result: KnowledgeSearchResult): void;
  (e: "selectFolderConfig", path: string): void;
  (e: "toggle", path: string): void;
  (e: "importSkillPackage"): void;
  (e: "exportPackage", node: PackageNode): void;
  (e: "requestExternalImportFolder", parentDir: string): void;
  (e: "createFolder", parentDir: string, name: string): void;
  (e: "createDocument", parentDir: string, name: string): void;
  (e: "renameFolder", path: string, name: string): void;
  (
    e: "renameDocument",
    path: string,
    name: string,
    type: KnowledgeDocumentType,
  ): void;
  (e: "copyRelativePath", node: ExplorerNode): void;
  (e: "openInFileSystem", node: ExplorerNode): void;
  (e: "requestDeleteNodes", nodes: ExplorerNode[]): void;
  (e: "moveNodes", nodes: ExplorerNode[], targetDir: string): void;
  (e: "collapseAll"): void;
  (e: "expandToSelection"): void;
  (e: "revealSearchResult", result: KnowledgeSearchResult): void;
  (e: "copySearchResultPath", result: KnowledgeSearchResult): void;
  (e: "loadMoreRoot"): void;
  (e: "loadMoreFolder", path: string): void;
  (e: "dragStateChange", dragging: boolean): void;
}>();

interface FlatRow {
  node: ExplorerNode;
  expanded: boolean;
  directChildCount: number;
}

type ContextMenuState =
  | {
      x: number;
      y: number;
      kind: "folder";
      node: FolderNode;
      parentDir: string;
      anchorPath: string;
      depth: number;
      expanded: boolean;
      childCount: number;
      targetNodes: ExplorerNode[];
    }
  | {
      x: number;
      y: number;
      kind: "package";
      node: PackageNode;
      targetNodes: ExplorerNode[];
    }
  | {
      x: number;
      y: number;
      kind: "root";
      anchorPath: string;
    }
  | {
      x: number;
      y: number;
      kind: "leaf";
      node: DocumentNode;
      targetNodes: ExplorerNode[];
    };

interface InlineCreateState {
  kind: "folder" | "document";
  parentDir: string;
  anchorPath: string;
  depth: number;
  name: string;
}

interface InlineRenameState {
  kind: "folder" | "document";
  anchorPath: string;
  relativePath: string;
  currentName: string;
  name: string;
}

type VisibleEntry =
  | { type: "row"; key: string; row: FlatRow }
  | { type: "create"; key: string; draft: InlineCreateState }
  | {
      type: "loadMore";
      key: string;
      path: string | null;
      depth: number;
      loading: boolean;
    };

const ctxMenu = ref<ContextMenuState | null>(null);
const inlineCreate = ref<InlineCreateState | null>(null);
const inlineRename = ref<InlineRenameState | null>(null);
const inlineInputRef = ref<HTMLInputElement | null>(null);
const inlineCreateRowRef = ref<HTMLElement | null>(null);
const inlineRenameInputRef = ref<HTMLInputElement | null>(null);
const inlineRenameRowRef = ref<HTMLElement | null>(null);
const treeListRef = ref<InstanceType<typeof FileTreeList> | null>(null);
const draggingNodes = ref<ExplorerNode[]>([]);
const dragTargetPath = ref<string | null>(null);
const isSearchMode = computed(() => !!props.searchQuery.trim());
const selectedPaths = ref<Set<string>>(new Set());
const lastAnchorPath = ref<string | null>(null);
const focusedPath = ref<string | null>(null);
const pendingRevealPath = ref<string | null>(null);
const legendMenu = ref<{ x: number; y: number } | null>(null);
const searchCtxMenu = ref<{
  x: number;
  y: number;
  result: KnowledgeSearchResult;
} | null>(null);
const legendEntries = computed(() => buildKnowledgeLegendEntries());
const draggingPaths = computed(
  () => new Set(draggingNodes.value.map((node) => node.path)),
);
const contextMenuPath = computed(() => {
  const menu = ctxMenu.value;
  if (!menu || menu.kind === "root") return null;
  return menu.node.path;
});
const contextSelectedPath = computed(() => {
  const path = contextMenuPath.value;
  if (!path) return null;
  if (selectedPaths.value.has(path)) return null;
  if (props.selectedPath === path) return null;
  return path;
});
const folderDisplayStats = computed(() =>
  buildFolderDisplayStats(props.tree, props.folderStats),
);

function isBranchNode(node: ExplorerNode): node is BranchNode {
  return node.kind === "folder" || node.kind === "package";
}

const visibleRows = computed<VisibleEntry[]>(() => {
  const out: VisibleEntry[] = [];
  if (inlineCreate.value?.anchorPath === props.activeType) {
    out.push({
      type: "create",
      key: `create:${props.activeType}:${inlineCreate.value.kind}`,
      draft: inlineCreate.value,
    });
  }

  const walk = (nodes: ExplorerNode[]) => {
    for (const node of nodes) {
      const branch = isBranchNode(node);
      const expanded = branch ? props.isPathExpanded(node.path) : false;
      const folderStats =
        branch ? folderDisplayStats.value.get(node.path) : null;
      const folderLoaded =
        node.kind === "folder"
          ? props.folderDocumentsLoaded(node.relativePath)
          : false;
      const directChildCount =
        folderStats?.directChildCount ??
        (branch ? node.children.length : 0);
      out.push({
        type: "row",
        key: node.path,
        row: { node, expanded, directChildCount },
      });
      if (inlineCreate.value?.anchorPath === node.path) {
        out.push({
          type: "create",
          key: `create:${node.path}:${inlineCreate.value.kind}`,
          draft: inlineCreate.value,
        });
      }
      if (branch && expanded) {
        walk(node.children);
        if (
          node.kind === "folder" &&
          folderLoaded &&
          (props.hasMoreFolderDocuments(node.relativePath) ||
            props.folderDocumentsLoading(node.relativePath))
        ) {
          out.push({
            type: "loadMore",
            key: `${node.path}::load-more`,
            path: node.relativePath,
            depth: node.depth + 1,
            loading: props.folderDocumentsLoading(node.relativePath),
          });
        }
      }
    }
  };

  walk(props.tree);
  if (props.hasMoreRootDocuments || props.rootDocumentsLoading) {
    out.push({
      type: "loadMore",
      key: `${props.activeType}::root-load-more`,
      path: null,
      depth: 1,
      loading: props.rootDocumentsLoading,
    });
  }
  return out;
});

const selectableRows = computed(() =>
  visibleRows.value.filter(
    (entry): entry is Extract<VisibleEntry, { type: "row" }> =>
      entry.type === "row",
  ),
);

const selectablePaths = computed(() =>
  selectableRows.value.map((entry) => entry.row.node.path),
);

const selectableRowMap = computed(
  () =>
    new Map(
      selectableRows.value.map((entry) => [entry.row.node.path, entry.row]),
    ),
);

function resolveTemplateElement(
  element: Element | ComponentPublicInstance | null,
): Element | null {
  if (element instanceof Element) return element;
  if (element && "$el" in element && element.$el instanceof Element) {
    return element.$el;
  }
  return null;
}

function setInlineInputRef(element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  inlineInputRef.value = resolved instanceof HTMLInputElement ? resolved : null;
}

function setInlineCreateRowRef(element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  inlineCreateRowRef.value = resolved instanceof HTMLElement ? resolved : null;
}

function setInlineRenameInputRef(element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  inlineRenameInputRef.value =
    resolved instanceof HTMLInputElement ? resolved : null;
}

function setInlineRenameRowRef(element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  inlineRenameRowRef.value = resolved instanceof HTMLElement ? resolved : null;
}

const inlineRenameName = computed({
  get: () => inlineRename.value?.name ?? "",
  set: (value: string) => {
    if (!inlineRename.value) return;
    inlineRename.value.name = value;
  },
});

function clearMultiSelection(resetAnchor = false) {
  if (selectedPaths.value.size > 0) {
    selectedPaths.value = new Set();
  }
  if (resetAnchor) {
    lastAnchorPath.value = null;
  }
}

function selectedSeedPath(): string | null {
  const currentPath = props.selectedPath;
  if (!currentPath) return null;
  return selectablePaths.value.includes(currentPath) ? currentPath : null;
}

function rowClick(row: FlatRow, event: MouseEvent) {
  if (isSearchMode.value) return;
  closeContextMenu();
  closeInlineCreate();
  closeInlineRename();
  focusedPath.value = row.node.path;
  const selection = resolveKnowledgeExplorerSelection({
    visiblePaths: selectablePaths.value,
    selectedPaths: selectedPaths.value,
    lastAnchorPath: lastAnchorPath.value,
    clickedPath: row.node.path,
    shiftKey: event.shiftKey,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    seedPath: selectedSeedPath(),
  });
  selectedPaths.value = selection.nextSelectedPaths;
  lastAnchorPath.value = selection.nextLastAnchorPath;
  if (!selection.shouldHandleAsPlainClick) return;
  // The first click of a double-click already ran the plain-click action;
  // swallowing the second keeps dblclick-rename from toggling branches twice.
  if (event.detail >= 2) return;
  activateNode(row);
}

// Unified single-click semantics: every node selects (shows its detail on the
// right) and branch nodes additionally toggle. Packages keep their detail
// pane stable while expanded children are open, so a second click on the
// already-open package only toggles.
function activateNode(row: FlatRow) {
  if (row.node.kind === "folder") {
    emit("toggle", row.node.path);
    if (props.selectedPath !== row.node.path) {
      emit("selectFolderConfig", row.node.relativePath);
    }
    return;
  }
  if (row.node.kind === "package") {
    if (props.selectedPath === row.node.path) {
      emit("toggle", row.node.path);
      return;
    }
    emit("selectPackage", row.node.document);
    return;
  }
  emit("selectDocument", row.node.document);
}

function onRowDoubleClick(row: FlatRow) {
  if (isSearchMode.value) return;
  if (row.node.kind === "package") return;
  if (isManagedNode(row.node)) return;
  void startRenameNode(row.node);
}

function toggleExpansion(row: FlatRow) {
  if (!isBranchNode(row.node)) return;
  emit("toggle", row.node.path);
}

const TREE_INDENT_BASE_PX = 12;
const TREE_INDENT_STEP_PX = 20;

function treeIndentPx(depth: number): number {
  if (depth <= 1) return TREE_INDENT_BASE_PX;
  return TREE_INDENT_BASE_PX + (depth - 1) * TREE_INDENT_STEP_PX;
}

function indentPx(node: ExplorerNode): number {
  return treeIndentPx(node.depth);
}

function createIndentPx(depth: number): number {
  return treeIndentPx(depth);
}

function loadMoreIndentPx(depth: number): number {
  return treeIndentPx(depth);
}

function nodeParentDir(node: FolderNode): string {
  return node.relativePath;
}

function openContextMenu(event: MouseEvent, row: FlatRow) {
  if (isSearchMode.value) return;
  event.preventDefault();
  event.stopPropagation();
  closeInlineCreate();
  closeInlineRename();
  const targetPaths = resolveKnowledgeContextSelection({
    visiblePaths: selectablePaths.value,
    selectedPaths: selectedPaths.value,
    targetPath: row.node.path,
  });
  const targetNodes = targetPaths
    .map((path) => selectableRowMap.value.get(path)?.node)
    .filter((node): node is ExplorerNode => !!node);
  if (targetNodes.length <= 1) {
    clearMultiSelection();
  }
  if (row.node.kind === "folder") {
    ctxMenu.value = {
      x: event.clientX,
      y: event.clientY,
      kind: "folder",
      node: row.node,
      parentDir: nodeParentDir(row.node),
      anchorPath: row.node.path,
      depth: row.node.depth,
      expanded: row.expanded,
      childCount: row.directChildCount,
      targetNodes,
    };
    return;
  }
  if (row.node.kind === "package") {
    ctxMenu.value = {
      x: event.clientX,
      y: event.clientY,
      kind: "package",
      node: row.node,
      targetNodes,
    };
    return;
  }
  ctxMenu.value = {
    x: event.clientX,
    y: event.clientY,
    kind: "leaf",
    node: row.node,
    targetNodes,
  };
}

function openRootContextMenu(event: MouseEvent) {
  if (isSearchMode.value) return;
  event.preventDefault();
  event.stopPropagation();
  clearMultiSelection(true);
  closeInlineCreate();
  closeInlineRename();
  ctxMenu.value = {
    x: event.clientX,
    y: event.clientY,
    kind: "root",
    anchorPath: props.activeType,
  };
}

function onTreeContextMenu(event: MouseEvent) {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".kx-row-shell, .kx-create-row, .kx-load-row, .kx-search-row",
    )
  ) {
    return;
  }
  openRootContextMenu(event);
}

function closeContextMenu() {
  ctxMenu.value = null;
}

function closeInlineCreate() {
  inlineCreate.value = null;
}

function closeInlineRename() {
  inlineRename.value = null;
}

const DRAG_EXPAND_DELAY_MS = 600;
let dragExpandTimer: number | null = null;
let dragExpandPath: string | null = null;

function cancelDragExpand() {
  if (dragExpandTimer !== null) {
    window.clearTimeout(dragExpandTimer);
    dragExpandTimer = null;
  }
  dragExpandPath = null;
}

// Hovering a collapsed folder mid-drag expands it after a short dwell so deep
// targets are reachable in one drag.
function scheduleDragExpand(row: FlatRow) {
  if (row.node.kind !== "folder") return;
  if (row.expanded || row.directChildCount === 0) {
    cancelDragExpand();
    return;
  }
  if (dragExpandPath === row.node.path) return;
  cancelDragExpand();
  dragExpandPath = row.node.path;
  dragExpandTimer = window.setTimeout(() => {
    dragExpandTimer = null;
    const path = dragExpandPath;
    dragExpandPath = null;
    if (path && dragTargetPath.value === path) emit("toggle", path);
  }, DRAG_EXPAND_DELAY_MS);
}

function clearDragState() {
  if (draggingNodes.value.length) emit("dragStateChange", false);
  draggingNodes.value = [];
  dragTargetPath.value = null;
  cancelDragExpand();
}

function normalizeRelativePath(path: string): string {
  return path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function parentDirectory(node: ExplorerNode): string {
  const path =
    node.kind === "folder"
      ? normalizeRelativePath(node.relativePath)
      : node.kind === "package"
        ? normalizeRelativePath(node.relativePath)
      : normalizeRelativePath(node.document.path);
  const segments = path.split("/").filter(Boolean);
  return segments.slice(0, -1).join("/");
}

function canDragNode(node: ExplorerNode): boolean {
  if (isManagedNode(node)) return false;
  if (node.kind === "document") return true;
  if (node.kind === "package") return false;
  return !!node.relativePath.trim();
}

function canDropOnDir(node: ExplorerNode, targetDir: string): boolean {
  const normalizedTargetDir = normalizeRelativePath(targetDir);
  if (node.kind === "package") return false;
  if (node.kind === "document") {
    return parentDirectory(node) !== normalizedTargetDir;
  }

  const sourceDir = normalizeRelativePath(node.relativePath);
  if (!sourceDir) return false;
  if (normalizedTargetDir === sourceDir) return false;
  if (normalizedTargetDir.startsWith(`${sourceDir}/`)) return false;
  return parentDirectory(node) !== normalizedTargetDir;
}

function dropTargetAccepts(targetDir: string): boolean {
  return draggingNodes.value.some((node) => canDropOnDir(node, targetDir));
}

function droppableNodes(targetDir: string): ExplorerNode[] {
  return draggingNodes.value.filter((node) => canDropOnDir(node, targetDir));
}

function onNodeDragStart(row: FlatRow, event: DragEvent) {
  if (isSearchMode.value) {
    event.preventDefault();
    return;
  }
  if (!canDragNode(row.node)) {
    event.preventDefault();
    return;
  }
  closeContextMenu();
  closeInlineCreate();
  closeInlineRename();
  // Dragging a row that belongs to the multi-selection carries the whole
  // selection; nested entries are pruned so an ancestor folder moves them.
  const dragPaths =
    selectedPaths.value.has(row.node.path) && selectedPaths.value.size > 1
      ? selectablePaths.value.filter((path) => selectedPaths.value.has(path))
      : [row.node.path];
  const dragNodes = pruneKnowledgeDragNodes(
    dragPaths
      .map((path) => selectableRowMap.value.get(path)?.node)
      .filter((node): node is ExplorerNode => !!node && canDragNode(node)),
  );
  draggingNodes.value = dragNodes.length ? dragNodes : [row.node];
  dragTargetPath.value = null;
  emit("dragStateChange", true);
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData(
      "text/plain",
      draggingNodes.value.map((node) => node.path).join("\n"),
    );
  }
}

function onNodeDragEnd() {
  clearDragState();
}

function onFolderDragOver(row: FlatRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNodes.value.length) return;
  if (isManagedNode(row.node)) {
    cancelDragExpand();
    return;
  }
  const targetDir = row.node.relativePath;
  if (!dropTargetAccepts(targetDir)) {
    cancelDragExpand();
    return;
  }
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetPath.value = row.node.path;
  scheduleDragExpand(row);
}

function onFolderDrop(row: FlatRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNodes.value.length) return;
  if (isManagedNode(row.node)) {
    clearDragState();
    return;
  }
  const targetDir = row.node.relativePath;
  const movable = droppableNodes(targetDir);
  if (!movable.length) {
    clearDragState();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  clearDragState();
  emit("moveNodes", movable, targetDir);
}

function onTreeDragOver(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNodes.value.length) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".kx-row-shell, .kx-create-row, .kx-load-row, .kx-search-row",
    )
  ) {
    return;
  }
  if (!dropTargetAccepts("")) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetPath.value = props.activeType;
}

function onTreeDrop(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNodes.value.length) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".kx-row-shell, .kx-create-row, .kx-load-row, .kx-search-row",
    )
  ) {
    return;
  }
  const movable = droppableNodes("");
  if (!movable.length) {
    clearDragState();
    return;
  }
  event.preventDefault();
  clearDragState();
  emit("moveNodes", movable, "");
}

function canDeleteFolder(
  menu: Extract<ContextMenuState, { kind: "folder" }>,
): boolean {
  return menu.depth > 0 && !menu.targetNodes.some(isManagedNode);
}

function isPluginManagedNode(node: ExplorerNode): boolean {
  if (node.kind === "package") {
    return (
      !!node.managedByPlugin ||
      !!node.document.externalSource?.locator?.startsWith("plugin://")
    );
  }
  if (node.kind === "document") {
    return !!node.document.externalSource?.locator?.startsWith("plugin://");
  }
  return node.children.some(isPluginManagedNode);
}

// Skill package contents are read-only virtual paths mounted into the tree;
// the package root node keeps its own lifecycle menu, but nodes inside a
// package cannot be created, renamed, moved, or deleted from here.
function isPackageContentNode(node: ExplorerNode): boolean {
  if (node.kind === "package") return false;
  if (node.kind === "document") {
    return node.document.externalSource?.provider === "package";
  }
  return node.children.some(isPackageContentNode);
}

function isManagedNode(node: ExplorerNode): boolean {
  return isPluginManagedNode(node) || isPackageContentNode(node);
}

function managedHint(nodes: ExplorerNode[]): string | undefined {
  if (nodes.some(isPluginManagedNode)) {
    return t("knowledge.explorer.pluginManagedHint");
  }
  if (nodes.some(isPackageContentNode)) {
    return t("knowledge.explorer.packageManagedHint");
  }
  return undefined;
}

function rowLockTitle(node: ExplorerNode): string | null {
  if (isPluginManagedNode(node)) return t("knowledge.explorer.pluginManaged");
  if (isPackageContentNode(node)) return t("knowledge.explorer.packageManaged");
  return null;
}

function createBlockHint(menu: ContextMenuState): string | undefined {
  return menu.kind === "folder" ? managedHint([menu.node]) : undefined;
}

function canDeleteContextTargets(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" | "package" }>,
): boolean {
  if (menu.targetNodes.some(isManagedNode)) return false;
  return menu.kind !== "folder" || menu.targetNodes.length > 1 || canDeleteFolder(menu);
}

// Plugin-managed targets keep their destructive menu items visible but
// disabled (with an explanatory title) instead of silently vanishing.
function canShowDeleteItem(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" | "package" }>,
): boolean {
  if (menu.kind !== "folder") return true;
  return menu.targetNodes.length > 1 || menu.depth > 0;
}

function deleteBlocked(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" | "package" }>,
): boolean {
  return menu.targetNodes.some(isManagedNode);
}

function renameBlocked(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" }>,
): boolean {
  return menu.targetNodes.some(isManagedNode);
}

function requestDeleteSelectedNodes() {
  const menu = ctxMenu.value;
  if (!menu || menu.kind === "root") return;
  if (!canDeleteContextTargets(menu)) return;
  closeContextMenu();
  emit("requestDeleteNodes", menu.targetNodes);
}

function openSelectedFolderConfig() {
  const menu = ctxMenu.value;
  if (!menu || menu.kind !== "folder" || menu.targetNodes.length !== 1) return;
  closeContextMenu();
  emit("selectFolderConfig", menu.node.relativePath);
}

function createActionLabel(kind: InlineCreateState["kind"]): string {
  return kind === "folder"
    ? t("knowledge.explorer.createFolder")
    : t("knowledge.explorer.createDoc");
}

function openExternalImportFolderDialog() {
  if (!ctxMenu.value || props.activeType !== "reference") return;
  if (ctxMenu.value.kind === "leaf" || ctxMenu.value.kind === "package") return;
  const parentDir =
    ctxMenu.value.kind === "folder" ? ctxMenu.value.parentDir : "";
  closeContextMenu();
  emit("requestExternalImportFolder", parentDir);
}

function importSkillPackageArchive() {
  if (!ctxMenu.value || props.activeType !== "skill") return;
  if (ctxMenu.value.kind !== "root") return;
  closeContextMenu();
  emit("importSkillPackage");
}

function exportSelectedPackage() {
  const menu = ctxMenu.value;
  if (!menu || menu.kind !== "package" || menu.targetNodes.length !== 1) return;
  closeContextMenu();
  emit("exportPackage", menu.node);
}

async function openInlineCreateAt(
  kind: InlineCreateState["kind"],
  target: {
    parentDir: string;
    anchorPath: string;
    depth: number;
    expandPath?: string | null;
  },
) {
  closeInlineRename();
  if (target.expandPath && !props.isPathExpanded(target.expandPath)) {
    emit("toggle", target.expandPath);
  }
  inlineCreate.value = {
    kind,
    parentDir: target.parentDir,
    anchorPath: target.anchorPath,
    depth: target.depth,
    name: "",
  };
  closeContextMenu();
  await nextTick();
  inlineInputRef.value?.focus();
  inlineInputRef.value?.select();
}

async function openCreateInline(kind: InlineCreateState["kind"]) {
  if (!ctxMenu.value) return;
  if (ctxMenu.value.kind === "leaf" || ctxMenu.value.kind === "package") return;
  if (ctxMenu.value.kind === "folder" && isManagedNode(ctxMenu.value.node)) {
    return;
  }
  const menu = ctxMenu.value;
  await openInlineCreateAt(kind, {
    parentDir: menu.kind === "folder" ? menu.parentDir : "",
    anchorPath: menu.anchorPath,
    depth: menu.kind === "folder" ? menu.depth + 1 : 1,
    expandPath:
      menu.kind === "folder" && !menu.expanded ? menu.anchorPath : null,
  });
}

// Toolbar create targets the selected folder when one is open in the detail
// pane, otherwise the type root.
async function openToolbarCreate(kind: InlineCreateState["kind"]) {
  if (isSearchMode.value) return;
  closeContextMenu();
  const selected = props.selectedPath
    ? selectableRowMap.value.get(props.selectedPath)
    : undefined;
  if (
    selected &&
    selected.node.kind === "folder" &&
    !isManagedNode(selected.node)
  ) {
    await openInlineCreateAt(kind, {
      parentDir: selected.node.relativePath,
      anchorPath: selected.node.path,
      depth: selected.node.depth + 1,
      expandPath: selected.expanded ? null : selected.node.path,
    });
    return;
  }
  await openInlineCreateAt(kind, {
    parentDir: "",
    anchorPath: props.activeType,
    depth: 1,
  });
}

const showToolbarImport = computed(
  () => props.activeType === "skill" || props.activeType === "reference",
);

const toolbarImportLabel = computed(() =>
  props.activeType === "skill"
    ? t("knowledge.explorer.importSkillPackage")
    : t("knowledge.explorer.importExternalFolder"),
);

function toolbarImport() {
  if (props.activeType === "skill") {
    emit("importSkillPackage");
    return;
  }
  if (props.activeType === "reference") {
    emit("requestExternalImportFolder", "");
  }
}

function openLegendMenu(event: MouseEvent) {
  const button = event.currentTarget;
  const rect =
    button instanceof HTMLElement ? button.getBoundingClientRect() : null;
  legendMenu.value = rect
    ? { x: rect.left, y: rect.bottom + 4 }
    : { x: event.clientX, y: event.clientY };
}

function legendFlagClass(tone: string): Record<string, boolean> {
  return {
    "flag-inject": tone === "inject",
    "flag-inject-strong": tone === "inject-strong",
    "flag-auto": tone === "auto",
    "flag-command": tone === "command",
    "flag-search-on": tone === "search-on",
    "flag-external": tone === "external",
  };
}

async function startRenameNode(node: FolderNode | DocumentNode) {
  closeInlineCreate();
  inlineRename.value =
    node.kind === "folder"
      ? {
          kind: "folder",
          anchorPath: node.path,
          relativePath: node.relativePath,
          currentName: node.name,
          name: node.name,
        }
      : {
          kind: "document",
          anchorPath: node.path,
          relativePath: node.document.path,
          currentName: node.name,
          name: node.name,
        };
  closeContextMenu();
  await nextTick();
  inlineRenameInputRef.value?.focus();
  inlineRenameInputRef.value?.select();
}

async function startRenameSelection() {
  const menu = ctxMenu.value;
  if (
    !menu ||
    menu.kind === "root" ||
    menu.kind === "package" ||
    menu.targetNodes.length !== 1
  )
    return;
  if (renameBlocked(menu)) return;
  await startRenameNode(menu.node);
}

function copySelectedRelativePath() {
  const menu = ctxMenu.value;
  if (!menu || menu.kind === "root" || menu.targetNodes.length !== 1) return;
  closeContextMenu();
  emit("copyRelativePath", menu.node);
}

function openSelectedInFileSystem() {
  const menu = ctxMenu.value;
  if (!menu || menu.kind === "root" || menu.targetNodes.length !== 1) return;
  closeContextMenu();
  emit("openInFileSystem", menu.node);
}

function submitInlineCreate() {
  const draft = inlineCreate.value;
  if (!draft) return;
  const name = draft.name.trim();
  if (!name) return;
  if (draft.kind === "folder") emit("createFolder", draft.parentDir, name);
  else emit("createDocument", draft.parentDir, name);
  closeInlineCreate();
}

function submitInlineRename() {
  const draft = inlineRename.value;
  if (!draft) return;
  const name = draft.name.trim();
  closeInlineRename();
  if (!name || name === draft.currentName) return;
  if (draft.kind === "folder") {
    emit("renameFolder", draft.relativePath, name);
    return;
  }
  emit("renameDocument", draft.relativePath, name, props.activeType);
}

function isRenamingRow(row: FlatRow): boolean {
  return inlineRename.value?.anchorPath === row.node.path;
}

function handleDocumentPointerDown(event: PointerEvent) {
  const target = event.target;
  if (!(target instanceof Node)) return;
  // Clicking elsewhere commits both inline editors alike (empty input simply
  // cancels) so create and rename do not behave differently on outside click.
  if (inlineCreate.value && !inlineCreateRowRef.value?.contains(target)) {
    if (inlineCreate.value.name.trim()) submitInlineCreate();
    else closeInlineCreate();
  }
  if (inlineRename.value && !inlineRenameRowRef.value?.contains(target)) {
    submitInlineRename();
  }
}

onMounted(() => {
  document.addEventListener("pointerdown", handleDocumentPointerDown, true);
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", handleDocumentPointerDown, true);
  cancelDragExpand();
});

function indexOfVisiblePath(path: string): number {
  return visibleRows.value.findIndex(
    (entry) => entry.type === "row" && entry.row.node.path === path,
  );
}

function revealVisiblePath(
  path: string,
  options?: { align?: "auto" | "center" },
): boolean {
  const index = indexOfVisiblePath(path);
  if (index < 0) return false;
  treeListRef.value?.scrollToIndex(index, options);
  return true;
}

// Toolbar "reveal selection": the host expands ancestors, then the row scrolls
// into view once it lands in the flattened rows.
function requestRevealSelection() {
  const path = props.selectedPath;
  if (!path) return;
  pendingRevealPath.value = path;
  emit("expandToSelection");
  void nextTick(() => flushPendingReveal());
}

function flushPendingReveal() {
  const path = pendingRevealPath.value;
  if (!path) return;
  if (revealVisiblePath(path, { align: "center" })) {
    pendingRevealPath.value = null;
    focusedPath.value = path;
  }
}

watch(
  selectablePaths,
  (paths) => {
    const visible = new Set(paths);
    if (selectedPaths.value.size > 0) {
      const next = new Set(
        Array.from(selectedPaths.value).filter((path) => visible.has(path)),
      );
      if (next.size !== selectedPaths.value.size) {
        selectedPaths.value = next;
      }
    }
    if (lastAnchorPath.value && !visible.has(lastAnchorPath.value)) {
      lastAnchorPath.value = null;
    }
    if (focusedPath.value && !visible.has(focusedPath.value)) {
      focusedPath.value = null;
    }
    if (inlineRename.value && !visible.has(inlineRename.value.anchorPath)) {
      closeInlineRename();
    }
    if (
      ctxMenu.value &&
      ctxMenu.value.kind !== "root" &&
      !visible.has(ctxMenu.value.node.path)
    ) {
      closeContextMenu();
    }
    if (pendingRevealPath.value) {
      void nextTick(() => flushPendingReveal());
    }
  },
  { immediate: true },
);

watch(
  () => props.activeType,
  () => {
    clearMultiSelection(true);
    closeContextMenu();
    closeInlineCreate();
    closeInlineRename();
    focusedPath.value = null;
    pendingRevealPath.value = null;
    legendMenu.value = null;
    searchCtxMenu.value = null;
  },
);

watch(isSearchMode, (value) => {
  if (value) {
    clearMultiSelection(true);
    closeContextMenu();
    closeInlineCreate();
    closeInlineRename();
    return;
  }
  searchCtxMenu.value = null;
  // Returning from search keeps the opened document on screen.
  const path = props.selectedPath;
  if (path) void nextTick(() => revealVisiblePath(path));
});

watch(
  () => props.selectedPath,
  (path) => {
    if (!path || isSearchMode.value) return;
    void nextTick(() => revealVisiblePath(path));
  },
);

const keyboardRows = computed<KnowledgeTreeKeyboardRow[]>(() =>
  selectableRows.value.map((entry) => ({
    path: entry.row.node.path,
    kind: entry.row.node.kind,
    depth: entry.row.node.depth,
    expanded: entry.row.expanded,
    hasChildren: entry.row.directChildCount > 0,
  })),
);

function rowDomId(path: string): string {
  return `kx-node-${path.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
}

const focusedRowDomId = computed(() =>
  focusedPath.value && selectableRowMap.value.has(focusedPath.value)
    ? rowDomId(focusedPath.value)
    : undefined,
);

function onTreeKeydown(event: KeyboardEvent) {
  if (isSearchMode.value) return;
  if (inlineCreate.value || inlineRename.value) return;
  const target = event.target;
  if (
    target instanceof HTMLElement &&
    target.closest("input, textarea, [contenteditable]")
  ) {
    return;
  }
  const action = resolveKnowledgeTreeKeyboardAction({
    key: event.key,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    rows: keyboardRows.value,
    focusedPath: focusedPath.value ?? props.selectedPath,
  });
  if (!action) return;
  event.preventDefault();
  event.stopPropagation();
  applyKeyboardAction(action);
}

function applyKeyboardAction(action: KnowledgeTreeKeyboardAction) {
  switch (action.type) {
    case "focus":
      focusedPath.value = action.path;
      revealVisiblePath(action.path);
      return;
    case "expand":
    case "collapse":
      focusedPath.value = action.path;
      emit("toggle", action.path);
      return;
    case "activate": {
      const row = selectableRowMap.value.get(action.path);
      if (!row) return;
      focusedPath.value = action.path;
      clearMultiSelection();
      activateNode(row);
      return;
    }
    case "rename": {
      const row = selectableRowMap.value.get(action.path);
      if (!row || row.node.kind === "package") return;
      if (isManagedNode(row.node)) return;
      void startRenameNode(row.node);
      return;
    }
    case "delete": {
      const row = selectableRowMap.value.get(action.path);
      if (!row) return;
      const targetPaths = resolveKnowledgeContextSelection({
        visiblePaths: selectablePaths.value,
        selectedPaths: selectedPaths.value,
        targetPath: action.path,
      });
      const targetNodes = targetPaths
        .map((path) => selectableRowMap.value.get(path)?.node)
        .filter((node): node is ExplorerNode => !!node);
      if (!targetNodes.length || targetNodes.some(isManagedNode)) return;
      emit("requestDeleteNodes", targetNodes);
      return;
    }
    case "select-all":
      selectedPaths.value = new Set(selectablePaths.value);
      lastAnchorPath.value = selectablePaths.value[0] ?? null;
      return;
    case "clear-selection":
      clearMultiSelection(true);
      return;
  }
}

function documentTags(node: DocumentNode): Array<{
  text: string;
  tone: KnowledgeListTag["tone"] | "command";
  title: string;
}> {
  const tags: Array<{
    text: string;
    tone: KnowledgeListTag["tone"] | "command";
    title: string;
  }> = [];
  // The package folder row already renders the command trigger via
  // packageTags(); its SKILL.md child reuses the same document, so skip it here
  // to avoid showing the same /command twice.
  const trigger = node.document.commandTrigger?.trim();
  if (trigger && !isSkillPackageRootDocument(node.document)) {
    tags.push({
      text: trigger,
      tone: "command",
      title: t("knowledge.skill.commandTrigger"),
    });
  }
  tags.push(
    ...buildKnowledgeListTags({
      injectMode: node.document.injectMode,
      aiMaintained: node.document.aiMaintained,
    }),
  );
  return tags;
}

function searchResultTags(result: KnowledgeSearchResult) {
  return buildKnowledgeSearchMatchTags(result.matchKind);
}

function searchSnippetSegments(result: KnowledgeSearchResult) {
  return buildKnowledgeSnippetSegments(
    result.snippet,
    result.matchedTerms,
    props.searchQuery,
  );
}

function openSearchContextMenu(
  event: MouseEvent,
  result: KnowledgeSearchResult,
) {
  event.preventDefault();
  event.stopPropagation();
  searchCtxMenu.value = { x: event.clientX, y: event.clientY, result };
}

function closeSearchContextMenu() {
  searchCtxMenu.value = null;
}

function openSearchResultFromMenu() {
  const menu = searchCtxMenu.value;
  if (!menu) return;
  closeSearchContextMenu();
  emit("selectSearchResult", menu.result);
}

function revealSearchResultFromMenu() {
  const menu = searchCtxMenu.value;
  if (!menu) return;
  closeSearchContextMenu();
  emit("revealSearchResult", menu.result);
}

function copySearchResultPathFromMenu() {
  const menu = searchCtxMenu.value;
  if (!menu) return;
  closeSearchContextMenu();
  emit("copySearchResultPath", menu.result);
}

function isSelectedSearchResult(result: KnowledgeSearchResult): boolean {
  const normalizedPath = result.path.replace(/\\/g, "/").replace(/^\/+/, "");
  return props.selectedPath === `${result.type}/${normalizedPath}`;
}

function folderTags(node: FolderNode) {
  const tags = [];
  const externalTag = buildExternalFolderTag(
    props.externalDirectorySources[node.relativePath],
  );
  if (externalTag) tags.push(externalTag);
  if (isBuiltinSkillGroupFolder(node)) return tags;
  if (node.depth !== 1) return tags;
  const config = props.rootDirectoryConfigs[node.relativePath];
  if (!config) return tags;
  tags.push(
    ...buildFolderListTags({
      injectMode: config.injectMode,
      lexicalEnabled: config.effectiveLexicalSearch.enabled,
      semanticEnabled: config.effectiveVectorSearch.enabled,
    }),
  );
  return tags;
}

function isBuiltinSkillGroupFolder(node: FolderNode): boolean {
  return props.activeType === "skill" && node.depth === 1 && node.relativePath === "builtin";
}

function documentKindIconClass(node: DocumentNode): string {
  return node.document.type === "skill" ? "skill-document" : "document";
}

function documentIconNode(node: DocumentNode) {
  return unityAssetIconNodeForPath(node.document.path || node.path, {
    isFolder: false,
  });
}

function documentIconClass(node: DocumentNode) {
  return unityAssetIconClassForPath(node.document.path || node.path, {
    isFolder: false,
  });
}

function packageTags(node: PackageNode): Array<{
  text: string;
  tone: KnowledgeListTag["tone"] | "command";
  title: string;
}> {
  const tags: Array<{
    text: string;
    tone: KnowledgeListTag["tone"] | "command";
    title: string;
  }> = [];
  const trigger = node.document.commandTrigger?.trim();
  if (trigger) {
    tags.push({
      text: trigger,
      tone: "command",
      title: t("knowledge.skill.commandTrigger"),
    });
  }
  if (node.document.injectMode === "excerpt") {
    tags.push(
      ...buildKnowledgeListTags({
        injectMode: node.document.injectMode,
        aiMaintained: false,
      }),
    );
  }
  return tags;
}

function deleteMenuLabel(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" | "package" }>,
): string {
  if (menu.targetNodes.length > 1) {
    return t("knowledge.explorer.deleteMany", menu.targetNodes.length);
  }
  if (menu.kind === "package") return t("knowledge.explorer.deletePackage");
  return menu.kind === "folder"
    ? t("knowledge.ctx.deleteFolder")
    : t("knowledge.explorer.delete");
}

function loadMoreLabel(entry: VisibleEntry): string {
  if (entry.type !== "loadMore") return "";
  return entry.loading ? t("common.loading") : t("asset.explorer.loadMore");
}

let lastVisibleRangeRowCount = -1;

function handleVisibleRangeChange(payload: { start: number; end: number }) {
  if (payload.end < payload.start) return;
  const rowCount = visibleRows.value.length;
  // Folder pages only chain on scroll-driven range changes (row count stable).
  // Structural changes — expanding a folder, a page landing — must not cascade
  // extra folder loads; the user keeps explicit control right after expansion.
  const scrollDriven = rowCount === lastVisibleRangeRowCount;
  lastVisibleRangeRowCount = rowCount;
  for (const entry of visibleRows.value.slice(payload.start, payload.end + 1)) {
    if (entry.type !== "loadMore") continue;
    if (entry.loading) continue;
    if (entry.path) {
      if (!scrollDriven) continue;
      if (!props.hasMoreFolderDocuments(entry.path)) continue;
      emit("loadMoreFolder", entry.path);
      continue;
    }
    if (!props.hasMoreRootDocuments) continue;
    emit("loadMoreRoot");
  }
}

function requestLoadMore(entry: VisibleEntry) {
  if (entry.type !== "loadMore" || entry.loading) return;
  if (entry.path) {
    if (!props.hasMoreFolderDocuments(entry.path)) return;
    emit("loadMoreFolder", entry.path);
    return;
  }
  if (!props.hasMoreRootDocuments) return;
  emit("loadMoreRoot");
}

function asVisibleEntry(item: { key: string }): VisibleEntry {
  return item as VisibleEntry;
}
</script>

<template>
  <div class="kx-explorer">
    <div v-if="!isSearchMode" class="kx-toolbar">
      <BaseButton
        class="kx-toolbar-btn"
        type="button"
        :title="t('knowledge.explorer.createDoc')"
        @click="openToolbarCreate('document')"
      >
        <LucideIcon :icon="FilePlus" :size="13" :stroke-width="2" />
      </BaseButton>
      <BaseButton
        class="kx-toolbar-btn"
        type="button"
        :title="t('knowledge.explorer.createFolder')"
        @click="openToolbarCreate('folder')"
      >
        <LucideIcon :icon="FolderPlus" :size="13" :stroke-width="2" />
      </BaseButton>
      <BaseButton
        v-if="showToolbarImport"
        class="kx-toolbar-btn"
        type="button"
        :title="toolbarImportLabel"
        @click="toolbarImport"
      >
        <LucideIcon
          :icon="activeType === 'skill' ? PackagePlus : FolderInput"
          :size="13"
          :stroke-width="2"
        />
      </BaseButton>
      <span class="kx-toolbar-spacer"></span>
      <BaseButton
        class="kx-toolbar-btn"
        type="button"
        :title="t('knowledge.explorer.collapseAll')"
        @click="emit('collapseAll')"
      >
        <LucideIcon :icon="ChevronsDownUp" :size="13" :stroke-width="2" />
      </BaseButton>
      <BaseButton
        class="kx-toolbar-btn"
        type="button"
        :disabled="!selectedPath"
        :title="t('knowledge.explorer.revealSelection')"
        @click="requestRevealSelection"
      >
        <LucideIcon :icon="LocateFixed" :size="13" :stroke-width="2" />
      </BaseButton>
      <BaseButton
        class="kx-toolbar-btn"
        type="button"
        :title="t('knowledge.explorer.legend')"
        @click="openLegendMenu"
      >
        <LucideIcon :icon="BadgeInfo" :size="13" :stroke-width="2" />
      </BaseButton>
    </div>
    <div
      class="kx-tree-shell"
      :class="{ 'is-root-drop-target': dragTargetPath === activeType }"
      role="tree"
      :aria-label="t('knowledge.explorer.title')"
      :tabindex="isSearchMode ? -1 : 0"
      :aria-activedescendant="focusedRowDomId"
      @keydown="onTreeKeydown"
      @contextmenu.prevent="onTreeContextMenu($event)"
      @dragover="onTreeDragOver"
      @drop="onTreeDrop"
    >
      <div v-if="isSearchMode && searching" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <div v-else-if="isSearchMode" class="kx-tree-static">
        <div
          v-for="result in searchResults"
          :key="`${result.id}-${result.path}`"
          role="button"
          tabindex="0"
          class="kx-search-row"
          :class="{ selected: isSelectedSearchResult(result) }"
          @click="emit('selectSearchResult', result)"
          @keydown.enter.prevent="emit('selectSearchResult', result)"
          @keydown.space.prevent="emit('selectSearchResult', result)"
          @contextmenu="openSearchContextMenu($event, result)"
        >
          <div class="kx-search-main">
            <span class="kx-search-title">{{ result.title }}</span>
            <span class="kx-search-path">{{ result.path }}</span>
            <span
              v-if="searchSnippetSegments(result).length"
              class="kx-search-snippet"
            >
              <template
                v-for="(segment, segmentIndex) in searchSnippetSegments(result)"
                :key="segmentIndex"
              >
                <mark v-if="segment.highlighted" class="kx-search-mark">{{
                  segment.text
                }}</mark>
                <template v-else>{{ segment.text }}</template>
              </template>
            </span>
          </div>
          <div class="kx-search-side">
            <div v-if="searchResultTags(result).length" class="kx-search-tags">
              <span
                v-for="tag in searchResultTags(result)"
                :key="`${result.id}-${tag.text}`"
                class="kx-flag"
                :class="{
                  'flag-inject': tag.tone === 'inject',
                  'flag-auto': tag.tone === 'auto',
                }"
                :title="tag.title"
              >
                {{ tag.text }}
              </span>
            </div>
            <button
              type="button"
              class="kx-search-reveal"
              :title="t('knowledge.search.revealInTree')"
              @click.stop="emit('revealSearchResult', result)"
            >
              <LucideIcon :icon="ListTree" :size="13" :stroke-width="2" />
            </button>
          </div>
        </div>
        <div v-if="!searchResults.length" class="kx-empty">
          {{ t("knowledge.search.noResults") }}
        </div>
      </div>
      <div v-else-if="loading && !tree.length" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <FileTreeList
        v-else-if="visibleRows.length"
        ref="treeListRef"
        class="kx-tree"
        :items="visibleRows"
        :row-height="30"
        @visible-range-change="handleVisibleRangeChange"
      >
        <template #item="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <div
              v-if="entry.type === 'row'"
              :id="rowDomId(entry.row.node.path)"
              class="kx-row-shell"
              :class="{
                'kx-folder': entry.row.node.kind === 'folder',
                'kx-package': entry.row.node.kind === 'package',
                'kx-leaf': entry.row.node.kind === 'document',
                'is-open': selectedPath === entry.row.node.path,
                'is-marked': selectedPaths.has(entry.row.node.path),
                'is-focused': focusedPath === entry.row.node.path,
                'context-selected': contextSelectedPath === entry.row.node.path,
                dragging: draggingPaths.has(entry.row.node.path),
                'drop-target': dragTargetPath === entry.row.node.path,
              }"
              role="treeitem"
              :aria-level="entry.row.node.depth"
              :aria-expanded="
                entry.row.node.kind !== 'document' &&
                entry.row.directChildCount > 0
                  ? entry.row.expanded
                  : undefined
              "
              :aria-selected="
                selectedPath === entry.row.node.path ||
                selectedPaths.has(entry.row.node.path)
              "
              :draggable="!isRenamingRow(entry.row) && canDragNode(entry.row.node)"
              @contextmenu.prevent="openContextMenu($event, entry.row)"
              @dblclick="onRowDoubleClick(entry.row)"
              @dragstart="onNodeDragStart(entry.row, $event)"
              @dragend="onNodeDragEnd"
              @dragover="onFolderDragOver(entry.row, $event)"
              @drop="onFolderDrop(entry.row, $event)"
            >
              <component
                :is="isRenamingRow(entry.row) ? 'div' : 'button'"
                :type="isRenamingRow(entry.row) ? undefined : 'button'"
                class="kx-row"
                :class="{ 'kx-row-editing': isRenamingRow(entry.row) }"
                :style="{ paddingLeft: indentPx(entry.row.node) + 'px' }"
                tabindex="-1"
                @click="
                  !isRenamingRow(entry.row) && rowClick(entry.row, $event)
                "
              >
                <span
                  v-if="
                    entry.row.node.kind !== 'document' &&
                    entry.row.directChildCount > 0
                  "
                  class="kx-branch-slot"
                  @click.stop="toggleExpansion(entry.row)"
                >
                  <LucideIcon
                    class="kx-chevron"
                    :class="{ open: entry.row.expanded }"
                    :icon="ChevronRight"
                    :size="10"
                    :stroke-width="2.4"
                  />
                </span>
                <span
                  v-else
                  class="kx-branch-spacer"
                  aria-hidden="true"
                ></span>
                <span
                  v-if="entry.row.node.kind === 'folder'"
                  class="kx-kind-icon folder"
                  :class="{ open: entry.row.expanded }"
                  aria-hidden="true"
                >
                  <LucideIcon
                    :icon="entry.row.expanded ? FolderOpen : Folder"
                    :size="13"
                    :stroke-width="2"
                  />
                </span>
                <span
                  v-else-if="entry.row.node.kind === 'package'"
                  class="kx-kind-icon package"
                  :class="{ open: entry.row.expanded }"
                  aria-hidden="true"
                >
                  <LucideIcon :icon="Package" :size="13" :stroke-width="2" />
                </span>
                <span
                  v-else
                  class="kx-kind-icon document"
                  :class="documentKindIconClass(entry.row.node)"
                  aria-hidden="true"
                >
                  <LucideIcon
                    :class="documentIconClass(entry.row.node)"
                    :icon="documentIconNode(entry.row.node)"
                    :size="13"
                    :stroke-width="2"
                  />
                </span>

                <template v-if="isRenamingRow(entry.row)">
                  <span class="kx-name-edit" :ref="setInlineRenameRowRef">
                    <input
                      :ref="setInlineRenameInputRef"
                      v-model="inlineRenameName"
                      class="kx-rename-input"
                      :placeholder="t('knowledge.explorer.namePlaceholder')"
                      :aria-label="t('knowledge.explorer.rename')"
                      @pointerdown.stop
                      @click.stop
                      @keydown.enter.prevent="submitInlineRename"
                      @keydown.esc.prevent="closeInlineRename"
                      @blur="submitInlineRename"
                    />
                  </span>
                </template>
                <span v-else class="kx-name">{{ entry.row.node.name }}</span>
              </component>

              <div v-if="entry.row.node.kind === 'folder'" class="kx-row-side">
                <span
                  v-if="rowLockTitle(entry.row.node)"
                  class="kx-lock"
                  :title="rowLockTitle(entry.row.node) ?? undefined"
                >
                  <LucideIcon :icon="Lock" :size="11" :stroke-width="2.2" />
                </span>
                <span
                  v-for="tag in folderTags(entry.row.node)"
                  :key="`${entry.row.node.path}-${tag.text}`"
                  class="kx-flag"
                  :class="{
                    'flag-external': tag.tone === 'external',
                    'flag-inject': tag.tone === 'inject',
                    'flag-inject-strong': tag.tone === 'inject-strong',
                    'flag-search-on': tag.tone === 'search-on',
                  }"
                  :title="tag.title"
                >
                  {{ tag.text }}
                </span>
              </div>
              <div
                v-else-if="entry.row.node.kind === 'package'"
                class="kx-row-side"
              >
                <span
                  v-if="isPluginManagedNode(entry.row.node)"
                  class="kx-lock"
                  :title="t('knowledge.explorer.pluginManaged')"
                >
                  <LucideIcon :icon="Lock" :size="11" :stroke-width="2.2" />
                </span>
                <span
                  v-for="tag in packageTags(entry.row.node)"
                  :key="`${entry.row.node.path}-${tag.text}`"
                  class="kx-flag"
                  :class="{
                    'flag-inject': tag.tone === 'inject',
                    'flag-inject-strong': tag.tone === 'inject-strong',
                    'flag-command': tag.tone === 'command',
                  }"
                  :title="tag.title"
                >
                  {{ tag.text }}
                </span>
              </div>
              <div
                v-else-if="
                  documentTags(entry.row.node).length ||
                  isPluginManagedNode(entry.row.node)
                "
                class="kx-row-side"
              >
                <span
                  v-if="isPluginManagedNode(entry.row.node)"
                  class="kx-lock"
                  :title="t('knowledge.explorer.pluginManaged')"
                >
                  <LucideIcon :icon="Lock" :size="11" :stroke-width="2.2" />
                </span>
                <span
                  v-for="tag in documentTags(entry.row.node)"
                  :key="`${entry.row.node.document.id}-${tag.text}`"
                  class="kx-flag"
                  :class="{
                    'flag-inject': tag.tone === 'inject',
                    'flag-inject-strong': tag.tone === 'inject-strong',
                    'flag-auto': tag.tone === 'auto',
                    'flag-command': tag.tone === 'command',
                  }"
                  :title="tag.title"
                >
                  {{ tag.text }}
                </span>
              </div>
            </div>

            <div
              v-else-if="entry.type === 'create'"
              class="kx-create-row"
              :ref="setInlineCreateRowRef"
              :style="{ paddingLeft: createIndentPx(entry.draft.depth) + 'px' }"
            >
              <span class="kx-bullet"></span>
              <div class="kx-create-body">
                <input
                  :ref="setInlineInputRef"
                  v-model="entry.draft.name"
                  class="kx-create-input"
                  :placeholder="t('knowledge.explorer.namePlaceholder')"
                  :aria-label="createActionLabel(entry.draft.kind)"
                  @keydown.enter.prevent="submitInlineCreate"
                  @keydown.esc.prevent="closeInlineCreate"
                />
                <div class="kx-create-actions">
                  <BaseButton
                    class="kx-create-action"
                    type="button"
                    :title="t('common.confirm')"
                    :disabled="!entry.draft.name.trim()"
                    @click="submitInlineCreate"
                  >
                    <LucideIcon :icon="Check" :size="12" :stroke-width="2.4" />
                  </BaseButton>
                  <BaseButton
                    class="kx-create-action"
                    type="button"
                    :title="t('common.cancel')"
                    @click="closeInlineCreate"
                  >
                    <LucideIcon :icon="X" :size="12" :stroke-width="2.4" />
                  </BaseButton>
                </div>
              </div>
            </div>

            <button
              v-else
              class="kx-load-row"
              :class="{ 'is-loading': entry.loading }"
              type="button"
              :style="{ paddingLeft: `${loadMoreIndentPx(entry.depth)}px` }"
              :disabled="entry.loading"
              @click="requestLoadMore(entry)"
            >
              <span class="kx-bullet-slot">
                <span class="kx-bullet"></span>
              </span>
              <span class="kx-load-label">{{ loadMoreLabel(entry) }}</span>
            </button>
          </template>
        </template>
      </FileTreeList>
      <div
        v-else
        class="kx-empty-state"
        @contextmenu.prevent="openRootContextMenu($event)"
      >
        <div class="kx-empty-title">{{ t("knowledge.explorer.empty") }}</div>
        <div class="kx-empty-hint">{{ t("knowledge.noFilesHint") }}</div>
        <div class="kx-empty-actions">
          <BaseButton
            class="kx-empty-action"
            type="button"
            @click="openToolbarCreate('document')"
          >
            {{ t("knowledge.explorer.createDoc") }}
          </BaseButton>
          <BaseButton
            v-if="showToolbarImport"
            class="kx-empty-action"
            type="button"
            @click="toolbarImport"
          >
            {{ toolbarImportLabel }}
          </BaseButton>
        </div>
      </div>
    </div>

    <BaseContextMenu
      v-if="ctxMenu"
      class="kx-ctx-menu"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :z-index="80"
      @close="closeContextMenu"
    >
          <template v-if="ctxMenu.kind === 'folder' || ctxMenu.kind === 'root'">
            <button
              v-if="
                ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="openSelectedFolderConfig"
            >
              {{ t("knowledge.explorer.folderConfig") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              :disabled="renameBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="startRenameSelection"
            >
              {{ t("knowledge.explorer.rename") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              {{ t("knowledge.explorer.openInFileSystem") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'root' ||
                (ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1)
              "
              type="button"
              class="kx-ctx-item"
              :disabled="!!createBlockHint(ctxMenu)"
              :title="createBlockHint(ctxMenu)"
              @click="openCreateInline('folder')"
            >
              {{ t("knowledge.explorer.createFolder") }}
            </button>
            <button
              v-if="
                props.activeType === 'reference' &&
                (ctxMenu.kind === 'root' ||
                  (ctxMenu.kind === 'folder' &&
                    ctxMenu.targetNodes.length === 1))
              "
              type="button"
              class="kx-ctx-item"
              @click="openExternalImportFolderDialog"
            >
              {{ t("knowledge.explorer.importExternalFolder") }}
            </button>
            <button
              v-if="props.activeType === 'skill' && ctxMenu.kind === 'root'"
              type="button"
              class="kx-ctx-item"
              @click="importSkillPackageArchive"
            >
              {{ t("knowledge.explorer.importSkillPackage") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'root' ||
                (ctxMenu.kind === 'folder' && ctxMenu.targetNodes.length === 1)
              "
              type="button"
              class="kx-ctx-item"
              :disabled="!!createBlockHint(ctxMenu)"
              :title="createBlockHint(ctxMenu)"
              @click="openCreateInline('document')"
            >
              {{ t("knowledge.explorer.createDoc") }}
            </button>
            <button
              v-if="ctxMenu.kind === 'folder' && canShowDeleteItem(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
              :disabled="deleteBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="requestDeleteSelectedNodes"
            >
              {{ deleteMenuLabel(ctxMenu) }}
            </button>
          </template>
          <template v-else-if="ctxMenu.kind === 'package'">
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="exportSelectedPackage"
            >
              {{ t("knowledge.explorer.exportSkillPackage") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              {{ t("knowledge.explorer.openInFileSystem") }}
            </button>
            <button
              v-if="canShowDeleteItem(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
              :disabled="deleteBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="requestDeleteSelectedNodes"
            >
              {{ deleteMenuLabel(ctxMenu) }}
            </button>
          </template>
          <template v-else>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              :disabled="renameBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="startRenameSelection"
            >
              {{ t("knowledge.explorer.rename") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              {{ t("knowledge.explorer.openInFileSystem") }}
            </button>
            <button
              v-if="canShowDeleteItem(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
              :disabled="deleteBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="requestDeleteSelectedNodes"
            >
              {{ deleteMenuLabel(ctxMenu) }}
            </button>
          </template>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="searchCtxMenu"
      class="kx-ctx-menu"
      :x="searchCtxMenu.x"
      :y="searchCtxMenu.y"
      :z-index="80"
      @close="closeSearchContextMenu"
    >
      <button type="button" class="kx-ctx-item" @click="openSearchResultFromMenu">
        {{ t("knowledge.search.openResult") }}
      </button>
      <button
        type="button"
        class="kx-ctx-item"
        @click="revealSearchResultFromMenu"
      >
        {{ t("knowledge.search.revealInTree") }}
      </button>
      <button
        type="button"
        class="kx-ctx-item"
        @click="copySearchResultPathFromMenu"
      >
        {{ t("knowledge.explorer.copyRelativePath") }}
      </button>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="legendMenu"
      class="kx-legend-menu"
      :x="legendMenu.x"
      :y="legendMenu.y"
      :z-index="80"
      role="dialog"
      :aria-label="t('knowledge.explorer.legend')"
      @close="legendMenu = null"
    >
      <div class="kx-legend-title">{{ t("knowledge.explorer.legend") }}</div>
      <div
        v-for="entry in legendEntries"
        :key="`${entry.tag.text}-${entry.label}`"
        class="kx-legend-row"
      >
        <span class="kx-flag" :class="legendFlagClass(entry.tag.tone)">{{
          entry.tag.text
        }}</span>
        <span class="kx-legend-text">
          <span class="kx-legend-label">{{ entry.label }}</span>
          <span class="kx-legend-desc">{{ entry.description }}</span>
        </span>
      </div>
    </BaseContextMenu>
  </div>
</template>

<style scoped>
.kx-explorer {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  overflow: hidden;
  /* Inline-size container so badges can degrade at narrow sidebar widths. */
  container-type: inline-size;
}

.kx-toolbar {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px 8px;
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 72%, transparent);
  flex-shrink: 0;
}

.kx-toolbar-spacer {
  flex: 1;
}

.kx-toolbar-btn {
  width: 24px;
  min-width: 24px;
  min-height: 24px;
  height: 24px;
  padding: 0;
  border-color: transparent;
}

.kx-tree-shell {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  outline: none;
}

.kx-tree {
  flex: 1;
  padding: 4px 0;
}

.kx-tree-static {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.kx-tree-shell.is-root-drop-target {
  background: color-mix(in srgb, var(--active-bg) 38%, transparent);
}

.kx-empty {
  padding: 16px 14px;
  font-size: 12px;
  color: var(--text-secondary);
}

.kx-empty-state {
  min-height: 100%;
  padding: 24px 16px 56px;
  display: flex;
  flex-direction: column;
  justify-content: flex-start;
  gap: 6px;
}

.kx-empty-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.5;
}

.kx-empty-hint {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.kx-empty-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 10px;
}

.kx-empty-action {
  min-height: 26px;
  padding: 0 10px;
  font-size: 12px;
}

.kx-search-row {
  width: 100%;
  padding: 8px 12px;
  border: none;
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 74%, transparent);
  background: transparent;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  text-align: left;
  cursor: pointer;
}

.kx-search-row:hover {
  background: var(--hover-bg);
}

.kx-search-row.selected,
.kx-search-row.selected:hover {
  background: var(--active-bg);
}

.kx-search-main,
.kx-search-side {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.kx-search-main {
  flex: 1;
}

.kx-search-side {
  align-items: flex-end;
  flex-shrink: 0;
}

.kx-search-tags {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 4px;
  min-height: 16px;
}

.kx-search-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.kx-search-path {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.35;
}

.kx-search-path {
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kx-search-snippet {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  margin-top: 2px;
  font-size: 11px;
  line-height: 1.45;
  color: var(--text-secondary);
  word-break: break-word;
}

.kx-search-mark {
  background: color-mix(in srgb, var(--accent-color) 22%, transparent);
  color: var(--text-color);
  border-radius: 2px;
  padding: 0 1px;
}

.kx-search-reveal {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  opacity: 0;
  transition:
    opacity 0.12s,
    background 0.12s;
}

.kx-search-row:hover .kx-search-reveal,
.kx-search-row:focus-within .kx-search-reveal,
.kx-search-reveal:focus-visible {
  opacity: 1;
}

.kx-search-reveal:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.kx-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  gap: 6px;
  width: 100%;
  min-width: 0;
  /* Must equal FileTreeList's row-height so virtualization math stays exact. */
  height: 30px;
  background: transparent;
  transition: background 0.1s;
}

.kx-row-shell:hover {
  background: var(--hover-bg);
}

/* The document open in the detail pane: filled row plus an accent bar. */
.kx-row-shell.is-open,
.kx-row-shell.is-open:hover {
  background: var(--active-bg);
  box-shadow: inset 2px 0 0 var(--accent-color);
}

/* Multi-select marks stay visually lighter than the opened row. */
.kx-row-shell.is-marked,
.kx-row-shell.is-marked:hover {
  background: color-mix(in srgb, var(--active-bg) 55%, transparent);
}

.kx-row-shell.is-open.is-marked,
.kx-row-shell.is-open.is-marked:hover {
  background: var(--active-bg);
  box-shadow: inset 2px 0 0 var(--accent-color);
}

.kx-tree-shell:focus-within .kx-row-shell.is-focused {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 55%, transparent);
  outline-offset: -1px;
}

.kx-row-shell.context-selected,
.kx-row-shell.context-selected:hover {
  background: color-mix(in srgb, var(--active-bg) 52%, var(--hover-bg) 48%);
  box-shadow: inset 0 0 0 1px
    color-mix(in srgb, var(--accent-color) 16%, var(--border-color));
}

.kx-row-shell.dragging {
  opacity: 0.48;
}

.kx-row-shell.drop-target,
.kx-row-shell.drop-target:hover {
  background: color-mix(in srgb, var(--active-bg) 62%, transparent);
  box-shadow: inset 0 0 0 1px
    color-mix(in srgb, var(--accent-color) 32%, var(--border-color));
}

.kx-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  height: 100%;
  padding: 2px 12px 2px 16px;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 13px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  min-width: 0;
}

.kx-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.kx-row-side {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  min-width: 30px;
  padding-right: 8px;
  flex-shrink: 0;
}

.kx-lock {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  opacity: 0.75;
  flex-shrink: 0;
}

.kx-branch-slot,
.kx-branch-spacer,
.kx-bullet-slot,
.kx-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.kx-branch-slot {
  border-radius: 4px;
}

.kx-branch-slot:hover {
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
}

.kx-chevron {
  flex-shrink: 0;
  opacity: 0.55;
  transition: transform 0.15s;
}

.kx-chevron.open {
  transform: rotate(90deg);
}

.kx-kind-icon {
  transition: color 0.15s ease;
}

.kx-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.kx-kind-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
}

.kx-kind-icon.package {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.kx-kind-icon.package.open {
  color: var(--accent-color);
}

.kx-kind-icon.document {
  color: color-mix(in srgb, var(--text-secondary) 82%, var(--text-color) 18%);
}

.kx-kind-icon.document.skill-document {
  color: color-mix(in srgb, var(--accent-color) 46%, var(--text-secondary) 54%);
}

.kx-bullet {
  display: inline-block;
  width: 10px;
  height: 10px;
  position: relative;
}

.kx-bullet::before {
  content: "";
  display: block;
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--text-secondary);
  opacity: 0.5;
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
}

.kx-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-color);
}

.kx-name-edit {
  flex: 1;
  min-width: 0;
}

.kx-rename-input {
  width: 100%;
  height: 22px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.kx-rename-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.kx-flag {
  font-size: 9px;
  font-weight: 700;
  padding: 1px 5px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--hover-bg) 80%, transparent);
  color: var(--text-secondary);
  border: 1px solid var(--border-color);
  flex-shrink: 0;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.kx-flag.flag-inject {
  color: var(--accent-color);
  border-color: color-mix(
    in srgb,
    var(--accent-color) 28%,
    var(--border-color)
  );
  background: color-mix(in srgb, var(--accent-color) 9%, transparent);
}

.kx-flag.flag-inject-strong {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
  background: color-mix(in srgb, var(--status-danger-bg) 92%, transparent);
}

.kx-flag.flag-auto {
  color: var(--text-color);
  border-color: color-mix(
    in srgb,
    var(--border-color) 78%,
    var(--text-secondary) 22%
  );
  background: color-mix(in srgb, var(--hover-bg) 86%, transparent);
}

.kx-flag.flag-command {
  color: var(--text-color);
  border-color: color-mix(
    in srgb,
    var(--border-color) 78%,
    var(--text-secondary) 22%
  );
  background: color-mix(in srgb, var(--hover-bg) 86%, transparent);
  font-family: var(--font-mono-identifier);
  font-weight: 600;
  text-transform: none;
  letter-spacing: 0;
}

.kx-flag.flag-search-on {
  color: var(--text-color);
  border-color: color-mix(
    in srgb,
    var(--accent-color) 20%,
    var(--border-color)
  );
  background: color-mix(in srgb, var(--accent-color) 8%, transparent);
}

.kx-flag.flag-external {
  color: color-mix(in srgb, var(--text-color) 88%, var(--text-secondary));
  border-color: color-mix(
    in srgb,
    var(--border-strong) 72%,
    var(--border-color)
  );
  background: color-mix(in srgb, var(--sidebar-bg) 82%, transparent);
}

.kx-create-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 30px;
  padding: 2px 12px 2px 16px;
  background: color-mix(in srgb, var(--active-bg) 78%, transparent);
}

.kx-create-body {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-width: 0;
}

.kx-create-input {
  flex: 1;
  min-width: 0;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.kx-create-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.kx-create-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}

.kx-create-action {
  width: 24px;
  min-width: 24px;
  height: 24px;
  padding: 0;
}

.kx-load-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  height: 30px;
  padding: 2px 12px 2px 16px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  text-align: left;
  font-size: 12px;
  transition: background 0.1s;
  cursor: pointer;
}

.kx-load-row:hover:not(:disabled) {
  background: var(--hover-bg);
}

.kx-load-row:disabled,
.kx-load-row.is-loading {
  cursor: default;
}

.kx-load-label {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-secondary);
}

.kx-legend-menu {
  width: 264px;
  padding: 8px;
}

.kx-legend-title {
  padding: 2px 4px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.kx-legend-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 4px;
  border-radius: 6px;
}

.kx-legend-row .kx-flag {
  margin-top: 1px;
}

.kx-legend-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.kx-legend-label {
  font-size: 12px;
  line-height: 1.35;
  color: var(--text-color);
}

.kx-legend-desc {
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-secondary);
  white-space: normal;
}

/* Narrow sidebar: keep only the inject level; secondary chips would
   otherwise squeeze the file name into an ellipsis. */
@container (max-width: 259px) {
  .kx-row-side .kx-flag.flag-command,
  .kx-row-side .kx-flag.flag-auto,
  .kx-row-side .kx-flag.flag-search-on,
  .kx-row-side .kx-flag.flag-external {
    display: none;
  }
}
</style>
