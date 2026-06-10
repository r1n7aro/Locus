<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import {
  Check,
  ChevronRight,
  Folder,
  FolderOpen,
  Package,
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
  buildKnowledgeListTags,
  buildKnowledgeSearchMatchTags,
  type KnowledgeListTag,
} from "./knowledgeMetaLabels";
import { buildFolderDisplayStats } from "./knowledgeExplorerFolderCounts";
import {
  resolveKnowledgeContextSelection,
  resolveKnowledgeExplorerSelection,
} from "./knowledgeExplorerSelection";
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
  (e: "moveNode", node: ExplorerNode, targetDir: string): void;
  (e: "loadMoreRoot"): void;
  (e: "loadMoreFolder", path: string): void;
  (e: "dragStateChange", dragging: boolean): void;
}>();

interface FlatRow {
  node: ExplorerNode;
  expanded: boolean;
  directChildCount: number;
  descendantDocumentCount: number;
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
const draggingNode = ref<ExplorerNode | null>(null);
const dragTargetPath = ref<string | null>(null);
const isSearchMode = computed(() => !!props.searchQuery.trim());
const selectedPaths = ref<Set<string>>(new Set());
const lastAnchorPath = ref<string | null>(null);
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
      const descendantDocumentCount = folderStats?.descendantDocumentCount ?? 0;
      out.push({
        type: "row",
        key: node.path,
        row: { node, expanded, directChildCount, descendantDocumentCount },
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
  if (row.node.kind === "folder") {
    emit("toggle", row.node.path);
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

function clearDragState() {
  if (draggingNode.value) emit("dragStateChange", false);
  draggingNode.value = null;
  dragTargetPath.value = null;
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
  if (isPluginManagedNode(node)) return false;
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
  draggingNode.value = row.node;
  dragTargetPath.value = null;
  emit("dragStateChange", true);
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", row.node.path);
  }
}

function onNodeDragEnd() {
  clearDragState();
}

function onFolderDragOver(row: FlatRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNode.value) return;
  const targetDir = row.node.relativePath;
  if (!canDropOnDir(draggingNode.value, targetDir)) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetPath.value = row.node.path;
}

function onFolderDrop(row: FlatRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNode.value) return;
  const targetDir = row.node.relativePath;
  if (!canDropOnDir(draggingNode.value, targetDir)) {
    clearDragState();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  const node = draggingNode.value;
  clearDragState();
  emit("moveNode", node, targetDir);
}

function onTreeDragOver(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNode.value) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".kx-row-shell, .kx-create-row, .kx-load-row, .kx-search-row",
    )
  ) {
    return;
  }
  if (!canDropOnDir(draggingNode.value, "")) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetPath.value = props.activeType;
}

function onTreeDrop(event: DragEvent) {
  if (isSearchMode.value) return;
  if (!draggingNode.value) return;
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(
      ".kx-row-shell, .kx-create-row, .kx-load-row, .kx-search-row",
    )
  ) {
    return;
  }
  if (!canDropOnDir(draggingNode.value, "")) {
    clearDragState();
    return;
  }
  event.preventDefault();
  const node = draggingNode.value;
  clearDragState();
  emit("moveNode", node, "");
}

function canDeleteFolder(
  menu: Extract<ContextMenuState, { kind: "folder" }>,
): boolean {
  return menu.depth > 0 && !menu.targetNodes.some(isPluginManagedNode);
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

function canDeleteContextTargets(
  menu: Extract<ContextMenuState, { kind: "folder" | "leaf" | "package" }>,
): boolean {
  if (menu.targetNodes.some(isPluginManagedNode)) return false;
  return menu.kind !== "folder" || menu.targetNodes.length > 1 || canDeleteFolder(menu);
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

async function openCreateInline(kind: InlineCreateState["kind"]) {
  if (!ctxMenu.value) return;
  if (ctxMenu.value.kind === "leaf" || ctxMenu.value.kind === "package") return;
  closeInlineRename();

  if (ctxMenu.value.kind === "folder" && !ctxMenu.value.expanded) {
    emit("toggle", ctxMenu.value.anchorPath);
  }

  inlineCreate.value = {
    kind,
    parentDir: ctxMenu.value.kind === "folder" ? ctxMenu.value.parentDir : "",
    anchorPath: ctxMenu.value.anchorPath,
    depth: ctxMenu.value.kind === "folder" ? ctxMenu.value.depth + 1 : 1,
    name: "",
  };
  closeContextMenu();
  await nextTick();
  inlineInputRef.value?.focus();
  inlineInputRef.value?.select();
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
  closeInlineCreate();
  inlineRename.value =
    menu.kind === "folder"
      ? {
          kind: "folder",
          anchorPath: menu.node.path,
          relativePath: menu.node.relativePath,
          currentName: menu.node.name,
          name: menu.node.name,
        }
      : {
          kind: "document",
          anchorPath: menu.node.path,
          relativePath: menu.node.document.path,
          currentName: menu.node.name,
          name: menu.node.name,
        };
  closeContextMenu();
  await nextTick();
  inlineRenameInputRef.value?.focus();
  inlineRenameInputRef.value?.select();
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
  if (inlineCreate.value && !inlineCreateRowRef.value?.contains(target)) {
    closeInlineCreate();
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
});

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
  },
);

watch(isSearchMode, (value) => {
  if (!value) return;
  clearMultiSelection(true);
  closeContextMenu();
  closeInlineCreate();
  closeInlineRename();
});

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
  tone: "command";
  title: string;
}> {
  const tags: Array<{
    text: string;
    tone: "command";
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

function handleVisibleRangeChange(payload: { start: number; end: number }) {
  if (payload.end < payload.start) return;
  for (const entry of visibleRows.value.slice(payload.start, payload.end + 1)) {
    if (entry.type !== "loadMore") continue;
    if (entry.loading) continue;
    if (entry.path) continue;
    if (!props.hasMoreRootDocuments) continue;
    emit("loadMoreRoot");
    return;
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
    <div
      class="kx-tree-shell"
      :class="{ 'is-root-drop-target': dragTargetPath === activeType }"
      @contextmenu.prevent="onTreeContextMenu($event)"
      @dragover="onTreeDragOver"
      @drop="onTreeDrop"
    >
      <div v-if="isSearchMode && searching" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <div v-else-if="isSearchMode" class="kx-tree-static">
        <button
          v-for="result in searchResults"
          :key="`${result.id}-${result.path}`"
          type="button"
          class="kx-search-row"
          :class="{ selected: isSelectedSearchResult(result) }"
          @click="emit('selectSearchResult', result)"
        >
          <div class="kx-search-main">
            <span class="kx-search-title">{{ result.title }}</span>
            <span class="kx-search-path">{{ result.path }}</span>
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
          </div>
        </button>
        <div v-if="!searchResults.length" class="kx-empty">
          {{ t("knowledge.search.noResults") }}
        </div>
      </div>
      <div v-else-if="loading && !tree.length" class="kx-tree-static">
        <div class="kx-empty">{{ t("common.loading") }}</div>
      </div>
      <FileTreeList
        v-else-if="visibleRows.length"
        class="kx-tree"
        :items="visibleRows"
        :row-height="30"
        @visible-range-change="handleVisibleRangeChange"
      >
        <template #item="{ item }">
          <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
            <div
              v-if="entry.type === 'row'"
              class="kx-row-shell"
              :class="{
                'kx-folder': entry.row.node.kind === 'folder',
                'kx-package': entry.row.node.kind === 'package',
                'kx-leaf': entry.row.node.kind === 'document',
                selected:
                  selectedPath === entry.row.node.path ||
                  selectedPaths.has(entry.row.node.path),
                'context-selected': contextSelectedPath === entry.row.node.path,
                dragging: draggingNode?.path === entry.row.node.path,
                'drop-target': dragTargetPath === entry.row.node.path,
              }"
              :draggable="!isRenamingRow(entry.row) && canDragNode(entry.row.node)"
              @contextmenu.prevent="openContextMenu($event, entry.row)"
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
                  v-else-if="entry.row.node.kind !== 'document'"
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
                <span class="kx-count">{{
                  entry.row.descendantDocumentCount
                }}</span>
              </div>
              <div
                v-else-if="entry.row.node.kind === 'package'"
                class="kx-row-side"
              >
                <span
                  v-for="tag in packageTags(entry.row.node)"
                  :key="`${entry.row.node.path}-${tag.text}`"
                  class="kx-flag"
                  :class="{
                    'flag-command': tag.tone === 'command',
                  }"
                  :title="tag.title"
                >
                  {{ tag.text }}
                </span>
                <span class="kx-count">{{
                  entry.row.descendantDocumentCount
                }}</span>
              </div>
              <div
                v-else-if="documentTags(entry.row.node).length"
                class="kx-row-side"
              >
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
              @click="openCreateInline('document')"
            >
              {{ t("knowledge.explorer.createDoc") }}
            </button>
            <button
              v-if="ctxMenu.kind === 'folder' && canDeleteContextTargets(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
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
              v-if="canDeleteContextTargets(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
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
              v-if="canDeleteContextTargets(ctxMenu)"
              type="button"
              class="kx-ctx-item kx-ctx-item-danger"
              @click="requestDeleteSelectedNodes"
            >
              {{ deleteMenuLabel(ctxMenu) }}
            </button>
          </template>
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
}

.kx-tree-shell {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
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

.kx-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  gap: 6px;
  width: 100%;
  min-width: 0;
  background: transparent;
  transition: background 0.1s;
}

.kx-row-shell:hover {
  background: var(--hover-bg);
}

.kx-row-shell.selected,
.kx-row-shell.selected:hover {
  background: var(--active-bg);
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
  min-height: 26px;
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

.kx-count {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.7;
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
  min-height: 30px;
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
  min-height: 30px;
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

</style>
