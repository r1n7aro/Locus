<script setup lang="ts">
import type { SessionSummary, SaveRawContextRequest } from "../../types";
import type { SessionTreeNode, SessionTreeSessionNode } from "./sessionTree";
import {
  computed,
  ref,
  nextTick,
  onMounted,
  onUnmounted,
  watch,
  type ComponentPublicInstance,
} from "vue";
import { Check, ChevronRight, Folder, FolderOpen, HelpCircle, X } from "lucide";
import { t } from "../../i18n";
import { buildSessionTree, nodeContainsSession, nodeHasActiveDescendant } from "./sessionTree";
import BaseButton from "../ui/BaseButton.vue";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "../icons/locusViewIcons";
import { formatShortcut, useKeyboardShortcuts } from "../../composables/useKeyboardShortcuts";
import { normalizeAppError } from "../../services/errors";
import {
  checkViewOpenRequirements,
  normalizeViewError,
  viewCreateFolder,
  viewDeleteEntry,
  viewMoveEntry,
  viewRequiresUnityConnection,
  viewRun,
  viewRunInUnity,
  viewTree,
  viewUnityConnectionRequiredError,
  type ViewFolderSummary,
  type ViewPackageSummary,
} from "../../services/view";
import { openUnityEmbeddedSessionWindow } from "../../services/unity";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../../services/locusRuntime";
import { useNotificationStore } from "../../stores/notification";
import { useProjectStore } from "../../stores/project";

interface VisibleTreeRow {
  node: SessionTreeNode;
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

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

const STORAGE_KEY_EXPANDED = "locus:sessionPanelExpanded";
const STORAGE_KEY_VIEW_EXPANDED = "locus:sessionPanelViewExpanded";
const STORAGE_KEY_VIEW_SPLIT = "locus:sessionPanelViewSplitRatio";
const VIEW_ROOT_ANCHOR_KEY = "view-root";
const DEFAULT_VIEW_SECTION_RATIO = 1 / 3;
const MIN_VIEW_SECTION_RATIO = 0.24;
const MAX_VIEW_SECTION_RATIO = 0.72;
const VIEW_RESIZE_HANDLE_PX = 10;
const VIEW_TREE_INDENT_BASE_PX = 12;
const VIEW_TREE_INDENT_STEP_PX = 20;

const props = defineProps<{
  sessions: SessionSummary[];
  activeSessionId: string | null;
  streamingSessionIds?: Set<string>;
  sessionPanelWidth: number;
  workingDir?: string;
  showViews?: boolean;
}>();

const emit = defineEmits<{
  selectSession: [id: string];
  newChat: [];
  archiveSession: [id: string];
  deleteSession: [id: string];
  renameSession: [id: string, title: string];
  saveRawContext: [request: SaveRawContextRequest];
  togglePanelCollapsed: [];
}>();

function loadExpandedState(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_EXPANDED);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

const expandedState = ref<Record<string, boolean>>(loadExpandedState());

function persistExpandedState() {
  try {
    localStorage.setItem(STORAGE_KEY_EXPANDED, JSON.stringify(expandedState.value));
  } catch {
    // ignore persistence failures
  }
}

const { state: shortcutState } = useKeyboardShortcuts();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const newChatTitle = computed(() =>
  t("chat.session.newWithShortcut", formatShortcut(shortcutState.newChat)),
);
const viewSummaries = ref<ViewPackageSummary[]>([]);
const viewFolders = ref<ViewFolderSummary[]>([]);
const viewsLoading = ref(false);
const viewOpeningKey = ref("");
const viewCtxMenu = ref<ViewContextMenuState | null>(null);
const viewDeleteConfirm = ref<ViewTreeNode | null>(null);
const viewCreateFolderDraft = ref<ViewCreateFolderDraft | null>(null);
const viewCreateFolderInputRef = ref<HTMLInputElement | null>(null);
const draggingViewNode = ref<ViewTreeNode | null>(null);
const viewDragTargetKey = ref("");
const viewHelpOpen = ref(false);
const viewHelpDialogRef = ref<HTMLElement | null>(null);
const hasWorkspace = computed(() => !!props.workingDir?.trim());
const viewExpandedState = ref<Record<string, boolean>>(loadViewExpandedState());
const sessionPanelRef = ref<HTMLElement | null>(null);
const showSessionViews = computed(() => props.showViews !== false);
const viewSectionRatio = ref(loadViewSplitRatio());
let viewResizeMoveListener: ((event: MouseEvent) => void) | null = null;
let viewResizeUpListener: (() => void) | null = null;
let viewReloadUnsubscribe: RuntimeUnsubscribe | null = null;
let viewTreeChangedUnsubscribe: RuntimeUnsubscribe | null = null;
let viewPointerDragState: ViewPointerDragState | null = null;
let viewPointerMoveListener: ((event: PointerEvent) => void) | null = null;
let viewPointerUpListener: ((event: PointerEvent) => void) | null = null;
let viewPointerCancelListener: ((event: PointerEvent) => void) | null = null;
let suppressNextViewClick = false;
let suppressNextViewClickTimer: number | null = null;

function clampViewSplitRatio(value: number): number {
  return Math.min(MAX_VIEW_SECTION_RATIO, Math.max(MIN_VIEW_SECTION_RATIO, value));
}

function loadViewSplitRatio(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_VIEW_SPLIT);
    const parsed = raw ? Number.parseFloat(raw) : Number.NaN;
    if (Number.isFinite(parsed)) return clampViewSplitRatio(parsed);
  } catch {
    // ignore persistence failures
  }
  return DEFAULT_VIEW_SECTION_RATIO;
}

function persistViewSplitRatio() {
  try {
    localStorage.setItem(STORAGE_KEY_VIEW_SPLIT, String(viewSectionRatio.value));
  } catch {
    // ignore persistence failures
  }
}

const sessionListStyle = computed(() =>
  showSessionViews.value
    ? {
        flexBasis: `calc(${((1 - viewSectionRatio.value) * 100).toFixed(2)}% - ${VIEW_RESIZE_HANDLE_PX / 2}px)`,
      }
    : undefined,
);

const viewSectionStyle = computed(() => ({
  flexBasis: `calc(${(viewSectionRatio.value * 100).toFixed(2)}% - ${VIEW_RESIZE_HANDLE_PX / 2}px)`,
}));

try {
  localStorage.removeItem("locus:sessionPanelPinned");
} catch {
  // ignore persistence failures
}

const sessionTree = computed(() => buildSessionTree({
  sessions: props.sessions,
  streamingSessionIds: props.streamingSessionIds,
}));

function loadViewExpandedState(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_VIEW_EXPANDED);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function persistViewExpandedState() {
  try {
    localStorage.setItem(STORAGE_KEY_VIEW_EXPANDED, JSON.stringify(viewExpandedState.value));
  } catch {
    // ignore persistence failures
  }
}

function normalizeViewPath(value: string): string {
  return value.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/\/$/, "");
}

function physicalPackageRelPath(view: ViewPackageSummary): string {
  const explicitRelPath = view.packageRelPath?.trim();
  if (explicitRelPath) return normalizeViewPath(explicitRelPath);

  const packageRoot = normalizeViewPath(view.packageRoot);
  const workingDir = props.workingDir?.trim();
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

function viewDisplayPath(view: ViewPackageSummary): string {
  const explicitDisplayPath = view.displayPath?.trim();
  if (explicitDisplayPath) return normalizeViewPath(explicitDisplayPath);
  return physicalPackageRelPath(view);
}

function makeFolderNode(relPath: string, label: string, folder?: ViewFolderSummary): ViewTreeNode {
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

function buildViewTree(views: ViewPackageSummary[], folders: ViewFolderSummary[]): ViewTreeNode[] {
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

  const sortedFolders = [...folders].sort((left, right) =>
    normalizeViewPath(left.relPath).localeCompare(normalizeViewPath(right.relPath), undefined, { sensitivity: "base" }),
  );
  for (const folder of sortedFolders) {
    ensureFolder(folder.relPath, folder);
  }

  const sorted = [...views].sort((left, right) =>
    viewDisplayPath(left).localeCompare(viewDisplayPath(right), undefined, { sensitivity: "base" })
      || left.name.localeCompare(right.name, undefined, { sensitivity: "base" })
      || left.id.localeCompare(right.id, undefined, { sensitivity: "base" }),
  );

  for (const view of sorted) {
    const relPath = viewDisplayPath(view);
    const parts = relPath.split("/").filter(Boolean);
    const dirParts = parts.length > 1 ? parts.slice(0, -1) : [];
    const parent = ensureFolder(dirParts.join("/"));

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

const viewTreeNodes = computed(() => buildViewTree(viewSummaries.value, viewFolders.value));

function isViewNodeExpanded(node: ViewTreeNode): boolean {
  if (node.kind !== "folder") return false;
  const stored = viewExpandedState.value[node.key];
  return stored ?? true;
}

function setViewNodeExpanded(key: string, value: boolean) {
  viewExpandedState.value = { ...viewExpandedState.value, [key]: value };
  persistViewExpandedState();
}

function toggleViewRow(row: VisibleViewRow) {
  if (!row.hasChildren) return;
  setViewNodeExpanded(row.node.key, !row.expanded);
}

function viewTreeIndentPx(depth: number): number {
  return VIEW_TREE_INDENT_BASE_PX + Math.max(0, depth) * VIEW_TREE_INDENT_STEP_PX;
}

const visibleViewEntries = computed<VisibleViewEntry[]>(() => {
  const entries: VisibleViewEntry[] = [];
  if (viewCreateFolderDraft.value?.anchorKey === VIEW_ROOT_ANCHOR_KEY) {
    entries.push({
      type: "create",
      key: `view-create:root`,
      draft: viewCreateFolderDraft.value,
    });
  }
  const walk = (nodes: ViewTreeNode[], depth: number) => {
    for (const node of nodes) {
      const expanded = isViewNodeExpanded(node);
      const hasChildren = node.children.length > 0;
      const row: VisibleViewRow = { node, depth, expanded, hasChildren };
      entries.push({ type: "row", key: node.key, row });
      if (viewCreateFolderDraft.value?.anchorKey === node.key) {
        entries.push({
          type: "create",
          key: `view-create:${node.key}`,
          draft: viewCreateFolderDraft.value,
        });
      }
      if (node.kind === "folder" && hasChildren && expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(viewTreeNodes.value, 0);
  return entries;
});

async function loadViews() {
  if (!showSessionViews.value || !hasWorkspace.value) {
    viewSummaries.value = [];
    viewFolders.value = [];
    return;
  }
  viewsLoading.value = true;
  try {
    const snapshot = await viewTree();
    viewSummaries.value = snapshot.views;
    viewFolders.value = snapshot.folders;
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadSessionPanelViews",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  } finally {
    viewsLoading.value = false;
  }
}

function notifyViewOpenError(error: { code?: string; message: string }) {
  notificationStore.addNotice("error", error.message, {
    code: error.code,
    operation: "openViewFromSessionPanel",
    skipConsoleLog: true,
  });
}

function cachedViewOpenRequirementError(view: ViewPackageSummary) {
  if (!viewRequiresUnityConnection(view)) return null;
  const status = projectStore.unityConnectionStatus;
  return status && !status.connected
    ? viewUnityConnectionRequiredError(view.name)
    : null;
}

async function openView(view: ViewPackageSummary) {
  if (viewOpeningKey.value) return;
  const cachedRequirementError = cachedViewOpenRequirementError(view);
  if (cachedRequirementError) {
    notifyViewOpenError(cachedRequirementError);
    return;
  }

  const key = `${viewDisplayPath(view)}:${view.id}`;
  viewOpeningKey.value = key;
  try {
    const requirementError = await checkViewOpenRequirements(view);
    if (requirementError) {
      notifyViewOpenError(requirementError);
      return;
    }
    await viewRun(view.id);
  } catch (error) {
    const err = normalizeViewError(error, { viewName: view.name });
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openViewFromSessionPanel",
      skipConsoleLog: true,
    });
  } finally {
    viewOpeningKey.value = "";
  }
}

async function openViewInUnity(view: ViewPackageSummary) {
  if (viewOpeningKey.value) return;
  const key = `${viewDisplayPath(view)}:${view.id}:unity`;
  viewOpeningKey.value = key;
  try {
    const requirementError = await checkViewOpenRequirements(view);
    if (requirementError) {
      notifyViewOpenError(requirementError);
      return;
    }
    await viewRunInUnity(view.id);
  } catch (error) {
    const err = normalizeViewError(error, { viewName: view.name });
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openViewInUnityFromSessionPanel",
      skipConsoleLog: true,
    });
  } finally {
    viewOpeningKey.value = "";
  }
}

function onViewRowClick(row: VisibleViewRow, event?: MouseEvent) {
  if (suppressNextViewClick) {
    event?.preventDefault();
    event?.stopPropagation();
    suppressNextViewClick = false;
    return;
  }
  if (row.node.kind === "folder") {
    toggleViewRow(row);
    return;
  }
  if (row.node.view) {
    void openView(row.node.view);
  }
}

function closeViewContextMenu() {
  viewCtxMenu.value = null;
}

function onViewTreeContextMenu(event: MouseEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest(".sp-view-row-shell, .sp-view-create-row")) return;
  event.preventDefault();
  event.stopPropagation();
  viewDeleteConfirm.value = null;
  viewCtxMenu.value = { x: event.clientX, y: event.clientY, kind: "root" };
}

function openViewContextMenu(event: MouseEvent, row: VisibleViewRow) {
  event.preventDefault();
  event.stopPropagation();
  viewDeleteConfirm.value = null;
  viewCtxMenu.value = {
    x: event.clientX,
    y: event.clientY,
    kind: row.node.kind,
    node: row.node,
  };
}

function viewCreateDepthForMenu(menu: ViewContextMenuState): number {
  if (menu.kind === "folder") {
    const row = visibleViewEntries.value.find(
      (entry): entry is Extract<VisibleViewEntry, { type: "row" }> =>
        entry.type === "row" && entry.row.node.key === menu.node.key,
    );
    return (row?.row.depth ?? 0) + 1;
  }
  return 0;
}

async function beginCreateViewFolder() {
  const menu = viewCtxMenu.value;
  if (!menu || menu.kind === "view") return;
  if (menu.kind === "folder" && !isViewNodeExpanded(menu.node)) {
    setViewNodeExpanded(menu.node.key, true);
  }
  viewCreateFolderDraft.value = {
    parentRelPath: menu.kind === "folder" ? menu.node.relPath : "",
    anchorKey: menu.kind === "folder" ? menu.node.key : VIEW_ROOT_ANCHOR_KEY,
    depth: viewCreateDepthForMenu(menu),
    name: "",
  };
  closeViewContextMenu();
  await nextTick();
  viewCreateFolderInputRef.value?.focus();
  viewCreateFolderInputRef.value?.select();
}

function closeViewCreateFolder() {
  viewCreateFolderDraft.value = null;
}

function setViewCreateFolderInputRef(element: Element | ComponentPublicInstance | null) {
  viewCreateFolderInputRef.value = element instanceof HTMLInputElement ? element : null;
}

async function submitViewCreateFolder() {
  const draft = viewCreateFolderDraft.value;
  const name = draft?.name.trim();
  if (!draft || !name) return;
  try {
    await viewCreateFolder({ parentRelPath: draft.parentRelPath, name });
    closeViewCreateFolder();
    await loadViews();
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "createViewFolder",
      skipConsoleLog: true,
    });
  }
}

function requestDeleteViewEntry() {
  const menu = viewCtxMenu.value;
  if (!menu || menu.kind === "root") return;
  viewDeleteConfirm.value = menu.node;
}

function closeViewDeleteConfirm() {
  viewDeleteConfirm.value = null;
  closeViewContextMenu();
}

async function confirmDeleteViewEntry() {
  const node = viewDeleteConfirm.value;
  if (!node) return;
  try {
    const snapshot = await viewDeleteEntry({ relPath: node.relPath });
    viewSummaries.value = snapshot.views;
    viewFolders.value = snapshot.folders;
    closeViewDeleteConfirm();
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "deleteViewEntry",
      skipConsoleLog: true,
    });
  }
}

function clearViewDragState() {
  draggingViewNode.value = null;
  viewDragTargetKey.value = "";
}

function viewNodeParentRelPath(node: ViewTreeNode): string {
  return viewPathDirname(node.relPath);
}

function canDropViewNodeOnDir(node: ViewTreeNode | null, targetDirRelPath: string): boolean {
  if (!node) return false;
  const targetDir = normalizeViewPath(targetDirRelPath);
  if (targetDir === viewNodeParentRelPath(node)) return false;
  if (node.kind === "folder") {
    return targetDir !== node.relPath && !targetDir.startsWith(`${node.relPath}/`);
  }
  return true;
}

function canDragViewNode(node: ViewTreeNode): boolean {
  return !!node.relPath.trim();
}

function visibleViewRowByNodeKey(key: string): VisibleViewRow | null {
  const entry = visibleViewEntries.value.find(
    (item): item is Extract<VisibleViewEntry, { type: "row" }> =>
      item.type === "row" && item.row.node.key === key,
  );
  return entry?.row ?? null;
}

function viewFolderKeyForRelPath(relPath: string): string {
  return `view-dir:${normalizeViewPath(relPath)}`;
}

function clearViewPointerDragListeners() {
  if (viewPointerMoveListener) {
    window.removeEventListener("pointermove", viewPointerMoveListener);
    viewPointerMoveListener = null;
  }
  if (viewPointerUpListener) {
    window.removeEventListener("pointerup", viewPointerUpListener);
    viewPointerUpListener = null;
  }
  if (viewPointerCancelListener) {
    window.removeEventListener("pointercancel", viewPointerCancelListener);
    viewPointerCancelListener = null;
  }
  document.body.classList.remove("sp-view-pointer-dragging");
}

function clearViewPointerDragState() {
  viewPointerDragState = null;
  clearViewPointerDragListeners();
}

function scheduleSuppressNextViewClick() {
  suppressNextViewClick = true;
  if (suppressNextViewClickTimer) {
    window.clearTimeout(suppressNextViewClickTimer);
  }
  suppressNextViewClickTimer = window.setTimeout(() => {
    suppressNextViewClick = false;
    suppressNextViewClickTimer = null;
  }, 240);
}

function shouldIgnoreViewPointerDrag(event: PointerEvent): boolean {
  const target = event.target;
  return (
    target instanceof Element &&
    !!target.closest(
      ".sp-view-branch-slot, .sp-view-create-row, input, textarea, select",
    )
  );
}

function resolveViewDropTargetFromPoint(
  x: number,
  y: number,
  node: ViewTreeNode,
): ViewDropTarget | null {
  const target = document.elementFromPoint(x, y);
  if (!(target instanceof Element)) return null;

  const rowElement = target.closest<HTMLElement>(".sp-view-row-shell");
  if (rowElement) {
    if (rowElement.dataset.viewNodeKind !== "folder") return null;
    const row = visibleViewRowByNodeKey(rowElement.dataset.viewNodeKey ?? "");
    if (!row || row.node.kind !== "folder") return null;
    if (!canDropViewNodeOnDir(node, row.node.relPath)) return null;
    return {
      key: row.node.key,
      relPath: row.node.relPath,
    };
  }

  const listElement = target.closest<HTMLElement>(".sp-view-list");
  if (listElement && canDropViewNodeOnDir(node, "")) {
    return {
      key: VIEW_ROOT_ANCHOR_KEY,
      relPath: "",
    };
  }

  return null;
}

function updateViewPointerDropTarget(event: PointerEvent) {
  const state = viewPointerDragState;
  if (!state?.active) return;
  const target = resolveViewDropTargetFromPoint(event.clientX, event.clientY, state.node);
  viewDragTargetKey.value = target?.key ?? "";
}

function onViewPointerDown(row: VisibleViewRow, event: PointerEvent) {
  if (
    event.button !== 0 ||
    viewCreateFolderDraft.value ||
    !canDragViewNode(row.node) ||
    shouldIgnoreViewPointerDrag(event)
  ) {
    return;
  }

  clearViewPointerDragState();
  viewPointerDragState = {
    node: row.node,
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    active: false,
  };

  viewPointerMoveListener = onViewPointerMove;
  viewPointerUpListener = (upEvent) => {
    void finishViewPointerDrag(upEvent);
  };
  viewPointerCancelListener = () => {
    clearViewDragState();
    clearViewPointerDragState();
  };

  window.addEventListener("pointermove", viewPointerMoveListener);
  window.addEventListener("pointerup", viewPointerUpListener);
  window.addEventListener("pointercancel", viewPointerCancelListener);
}

function onViewPointerMove(event: PointerEvent) {
  const state = viewPointerDragState;
  if (!state || event.pointerId !== state.pointerId) return;

  const dx = event.clientX - state.startX;
  const dy = event.clientY - state.startY;
  if (!state.active) {
    if (Math.hypot(dx, dy) < 5) return;
    state.active = true;
    draggingViewNode.value = state.node;
    viewDragTargetKey.value = "";
    closeViewContextMenu();
    closeViewCreateFolder();
    document.body.classList.add("sp-view-pointer-dragging");
  }

  event.preventDefault();
  updateViewPointerDropTarget(event);
}

async function finishViewPointerDrag(event: PointerEvent) {
  const state = viewPointerDragState;
  if (!state || event.pointerId !== state.pointerId) return;

  const target = state.active
    ? resolveViewDropTargetFromPoint(event.clientX, event.clientY, state.node)
    : null;
  const shouldSuppressClick = state.active;
  clearViewPointerDragState();

  if (shouldSuppressClick) {
    scheduleSuppressNextViewClick();
  }

  if (!target) {
    clearViewDragState();
    return;
  }

  await moveViewNodeToDir(state.node, target.relPath, target.key);
}

function onViewDragStart(row: VisibleViewRow, event: DragEvent) {
  if (viewCreateFolderDraft.value || !canDragViewNode(row.node)) {
    event.preventDefault();
    return;
  }
  closeViewContextMenu();
  draggingViewNode.value = row.node;
  viewDragTargetKey.value = "";
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", row.node.relPath);
  }
}

function onViewDragEnd() {
  clearViewDragState();
}

function onViewFolderDragOver(row: VisibleViewRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingViewNode.value) return;
  if (!canDropViewNodeOnDir(draggingViewNode.value, row.node.relPath)) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  viewDragTargetKey.value = row.node.key;
}

async function moveViewNodeToDir(
  node: ViewTreeNode,
  targetDirRelPath: string,
  targetKey = targetDirRelPath ? viewFolderKeyForRelPath(targetDirRelPath) : VIEW_ROOT_ANCHOR_KEY,
) {
  if (!canDropViewNodeOnDir(node, targetDirRelPath)) {
    clearViewDragState();
    return;
  }
  clearViewDragState();
  if (targetKey !== VIEW_ROOT_ANCHOR_KEY) {
    setViewNodeExpanded(targetKey, true);
  }
  try {
    const snapshot = await viewMoveEntry({
      sourceRelPath: node.relPath,
      targetDirRelPath,
    });
    viewSummaries.value = snapshot.views;
    viewFolders.value = snapshot.folders;
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "moveViewEntry",
      skipConsoleLog: true,
    });
  }
}

async function moveDraggingViewNode(targetDirRelPath: string, targetKey?: string) {
  const node = draggingViewNode.value;
  if (!node) {
    clearViewDragState();
    return;
  }
  await moveViewNodeToDir(node, targetDirRelPath, targetKey);
}

function onViewFolderDrop(row: VisibleViewRow, event: DragEvent) {
  if (row.node.kind !== "folder" || !draggingViewNode.value) return;
  if (!canDropViewNodeOnDir(draggingViewNode.value, row.node.relPath)) {
    clearViewDragState();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  void moveDraggingViewNode(row.node.relPath, row.node.key);
}

function onViewRootDragOver(event: DragEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest(".sp-view-row-shell, .sp-view-create-row")) return;
  if (!canDropViewNodeOnDir(draggingViewNode.value, "")) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  viewDragTargetKey.value = VIEW_ROOT_ANCHOR_KEY;
}

function onViewRootDrop(event: DragEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest(".sp-view-row-shell, .sp-view-create-row")) return;
  if (!canDropViewNodeOnDir(draggingViewNode.value, "")) {
    clearViewDragState();
    return;
  }
  event.preventDefault();
  void moveDraggingViewNode("", VIEW_ROOT_ANCHOR_KEY);
}

function clearViewResizeListeners() {
  if (viewResizeMoveListener) {
    window.removeEventListener("mousemove", viewResizeMoveListener);
    viewResizeMoveListener = null;
  }
  if (viewResizeUpListener) {
    window.removeEventListener("mouseup", viewResizeUpListener);
    viewResizeUpListener = null;
  }
  document.body.classList.remove("sp-view-resizing");
}

function applyViewSplit(clientY: number) {
  const panel = sessionPanelRef.value;
  if (!panel) return;
  const rect = panel.getBoundingClientRect();
  const header = panel.querySelector<HTMLElement>(".sp-header");
  const headerHeight = header?.getBoundingClientRect().height ?? 0;
  const availableHeight = Math.max(1, rect.height - headerHeight - VIEW_RESIZE_HANDLE_PX);
  const viewHeight = rect.bottom - clientY - VIEW_RESIZE_HANDLE_PX / 2;
  viewSectionRatio.value = clampViewSplitRatio(viewHeight / availableHeight);
  persistViewSplitRatio();
}

async function openViewContextInUnity() {
  const menu = viewCtxMenu.value;
  if (!menu || menu.kind !== "view" || !menu.node.view) return;
  const view = menu.node.view;
  closeViewContextMenu();
  await openViewInUnity(view);
}

function viewContextLocation(menu: ViewContextMenuState | null): string {
  if (!menu || menu.kind !== "view") return "";
  return menu.node.view?.packageRoot || "";
}

async function revealViewContextLocation() {
  const targetPath = viewContextLocation(viewCtxMenu.value);
  if (!targetPath) return;
  closeViewContextMenu();
  try {
    await projectStore.openDirInFileExplorer(targetPath);
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "revealViewContextLocation",
      skipConsoleLog: true,
    });
  }
}

function onViewResizeMouseDown(event: MouseEvent) {
  if (!showSessionViews.value) return;
  event.preventDefault();
  applyViewSplit(event.clientY);
  clearViewResizeListeners();

  viewResizeMoveListener = (moveEvent: MouseEvent) => {
    moveEvent.preventDefault();
    applyViewSplit(moveEvent.clientY);
  };
  viewResizeUpListener = () => clearViewResizeListeners();

  window.addEventListener("mousemove", viewResizeMoveListener);
  window.addEventListener("mouseup", viewResizeUpListener);
  document.body.classList.add("sp-view-resizing");
}

function closeViewHelp() {
  viewHelpOpen.value = false;
}

function openViewHelp() {
  viewHelpOpen.value = true;
  void nextTick(() => viewHelpDialogRef.value?.focus());
}

function toggleViewHelp() {
  if (viewHelpOpen.value) {
    closeViewHelp();
  } else {
    openViewHelp();
  }
}

function onViewHelpKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    closeViewHelp();
  }
}

watch(
  () => [props.workingDir, showSessionViews.value] as const,
  () => {
    void loadViews();
  },
);

onMounted(async () => {
  viewReloadUnsubscribe = await getLocusRuntime().subscribe<ViewPackageSummary>(
    "view-package-reloaded",
    () => {
      void loadViews();
    },
  );
  viewTreeChangedUnsubscribe = await getLocusRuntime().subscribe(
    "view-tree-changed",
    () => {
      void loadViews();
    },
  );
  void loadViews();
});

onUnmounted(() => {
  viewReloadUnsubscribe?.();
  viewReloadUnsubscribe = null;
  viewTreeChangedUnsubscribe?.();
  viewTreeChangedUnsubscribe = null;
  clearViewResizeListeners();
  clearViewPointerDragState();
  if (suppressNextViewClickTimer) {
    window.clearTimeout(suppressNextViewClickTimer);
    suppressNextViewClickTimer = null;
  }
});

function isNodeExpanded(node: SessionTreeNode): boolean {
  const stored = expandedState.value[node.key];
  if (stored !== undefined) return stored;
  // Default: auto-expand if contains active session
  if (nodeContainsSession(node, props.activeSessionId) || nodeHasActiveDescendant(node)) {
    return true;
  }
  return false;
}

function setNodeExpanded(key: string, value: boolean) {
  expandedState.value = { ...expandedState.value, [key]: value };
  persistExpandedState();
}

function toggleNode(row: VisibleTreeRow) {
  setNodeExpanded(row.node.key, !row.expanded);
}

const visibleRows = computed<VisibleTreeRow[]>(() => {
  const rows: VisibleTreeRow[] = [];
  const walk = (nodes: SessionTreeNode[], depth: number) => {
    for (const node of nodes) {
      const expanded = isNodeExpanded(node);
      const hasChildren = node.children.length > 0;
      rows.push({ node, depth, expanded, hasChildren });
      if (hasChildren && expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(sessionTree.value, 0);
  return rows;
});

function formatSessionTime(ts: number): string {
  const nowTs = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, nowTs - ts);

  if (diff < 60) return t("common.timeJustNow");

  const units: Array<[number, string]> = [
    [60, "chat.session.time.minute"],
    [60 * 60, "chat.session.time.hour"],
    [60 * 60 * 24, "chat.session.time.day"],
    [60 * 60 * 24 * 7, "chat.session.time.week"],
    [60 * 60 * 24 * 30, "chat.session.time.month"],
    [60 * 60 * 24 * 365, "chat.session.time.year"],
  ];

  for (let i = units.length - 1; i >= 0; i--) {
    const [seconds, key] = units[i];
    if (diff >= seconds) {
      return t(key, Math.floor(diff / seconds));
    }
  }

  return t("common.timeJustNow");
}

function rowLabel(node: SessionTreeNode): string {
  if (node.kind === "folder") return node.label;
  return node.title || t("chat.session.newSession");
}

function isSubagentNode(node: SessionTreeNode): boolean {
  return node.kind === "session" && node.sessionType === "chat" && !!node.parentSessionId;
}

function isDevNode(node: SessionTreeNode): boolean {
  return node.kind === "session" && node.sessionType === "chat" && !node.parentSessionId;
}

function rowRoleClass(node: SessionTreeNode): string {
  if (node.kind === "folder") return "role-folder";
  if (isSubagentNode(node)) return "role-subagent";
  if (isDevNode(node)) return "role-dev";
  return `role-${node.sessionType}`;
}

function sessionStatusLabel(status: SessionTreeNode["status"]): string {
  if (!status) return "";
  return t(`chat.session.status.${status}`);
}

/* Multi-selection state (Ctrl/Cmd toggle, Shift range) */
const selectedIds = ref<Set<string>>(new Set());
const lastAnchorId = ref<string | null>(null);

function clearMultiSelection() {
  if (selectedIds.value.size > 0) {
    selectedIds.value = new Set();
  }
}

type SessionVisibleTreeRow = VisibleTreeRow & { node: SessionTreeSessionNode };

function isSelectableSessionRow(row: VisibleTreeRow): row is SessionVisibleTreeRow {
  return row.node.kind === "session" && row.node.selectable && !!row.node.sessionId;
}

function selectableSessionRows(): SessionVisibleTreeRow[] {
  return visibleRows.value.filter(isSelectableSessionRow);
}

function onRowClick(row: VisibleTreeRow, e: MouseEvent) {
  if (row.node.kind === "folder") {
    if (row.hasChildren) toggleNode(row);
    return;
  }
  if (!row.node.selectable || !row.node.sessionId) return;
  const id = row.node.sessionId;

  if (e.ctrlKey || e.metaKey) {
    const next = new Set(selectedIds.value);
    if (next.has(id)) {
      next.delete(id);
    } else {
      // When starting a multi-selection from single-selected state,
      // seed the set with the currently active session so it feels natural.
      if (next.size === 0 && props.activeSessionId) {
        next.add(props.activeSessionId);
      }
      next.add(id);
    }
    selectedIds.value = next;
    lastAnchorId.value = id;
    return;
  }

  if (e.shiftKey && lastAnchorId.value) {
    const rows = selectableSessionRows();
    const anchorIdx = rows.findIndex((r) => r.node.sessionId === lastAnchorId.value);
    const currIdx = rows.findIndex((r) => r.node.sessionId === id);
    if (anchorIdx >= 0 && currIdx >= 0) {
      const [lo, hi] = anchorIdx <= currIdx ? [anchorIdx, currIdx] : [currIdx, anchorIdx];
      const next = new Set<string>();
      for (let i = lo; i <= hi; i++) {
        const sid = rows[i].node.sessionId;
        if (sid) next.add(sid);
      }
      selectedIds.value = next;
      return;
    }
  }

  // Plain click — reset multi-selection and activate this session.
  clearMultiSelection();
  lastAnchorId.value = id;
  emit("selectSession", id);
}

/* Context menu */
const ctxMenu = ref<{
  x: number;
  y: number;
  session: SessionSummary;
  ids: string[]; // targets — may include the single right-clicked session or the whole selection
} | null>(null);

const DELETE_CONFIRM_WIDTH = 244;
const DELETE_CONFIRM_HEIGHT = 136;
const DELETE_CONFIRM_GAP = 8;

const deleteConfirm = ref<{
  x: number;
  y: number;
  ids: string[];
} | null>(null);

function onContextMenu(e: MouseEvent, session: SessionSummary) {
  e.preventDefault();
  e.stopPropagation();
  deleteConfirm.value = null;
  let ids: string[];
  if (selectedIds.value.size > 1 && selectedIds.value.has(session.id)) {
    // Right-click inside an existing multi-selection → act on the whole set.
    ids = Array.from(selectedIds.value);
  } else {
    // Right-click outside selection → reset and target this one session.
    clearMultiSelection();
    ids = [session.id];
  }
  ctxMenu.value = { x: e.clientX, y: e.clientY, session, ids };
}

function closeCtxMenu() {
  ctxMenu.value = null;
  deleteConfirm.value = null;
}

/* Inline rename */
const editingId = ref<string | null>(null);
const editingTitle = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

function startRename(session: SessionSummary) {
  closeCtxMenu();
  editingId.value = session.id;
  editingTitle.value = session.title || "";
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

function commitRename() {
  if (editingId.value && editingTitle.value.trim()) {
    emit("renameSession", editingId.value, editingTitle.value.trim());
  }
  editingId.value = null;
  editingTitle.value = "";
}

function cancelRename() {
  editingId.value = null;
  editingTitle.value = "";
}

function performArchive(ids: string[]) {
  if (ids.length === 0) return;
  for (const id of ids) {
    emit("archiveSession", id);
  }
  clearMultiSelection();
  closeCtxMenu();
}

function requestArchive(ids: string[]) {
  performArchive(ids);
}

function performDelete(ids: string[]) {
  if (ids.length === 0) return;
  for (const id of ids) {
    emit("deleteSession", id);
  }
  clearMultiSelection();
  closeCtxMenu();
}

function deleteConfirmLabel(ids: string[]): string {
  return ids.length > 1
    ? t("chat.session.deleteMany", ids.length)
    : t("chat.session.delete");
}

function deleteConfirmMessage(ids: string[]): string {
  return ids.length > 1
    ? t("chat.session.deleteManyConfirm", ids.length)
    : t("chat.session.deleteConfirm");
}

function requestDelete(e: MouseEvent) {
  if (!ctxMenu.value) return;
  const anchor = e.currentTarget as HTMLElement | null;
  if (!anchor) return;
  const rect = anchor.getBoundingClientRect();
  const margin = 12;

  let x = rect.right + DELETE_CONFIRM_GAP;
  if (x + DELETE_CONFIRM_WIDTH > window.innerWidth - margin) {
    x = Math.max(margin, rect.left - DELETE_CONFIRM_WIDTH - DELETE_CONFIRM_GAP);
  }

  const maxY = Math.max(margin, window.innerHeight - DELETE_CONFIRM_HEIGHT - margin);
  const y = Math.min(Math.max(margin, rect.top - 10), maxY);

  deleteConfirm.value = {
    x,
    y,
    ids: [...ctxMenu.value.ids],
  };
}

function confirmDelete() {
  if (!deleteConfirm.value) return;
  performDelete(deleteConfirm.value.ids);
}

function ctxSaveContext(includeSystemPrompt: boolean) {
  if (ctxMenu.value) {
    emit("saveRawContext", {
      sessionId: ctxMenu.value.session.id,
      includeSystemPrompt,
    });
  }
  closeCtxMenu();
}

async function ctxOpenSessionInUnity() {
  const menu = ctxMenu.value;
  if (!menu || menu.ids.length !== 1) return;
  const session = menu.session;
  closeCtxMenu();
  try {
    await openUnityEmbeddedSessionWindow({
      sessionId: session.id,
      title: session.title || session.id,
    });
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openSessionInUnity",
      skipConsoleLog: true,
    });
  }
}

function ctxArchive() {
  if (ctxMenu.value) performArchive(ctxMenu.value.ids);
}
</script>

<template>
  <div
    ref="sessionPanelRef"
    class="session-panel"
    :style="{ width: sessionPanelWidth + 'px', minWidth: sessionPanelWidth + 'px', '--session-panel-width': sessionPanelWidth + 'px' }"
  >
    <div class="sp-header">
      <span class="sp-title">{{ t('chat.session.title') }}</span>
      <div class="sp-header-actions">
        <button
          type="button"
          class="sp-collapse-btn"
          :title="t('chat.session.collapseList')"
          :aria-label="t('chat.session.collapseList')"
          @click="emit('togglePanelCollapsed')"
        >
          <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor" aria-hidden="true">
            <path d="M7.78 12.53a.75.75 0 0 1-1.06 0L2.47 8.28a.75.75 0 0 1 0-1.06l4.25-4.25a.75.75 0 0 1 1.06 1.06L4.81 7h7.44a.75.75 0 0 1 0 1.5H4.81l2.97 2.97a.75.75 0 0 1 0 1.06z"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="sp-session-list" :style="sessionListStyle">
      <button
        type="button"
        class="sp-session-item sp-new-session-item"
        :class="{ active: activeSessionId === null }"
        :title="newChatTitle"
        @click="emit('newChat')"
      >
        <span class="sp-expand-spacer">
          <span class="sp-new-session-plus" aria-hidden="true">+</span>
        </span>
        <div class="sp-session-info">
          <div class="sp-session-main">
            <span class="sp-session-title">{{ t('chat.session.createNew') }}</span>
          </div>
        </div>
      </button>
      <div
        v-for="row in visibleRows"
        :key="row.node.key"
        class="sp-session-item sp-tree-row"
        :class="[
          rowRoleClass(row.node),
          {
            active: row.node.kind === 'session' && !!row.node.sessionId && (row.node.sessionId === activeSessionId || selectedIds.has(row.node.sessionId) || (ctxMenu && ctxMenu.ids.includes(row.node.sessionId))),
            streaming: row.node.status === 'running' || row.node.status === 'finishing',
            folder: row.node.kind === 'folder',
            child: row.depth > 0,
            virtual: row.node.kind === 'session' && row.node.isVirtual,
            disabled: row.node.kind === 'session' && !row.node.selectable,
            expandable: row.hasChildren,
          },
        ]"
        :style="{ paddingLeft: `${6 + row.depth * 12}px` }"
        @click="onRowClick(row, $event)"
        @contextmenu="row.node.kind === 'session' && row.node.session ? onContextMenu($event, row.node.session) : undefined"
      >
        <button
          v-if="row.hasChildren"
          class="sp-expand-btn"
          :class="{
            open: row.expanded,
            'is-running': row.node.status === 'running' || row.node.status === 'finishing',
          }"
          @click.stop="toggleNode(row)"
          :title="row.expanded ? t('chat.session.collapse') : t('chat.session.expand')"
        >
          <svg viewBox="0 0 12 12" width="10" height="10" fill="currentColor" aria-hidden="true">
            <path d="M4 2.5 8 6 4 9.5z" />
          </svg>
        </button>
        <span
          v-else
          class="sp-expand-spacer"
          :title="row.node.status ? sessionStatusLabel(row.node.status) : undefined"
        >
          <span
            class="sp-session-dot"
            :class="row.node.status ? `is-${row.node.status}` : ''"
            aria-hidden="true"
          ></span>
        </span>

        <div class="sp-session-info">
          <template v-if="row.node.kind === 'session' && editingId === row.node.sessionId">
            <input
              ref="renameInput"
              class="sp-rename-input"
              v-model="editingTitle"
              @click.stop
              @keydown.enter="commitRename"
              @keydown.escape="cancelRename"
              @blur="commitRename"
            />
          </template>
          <template v-else>
            <div class="sp-session-main">
              <span class="sp-session-title">{{ rowLabel(row.node) }}</span>
              <div class="sp-session-meta">
                <span
                  v-if="row.node.status && row.node.status !== 'running'"
                  class="sp-session-status"
                  :class="`is-${row.node.status}`"
                >
                  {{ sessionStatusLabel(row.node.status) }}
                </span>
                <span class="sp-session-time">{{ formatSessionTime(row.node.updatedAt) }}</span>
                <button
                  v-if="row.node.kind === 'session' && row.node.sessionId"
                  class="sp-row-archive-btn"
                  :title="t('chat.session.archive')"
                  @click.stop="requestArchive([row.node.sessionId])"
                >
                  <svg viewBox="0 0 16 16" width="13" height="13" fill="none" aria-hidden="true">
                    <path d="M3.75 4.5h8.5m-7.75 0v5.7c0 .43.35.8.78.8h5.44c.43 0 .78-.37.78-.8V4.5m-5.82 3.1h4.68M6 2.75h4c.28 0 .5.22.5.5v1.25h-5V3.25c0-.28.22-.5.5-.5Z" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </button>
              </div>
            </div>
          </template>
        </div>
      </div>
      <div v-if="visibleRows.length === 0" class="sp-empty-hint">{{ t('chat.session.noSessions') }}</div>
    </div>

    <div
      v-if="showSessionViews"
      class="sp-view-resize"
      role="separator"
      aria-orientation="horizontal"
      @mousedown="onViewResizeMouseDown"
    ></div>

    <section
      v-if="showSessionViews"
      class="sp-view-section"
      :style="viewSectionStyle"
      :aria-label="t('view.list.title')"
    >
      <div class="sp-view-header">
        <span class="sp-view-title">{{ t('view.list.title') }}</span>
        <div class="sp-view-help-wrap">
          <button
            type="button"
            class="sp-view-help-btn"
            :title="t('view.list.helpLabel')"
            :aria-label="t('view.list.helpLabel')"
            :aria-expanded="viewHelpOpen"
            aria-controls="session-view-help-dialog"
            @click.stop="toggleViewHelp"
            @keydown="onViewHelpKeydown"
          >
            <LucideIcon :icon="HelpCircle" :size="14" :stroke-width="2" />
          </button>
        </div>
      </div>

      <div
        class="sp-view-list"
        :class="{ 'is-root-drop-target': viewDragTargetKey === VIEW_ROOT_ANCHOR_KEY }"
        @contextmenu.prevent="onViewTreeContextMenu"
        @dragover="onViewRootDragOver"
        @drop="onViewRootDrop"
      >
        <template v-for="entry in visibleViewEntries" :key="entry.key">
          <div
            v-if="entry.type === 'row'"
            class="sp-view-row-shell"
            :class="{
              folder: entry.row.node.kind === 'folder',
              open: entry.row.expanded,
              dragging: draggingViewNode?.key === entry.row.node.key,
              'drop-target': viewDragTargetKey === entry.row.node.key,
              opening: entry.row.node.kind === 'view' && entry.row.node.view && viewOpeningKey === `${viewDisplayPath(entry.row.node.view)}:${entry.row.node.view.id}`,
            }"
            draggable="false"
            :data-view-node-key="entry.row.node.key"
            :data-view-node-kind="entry.row.node.kind"
            :title="entry.row.node.view?.packageRoot || entry.row.node.folder?.packageRoot || entry.row.node.label"
            @pointerdown="onViewPointerDown(entry.row, $event)"
            @contextmenu.prevent.stop="openViewContextMenu($event, entry.row)"
            @dragstart="onViewDragStart(entry.row, $event)"
            @dragend="onViewDragEnd"
            @dragover="onViewFolderDragOver(entry.row, $event)"
            @drop="onViewFolderDrop(entry.row, $event)"
          >
            <button
              type="button"
              class="sp-view-row"
              :style="{ paddingLeft: `${viewTreeIndentPx(entry.row.depth)}px` }"
              :disabled="!!viewOpeningKey && entry.row.node.kind === 'view'"
              @click="onViewRowClick(entry.row, $event)"
            >
              <span
                v-if="entry.row.node.kind === 'folder' && entry.row.hasChildren"
                class="sp-view-branch-slot"
                @click.stop="toggleViewRow(entry.row)"
              >
                <LucideIcon
                  class="sp-view-chevron"
                  :class="{ open: entry.row.expanded }"
                  :icon="ChevronRight"
                  :size="10"
                  :stroke-width="2.4"
                />
              </span>
              <span class="sp-view-branch-spacer" v-else aria-hidden="true"></span>
              <span
                v-if="entry.row.node.kind === 'folder'"
                class="sp-view-kind-icon folder"
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
                v-else
                class="sp-view-kind-icon view"
                aria-hidden="true"
              >
                <LucideIcon
                  :icon="resolveLocusViewIcon(entry.row.node.view?.icon)"
                  :size="13"
                  :stroke-width="2"
                />
              </span>
              <span class="sp-view-label">{{ entry.row.node.label }}</span>
            </button>
          </div>

          <div
            v-else
            class="sp-view-create-row"
            :style="{ paddingLeft: `${viewTreeIndentPx(entry.draft.depth)}px` }"
          >
            <span class="sp-view-bullet" aria-hidden="true"></span>
            <div class="sp-view-create-body">
              <input
                :ref="setViewCreateFolderInputRef"
                v-model="entry.draft.name"
                class="sp-view-create-input"
                :placeholder="t('view.tree.folderNamePlaceholder')"
                :aria-label="t('view.tree.createFolder')"
                @click.stop
                @keydown.enter.prevent="submitViewCreateFolder"
                @keydown.esc.prevent="closeViewCreateFolder"
              />
              <div class="sp-view-create-actions">
                <BaseButton
                  class="sp-view-create-action"
                  type="button"
                  :title="t('common.confirm')"
                  :disabled="!entry.draft.name.trim()"
                  @pointerdown.prevent
                  @click="submitViewCreateFolder"
                >
                  <LucideIcon :icon="Check" :size="12" :stroke-width="2.4" />
                </BaseButton>
                <BaseButton
                  class="sp-view-create-action"
                  type="button"
                  :title="t('common.cancel')"
                  @pointerdown.prevent
                  @click="closeViewCreateFolder"
                >
                  <LucideIcon :icon="X" :size="12" :stroke-width="2.4" />
                </BaseButton>
              </div>
            </div>
          </div>
        </template>

        <div v-if="viewsLoading" class="sp-view-empty">{{ t('common.loading') }}</div>
        <div v-else-if="!viewTreeNodes.length && !viewCreateFolderDraft" class="sp-view-empty">{{ t('view.list.empty') }}</div>
      </div>
    </section>

    <Teleport to="body">
      <Transition name="sp-view-help-modal">
        <div
          v-if="viewHelpOpen"
          class="sp-view-help-overlay"
          @mousedown.self="closeViewHelp"
        >
          <section
            id="session-view-help-dialog"
            ref="viewHelpDialogRef"
            class="sp-view-help-dialog"
            role="dialog"
            aria-modal="true"
            :aria-labelledby="'session-view-help-title'"
            tabindex="-1"
            @keydown.esc.stop="closeViewHelp"
          >
            <header class="sp-view-help-header">
              <div class="sp-view-help-header-copy">
                <h2 id="session-view-help-title" class="sp-view-help-title">{{ t('view.list.helpLabel') }}</h2>
              </div>
              <button
                type="button"
                class="sp-view-help-close"
                :title="t('common.close')"
                :aria-label="t('common.close')"
                @click="closeViewHelp"
              >
                <LucideIcon :icon="X" :size="14" :stroke-width="2" />
              </button>
            </header>
            <div class="sp-view-help-body">
              <section class="sp-view-help-section">
                <div class="sp-view-help-section-title">{{ t('view.list.helpFeatureTitle') }}</div>
                <p>{{ t('view.list.helpBody') }}</p>
              </section>
              <section class="sp-view-help-section">
                <div class="sp-view-help-section-title">{{ t('view.list.helpCreateTitle') }}</div>
                <p>{{ t('view.list.helpCreate') }}</p>
              </section>
              <section class="sp-view-help-section">
                <div class="sp-view-help-section-title">{{ t('view.list.helpSettingsTitle') }}</div>
                <p>{{ t('view.list.helpSettings') }}</p>
              </section>
            </div>
            <footer class="sp-view-help-footer">
              <BaseButton size="md" @click="closeViewHelp">
                {{ t('common.close') }}
              </BaseButton>
            </footer>
          </section>
        </div>
      </Transition>
    </Teleport>

    <BaseContextMenu
      v-if="viewCtxMenu"
      class="sp-ctx-menu"
      :x="viewCtxMenu.x"
      :y="viewCtxMenu.y"
      :min-width="120"
      @close="closeViewContextMenu"
    >
      <template v-if="viewCtxMenu.kind === 'root' || viewCtxMenu.kind === 'folder'">
        <button type="button" class="sp-ctx-item" @click="beginCreateViewFolder">{{ t('view.tree.createFolder') }}</button>
        <div v-if="viewCtxMenu.kind === 'folder'" class="sp-ctx-sep"></div>
      </template>
      <button
        v-if="viewCtxMenu.kind === 'view'"
        type="button"
        class="sp-ctx-item"
        @click.stop="openViewContextInUnity"
      >
        {{ t('view.action.openInUnity') }}
      </button>
      <button
        v-if="viewCtxMenu.kind === 'view'"
        type="button"
        class="sp-ctx-item"
        @click.stop="revealViewContextLocation"
      >
        {{ t('view.action.reveal') }}
      </button>
      <div v-if="viewCtxMenu.kind === 'folder' || viewCtxMenu.kind === 'view'" class="sp-ctx-sep"></div>
      <button
        v-if="viewCtxMenu.kind === 'folder' || viewCtxMenu.kind === 'view'"
        type="button"
        class="sp-ctx-item danger"
        @click.stop="requestDeleteViewEntry"
      >
        {{ t('view.tree.delete') }}
      </button>
    </BaseContextMenu>

    <Teleport to="body">
        <div
          v-if="viewCtxMenu && viewDeleteConfirm"
          class="sp-delete-confirm"
          :style="{ left: viewCtxMenu.x + 'px', top: viewCtxMenu.y + 'px' }"
          @click.stop
        >
          <div class="sp-delete-confirm-title">{{ t('view.tree.deleteConfirmTitle') }}</div>
          <div class="sp-delete-confirm-text">{{ t('view.tree.deleteConfirmMessage', viewDeleteConfirm.label) }}</div>
          <div class="sp-delete-confirm-actions">
            <BaseButton class="sp-delete-confirm-btn" @click="closeViewDeleteConfirm">
              {{ t('common.cancel') }}
            </BaseButton>
            <BaseButton class="sp-delete-confirm-btn" variant="danger" @click="confirmDeleteViewEntry">
              {{ t('common.confirm') }}
            </BaseButton>
          </div>
        </div>
    </Teleport>

    <BaseContextMenu
      v-if="ctxMenu"
      class="sp-ctx-menu"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :min-width="120"
      @close="closeCtxMenu"
    >
      <template v-if="ctxMenu.ids.length <= 1">
        <button type="button" class="sp-ctx-item" @click="startRename(ctxMenu!.session)">{{ t('chat.session.rename') }}</button>
        <button type="button" class="sp-ctx-item" @click="ctxOpenSessionInUnity">{{ t('chat.session.openInUnity') }}</button>
        <button type="button" class="sp-ctx-item" @click="ctxSaveContext(true)">{{ t('chat.saveContextWithSystemPrompt') }}</button>
        <button type="button" class="sp-ctx-item" @click="ctxSaveContext(false)">{{ t('chat.saveContextWithoutSystemPrompt') }}</button>
        <div class="sp-ctx-sep"></div>
        <button type="button" class="sp-ctx-item" @click="ctxArchive">{{ t('chat.session.archive') }}</button>
        <button type="button" class="sp-ctx-item danger" @click.stop="requestDelete">{{ t('chat.session.delete') }}</button>
      </template>
      <template v-else>
        <button type="button" class="sp-ctx-item" @click="ctxArchive">{{ t('chat.session.archiveMany', ctxMenu.ids.length) }}</button>
        <button type="button" class="sp-ctx-item danger" @click.stop="requestDelete">{{ t('chat.session.deleteMany', ctxMenu.ids.length) }}</button>
      </template>
    </BaseContextMenu>

    <Teleport to="body">
        <div
          v-if="deleteConfirm"
          class="sp-delete-confirm"
          :style="{ left: deleteConfirm.x + 'px', top: deleteConfirm.y + 'px' }"
          @click.stop
        >
          <div class="sp-delete-confirm-title">{{ deleteConfirmLabel(deleteConfirm.ids) }}</div>
          <div class="sp-delete-confirm-text">{{ deleteConfirmMessage(deleteConfirm.ids) }}</div>
          <div class="sp-delete-confirm-actions">
            <BaseButton class="sp-delete-confirm-btn" @click="deleteConfirm = null">
              {{ t('common.cancel') }}
            </BaseButton>
            <BaseButton class="sp-delete-confirm-btn" variant="danger" @click="confirmDelete">
              {{ t('common.confirm') }}
            </BaseButton>
          </div>
        </div>
    </Teleport>
  </div>
</template>

<style scoped>
.session-panel {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.sp-session-list {
  flex: 1 1 66.667%;
  min-height: 96px;
  height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding-bottom: 10px;
  scroll-padding-bottom: 10px;
}

:global(body.sp-view-resizing) {
  cursor: row-resize;
  user-select: none;
}

:global(body.sp-view-pointer-dragging) {
  cursor: grabbing;
  user-select: none;
}

:global(body.sp-view-pointer-dragging *) {
  cursor: grabbing !important;
}

.sp-view-resize {
  position: relative;
  flex: 0 0 10px;
  background: color-mix(in srgb, var(--sidebar-bg) 94%, var(--panel-bg) 6%);
  cursor: row-resize;
}

.sp-view-resize::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: 4px;
  height: 1px;
  background: var(--border-color);
  transition: background 0.12s ease;
}

.sp-view-resize:hover::before {
  background: var(--border-strong);
}

.sp-view-section {
  flex: 0 0 33.333%;
  min-height: 140px;
  display: flex;
  flex-direction: column;
  position: relative;
  background: color-mix(in srgb, var(--sidebar-bg) 94%, var(--panel-bg) 6%);
  overflow: hidden;
}

.sp-view-header {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 34px;
  padding: 6px 8px 4px 12px;
}

.sp-view-title {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.5px;
  text-transform: uppercase;
}

.sp-view-help-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
}

.sp-view-help-btn {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  padding: 0;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
}

.sp-view-help-btn:hover,
.sp-view-help-btn[aria-expanded="true"] {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-view-help-btn:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.sp-view-help-overlay {
  position: fixed;
  inset: 0;
  z-index: 10001;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(8, 10, 14, 0.34);
}

.sp-view-help-dialog {
  width: min(620px, 100%);
  max-height: min(680px, calc(100vh - 40px));
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: var(--surface-elevated);
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
}

.sp-view-help-dialog:focus {
  outline: none;
}

.sp-view-help-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px 14px;
}

.sp-view-help-header-copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.sp-view-help-title {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
  line-height: 1.3;
  color: var(--text-color);
}

.sp-view-help-close {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.sp-view-help-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-view-help-close:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.sp-view-help-body {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 0 20px 18px;
  overflow: auto;
}

.sp-view-help-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.sp-view-help-section + .sp-view-help-section {
  padding-top: 16px;
  border-top: 1px solid var(--border-color);
}

.sp-view-help-section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.sp-view-help-section p {
  margin: 0;
  font-size: 13px;
  line-height: 1.65;
  color: var(--text-secondary);
}

.sp-view-help-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 14px 20px 18px;
  border-top: 1px solid var(--border-color);
}

.sp-view-help-modal-enter-active,
.sp-view-help-modal-leave-active {
  transition: opacity 0.15s ease;
}

.sp-view-help-modal-enter-active .sp-view-help-dialog,
.sp-view-help-modal-leave-active .sp-view-help-dialog {
  transition: transform 0.15s ease, opacity 0.15s ease;
}

.sp-view-help-modal-enter-from,
.sp-view-help-modal-leave-to {
  opacity: 0;
}

.sp-view-help-modal-enter-from .sp-view-help-dialog,
.sp-view-help-modal-leave-to .sp-view-help-dialog {
  opacity: 0;
  transform: scale(0.96) translateY(8px);
}

@media (max-width: 720px) {
  .sp-view-help-dialog {
    max-height: min(720px, calc(100vh - 24px));
  }

  .sp-view-help-overlay {
    padding: 12px;
  }
}

.sp-view-list {
  flex: 1 1 0;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 0 6px 8px;
  transition: background 0.12s ease;
}

.sp-view-list.is-root-drop-target {
  background: color-mix(in srgb, var(--accent-color) 6%, transparent);
}

.sp-view-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  width: 100%;
  min-width: 0;
  background: transparent;
  transition: background 0.1s ease, box-shadow 0.1s ease, opacity 0.1s ease;
}

.sp-view-row-shell:hover {
  background: var(--hover-bg);
}

.sp-view-row-shell.opening,
.sp-view-row-shell.opening:hover {
  background: color-mix(in srgb, var(--active-bg) 74%, var(--sidebar-bg) 26%);
}

.sp-view-row-shell.dragging {
  opacity: 0.48;
}

.sp-view-row-shell.drop-target,
.sp-view-row-shell.drop-target:hover {
  background: color-mix(in srgb, var(--active-bg) 62%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 32%, var(--border-color));
}

.sp-view-row {
  width: 100%;
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

.sp-view-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.sp-view-row:disabled {
  cursor: progress;
  opacity: 0.76;
}

.sp-view-branch-slot,
.sp-view-branch-spacer,
.sp-view-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.sp-view-branch-slot {
  border-radius: 4px;
  cursor: pointer;
}

.sp-view-branch-slot:hover {
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
}

.sp-view-chevron {
  opacity: 0.58;
  transition: transform 0.15s ease, opacity 0.12s ease;
}

.sp-view-row-shell:hover .sp-view-chevron {
  opacity: 0.9;
}

.sp-view-chevron.open {
  transform: rotate(90deg);
}

.sp-view-kind-icon {
  transition: color 0.15s ease;
}

.sp-view-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.sp-view-kind-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
}

.sp-view-kind-icon.view {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.sp-view-row-shell.opening .sp-view-kind-icon.view,
.sp-view-row-shell:hover .sp-view-kind-icon.view {
  color: var(--accent-color);
}

.sp-view-label {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: currentColor;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.35;
  transition: color 0.12s ease;
}

.sp-view-row-shell.folder .sp-view-label {
  font-weight: 600;
}

.sp-view-create-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  padding: 2px 12px 2px 16px;
  background: color-mix(in srgb, var(--active-bg) 78%, transparent);
}

.sp-view-bullet {
  width: 10px;
  height: 10px;
  position: relative;
  flex-shrink: 0;
}

.sp-view-bullet::before {
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

.sp-view-create-body {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 6px;
}

.sp-view-create-input {
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

.sp-view-create-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.sp-view-create-actions {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.sp-view-create-action {
  width: 24px;
  min-width: 24px;
  height: 24px;
  padding: 0;
}

.sp-view-empty {
  padding: 8px 7px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.sp-tree-row.virtual {
  opacity: 0.86;
}

.sp-tree-row.disabled {
  cursor: default;
}

.sp-tree-row.child {
  position: relative;
}

.sp-expand-btn,
.sp-expand-spacer {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.sp-expand-btn {
  border: none;
  background: transparent;
  color: var(--text-secondary);
  border-radius: 3px;
  cursor: pointer;
  padding: 0;
  box-shadow: none;
  opacity: 0.5;
  margin-right: 2px;
}

.sp-tree-row:hover .sp-expand-btn {
  opacity: 1;
}

.sp-expand-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-expand-btn.is-running {
  color: var(--accent-color);
  opacity: 0.92;
}

.sp-expand-btn svg {
  transition: transform 0.15s ease;
}

.sp-expand-btn.is-running svg {
  animation: sp-session-pulse 1.2s ease-in-out infinite;
}

.sp-expand-btn.open svg {
  transform: rotate(90deg);
}

.sp-expand-spacer {
  margin-right: 0;
}

.sp-session-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  width: 100%;
}

.sp-tree-row.folder .sp-session-title {
  font-weight: 600;
}

.sp-session-title {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sp-session-meta {
  margin-left: auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  position: relative;
  flex-shrink: 0;
}

.sp-session-time {
  font-size: 12px;
  color: var(--text-secondary);
  transition: opacity 0.12s ease;
}

.sp-row-archive-btn {
  position: absolute;
  right: 0;
  top: 50%;
  z-index: 2;
  width: 18px;
  height: 18px;
  min-width: 18px;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 75%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg) 92%, var(--hover-bg));
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  pointer-events: none;
  box-shadow: none;
  transform: translateY(-50%) scale(0.92);
  transition: opacity 0.12s ease, transform 0.12s ease, background 0.12s ease, color 0.12s ease, border-color 0.12s ease;
}

.sp-tree-row:hover .sp-row-archive-btn,
.sp-row-archive-btn:focus-visible {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(-50%) scale(1);
}

.sp-row-archive-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-row-archive-btn:focus-visible {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  color: var(--text-color);
}

.sp-row-archive-btn svg {
  width: 12px;
  height: 12px;
}

.sp-tree-row:hover .sp-session-time {
  opacity: 0;
}

.sp-session-status {
  display: inline-flex;
  align-items: center;
  min-height: 18px;
  padding: 0 6px;
  border-radius: 4px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 88%, var(--hover-bg));
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1;
  white-space: nowrap;
}

.sp-session-status.is-running,
.sp-session-status.is-finishing {
  border-color: color-mix(in srgb, var(--accent-color) 26%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 8%, transparent);
  color: var(--accent-color);
}

.sp-session-status.is-queued,
.sp-session-status.is-starting {
  border-color: color-mix(in srgb, var(--status-warn-border, var(--border-color)) 78%, var(--border-color));
  background: color-mix(in srgb, var(--status-warn-bg, var(--hover-bg)) 82%, transparent);
  color: var(--status-warn-fg, var(--text-color));
}

.sp-session-status.is-error {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

@keyframes sp-session-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

.sp-session-dot {
  width: 4px;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 36%, transparent);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text-secondary) 20%, transparent);
  transition: opacity 0.12s ease;
}

.sp-session-dot.is-running,
.sp-session-dot.is-finishing {
  width: 6px;
  height: 6px;
  background: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent);
  animation: sp-session-pulse 1.2s ease-in-out infinite;
}

.sp-session-dot.is-queued,
.sp-session-dot.is-starting {
  width: 6px;
  height: 6px;
  background: var(--status-warn-fg, var(--text-color));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-warn-border, var(--border-color)) 58%, transparent);
}

.sp-session-dot.is-error {
  width: 6px;
  height: 6px;
  background: var(--status-danger-fg);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-danger-border) 60%, transparent);
}

.sp-delete-confirm {
  position: fixed;
  z-index: 10001;
  width: 244px;
  padding: 12px;
  border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color));
  border-radius: 10px;
  background: var(--sidebar-bg);
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.18);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.sp-delete-confirm-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.sp-delete-confirm-text {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.sp-delete-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.sp-delete-confirm-btn {
  min-width: 68px;
}

.sp-rename-input {
  width: 100%;
  background: var(--input-bg);
  color: var(--text-color);
  border: 1px solid var(--accent-color);
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 13px;
  outline: none;
}
</style>
