<script setup lang="ts">
import {
  computed,
  nextTick,
  onMounted,
  onUnmounted,
  ref,
  watch,
  type ComponentPublicInstance,
} from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ChevronRight,
  Download,
  Folder,
  FolderOpen,
  PanelTopOpen,
  Upload,
  X,
} from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import { useNotificationStore } from "../stores/notification";
import {
  checkViewOpenRequirements,
  normalizeViewError,
  viewCreateFolder,
  viewDeleteEntry,
  viewExportPackage,
  viewImportPackage,
  viewMoveEntry,
  viewRequiresUnityConnection,
  viewRun,
  viewTree,
  type ViewFolderSummary,
  type ViewPackageSummary,
  type ViewTreeSnapshot,
} from "../services/view";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "./icons/locusViewIcons";
import BaseButton from "./ui/BaseButton.vue";
import BaseContextMenu from "./ui/BaseContextMenu.vue";

interface ViewTreeNode {
  kind: "folder" | "view";
  key: string;
  label: string;
  relPath: string;
  children: ViewTreeNode[];
  folder?: ViewFolderSummary;
  view?: ViewPackageSummary;
}

interface VisibleViewRow {
  node: ViewTreeNode;
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

type VisibleViewEntry =
  | { type: "row"; key: string; row: VisibleViewRow }
  | { type: "create"; key: string; draft: ViewCreateFolderDraft };

interface ViewCreateFolderDraft {
  parentRelPath: string;
  anchorKey: string;
  depth: number;
  name: string;
}

type ViewContextMenuState =
  | { x: number; y: number; kind: "root" }
  | { x: number; y: number; kind: "folder" | "view"; node: ViewTreeNode };

interface ViewPointerDragState {
  node: ViewTreeNode;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
}

interface ViewDropTarget {
  key: string;
  relPath: string;
}

const props = defineProps<{
  workingDir: string;
}>();

const STORAGE_KEY_VIEW_EXPANDED = "locus:viewPackageExpanded";
const ROOT_ANCHOR_KEY = "view-root";
const VIEW_TREE_INDENT_BASE_PX = 12;
const VIEW_TREE_INDENT_STEP_PX = 20;

const notificationStore = useNotificationStore();
const views = ref<ViewPackageSummary[]>([]);
const folders = ref<ViewFolderSummary[]>([]);
const selectedViewId = ref("");
const loading = ref(false);
const running = ref(false);
const importing = ref(false);
const exportingViewId = ref("");
const loadError = ref("");
const expandedState = ref<Record<string, boolean>>(loadExpandedState());
const contextMenu = ref<ViewContextMenuState | null>(null);
const deleteConfirm = ref<ViewTreeNode | null>(null);
const createFolderDraft = ref<ViewCreateFolderDraft | null>(null);
const createFolderInputRef = ref<HTMLInputElement | null>(null);
const draggingNode = ref<ViewTreeNode | null>(null);
const dragTargetKey = ref("");
let unsubscribeViewReload: RuntimeUnsubscribe | null = null;
let unsubscribeViewTreeChanged: RuntimeUnsubscribe | null = null;
let pointerDragState: ViewPointerDragState | null = null;
let pointerMoveListener: ((event: PointerEvent) => void) | null = null;
let pointerUpListener: ((event: PointerEvent) => void) | null = null;
let pointerCancelListener: ((event: PointerEvent) => void) | null = null;
let suppressNextClick = false;
let suppressNextClickTimer: number | null = null;

const hasWorkspace = computed(() => !!props.workingDir.trim());
const selectedView = computed(() =>
  views.value.find((view) => view.id === selectedViewId.value) ?? null,
);
const selectedViewExporting = computed(() =>
  !!selectedView.value && exportingViewId.value === selectedView.value.id,
);
const selectedViewPath = computed(() => selectedView.value?.packageRoot || "");
const selectedViewUpdatedAt = computed(() =>
  selectedView.value ? formatTimestamp(selectedView.value.updatedAt) : "",
);
const selectedViewCapabilityText = computed(() => {
  const caps = selectedView.value?.capabilities;
  if (!caps) return "";
  const enabled = [
    caps.unity ? "Unity" : "",
    caps.bindings ? "Bindings" : "",
    caps.writeBack ? "Write Back" : "",
  ].filter(Boolean);
  return enabled.length ? enabled.join(" / ") : t("view.metadata.capabilityNone");
});
const selectedViewUnityRequirementText = computed(() => {
  const view = selectedView.value;
  if (!view) return "";
  return viewRequiresUnityConnection(view)
    ? t("view.metadata.unityConnectionRequired")
    : t("view.metadata.unityConnectionOptional");
});
const viewTreeNodes = computed(() => buildViewTree(views.value, folders.value));
const visibleViewEntries = computed<VisibleViewEntry[]>(() => {
  const entries: VisibleViewEntry[] = [];
  if (createFolderDraft.value?.anchorKey === ROOT_ANCHOR_KEY) {
    entries.push({
      type: "create",
      key: "view-create:root",
      draft: createFolderDraft.value,
    });
  }

  const walk = (nodes: ViewTreeNode[], depth: number) => {
    for (const node of nodes) {
      const expanded = isNodeExpanded(node);
      const hasChildren = node.children.length > 0;
      const row: VisibleViewRow = { node, depth, expanded, hasChildren };
      entries.push({ type: "row", key: node.key, row });
      if (createFolderDraft.value?.anchorKey === node.key) {
        entries.push({
          type: "create",
          key: `view-create:${node.key}`,
          draft: createFolderDraft.value,
        });
      }
      if (node.kind === "folder" && expanded && hasChildren) {
        walk(node.children, depth + 1);
      }
    }
  };

  walk(viewTreeNodes.value, 0);
  return entries;
});

function loadExpandedState(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_VIEW_EXPANDED);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function persistExpandedState() {
  try {
    localStorage.setItem(STORAGE_KEY_VIEW_EXPANDED, JSON.stringify(expandedState.value));
  } catch {
    // ignore persistence failures
  }
}

function formatTimestamp(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return t("view.metadata.empty");
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function safeArchiveName(value: string): string {
  const normalized = value
    .trim()
    .replace(/[^a-zA-Z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "view-package";
}

function normalizeViewPath(value: string): string {
  return value.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/\/$/, "");
}

function packageRelPath(view: ViewPackageSummary): string {
  const explicitRelPath = view.packageRelPath?.trim();
  if (explicitRelPath) return normalizeViewPath(explicitRelPath);

  const packageRoot = normalizeViewPath(view.packageRoot);
  const workingDir = props.workingDir.trim();
  if (workingDir) {
    const viewsRoot = normalizeViewPath(`${workingDir}/Locus/View`);
    const lowerPackageRoot = packageRoot.toLowerCase();
    const lowerViewsRoot = viewsRoot.toLowerCase();
    if (lowerPackageRoot.startsWith(`${lowerViewsRoot}/`)) {
      return packageRoot.slice(viewsRoot.length + 1);
    }
  }

  const parts = packageRoot.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : view.id;
}

function makeFolderNode(
  relPath: string,
  label: string,
  folder?: ViewFolderSummary,
): ViewTreeNode {
  return {
    kind: "folder",
    key: `view-dir:${relPath}`,
    label,
    relPath,
    children: [],
    folder,
  };
}

function viewPathDirname(relPath: string): string {
  const parts = normalizeViewPath(relPath).split("/").filter(Boolean);
  return parts.slice(0, -1).join("/");
}

function expandViewPathAncestors(relPath: string) {
  const parts = viewPathDirname(relPath).split("/").filter(Boolean);
  let current = "";
  for (const part of parts) {
    current = current ? `${current}/${part}` : part;
    setNodeExpanded(viewFolderKeyForRelPath(current), true);
  }
}

function buildViewTree(
  viewSummaries: ViewPackageSummary[],
  viewFolders: ViewFolderSummary[],
): ViewTreeNode[] {
  const root = makeFolderNode("", "views");
  const folderMap = new Map<string, ViewTreeNode>([["", root]]);
  const ensureFolder = (relPath: string, folder?: ViewFolderSummary) => {
    const normalized = normalizeViewPath(relPath);
    if (!normalized) return root;
    const parts = normalized.split("/").filter(Boolean);
    let parent = root;
    let currentPath = "";
    for (const part of parts) {
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      let node = folderMap.get(currentPath);
      if (!node) {
        node = makeFolderNode(currentPath, part);
        folderMap.set(currentPath, node);
        parent.children.push(node);
      }
      if (folder && currentPath === normalized) {
        node.folder = folder;
        node.label = folder.name || part;
      }
      parent = node;
    }
    return parent;
  };

  const sortedFolders = [...viewFolders].sort((left, right) =>
    normalizeViewPath(left.relPath).localeCompare(
      normalizeViewPath(right.relPath),
      undefined,
      { sensitivity: "base" },
    ),
  );
  for (const folder of sortedFolders) {
    ensureFolder(folder.relPath, folder);
  }

  const sortedViews = [...viewSummaries].sort(
    (left, right) =>
      packageRelPath(left).localeCompare(packageRelPath(right), undefined, {
        sensitivity: "base",
      }) ||
      left.name.localeCompare(right.name, undefined, { sensitivity: "base" }) ||
      left.id.localeCompare(right.id, undefined, { sensitivity: "base" }),
  );

  for (const view of sortedViews) {
    const relPath = packageRelPath(view);
    const parent = ensureFolder(viewPathDirname(relPath));
    parent.children.push({
      kind: "view",
      key: `view:${relPath}:${view.id}`,
      label: view.name || view.id,
      relPath,
      children: [],
      view,
    });
  }

  const sortChildren = (node: ViewTreeNode) => {
    node.children.sort((left, right) => {
      if (left.kind !== right.kind) return left.kind === "folder" ? -1 : 1;
      return left.label.localeCompare(right.label, undefined, { sensitivity: "base" });
    });
    node.children.forEach(sortChildren);
  };
  sortChildren(root);
  return root.children;
}

function applyTreeSnapshot(snapshot: ViewTreeSnapshot) {
  views.value = snapshot.views;
  folders.value = snapshot.folders;
  if (!views.value.some((view) => view.id === selectedViewId.value)) {
    selectedViewId.value = views.value[0]?.id ?? "";
  }
}

async function loadViews() {
  if (!hasWorkspace.value) return;
  loading.value = true;
  loadError.value = "";
  try {
    applyTreeSnapshot(await viewTree());
  } catch (error) {
    const err = normalizeViewError(error);
    views.value = [];
    folders.value = [];
    loadError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewTree",
      replaceOperation: true,
    });
  } finally {
    loading.value = false;
  }
}

function isNodeExpanded(node: ViewTreeNode): boolean {
  if (node.kind !== "folder") return false;
  return expandedState.value[node.key] ?? true;
}

function setNodeExpanded(key: string, value: boolean) {
  expandedState.value = { ...expandedState.value, [key]: value };
  persistExpandedState();
}

function toggleRow(row: VisibleViewRow) {
  if (row.node.kind !== "folder") return;
  setNodeExpanded(row.node.key, !row.expanded);
}

function treeIndentPx(depth: number): number {
  return VIEW_TREE_INDENT_BASE_PX + Math.max(0, depth) * VIEW_TREE_INDENT_STEP_PX;
}

function selectTreeRow(row: VisibleViewRow, event?: MouseEvent) {
  if (suppressNextClick) {
    event?.preventDefault();
    event?.stopPropagation();
    suppressNextClick = false;
    return;
  }
  closeContextMenu();
  if (row.node.kind === "folder") {
    toggleRow(row);
    return;
  }
  if (row.node.view) {
    selectedViewId.value = row.node.view.id;
  }
}

function closeContextMenu() {
  contextMenu.value = null;
}

function closeCreateFolder() {
  createFolderDraft.value = null;
}

function setCreateFolderInputRef(element: Element | ComponentPublicInstance | null) {
  createFolderInputRef.value = element instanceof HTMLInputElement ? element : null;
}

function onTreeContextMenu(event: MouseEvent) {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(".view-tree-row-shell, .view-tree-create-row")
  ) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  closeCreateFolder();
  deleteConfirm.value = null;
  contextMenu.value = { x: event.clientX, y: event.clientY, kind: "root" };
}

function openTreeContextMenu(event: MouseEvent, row: VisibleViewRow) {
  event.preventDefault();
  event.stopPropagation();
  closeCreateFolder();
  deleteConfirm.value = null;
  contextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    kind: row.node.kind,
    node: row.node,
  };
}

function createDepthForNode(node: ViewTreeNode | null): number {
  if (!node) return 0;
  const entry = visibleViewEntries.value.find(
    (item): item is Extract<VisibleViewEntry, { type: "row" }> =>
      item.type === "row" && item.row.node.key === node.key,
  );
  return (entry?.row.depth ?? 0) + 1;
}

async function startCreateFolder(parentNode: ViewTreeNode | null) {
  deleteConfirm.value = null;
  closeCreateFolder();
  if (parentNode?.kind === "view") return;
  if (parentNode && !isNodeExpanded(parentNode)) {
    setNodeExpanded(parentNode.key, true);
  }
  createFolderDraft.value = {
    parentRelPath: parentNode?.relPath ?? "",
    anchorKey: parentNode?.key ?? ROOT_ANCHOR_KEY,
    depth: createDepthForNode(parentNode),
    name: "",
  };
  closeContextMenu();
  await nextTick();
  createFolderInputRef.value?.focus();
  createFolderInputRef.value?.select();
}

async function beginCreateFolderFromContext() {
  const menu = contextMenu.value;
  if (!menu || menu.kind === "view") return;
  await startCreateFolder(menu.kind === "folder" ? menu.node : null);
}

async function submitCreateFolder() {
  const draft = createFolderDraft.value;
  const name = draft?.name.trim();
  if (!draft || !name) return;
  try {
    await viewCreateFolder({ parentRelPath: draft.parentRelPath, name });
    closeCreateFolder();
    await loadViews();
  } catch (error) {
    const err = normalizeAppError(error);
    loadError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewCreateFolder",
      replaceOperation: true,
    });
  }
}

function requestDeleteEntry() {
  const menu = contextMenu.value;
  if (!menu || menu.kind === "root") return;
  deleteConfirm.value = menu.node;
}

function closeDeleteConfirm() {
  deleteConfirm.value = null;
  closeContextMenu();
}

async function confirmDeleteEntry() {
  const node = deleteConfirm.value;
  if (!node) return;
  try {
    applyTreeSnapshot(await viewDeleteEntry({ relPath: node.relPath }));
    closeDeleteConfirm();
  } catch (error) {
    const err = normalizeAppError(error);
    loadError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewDeleteEntry",
      replaceOperation: true,
    });
  }
}

function clearDragState() {
  draggingNode.value = null;
  dragTargetKey.value = "";
}

function viewNodeParentRelPath(node: ViewTreeNode): string {
  return viewPathDirname(node.relPath);
}

function canDragNode(node: ViewTreeNode): boolean {
  return !!node.relPath.trim();
}

function canDropNodeOnDir(node: ViewTreeNode | null, targetDirRelPath: string): boolean {
  if (!node) return false;
  const targetDir = normalizeViewPath(targetDirRelPath);
  if (targetDir === viewNodeParentRelPath(node)) return false;
  if (node.kind === "folder") {
    return targetDir !== node.relPath && !targetDir.startsWith(`${node.relPath}/`);
  }
  return true;
}

function visibleRowByNodeKey(key: string): VisibleViewRow | null {
  const entry = visibleViewEntries.value.find(
    (item): item is Extract<VisibleViewEntry, { type: "row" }> =>
      item.type === "row" && item.row.node.key === key,
  );
  return entry?.row ?? null;
}

function viewFolderKeyForRelPath(relPath: string): string {
  return `view-dir:${normalizeViewPath(relPath)}`;
}

function clearPointerDragListeners() {
  if (pointerMoveListener) {
    window.removeEventListener("pointermove", pointerMoveListener);
    pointerMoveListener = null;
  }
  if (pointerUpListener) {
    window.removeEventListener("pointerup", pointerUpListener);
    pointerUpListener = null;
  }
  if (pointerCancelListener) {
    window.removeEventListener("pointercancel", pointerCancelListener);
    pointerCancelListener = null;
  }
  document.body.classList.remove("view-tree-pointer-dragging");
}

function scheduleSuppressNextClick() {
  suppressNextClick = true;
  if (suppressNextClickTimer) {
    window.clearTimeout(suppressNextClickTimer);
  }
  suppressNextClickTimer = window.setTimeout(() => {
    suppressNextClick = false;
    suppressNextClickTimer = null;
  }, 240);
}

function clearPointerDragState() {
  pointerDragState = null;
  clearPointerDragListeners();
}

function shouldIgnorePointerDrag(event: PointerEvent): boolean {
  const target = event.target;
  return (
    target instanceof Element &&
    !!target.closest(
      ".view-tree-branch-slot, .view-tree-create-row, input, textarea, select",
    )
  );
}

function resolveDropTargetFromPoint(
  x: number,
  y: number,
  node: ViewTreeNode,
): ViewDropTarget | null {
  const target = document.elementFromPoint(x, y);
  if (!(target instanceof Element)) return null;

  const rowElement = target.closest<HTMLElement>(".view-tree-row-shell");
  if (rowElement) {
    if (rowElement.dataset.viewNodeKind !== "folder") return null;
    const row = visibleRowByNodeKey(rowElement.dataset.viewNodeKey ?? "");
    if (!row || row.node.kind !== "folder") return null;
    if (!canDropNodeOnDir(node, row.node.relPath)) return null;
    return {
      key: row.node.key,
      relPath: row.node.relPath,
    };
  }

  const listElement = target.closest<HTMLElement>(".view-list");
  if (listElement && canDropNodeOnDir(node, "")) {
    return {
      key: ROOT_ANCHOR_KEY,
      relPath: "",
    };
  }

  return null;
}

function updatePointerDropTarget(event: PointerEvent) {
  const state = pointerDragState;
  if (!state?.active) return;
  const target = resolveDropTargetFromPoint(event.clientX, event.clientY, state.node);
  dragTargetKey.value = target?.key ?? "";
}

function onTreePointerDown(row: VisibleViewRow, event: PointerEvent) {
  if (
    event.button !== 0 ||
    createFolderDraft.value ||
    !canDragNode(row.node) ||
    shouldIgnorePointerDrag(event)
  ) {
    return;
  }

  clearPointerDragState();
  pointerDragState = {
    node: row.node,
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    active: false,
  };

  pointerMoveListener = onTreePointerMove;
  pointerUpListener = (upEvent) => {
    void finishPointerDrag(upEvent);
  };
  pointerCancelListener = () => {
    clearDragState();
    clearPointerDragState();
  };

  window.addEventListener("pointermove", pointerMoveListener);
  window.addEventListener("pointerup", pointerUpListener);
  window.addEventListener("pointercancel", pointerCancelListener);
}

function onTreePointerMove(event: PointerEvent) {
  const state = pointerDragState;
  if (!state || event.pointerId !== state.pointerId) return;

  const dx = event.clientX - state.startX;
  const dy = event.clientY - state.startY;
  if (!state.active) {
    if (Math.hypot(dx, dy) < 5) return;
    state.active = true;
    draggingNode.value = state.node;
    dragTargetKey.value = "";
    closeContextMenu();
    closeCreateFolder();
    document.body.classList.add("view-tree-pointer-dragging");
  }

  event.preventDefault();
  updatePointerDropTarget(event);
}

async function finishPointerDrag(event: PointerEvent) {
  const state = pointerDragState;
  if (!state || event.pointerId !== state.pointerId) return;

  const target = state.active
    ? resolveDropTargetFromPoint(event.clientX, event.clientY, state.node)
    : null;
  const shouldSuppressClick = state.active;
  clearPointerDragState();

  if (shouldSuppressClick) {
    scheduleSuppressNextClick();
  }

  if (!target) {
    clearDragState();
    return;
  }

  await moveNodeToDir(state.node, target.relPath, target.key);
}

function onTreeDragStart(row: VisibleViewRow, event: DragEvent) {
  if (createFolderDraft.value || !canDragNode(row.node)) {
    event.preventDefault();
    return;
  }
  closeContextMenu();
  draggingNode.value = row.node;
  dragTargetKey.value = "";
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", row.node.relPath);
  }
}

function onTreeDragEnd() {
  clearDragState();
}

function onTreeFolderDragOver(row: VisibleViewRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNode.value) return;
  if (!canDropNodeOnDir(draggingNode.value, row.node.relPath)) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetKey.value = row.node.key;
}

async function moveNodeToDir(
  node: ViewTreeNode,
  targetDirRelPath: string,
  targetKey = targetDirRelPath ? viewFolderKeyForRelPath(targetDirRelPath) : ROOT_ANCHOR_KEY,
) {
  if (!node || !canDropNodeOnDir(node, targetDirRelPath)) {
    clearDragState();
    return;
  }
  clearDragState();
  if (targetKey !== ROOT_ANCHOR_KEY) {
    setNodeExpanded(targetKey, true);
  }
  try {
    applyTreeSnapshot(
      await viewMoveEntry({
        sourceRelPath: node.relPath,
        targetDirRelPath,
      }),
    );
    if (node.kind === "view" && node.view) {
      selectedViewId.value = node.view.id;
    }
  } catch (error) {
    const err = normalizeAppError(error);
    loadError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewMoveEntry",
      replaceOperation: true,
    });
  }
}

async function moveDraggingNode(targetDirRelPath: string, targetKey?: string) {
  const node = draggingNode.value;
  if (!node) {
    clearDragState();
    return;
  }
  await moveNodeToDir(node, targetDirRelPath, targetKey);
}

function onTreeFolderDrop(row: VisibleViewRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingNode.value) return;
  if (!canDropNodeOnDir(draggingNode.value, row.node.relPath)) {
    clearDragState();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  void moveDraggingNode(row.node.relPath, row.node.key);
}

function onTreeRootDragOver(event: DragEvent) {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(".view-tree-row-shell, .view-tree-create-row")
  ) {
    return;
  }
  if (!canDropNodeOnDir(draggingNode.value, "")) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dragTargetKey.value = ROOT_ANCHOR_KEY;
}

function onTreeRootDrop(event: DragEvent) {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest(".view-tree-row-shell, .view-tree-create-row")
  ) {
    return;
  }
  if (!canDropNodeOnDir(draggingNode.value, "")) {
    clearDragState();
    return;
  }
  event.preventDefault();
  void moveDraggingNode("", ROOT_ANCHOR_KEY);
}

async function importViewPackage(targetDirRelPath = "") {
  if (importing.value) return;
  closeContextMenu();
  closeCreateFolder();
  deleteConfirm.value = null;
  importing.value = true;
  try {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: t("view.archive.filter"), extensions: ["zip"] }],
    });
    if (!selected || typeof selected !== "string") return;

    const result = await viewImportPackage({
      filePath: selected,
      targetDirRelPath,
    });
    applyTreeSnapshot(result.snapshot);
    selectedViewId.value = result.summary.id;
    expandViewPathAncestors(result.summary.packageRelPath || result.summary.id);
    notificationStore.addNotice(
      "success",
      t("view.import.imported", result.summary.name),
      {
        operation: "viewImportPackage",
        skipConsoleLog: true,
      },
    );
  } catch (error) {
    const err = normalizeAppError(error);
    loadError.value = err.message;
    notificationStore.addNotice("error", t("view.import.failed", err.message), {
      code: err.code,
      operation: "viewImportPackage",
      replaceOperation: true,
    });
  } finally {
    importing.value = false;
  }
}

async function importViewPackageFromContext() {
  const menu = contextMenu.value;
  if (!menu || menu.kind === "view") return;
  await importViewPackage(menu.kind === "folder" ? menu.node.relPath : "");
}

async function exportViewPackage(view: ViewPackageSummary | null = selectedView.value) {
  if (!view || exportingViewId.value) return;
  exportingViewId.value = view.id;
  try {
    const filePath = await save({
      defaultPath: `${safeArchiveName(view.id)}.zip`,
      filters: [{ name: t("view.archive.filter"), extensions: ["zip"] }],
    });
    if (!filePath) return;

    const savedPath = await viewExportPackage({
      viewId: view.id,
      filePath,
    });
    notificationStore.addNotice("success", t("view.export.exported", savedPath), {
      operation: "viewExportPackage",
      skipConsoleLog: true,
    });
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", t("view.export.failed", err.message), {
      code: err.code,
      operation: "viewExportPackage",
      replaceOperation: true,
    });
  } finally {
    exportingViewId.value = "";
  }
}

async function exportViewPackageFromContext() {
  const menu = contextMenu.value;
  if (!menu || menu.kind !== "view") return;
  const view = menu.node.view ?? null;
  closeContextMenu();
  await exportViewPackage(view);
}

async function openViewPackage(view: ViewPackageSummary) {
  if (!view || running.value) return;
  selectedViewId.value = view.id;
  running.value = true;
  try {
    const requirementError = await checkViewOpenRequirements(view);
    if (requirementError) {
      notificationStore.addNotice("error", requirementError.message, {
        code: requirementError.code,
        operation: "viewRun",
        replaceOperation: true,
      });
      return;
    }
    await viewRun(view.id);
  } catch (runError) {
    const err = normalizeViewError(runError, { viewName: view.name });
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewRun",
      replaceOperation: true,
    });
  } finally {
    running.value = false;
  }
}

async function openSelectedView() {
  const view = selectedView.value;
  if (!view) return;
  await openViewPackage(view);
}

async function openTreeView(row: VisibleViewRow) {
  if (row.node.kind !== "view" || !row.node.view) return;
  await openViewPackage(row.node.view);
}

watch(() => props.workingDir, () => {
  selectedViewId.value = "";
  closeContextMenu();
  closeCreateFolder();
  deleteConfirm.value = null;
  clearDragState();
  clearPointerDragState();
  if (hasWorkspace.value) void loadViews();
});

onMounted(async () => {
  unsubscribeViewReload = await getLocusRuntime().subscribe<ViewPackageSummary>(
    "view-package-reloaded",
    () => {
      void loadViews();
    },
  );
  unsubscribeViewTreeChanged = await getLocusRuntime().subscribe(
    "view-tree-changed",
    () => {
      void loadViews();
    },
  );
  if (hasWorkspace.value) await loadViews();
});

onUnmounted(() => {
  unsubscribeViewReload?.();
  unsubscribeViewReload = null;
  unsubscribeViewTreeChanged?.();
  unsubscribeViewTreeChanged = null;
  clearPointerDragState();
  if (suppressNextClickTimer) {
    window.clearTimeout(suppressNextClickTimer);
    suppressNextClickTimer = null;
  }
});
</script>

<template>
  <div class="view-package-view">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.viewDescription')"
    />

    <template v-else>
      <div class="view-layout">
        <aside class="view-sidebar">
          <div class="view-pane-header">
            <span>{{ t("view.list.title") }}</span>
            <div class="view-pane-header-actions">
              <BaseButton
                class="view-header-action"
                :title="importing ? t('view.action.importing') : t('view.import.action')"
                :disabled="importing"
                @click="importViewPackage('')"
              >
                <LucideIcon :icon="Upload" :size="13" :stroke-width="2" />
              </BaseButton>
            </div>
          </div>

          <div
            class="view-list"
            :class="{ 'is-root-drop-target': dragTargetKey === ROOT_ANCHOR_KEY }"
            @contextmenu.prevent="onTreeContextMenu"
            @dragover="onTreeRootDragOver"
            @drop="onTreeRootDrop"
          >
            <template v-for="entry in visibleViewEntries" :key="entry.key">
              <div
                v-if="entry.type === 'row'"
                class="view-tree-row-shell"
                :class="{
                  folder: entry.row.node.kind === 'folder',
                  open: entry.row.expanded,
                  active:
                    entry.row.node.kind === 'view' &&
                    entry.row.node.view?.id === selectedViewId,
                  dragging: draggingNode?.key === entry.row.node.key,
                  'drop-target': dragTargetKey === entry.row.node.key,
                }"
                draggable="false"
                :data-view-node-key="entry.row.node.key"
                :data-view-node-kind="entry.row.node.kind"
                :title="
                  entry.row.node.view?.packageRoot ||
                  entry.row.node.folder?.packageRoot ||
                  entry.row.node.label
                "
                @pointerdown="onTreePointerDown(entry.row, $event)"
                @contextmenu.prevent.stop="openTreeContextMenu($event, entry.row)"
                @dragstart="onTreeDragStart(entry.row, $event)"
                @dragend="onTreeDragEnd"
                @dragover="onTreeFolderDragOver(entry.row, $event)"
                @drop="onTreeFolderDrop(entry.row, $event)"
              >
                <button
                  type="button"
                  class="view-tree-row"
                  :style="{ paddingLeft: `${treeIndentPx(entry.row.depth)}px` }"
                  @click="selectTreeRow(entry.row, $event)"
                >
                  <span
                    v-if="entry.row.node.kind === 'folder' && entry.row.hasChildren"
                    class="view-tree-branch-slot"
                    @click.stop="toggleRow(entry.row)"
                  >
                    <LucideIcon
                      class="view-tree-chevron"
                      :class="{ open: entry.row.expanded }"
                      :icon="ChevronRight"
                      :size="10"
                      :stroke-width="2.4"
                    />
                  </span>
                  <span
                    v-else-if="entry.row.node.kind === 'folder' || entry.row.depth > 0"
                    class="view-tree-branch-spacer"
                    aria-hidden="true"
                  ></span>
                  <span
                    v-if="entry.row.node.kind === 'folder'"
                    class="view-tree-kind-icon folder"
                    :class="{ open: entry.row.expanded }"
                    aria-hidden="true"
                  >
                    <LucideIcon
                      :icon="entry.row.expanded ? FolderOpen : Folder"
                      :size="13"
                      :stroke-width="2"
                    />
                  </span>
                  <span v-else class="view-tree-kind-icon view" aria-hidden="true">
                    <LucideIcon
                      :icon="resolveLocusViewIcon(entry.row.node.view?.icon)"
                      :size="13"
                      :stroke-width="2"
                    />
                  </span>
                  <span class="view-tree-label">{{ entry.row.node.label }}</span>
                </button>
                <div
                  v-if="entry.row.node.kind === 'view'"
                  class="view-tree-row-actions"
                >
                  <button
                    type="button"
                    class="view-tree-open-action"
                    :title="t('view.action.open')"
                    :aria-label="`${t('view.action.open')} ${entry.row.node.label}`"
                    :disabled="running"
                    @pointerdown.stop
                    @dragstart.prevent.stop
                    @click.stop="openTreeView(entry.row)"
                  >
                    <LucideIcon :icon="PanelTopOpen" :size="12" :stroke-width="2" />
                    <span>{{ t("view.action.open") }}</span>
                  </button>
                </div>
              </div>

              <div
                v-else
                class="view-tree-create-row"
                :style="{ paddingLeft: `${treeIndentPx(entry.draft.depth)}px` }"
              >
                <span class="view-tree-bullet" aria-hidden="true"></span>
                <div class="view-tree-create-body">
                  <input
                    :ref="setCreateFolderInputRef"
                    v-model="entry.draft.name"
                    class="view-tree-create-input"
                    :placeholder="t('view.tree.folderNamePlaceholder')"
                    :aria-label="t('view.tree.createFolder')"
                    @click.stop
                    @keydown.enter.prevent="submitCreateFolder"
                    @keydown.esc.prevent="closeCreateFolder"
                  />
                  <div class="view-tree-create-actions">
                    <BaseButton
                      class="view-tree-create-action"
                      type="button"
                      :title="t('common.confirm')"
                      :disabled="!entry.draft.name.trim()"
                      @pointerdown.prevent
                      @click="submitCreateFolder"
                    >
                      <LucideIcon :icon="Check" :size="12" :stroke-width="2.4" />
                    </BaseButton>
                    <BaseButton
                      class="view-tree-create-action"
                      type="button"
                      :title="t('common.cancel')"
                      @pointerdown.prevent
                      @click="closeCreateFolder"
                    >
                      <LucideIcon :icon="X" :size="12" :stroke-width="2.4" />
                    </BaseButton>
                  </div>
                </div>
              </div>
            </template>
            <div v-if="loadError" class="view-empty is-error">{{ loadError }}</div>
            <div v-else-if="!viewTreeNodes.length && !loading && !createFolderDraft" class="view-empty">
              {{ t("view.list.empty") }}
            </div>
            <div v-if="loading" class="view-empty">{{ t("common.loading") }}</div>
          </div>
        </aside>

        <section class="view-detail">
          <div class="view-detail-toolbar">
            <div class="view-detail-title">
              <span>{{ selectedView?.name || t("view.detail.emptyTitle") }}</span>
              <small v-if="selectedView">{{ selectedView.id }}</small>
            </div>
            <div class="view-detail-actions">
              <BaseButton
                :disabled="!selectedView || !!exportingViewId"
                @click="exportViewPackage()"
              >
                <LucideIcon :icon="Download" :size="13" :stroke-width="2" />
                {{ selectedViewExporting ? t("view.action.exporting") : t("view.action.export") }}
              </BaseButton>
              <BaseButton :disabled="!selectedViewId || running" @click="openSelectedView">
                {{ running ? t("view.action.opening") : t("view.action.open") }}
              </BaseButton>
            </div>
          </div>

          <div v-if="!selectedView" class="view-detail-state">{{ t("view.detail.empty") }}</div>
          <div v-else class="view-detail-body">
            <div class="view-section-header">{{ t("view.metadata.title") }}</div>
            <dl class="view-metadata-list">
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.name") }}</dt>
                <dd>{{ selectedView.name }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.id") }}</dt>
                <dd class="mono">{{ selectedView.id }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.template") }}</dt>
                <dd class="mono">{{ selectedView.template }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.version") }}</dt>
                <dd class="mono">{{ selectedView.version }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.capabilities") }}</dt>
                <dd>{{ selectedViewCapabilityText }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.unityConnection") }}</dt>
                <dd>{{ selectedViewUnityRequirementText }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.updatedAt") }}</dt>
                <dd>{{ selectedViewUpdatedAt }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.location") }}</dt>
                <dd class="mono path">{{ selectedViewPath }}</dd>
              </div>
            </dl>
          </div>
        </section>
      </div>

      <BaseContextMenu
        v-if="contextMenu"
        class="view-ctx-menu"
        :x="contextMenu.x"
        :y="contextMenu.y"
        :min-width="132"
        @close="closeContextMenu"
      >
            <template v-if="contextMenu.kind === 'root' || contextMenu.kind === 'folder'">
              <button type="button" class="view-ctx-item" @click="importViewPackageFromContext">
                {{ importing ? t("view.action.importing") : t("view.action.import") }}
              </button>
              <button type="button" class="view-ctx-item" @click="beginCreateFolderFromContext">
                {{ t("view.tree.createFolder") }}
              </button>
              <div class="view-ctx-sep"></div>
            </template>
            <button
              v-if="contextMenu.kind === 'view'"
              type="button"
              class="view-ctx-item"
              :disabled="!!exportingViewId"
              @click.stop="exportViewPackageFromContext"
            >
              {{
                exportingViewId === contextMenu.node.view?.id
                  ? t("view.action.exporting")
                  : t("view.action.export")
              }}
            </button>
            <div v-if="contextMenu.kind === 'view'" class="view-ctx-sep"></div>
            <button
              v-if="contextMenu.kind === 'folder' || contextMenu.kind === 'view'"
              type="button"
              class="view-ctx-item danger"
              @click.stop="requestDeleteEntry"
            >
              {{ t("view.tree.delete") }}
            </button>
      </BaseContextMenu>

      <Teleport to="body">
          <div
            v-if="contextMenu && deleteConfirm"
            class="view-delete-confirm"
            :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
            @click.stop
          >
            <div class="view-delete-confirm-title">
              {{ t("view.tree.deleteConfirmTitle") }}
            </div>
            <div class="view-delete-confirm-text">
              {{ t("view.tree.deleteConfirmMessage", deleteConfirm.label) }}
            </div>
            <div class="view-delete-confirm-actions">
              <BaseButton class="view-delete-confirm-btn" @click="closeDeleteConfirm">
                {{ t("common.cancel") }}
              </BaseButton>
              <BaseButton
                class="view-delete-confirm-btn"
                variant="danger"
                @click="confirmDeleteEntry"
              >
                {{ t("common.confirm") }}
              </BaseButton>
            </div>
          </div>
      </Teleport>
    </template>
  </div>
</template>

<style scoped>
.view-package-view {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
}

.view-layout {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.view-sidebar {
  width: 320px;
  min-width: 280px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  overflow: hidden;
}

.view-pane-header {
  flex-shrink: 0;
  min-height: 38px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 10px 6px 12px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.view-pane-header-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.view-header-action {
  width: 26px;
  min-width: 26px;
  height: 26px;
  padding: 0;
  border-color: transparent;
  color: var(--text-secondary);
}

.view-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 4px 6px 8px;
  transition: background 0.12s ease;
}

.view-list.is-root-drop-target {
  background: color-mix(in srgb, var(--accent-color) 6%, transparent);
}

.view-tree-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  width: 100%;
  min-width: 0;
  background: transparent;
  touch-action: none;
  user-select: none;
  transition: background 0.1s ease, box-shadow 0.1s ease, opacity 0.1s ease;
}

.view-tree-row-shell:hover {
  background: var(--hover-bg);
}

.view-tree-row-shell.active,
.view-tree-row-shell.active:hover {
  background: var(--active-bg);
}

.view-tree-row-shell.dragging {
  opacity: 0.48;
}

.view-tree-row-shell.dragging .view-tree-row {
  cursor: grabbing;
}

.view-tree-row-shell.drop-target,
.view-tree-row-shell.drop-target:hover {
  background: color-mix(in srgb, var(--active-bg) 62%, transparent);
  box-shadow: inset 0 0 0 1px
    color-mix(in srgb, var(--accent-color) 32%, var(--border-color));
}

.view-tree-row {
  flex: 1 1 auto;
  width: auto;
  min-width: 0;
  min-height: 26px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 12px 2px 16px;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 13px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}

.view-tree-row-actions {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  max-width: 0;
  margin-right: 0;
  padding-left: 0;
  overflow: hidden;
  opacity: 0;
  pointer-events: none;
  transition:
    max-width 0.16s ease,
    margin-right 0.16s ease,
    padding-left 0.16s ease,
    opacity 0.12s ease;
}

.view-tree-row-shell:hover .view-tree-row-actions,
.view-tree-row-shell:focus-within .view-tree-row-actions,
.view-tree-row-actions:focus-within {
  max-width: 72px;
  margin-right: 6px;
  padding-left: 4px;
  opacity: 1;
  pointer-events: auto;
}

.view-tree-open-action {
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease, opacity 0.12s ease;
}

.view-tree-open-action:hover:not(:disabled),
.view-tree-open-action:focus-visible {
  border-color: var(--border-color);
  background: var(--active-bg);
  color: var(--text-color);
}

.view-tree-open-action:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.view-tree-open-action:disabled {
  cursor: progress;
  opacity: 0.52;
}

.view-tree-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.view-tree-branch-slot,
.view-tree-branch-spacer,
.view-tree-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.view-tree-branch-slot {
  border-radius: 4px;
  cursor: pointer;
}

.view-tree-branch-slot:hover {
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
}

.view-tree-chevron {
  opacity: 0.58;
  transition: transform 0.15s ease, opacity 0.12s ease;
}

.view-tree-row-shell:hover .view-tree-chevron {
  opacity: 0.9;
}

.view-tree-chevron.open {
  transform: rotate(90deg);
}

.view-tree-kind-icon {
  transition: color 0.15s ease;
}

.view-tree-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.view-tree-kind-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
}

.view-tree-kind-icon.view {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.view-tree-row-shell:hover .view-tree-kind-icon.view,
.view-tree-row-shell.active .view-tree-kind-icon.view {
  color: var(--accent-color);
}

.view-tree-label {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: currentColor;
  font-size: 13px;
  line-height: 1.35;
  font-weight: 500;
}

.view-tree-row-shell.folder .view-tree-label {
  font-weight: 600;
}

.view-tree-create-row {
  min-height: 30px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 12px 2px 16px;
  background: color-mix(in srgb, var(--active-bg) 78%, transparent);
}

.view-tree-bullet {
  width: 10px;
  height: 10px;
  position: relative;
  flex-shrink: 0;
}

.view-tree-bullet::before {
  content: "";
  width: 4px;
  height: 4px;
  display: block;
  border-radius: 50%;
  background: var(--text-secondary);
  opacity: 0.5;
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
}

.view-tree-create-body {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 6px;
}

.view-tree-create-input {
  min-width: 0;
  flex: 1 1 auto;
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

.view-tree-create-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.view-tree-create-actions {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.view-tree-create-action {
  width: 24px;
  min-width: 24px;
  height: 24px;
  padding: 0;
}

:global(body.view-tree-pointer-dragging) {
  cursor: grabbing;
  user-select: none;
}

:global(body.view-tree-pointer-dragging *) {
  cursor: grabbing !important;
}

.view-empty {
  padding: 8px 7px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.view-empty.is-error {
  color: var(--status-danger-fg);
}

.view-detail {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.view-detail-toolbar {
  flex-shrink: 0;
  min-height: 46px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.view-detail-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.view-detail-title span {
  color: var(--text-color);
  font-size: 14px;
  font-weight: 650;
}

.view-detail-title small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.view-detail-actions {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.view-detail-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.view-detail-body {
  flex: 1;
  min-width: 0;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
}

.view-section-header {
  flex-shrink: 0;
  min-height: 34px;
  display: flex;
  align-items: center;
  padding: 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.view-metadata-list {
  margin: 0;
  max-width: 780px;
  padding: 12px;
}

.view-metadata-row {
  min-height: 36px;
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  padding: 8px 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 62%, transparent);
}

.view-metadata-row dt {
  color: var(--text-secondary);
  font-size: 12px;
}

.view-metadata-row dd {
  min-width: 0;
  margin: 0;
  color: var(--text-color);
  font-size: 13px;
}

.view-metadata-row dd.mono {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.view-metadata-row dd.path {
  overflow-wrap: anywhere;
}

.view-delete-confirm {
  position: fixed;
  z-index: 10001;
  width: 244px;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color));
  border-radius: 10px;
  background: var(--sidebar-bg);
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.18);
}

.view-delete-confirm-title {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.view-delete-confirm-text {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.view-delete-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.view-delete-confirm-btn {
  min-width: 68px;
}
</style>
