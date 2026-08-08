<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import {
  BookOpen,
  Check,
  Copy,
  Download,
  FilePlus,
  Folder,
  FolderCog,
  FolderInput,
  FolderOpen,
  FolderPlus,
  LocateFixed,
  Lock,
  Package,
  PackagePlus,
  PencilLine,
  Trash2,
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
import WorkspaceTree, {
  type WorkspaceTreeItem,
  type WorkspaceTreeRow,
} from "../explorer/WorkspaceTree.vue";
import {
  buildFolderListTags,
  buildExternalFolderTag,
  buildKnowledgeListTags,
  type KnowledgeListTag,
} from "./knowledgeMetaLabels";
import { buildFolderDisplayStats } from "./knowledgeExplorerFolderCounts";
import {
  effectiveSkillInjectMode,
  skillActivationInactive,
} from "../../composables/skillCommands";
import { skillSurfaceAllowsCommand } from "../../types";
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
  rootDirectoryConfigs: Record<
    KnowledgeDocumentType,
    Record<string, KnowledgeDirectoryConfigRecord>
  >;
  externalDirectorySources: Record<string, KnowledgeExternalSource[]>;
  folderStats: Record<string, KnowledgeFolderDisplayStats>;
  selectedPath: string | null;
  isPathExpanded: (path: string) => boolean;
  hasMoreRootDocuments: (type: KnowledgeDocumentType) => boolean;
  rootDocumentsLoading: (type: KnowledgeDocumentType) => boolean;
  hasMoreFolderDocuments: (type: KnowledgeDocumentType, path: string) => boolean;
  folderDocumentsLoaded: (type: KnowledgeDocumentType, path: string) => boolean;
  folderDocumentsLoading: (type: KnowledgeDocumentType, path: string) => boolean;
  loading: boolean;
  searchQuery: string;
  searchResults: KnowledgeSearchResult[];
  searching: boolean;
}>();

const emit = defineEmits<{
  (e: "selectDocument", document: DocumentNode["document"]): void;
  (e: "selectPackage", document: PackageNode["document"]): void;
  (e: "selectSearchResult", result: KnowledgeSearchResult): void;
  (e: "selectTypeRoot", type: KnowledgeDocumentType): void;
  (e: "selectFolderConfig", type: KnowledgeDocumentType, path: string): void;
  (e: "toggle", path: string): void;
  (e: "importSkillPackage"): void;
  (e: "exportPackage", node: PackageNode): void;
  (e: "requestExternalImportFolder", parentDir: string): void;
  (e: "createFolder", parentDir: string, name: string, type: KnowledgeDocumentType): void;
  (e: "createDocument", parentDir: string, name: string, type: KnowledgeDocumentType): void;
  (e: "renameFolder", path: string, name: string, type: KnowledgeDocumentType): void;
  (
    e: "renameDocument",
    path: string,
    name: string,
    type: KnowledgeDocumentType,
  ): void;
  (e: "copyRelativePath", node: ExplorerNode): void;
  (e: "openInFileSystem", node: ExplorerNode): void;
  (e: "requestDeleteNodes", nodes: ExplorerNode[]): void;
  (
    e: "moveNodes",
    nodes: ExplorerNode[],
    targetDir: string,
    type: KnowledgeDocumentType,
  ): void;
  (e: "revealSearchResult", result: KnowledgeSearchResult): void;
  (e: "copySearchResultPath", result: KnowledgeSearchResult): void;
  (e: "loadMoreRoot", type: KnowledgeDocumentType): void;
  (e: "loadMoreFolder", type: KnowledgeDocumentType, path: string): void;
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
      type: KnowledgeDocumentType;
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
  type: KnowledgeDocumentType;
  parentDir: string;
  anchorPath: string;
  depth: number;
  name: string;
}

interface InlineRenameState {
  kind: "folder" | "document";
  type: KnowledgeDocumentType;
  anchorPath: string;
  relativePath: string;
  currentName: string;
  name: string;
}

type VisibleEntry =
  | { type: "row"; key: string; row: FlatRow; treeRow: WorkspaceTreeRow }
  | { type: "create"; key: string; draft: InlineCreateState; treeRow: null }
  | {
      type: "loadMore";
      key: string;
      nodeType: KnowledgeDocumentType;
      path: string | null;
      depth: number;
      loading: boolean;
      treeRow: null;
    };

const ctxMenu = ref<ContextMenuState | null>(null);
const inlineCreate = ref<InlineCreateState | null>(null);
const inlineRename = ref<InlineRenameState | null>(null);
const inlineInputRef = ref<HTMLInputElement | null>(null);
const inlineCreateRowRef = ref<HTMLElement | null>(null);
const inlineRenameInputRef = ref<HTMLInputElement | null>(null);
const inlineRenameRowRef = ref<HTMLElement | null>(null);
const treeListRef = ref<InstanceType<typeof WorkspaceTree> | null>(null);
const draggingNodes = ref<ExplorerNode[]>([]);
const dragTargetPath = ref<string | null>(null);
const isSearchMode = computed(() => !!props.searchQuery.trim());
const selectedPaths = ref<Set<string>>(new Set());
const lastAnchorPath = ref<string | null>(null);
const focusedPath = ref<string | null>(null);
const pendingRevealPath = ref<string | null>(null);
const searchCollapsedPaths = ref<Set<string>>(new Set());
const searchCtxMenu = ref<{
  x: number;
  y: number;
  result: KnowledgeSearchResult;
} | null>(null);
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
  buildFolderDisplayStats(
    props.tree,
    isSearchMode.value ? undefined : props.folderStats,
  ),
);
const searchResultsByPath = computed(() => {
  const byPath = new Map<string, KnowledgeSearchResult>();
  for (const result of props.searchResults) {
    const path = result.path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
    const key = `${result.type}/${path}`;
    if (!byPath.has(key)) byPath.set(key, result);
  }
  return byPath;
});

function isBranchNode(node: ExplorerNode): node is BranchNode {
  return node.kind === "folder" || node.kind === "package";
}

const visibleRows = computed<VisibleEntry[]>(() => {
  const out: VisibleEntry[] = [];

  const walk = (nodes: ExplorerNode[]) => {
    for (const node of nodes) {
      const branch = isBranchNode(node);
      const expanded = branch
        ? isSearchMode.value
          ? !searchCollapsedPaths.value.has(node.path)
          : props.isPathExpanded(node.path)
        : false;
      const folderStats =
        branch ? folderDisplayStats.value.get(node.path) : null;
      const folderLoaded =
        node.kind === "folder"
          ? props.folderDocumentsLoaded(node.type, node.relativePath)
          : false;
      const directChildCount =
        folderStats?.directChildCount ??
        (branch ? node.children.length : 0);
      const row: FlatRow = { node, expanded, directChildCount };
      out.push({
        type: "row",
        key: node.path,
        row,
        treeRow: {
          key: node.path,
          name: node.name,
          depth: node.depth,
          kind: node.kind === "document" ? "file" : node.kind,
          expandable: branch && directChildCount > 0,
          expanded,
          selected:
            props.selectedPath === node.path
            || selectedPaths.value.has(node.path)
            || (
              node.kind === "folder"
              && !!node.specialRoot
              && props.activeType === node.type
              && !props.selectedPath
            ),
          focused: focusedPath.value === node.path,
          editing: isRenamingRow(row),
          draggable: !isSearchMode.value && canDragNode(node),
          domId: rowDomId(node.path),
          title: skillNodeInactive(node)
            ? t("knowledge.explorer.skillInactiveHint")
            : node.path,
          classes: {
            "kx-folder": node.kind === "folder",
            "kx-package": node.kind === "package",
            "kx-leaf": node.kind === "document",
            "kx-skill-inactive": skillNodeInactive(node),
            "is-open": props.selectedPath === node.path,
            "is-marked": selectedPaths.value.has(node.path),
            "context-selected": contextSelectedPath.value === node.path,
            dragging: draggingPaths.value.has(node.path),
            "drop-target": dragTargetPath.value === node.path,
            "is-special-root": node.kind === "folder" && !!node.specialRoot,
          },
        },
      });
      if (inlineCreate.value?.anchorPath === node.path) {
        out.push({
          type: "create",
          key: `create:${node.path}:${inlineCreate.value.kind}`,
          draft: inlineCreate.value,
          treeRow: null,
        });
      }
      if (branch && expanded) {
        walk(node.children);
        if (
          !isSearchMode.value &&
          node.kind === "folder" &&
          folderLoaded &&
          (props.hasMoreFolderDocuments(node.type, node.relativePath) ||
            props.folderDocumentsLoading(node.type, node.relativePath))
        ) {
          out.push({
            type: "loadMore",
            key: `${node.path}::load-more`,
            nodeType: node.type,
            path: node.relativePath,
            depth: node.depth + 1,
            loading: props.folderDocumentsLoading(node.type, node.relativePath),
            treeRow: null,
          });
        }
        if (
          !isSearchMode.value &&
          node.kind === "folder" &&
          node.specialRoot &&
          (props.hasMoreRootDocuments(node.type) || props.rootDocumentsLoading(node.type))
        ) {
          out.push({
            type: "loadMore",
            key: `${node.type}::root-load-more`,
            nodeType: node.type,
            path: null,
            depth: 1,
            loading: props.rootDocumentsLoading(node.type),
            treeRow: null,
          });
        }
      }
    }
  };

  walk(props.tree);
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
  closeContextMenu();
  closeInlineCreate();
  closeInlineRename();
  focusedPath.value = row.node.path;
  if (isSearchMode.value) {
    if (event.detail >= 2) return;
    activateNode(row);
    return;
  }
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
  // Keep rapid repeated clicks from toggling the same branch twice.
  if (event.detail >= 2) return;
  activateNode(row);
}

// Folder rows are navigation controls: clicking the row or its chevron toggles
// the same branch. Folder configuration remains an explicit context-menu
// action, so ordinary browsing never replaces the current document preview.
function activateNode(row: FlatRow) {
  if (row.node.kind === "folder") {
    toggleExpansion(row);
    return;
  }
  const searchResult = searchResultForNode(row.node);
  if (isSearchMode.value && searchResult) {
    emit("selectSearchResult", searchResult);
    return;
  }
  if (row.node.kind === "package") {
    emit("selectPackage", row.node.document);
    return;
  }
  emit("selectDocument", row.node.document);
}

function toggleExpansion(row: FlatRow) {
  if (!isBranchNode(row.node)) return;
  if (isSearchMode.value) {
    const next = new Set(searchCollapsedPaths.value);
    if (next.has(row.node.path)) next.delete(row.node.path);
    else next.add(row.node.path);
    searchCollapsedPaths.value = next;
    return;
  }
  emit("toggle", row.node.path);
}

function searchResultForNode(
  node: DocumentNode | PackageNode,
): KnowledgeSearchResult | null {
  const path = node.document.path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  return searchResultsByPath.value.get(`${node.type}/${path}`) ?? null;
}

function treeIndentPx(depth: number): number {
  return 10 + Math.max(0, depth) * 14;
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
  if (isSearchMode.value) {
    event.preventDefault();
    event.stopPropagation();
    if (row.node.kind === "document" || row.node.kind === "package") {
      const result = searchResultForNode(row.node);
      if (result) openSearchContextMenu(event, result);
    }
    return;
  }
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
    type: "design",
    anchorPath: "design",
  };
}

function onTreeContextMenu(event: MouseEvent) {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".workspace-tree-row-shell, .kx-create-row, .kx-load-row",
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
  if (node.kind === "folder" && node.specialRoot) return false;
  if (isManagedNode(node)) return false;
  if (node.kind === "document") return true;
  if (node.kind === "package") return false;
  return !!node.relativePath.trim();
}

function canDropOnDir(
  node: ExplorerNode,
  targetDir: string,
  targetType: KnowledgeDocumentType,
): boolean {
  if (node.type !== targetType) return false;
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

function dropTargetAccepts(targetDir: string, targetType: KnowledgeDocumentType): boolean {
  return draggingNodes.value.some((node) => canDropOnDir(node, targetDir, targetType));
}

function droppableNodes(targetDir: string, targetType: KnowledgeDocumentType): ExplorerNode[] {
  return draggingNodes.value.filter((node) => canDropOnDir(node, targetDir, targetType));
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
  if (!dropTargetAccepts(targetDir, row.node.type)) {
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
  const movable = droppableNodes(targetDir, row.node.type);
  if (!movable.length) {
    clearDragState();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  clearDragState();
  emit("moveNodes", movable, targetDir, row.node.type);
}

function onTreeDragOver(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNodes.value.length) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".workspace-tree-row-shell, .kx-create-row, .kx-load-row",
    )
  ) {
    return;
  }
  if (!dropTargetAccepts("", "design")) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetPath.value = "design";
}

function onTreeDrop(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNodes.value.length) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".workspace-tree-row-shell, .kx-create-row, .kx-load-row",
    )
  ) {
    return;
  }
  const movable = droppableNodes("", "design");
  if (!movable.length) {
    clearDragState();
    return;
  }
  event.preventDefault();
  clearDragState();
  emit("moveNodes", movable, "", "design");
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

// External skills are discovered from agent directories (~/.claude/skills,
// ~/.agents/skills, ...) and are strictly read-only inside Locus: no rename,
// move, delete, export, or content edits.
function isExternalSkillNode(node: ExplorerNode): boolean {
  if (node.kind === "package" || node.kind === "document") {
    return !!node.document.externalSource?.locator?.startsWith("external://");
  }
  return node.children.some(isExternalSkillNode);
}

// Dim skills that will not take effect: master switch off, or both the
// command and auto channels off. Package-internal documents (references/)
// carry skillEnabled=false by design, so only the package root and plain
// skill documents participate.
function skillNodeInactive(node: ExplorerNode): boolean {
  if (node.kind === "package") {
    return skillActivationInactive({
      ...node.document,
      injectMode: node.document.effectiveInjectMode,
    });
  }
  if (node.kind !== "document") return false;
  const document = node.document;
  if (document.type !== "skill") return false;
  const isPackageDocument = document.externalSource?.provider === "package";
  if (isPackageDocument && !isSkillPackageRootDocument(document)) return false;
  return skillActivationInactive({
    ...document,
    injectMode: document.effectiveInjectMode,
  });
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
  return (
    isPluginManagedNode(node) ||
    isExternalSkillNode(node) ||
    isPackageContentNode(node)
  );
}

function managedHint(nodes: ExplorerNode[]): string | undefined {
  if (nodes.some(isPluginManagedNode)) {
    return t("knowledge.explorer.pluginManagedHint");
  }
  if (nodes.some(isExternalSkillNode)) {
    return t("knowledge.explorer.externalManagedHint");
  }
  if (nodes.some(isPackageContentNode)) {
    return t("knowledge.explorer.packageManagedHint");
  }
  return undefined;
}

function rowLockTitle(node: ExplorerNode): string | null {
  if (isPluginManagedNode(node)) return t("knowledge.explorer.pluginManaged");
  if (isExternalSkillNode(node)) return t("knowledge.explorer.externalManaged");
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
  return (
    (menu.kind === "folder" && !!menu.node.specialRoot)
    || menu.targetNodes.some(isManagedNode)
  );
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
  if (menu.node.specialRoot) {
    emit("selectTypeRoot", menu.node.type);
  } else {
    emit("selectFolderConfig", menu.node.type, menu.node.relativePath);
  }
}

function createActionLabel(kind: InlineCreateState["kind"]): string {
  return kind === "folder"
    ? t("knowledge.explorer.createFolder")
    : t("knowledge.explorer.createDoc");
}

function openExternalImportFolderDialog() {
  if (!ctxMenu.value) return;
  const menuType = ctxMenu.value.kind === "root"
    ? ctxMenu.value.type
    : ctxMenu.value.node.type;
  if (menuType !== "reference") return;
  if (ctxMenu.value.kind === "leaf" || ctxMenu.value.kind === "package") return;
  const parentDir =
    ctxMenu.value.kind === "folder" ? ctxMenu.value.parentDir : "";
  closeContextMenu();
  emit("requestExternalImportFolder", parentDir);
}

function importSkillPackageArchive() {
  if (!ctxMenu.value) return;
  const menuType = ctxMenu.value.kind === "root"
    ? ctxMenu.value.type
    : ctxMenu.value.node.type;
  if (menuType !== "skill") return;
  closeContextMenu();
  emit("importSkillPackage");
}

function contextMenuType(menu: ContextMenuState): KnowledgeDocumentType {
  return menu.kind === "root" ? menu.type : menu.node.type;
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
    type: KnowledgeDocumentType;
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
    type: target.type,
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
    type: menu.kind === "folder" ? menu.node.type : menu.type,
    parentDir: menu.kind === "folder" ? menu.parentDir : "",
    anchorPath: menu.anchorPath,
    depth: menu.kind === "folder" ? menu.depth + 1 : 1,
    expandPath:
      menu.kind === "folder" && !menu.expanded ? menu.anchorPath : null,
  });
}

async function openEmptyStateCreate(kind: InlineCreateState["kind"]) {
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
      type: selected.node.type,
      parentDir: selected.node.relativePath,
      anchorPath: selected.node.path,
      depth: selected.node.depth + 1,
      expandPath: selected.expanded ? null : selected.node.path,
    });
    return;
  }
  await openInlineCreateAt(kind, {
    type: "design",
    parentDir: "",
    anchorPath: "design",
    depth: 0,
  });
}

const showEmptyStateImport = computed(
  () => props.activeType === "skill" || props.activeType === "reference",
);

const emptyStateImportLabel = computed(() =>
  props.activeType === "skill"
    ? t("knowledge.explorer.importSkillPackage")
    : t("knowledge.explorer.importExternalFolder"),
);

function emptyStateImport() {
  if (props.activeType === "skill") {
    emit("importSkillPackage");
    return;
  }
  if (props.activeType === "reference") {
    emit("requestExternalImportFolder", "");
  }
}

async function startRenameNode(node: FolderNode | DocumentNode) {
  closeInlineCreate();
  inlineRename.value =
    node.kind === "folder"
      ? {
          kind: "folder",
          type: node.type,
          anchorPath: node.path,
          relativePath: node.relativePath,
          currentName: node.name,
          name: node.name,
        }
      : {
          kind: "document",
          type: node.type,
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
  if (draft.kind === "folder") emit("createFolder", draft.parentDir, name, draft.type);
  else emit("createDocument", draft.parentDir, name, draft.type);
  closeInlineCreate();
}

function submitInlineRename() {
  const draft = inlineRename.value;
  if (!draft) return;
  const name = draft.name.trim();
  closeInlineRename();
  if (!name || name === draft.currentName) return;
  if (draft.kind === "folder") {
    emit("renameFolder", draft.relativePath, name, draft.type);
    return;
  }
  emit("renameDocument", draft.relativePath, name, draft.type);
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
    searchCtxMenu.value = null;
  },
);

watch(isSearchMode, (value) => {
  if (value) {
    searchCollapsedPaths.value = new Set();
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
  () => props.searchQuery,
  () => {
    searchCollapsedPaths.value = new Set();
  },
);

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
    case "collapse": {
      focusedPath.value = action.path;
      const row = selectableRowMap.value.get(action.path);
      if (row) toggleExpansion(row);
      return;
    }
    case "activate": {
      const row = selectableRowMap.value.get(action.path);
      if (!row) return;
      focusedPath.value = action.path;
      clearMultiSelection();
      activateNode(row);
      return;
    }
    case "delete": {
      if (isSearchMode.value) return;
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
      if (isSearchMode.value) return;
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
  const isSkill = node.document.type === "skill";
  const trigger = node.document.commandTrigger?.trim();
  const commandChannelOn =
    !isSkill || skillSurfaceAllowsCommand(node.document.skillSurface ?? undefined);
  if (trigger && commandChannelOn && !isSkillPackageRootDocument(node.document)) {
    tags.push({
      text: trigger,
      tone: "command",
      title: t("knowledge.skill.commandTrigger"),
    });
  }
  tags.push(
    ...buildKnowledgeListTags({
      // Skills show the effective auto-channel mode: a surface without the
      // auto side reads as no injection regardless of the stored value.
      injectMode: isSkill
        ? effectiveSkillInjectMode(node.document.skillSurface, node.document.effectiveInjectMode)
        : node.document.effectiveInjectMode,
      aiMaintained: node.document.effectiveAiMaintained,
    }),
  );
  return tags;
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

function folderTags(node: FolderNode) {
  const tags = [];
  const externalTag = buildExternalFolderTag(
    node.type === "reference"
      ? props.externalDirectorySources[node.relativePath]
      : undefined,
  );
  if (externalTag) tags.push(externalTag);
  if (isBuiltinSkillGroupFolder(node)) return tags;
  if (node.specialRoot) {
    const config = props.rootDirectoryConfigs[node.type][""];
    if (!config) return tags;
    tags.push(
      ...buildFolderListTags({
        injectMode: config.effectiveInjectMode,
        lexicalEnabled: config.effectiveLexicalSearch.enabled,
        semanticEnabled: config.effectiveVectorSearch.enabled,
      }),
    );
    return tags;
  }
  const rootDirectoryDepth = 1;
  if (node.depth !== rootDirectoryDepth) return tags;
  const config = props.rootDirectoryConfigs[node.type][node.relativePath];
  if (!config) return tags;
  tags.push(
    ...buildFolderListTags({
      injectMode: config.effectiveInjectMode,
      lexicalEnabled: config.effectiveLexicalSearch.enabled,
      semanticEnabled: config.effectiveVectorSearch.enabled,
    }),
  );
  return tags;
}

function isBuiltinSkillGroupFolder(node: FolderNode): boolean {
  return node.type === "skill" && node.depth === 1 && node.relativePath === "builtin";
}

function documentIconNode(node: ExplorerNode) {
  const path = node.kind === "document" ? node.document.path : node.path;
  return unityAssetIconNodeForPath(path || node.path, {
    isFolder: false,
  });
}

function documentIconClass(node: ExplorerNode) {
  const path = node.kind === "document" ? node.document.path : node.path;
  return unityAssetIconClassForPath(path || node.path, {
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
  if (trigger && skillSurfaceAllowsCommand(node.document.skillSurface ?? undefined)) {
    tags.push({
      text: trigger,
      tone: "command",
      title: t("knowledge.skill.commandTrigger"),
    });
  }
  const effectiveInject = effectiveSkillInjectMode(
    node.document.skillSurface,
    node.document.effectiveInjectMode,
  );
  if (effectiveInject === "excerpt") {
    tags.push(
      ...buildKnowledgeListTags({
        injectMode: effectiveInject,
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
      if (!props.hasMoreFolderDocuments(entry.nodeType, entry.path)) continue;
      emit("loadMoreFolder", entry.nodeType, entry.path);
      continue;
    }
    if (!props.hasMoreRootDocuments(entry.nodeType)) continue;
    emit("loadMoreRoot", entry.nodeType);
  }
}

function requestLoadMore(entry: VisibleEntry) {
  if (entry.type !== "loadMore" || entry.loading) return;
  if (entry.path) {
    if (!props.hasMoreFolderDocuments(entry.nodeType, entry.path)) return;
    emit("loadMoreFolder", entry.nodeType, entry.path);
    return;
  }
  if (!props.hasMoreRootDocuments(entry.nodeType)) return;
  emit("loadMoreRoot", entry.nodeType);
}

function asVisibleEntry(item: { key: string }): VisibleEntry {
  return item as VisibleEntry;
}

function workspaceRow(item: WorkspaceTreeItem): Extract<VisibleEntry, { type: "row" }> | null {
  const entry = asVisibleEntry(item);
  return entry.type === "row" ? entry : null;
}

function activateWorkspaceItem(item: WorkspaceTreeItem, event: MouseEvent) {
  const entry = workspaceRow(item);
  if (entry) rowClick(entry.row, event);
}

function toggleWorkspaceItem(item: WorkspaceTreeItem) {
  const entry = workspaceRow(item);
  if (entry) toggleExpansion(entry.row);
}

function contextWorkspaceItem(item: WorkspaceTreeItem, event: MouseEvent) {
  const entry = workspaceRow(item);
  if (entry) openContextMenu(event, entry.row);
}

function dragStartWorkspaceItem(item: WorkspaceTreeItem, event: DragEvent) {
  const entry = workspaceRow(item);
  if (entry) onNodeDragStart(entry.row, event);
}

function dragOverWorkspaceItem(item: WorkspaceTreeItem, event: DragEvent) {
  const entry = workspaceRow(item);
  if (entry) onFolderDragOver(entry.row, event);
}

function dropWorkspaceItem(item: WorkspaceTreeItem, event: DragEvent) {
  const entry = workspaceRow(item);
  if (entry) onFolderDrop(entry.row, event);
}
</script>

<template>
  <div class="kx-explorer">
    <div
      class="kx-tree-shell"
      :class="{ 'is-root-drop-target': dragTargetPath === 'design' }"
      role="tree"
      :aria-label="t('knowledge.explorer.title')"
      tabindex="0"
      :aria-activedescendant="focusedRowDomId"
      @keydown="onTreeKeydown"
      @contextmenu.prevent="onTreeContextMenu($event)"
      @dragover="onTreeDragOver"
      @drop="onTreeDrop"
    >
      <div v-if="isSearchMode && searching" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <div v-else-if="loading && !tree.length" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <WorkspaceTree
        v-else-if="visibleRows.length"
        ref="treeListRef"
        class="kx-tree"
        :items="visibleRows"
        :row-height="30"
        :base-indent="10"
        :indent-size="14"
        @activate="activateWorkspaceItem"
        @toggle="toggleWorkspaceItem"
        @contextmenu="contextWorkspaceItem"
        @dragstart="dragStartWorkspaceItem"
        @dragend="onNodeDragEnd"
        @dragover="dragOverWorkspaceItem"
        @drop="dropWorkspaceItem"
        @visible-range-change="handleVisibleRangeChange"
      >
        <template #icon="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <LucideIcon
              v-if="entry.type === 'row' && entry.row.node.kind === 'folder'"
              :icon="entry.row.expanded ? FolderOpen : Folder"
              :size="13"
              :stroke-width="2"
            />
            <LucideIcon
              v-else-if="entry.type === 'row' && entry.row.node.kind === 'package'"
              :icon="Package"
              :size="13"
              :stroke-width="2"
            />
            <LucideIcon
              v-else-if="entry.type === 'row'"
              :class="documentIconClass(entry.row.node)"
              :icon="documentIconNode(entry.row.node)"
              :size="13"
              :stroke-width="2"
            />
          </template>
        </template>

        <template #name="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <span v-if="entry.type === 'row'" class="kx-name">
              {{ entry.row.node.name }}
            </span>
          </template>
        </template>

        <template #editor="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <span
              v-if="entry.type === 'row'"
              class="kx-name-edit"
              :ref="setInlineRenameRowRef"
            >
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
        </template>

        <template #trailing="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <div v-if="entry.type === 'row' && entry.row.node.kind === 'folder'" class="kx-row-side">
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
                v-else-if="entry.type === 'row' && entry.row.node.kind === 'package'"
                class="kx-row-side"
              >
                <span
                  v-if="isPluginManagedNode(entry.row.node) || isExternalSkillNode(entry.row.node)"
                  class="kx-lock"
                  :title="rowLockTitle(entry.row.node) ?? undefined"
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
                  entry.type === 'row' &&
                  entry.row.node.kind === 'document' &&
                  (documentTags(entry.row.node).length ||
                    isPluginManagedNode(entry.row.node))
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
          </template>
        </template>

        <template #custom="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <div
              v-if="entry.type === 'create'"
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
              v-else-if="entry.type === 'loadMore'"
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
      </WorkspaceTree>
      <div v-else-if="isSearchMode" class="kx-tree-static">
        <div class="kx-empty">{{ t("knowledge.search.noResults") }}</div>
      </div>
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
            @click="openEmptyStateCreate('document')"
          >
            {{ t("knowledge.explorer.createDoc") }}
          </BaseButton>
          <BaseButton
            v-if="showEmptyStateImport"
            class="kx-empty-action"
            type="button"
            @click="emptyStateImport"
          >
            {{ emptyStateImportLabel }}
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
                ctxMenu.kind === 'folder' &&
                ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="openSelectedFolderConfig"
            >
              <LucideIcon :icon="FolderCog" :size="13" />
              {{ t("knowledge.explorer.folderConfig") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' &&
                !ctxMenu.node.specialRoot &&
                ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              :disabled="renameBlocked(ctxMenu)"
              :title="managedHint(ctxMenu.targetNodes)"
              @click="startRenameSelection"
            >
              <LucideIcon :icon="PencilLine" :size="13" />
              {{ t("knowledge.explorer.rename") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' &&
                !ctxMenu.node.specialRoot &&
                ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              <LucideIcon :icon="Copy" :size="13" />
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="
                ctxMenu.kind === 'folder' &&
                !ctxMenu.node.specialRoot &&
                ctxMenu.targetNodes.length === 1
              "
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              <LucideIcon :icon="FolderOpen" :size="13" />
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
              <LucideIcon :icon="FolderPlus" :size="13" />
              {{ t("knowledge.explorer.createFolder") }}
            </button>
            <button
              v-if="
                contextMenuType(ctxMenu) === 'reference' &&
                (ctxMenu.kind === 'root' ||
                  (ctxMenu.kind === 'folder' &&
                    ctxMenu.targetNodes.length === 1))
              "
              type="button"
              class="kx-ctx-item"
              @click="openExternalImportFolderDialog"
            >
              <LucideIcon :icon="FolderInput" :size="13" />
              {{ t("knowledge.explorer.importExternalFolder") }}
            </button>
            <button
              v-if="contextMenuType(ctxMenu) === 'skill'"
              type="button"
              class="kx-ctx-item"
              @click="importSkillPackageArchive"
            >
              <LucideIcon :icon="PackagePlus" :size="13" />
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
              <LucideIcon :icon="FilePlus" :size="13" />
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
              <LucideIcon :icon="Trash2" :size="13" />
              {{ deleteMenuLabel(ctxMenu) }}
            </button>
          </template>
          <template v-else-if="ctxMenu.kind === 'package'">
            <button
              v-if="ctxMenu.targetNodes.length === 1 && !isExternalSkillNode(ctxMenu.node)"
              type="button"
              class="kx-ctx-item"
              @click="exportSelectedPackage"
            >
              <LucideIcon :icon="Download" :size="13" />
              {{ t("knowledge.explorer.exportSkillPackage") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              <LucideIcon :icon="Copy" :size="13" />
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              <LucideIcon :icon="FolderOpen" :size="13" />
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
              <LucideIcon :icon="Trash2" :size="13" />
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
              <LucideIcon :icon="PencilLine" :size="13" />
              {{ t("knowledge.explorer.rename") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="copySelectedRelativePath"
            >
              <LucideIcon :icon="Copy" :size="13" />
              {{ t("knowledge.explorer.copyRelativePath") }}
            </button>
            <button
              v-if="ctxMenu.targetNodes.length === 1"
              type="button"
              class="kx-ctx-item"
              @click="openSelectedInFileSystem"
            >
              <LucideIcon :icon="FolderOpen" :size="13" />
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
              <LucideIcon :icon="Trash2" :size="13" />
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
        <LucideIcon :icon="BookOpen" :size="13" />
        {{ t("knowledge.search.openResult") }}
      </button>
      <button
        type="button"
        class="kx-ctx-item"
        @click="revealSearchResultFromMenu"
      >
        <LucideIcon :icon="LocateFixed" :size="13" />
        {{ t("knowledge.search.revealInTree") }}
      </button>
      <button
        type="button"
        class="kx-ctx-item"
        @click="copySearchResultPathFromMenu"
      >
        <LucideIcon :icon="Copy" :size="13" />
        {{ t("knowledge.explorer.copyRelativePath") }}
      </button>
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

.kx-tree :deep(.workspace-tree-row-shell.is-open),
.kx-tree :deep(.workspace-tree-row-shell.is-open:hover) {
  background: var(--active-bg);
  box-shadow: inset 2px 0 0 var(--accent-color);
}

.kx-tree :deep(.workspace-tree-row-shell.is-marked:not(.is-open)) {
  background: color-mix(in srgb, var(--active-bg) 55%, transparent);
}

.kx-tree-shell:focus-within :deep(.workspace-tree-row-shell.focused) {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 55%, transparent);
  outline-offset: -1px;
}

.kx-tree :deep(.workspace-tree-row-shell.context-selected) {
  background: color-mix(in srgb, var(--active-bg) 52%, var(--hover-bg) 48%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 16%, var(--border-color));
}

.kx-tree :deep(.workspace-tree-row-shell.dragging) {
  opacity: 0.48;
}

.kx-tree :deep(.workspace-tree-row-shell.drop-target) {
  background: color-mix(in srgb, var(--active-bg) 62%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 32%, var(--border-color));
}

.kx-tree :deep(.workspace-tree-icon.kind-folder) {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.kx-tree :deep(.workspace-tree-icon.kind-package) {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.kx-tree :deep(.workspace-tree-row-shell.is-special-root .workspace-tree-name) {
  font-weight: 600;
}

.kx-tree :deep(.workspace-tree-row-shell.kx-skill-inactive .workspace-tree-name),
.kx-tree :deep(.workspace-tree-row-shell.kx-skill-inactive .workspace-tree-icon),
.kx-tree :deep(.workspace-tree-row-shell.kx-skill-inactive .kx-flag) {
  opacity: 0.48;
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

.kx-bullet-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
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
