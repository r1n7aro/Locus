<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  AppWindow,
  Archive,
  Box,
  Check,
  BookOpen,
  ChevronRight,
  Copy,
  Eye,
  EyeOff,
  File,
  FileSearch,
  Folder,
  FolderCog,
  FolderOpen,
  FolderPlus,
  GitBranch,
  GitMerge,
  MessageSquare,
  MoreHorizontal,
  Package,
  PencilLine,
  Plus,
  Save,
  Trash2,
  X,
} from "lucide";
import { t } from "../../i18n";
import {
  isSkillPackageRootDocument,
} from "../../composables/useKnowledgeState";
import {
  openUnityEmbeddedSessionWindow,
  subscribeLocusFileDragState,
  subscribeLocusFileDrop,
  subscribeUnityEmbedAssetDragState,
  subscribeUnityEmbedAssetDrop,
  type LocusFileDragStatePayload,
  type LocusFileDropPayload,
  type LocusFileDropRef,
  type UnityEmbedAssetDragStatePayload,
  type UnityEmbedAssetDropPayload,
} from "../../services/unity";
import {
  openChatSessionWindow,
  openNewChatSessionWindow,
} from "../../services/chatSessionWindow";
import { exportSessionContext } from "../../services/session";
import { sessionContextExportFileName } from "../../composables/sessionContextExport";
import {
  useDisplaySettings,
  type KnowledgeFolderKind,
} from "../../composables/useDisplaySettings";
import { normalizeAppError } from "../../services/errors";
import {
  openWorkspace,
  type ProjectContextDescriptor,
  type WorkspaceCheckoutDescriptor,
  type WorkspaceRef,
} from "../../services/project";
import {
  buildScopedMcpServerArtifacts,
  mcpServerGetState,
} from "../../services/mcpServer";
import { openExtraWorkdirsWindow } from "../../services/extraWorkdirsWindow";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useProjectStore } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import type {
  DevelopmentResourceRef,
  ProjectExplorerMountEntry,
  ProjectExplorerNode,
  ProjectKnowledgeDocument,
} from "../../types/workbench";
import type { AssetRefAttachment, SessionSummary } from "../../types";
import type { UserMessageDraft } from "../../composables/chatMessageDraft";
import { emptyComposerIntent } from "../../composables/chatInputIntents";
import ChatWorkspaceView from "../ChatWorkspaceView.vue";
import CollabView from "../CollabView.vue";
import KnowledgeView from "../KnowledgeView.vue";
import WorkspaceFilePreview from "./WorkspaceFilePreview.vue";
import {
  KNOWLEDGE_INTERNAL_DRAG_TYPE,
  type KnowledgeInternalDragData,
  type KnowledgeWorkspaceDragPayload,
} from "../knowledge/knowledgeWorkspaceDrag";
import LucideIcon from "../icons/LucideIcon.vue";
import { projectIconForServices } from "../icons/projectIcons";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
} from "../icons/unityAssetIcons";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import BaseButton from "../ui/BaseButton.vue";
import WorkspaceTree, {
  type WorkspaceTreeItem,
  type WorkspaceTreeRow,
} from "../explorer/WorkspaceTree.vue";
import {
  maxSessionTreeStatus,
  sessionTreeDisplayTitle,
  sessionTreeStatusForSession,
  type SessionTreeStatus,
} from "../chat/sessionTree";
import type { IconNode } from "lucide";
import {
  type InternalDropDecision,
  type InternalDropResolveContext,
  type InternalDragSource,
  type InternalDropTargetRegistration,
  useInternalDragController,
} from "../../composables/useInternalDrag";

type ItemKind =
  | "project"
  | "newSession"
  | "knowledgeRoot"
  | "collaboration"
  | "checkout"
  | "folder"
  | "empty"
  | "session"
  | "knowledge"
  | "localFile"
  | "mountedFolder"
  | "mountedFile"
  | "inlineCreate"
  | "dropPreview";

interface WorkspaceDragPreview {
  name: string;
  rowKind: WorkspaceTreeRow["kind"];
  icon: IconNode;
  iconClass?: string;
  count: number;
}

interface WorkspaceLayoutInternalDragData {
  item: DevelopmentTreeItem;
}

type WorkbenchInternalDropIntent =
  | { kind: "layout"; layout: LayoutDropIntent; target: DevelopmentTreeItem | null }
  | { kind: "newSession"; target: DevelopmentTreeItem };

const WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE = "locus/workspace-layout";

interface DevelopmentTreeItem extends WorkspaceTreeItem {
  meta: {
    kind: ItemKind;
    projectId: string;
    checkoutId?: string;
    explorerNode?: ProjectExplorerNode;
    session?: SessionSummary;
    runtimeStatus?: SessionTreeStatus | null;
    knowledge?: ProjectKnowledgeDocument;
    mountEntry?: ProjectExplorerMountEntry;
    inlineCreate?: WorkspaceInlineCreateState;
    inlineCreateDepth?: number;
    dropPreview?: WorkspaceDragPreview;
    dropParentNodeId?: string | null;
  };
}

interface FolderDialogState {
  mode: "rename" | "delete";
  projectId: string;
  nodeId?: string;
  value: string;
}

interface WorkspaceInlineCreateState {
  kind: "folder";
  projectId: string;
  parentNodeId: string | null;
  name: string;
}

interface CollabHeadFocusRequest {
  id: number;
  checkoutId: string;
}

interface PresetDialogState {
  mode: "create" | "rename" | "delete";
  projectId: string;
  presetId?: string;
  value: string;
}

interface SessionDialogState {
  mode: "rename" | "delete";
  projectId: string;
  session: SessionSummary;
  value: string;
}

interface LayoutDropIntent {
  projectId: string;
  parentNodeId: string | null;
  position: number;
  targetKey: string;
}

interface SettlingLayoutDrop {
  id: number;
  source: DevelopmentTreeItem;
  intent: LayoutDropIntent;
  preview: WorkspaceDragPreview;
}

const workspaceContextStore = useWorkspaceContextStore();
const explorerStore = useWorkspaceExplorerStore();
const chatStore = useChatStore();
const modelStore = useModelStore();
const agentStore = useAgentStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const uiStore = useUiStore();
const { state: displaySettings, set: setDisplaySetting } = useDisplaySettings();

const expanded = ref<Set<string>>(new Set());
const activeResource = ref<DevelopmentResourceRef | null>(null);
const contextMenu = ref<{ x: number; y: number; item: DevelopmentTreeItem } | null>(null);
const displayMenu = ref<{ x: number; y: number } | null>(null);
const workspaceMenu = ref<{ x: number; y: number } | null>(null);
const folderDialog = ref<FolderDialogState | null>(null);
const folderInput = ref<HTMLInputElement | null>(null);
const inlineCreate = ref<WorkspaceInlineCreateState | null>(null);
const inlineCreateInput = ref<HTMLInputElement | null>(null);
const inlineCreateRow = ref<HTMLElement | null>(null);
const presetDialog = ref<PresetDialogState | null>(null);
const presetInput = ref<HTMLInputElement | null>(null);
const sessionDialog = ref<SessionDialogState | null>(null);
const sessionInput = ref<HTMLInputElement | null>(null);
const chatWorkspaceView = ref<InstanceType<typeof ChatWorkspaceView> | null>(null);
const internalDrag = useInternalDragController();
const workbenchRootRef = ref<HTMLElement | null>(null);
const explorerRootRef = ref<HTMLElement | null>(null);
const dragging = computed<DevelopmentTreeItem | null>(() => {
  if (!internalDrag.dragging.value) return null;
  const source = internalDrag.source.value;
  if (source?.payload.type !== WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) return null;
  return (source.payload.data as WorkspaceLayoutInternalDragData).item;
});
const dropTargetKey = ref<string | null>(null);
const layoutDropIntent = ref<LayoutDropIntent | null>(null);
const settlingLayoutDrop = ref<SettlingLayoutDrop | null>(null);
const renderedLayoutDropIntent = computed(() => (
  layoutDropIntent.value ?? settlingLayoutDrop.value?.intent ?? null
));
let settlingLayoutDropId = 0;
const explorerRootDropActive = computed(() => (
  layoutDropIntent.value?.targetKey.startsWith("explorer-root:") ?? false
));
const locusFileWorkspaceDragActive = ref(false);
const locusFileWorkspaceDragCount = ref(0);
const unityAssetWorkspaceDragActive = ref(false);
const unityAssetWorkspaceDragRefs = ref<AssetRefAttachment[]>([]);
const workspaceDragPointer = ref({ x: 0, y: 0, visible: false });
const workspaceDropAffordanceActive = computed(() => (
  internalDrag.isDraggingType(KNOWLEDGE_INTERNAL_DRAG_TYPE)
  || internalDrag.isDraggingType(WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE)
  || locusFileWorkspaceDragActive.value
  || unityAssetWorkspaceDragActive.value
  || explorerRootDropActive.value
));
const UNITY_WORKSPACE_DRAG_STATE_TTL_MS = 1200;
const showHiddenNodes = ref(false);
const externalDropTarget = ref<DevelopmentTreeItem | null>(null);
const WORKSPACE_EXPLORER_WIDTH_KEY = "locus:developmentExplorerWidth";
const explorerWidth = ref((() => {
  const saved = Number(window.localStorage.getItem(WORKSPACE_EXPLORER_WIDTH_KEY));
  return Number.isFinite(saved) ? Math.min(520, Math.max(220, saved)) : 300;
})());
const resizingExplorer = ref(false);
let explorerResizeStartX = 0;
let explorerResizeStartWidth = 0;
let unityWorkspaceDragStateClearTimer = 0;
let releaseLocusFileDrop: (() => void) | null = null;
let releaseLocusFileDragState: (() => void) | null = null;
let releaseUnityAssetDragState: (() => void) | null = null;
let releaseUnityAssetDrop: (() => void) | null = null;
let unregisterWorkbenchInternalDropTarget: (() => void) | null = null;

const KNOWLEDGE_ROOT_ORDER: KnowledgeFolderKind[] = [
  "design",
  "plan",
  "memory",
  "skill",
  "reference",
];
const SYSTEM_RESOURCE_KIND = "system";
const NEW_SESSION_SYSTEM_RESOURCE_ID = "newSession";
const KNOWLEDGE_SYSTEM_RESOURCE_ID = "knowledge";
const COLLABORATION_SYSTEM_RESOURCE_ID = "collaboration";
const collabHeadFocusRequest = ref<CollabHeadFocusRequest | null>(null);
let collabHeadFocusRequestId = 0;

const visibleProjects = computed<ProjectContextDescriptor[]>(() => {
  if (displaySettings.workspaceDisplayMode === "multi") return workspaceContextStore.projects;
  const focused = workspaceContextStore.focusedProject;
  return focused ? [focused] : workspaceContextStore.projects.slice(0, 1);
});

const explorerHeaderLabel = computed(() => {
  if (displaySettings.workspaceDisplayMode === "multi") return t("development.explorer");
  const root = workspaceContextStore.focusedCheckout?.root
    ?? visibleProjects.value[0]?.checkouts[0]?.root
    ?? "";
  return root ? shortPath(root) : t("development.explorer");
});

const explorerHeaderTitle = computed(() => {
  if (displaySettings.workspaceDisplayMode === "multi") return undefined;
  return workspaceContextStore.focusedCheckout?.root
    ?? visibleProjects.value[0]?.checkouts[0]?.root
    ?? undefined;
});

const presetProjectId = computed(() => activeResource.value?.projectId
  ?? workspaceContextStore.focusedProject?.projectId
  ?? visibleProjects.value[0]?.projectId
  ?? "");

const activePresetId = computed(() => (
  presetProjectId.value
    ? explorerStore.snapshots[presetProjectId.value]?.presetId ?? ""
    : ""
));

function onExplorerResizeStart(event: MouseEvent): void {
  if (event.button !== 0) return;
  event.preventDefault();
  resizingExplorer.value = true;
  explorerResizeStartX = event.clientX;
  explorerResizeStartWidth = explorerWidth.value;
  document.addEventListener("mousemove", onExplorerResizeMove);
  document.addEventListener("mouseup", onExplorerResizeEnd);
  document.body.style.cursor = "col-resize";
  document.body.classList.add("is-dragging-select-lock");
}

function onExplorerResizeMove(event: MouseEvent): void {
  if (!resizingExplorer.value) return;
  const viewportMax = Math.max(220, Math.min(520, window.innerWidth - 360));
  explorerWidth.value = Math.min(
    viewportMax,
    Math.max(220, explorerResizeStartWidth + event.clientX - explorerResizeStartX),
  );
}

function onExplorerResizeEnd(): void {
  if (!resizingExplorer.value) return;
  resizingExplorer.value = false;
  document.removeEventListener("mousemove", onExplorerResizeMove);
  document.removeEventListener("mouseup", onExplorerResizeEnd);
  document.body.style.cursor = "";
  document.body.classList.remove("is-dragging-select-lock");
  window.localStorage.setItem(WORKSPACE_EXPLORER_WIDTH_KEY, String(Math.round(explorerWidth.value)));
}

function shortPath(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  return normalized.split(/[\\/]/).pop() || normalized;
}

function parentPath(path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const separator = normalized.lastIndexOf("/");
  return separator > 0 ? normalized.slice(0, separator) : normalized;
}

function isCurrentWorkspacePath(path: string): boolean {
  const current = workspaceContextStore.focusedCheckout?.root ?? "";
  return path.replace(/\\/g, "/").replace(/\/+$/, "").toLocaleLowerCase()
    === current.replace(/\\/g, "/").replace(/\/+$/, "").toLocaleLowerCase();
}

function normalizeKnowledgeSelectionPath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLocaleLowerCase();
}

function knowledgeDocumentMatchesPendingSelection(
  document: ProjectKnowledgeDocument,
  selection: NonNullable<typeof uiStore.pendingKnowledgeSelection>,
): boolean {
  if (document.type !== selection.dashboard) return false;
  const documentPath = normalizeKnowledgeSelectionPath(document.path);
  const requestedPath = normalizeKnowledgeSelectionPath(selection.path);
  return requestedPath === documentPath
    || requestedPath === `${document.type}/${documentPath}`
    || requestedPath.endsWith(`/${document.type}/${documentPath}`)
    || requestedPath.endsWith(`/${documentPath}`);
}

function projectLabel(project: ProjectContextDescriptor): string {
  const preferred = project.checkouts.find(
    (checkout) => checkout.checkoutId === workspaceContextStore.focusedCheckout?.checkoutId,
  ) ?? project.checkouts[0];
  return preferred ? shortPath(preferred.root) : project.projectId;
}

function sessionBranchLabel(session?: SessionSummary): string {
  const branchRef = session?.executionTarget?.branchRef?.trim();
  if (branchRef) return branchRef.replace(/^refs\/heads\//, "");
  return session?.executionTarget?.headOid?.trim().slice(0, 8) ?? "";
}

function sessionStatusLabel(status: SessionTreeStatus | null | undefined): string {
  return status ? t(`chat.session.status.${status}`) : "";
}

function isActiveSessionStatus(status: SessionTreeStatus | null | undefined): boolean {
  return status === "running"
    || status === "waiting_input"
    || status === "finishing"
    || status === "cancelling"
    || status === "starting"
    || status === "queued";
}

function isAnimatedSessionStatus(status: SessionTreeStatus | null | undefined): boolean {
  return status === "running" || status === "finishing";
}

function runtimeStatusClasses(
  status: SessionTreeStatus | null,
  kind: "session" | "folder",
): Record<string, boolean> {
  const classes: Record<string, boolean> = {
    "is-session-row": kind === "session",
    "has-session-runtime": status !== null,
    "has-active-session": isActiveSessionStatus(status),
    "is-session-animated": kind === "session" && isAnimatedSessionStatus(status),
  };
  if (status) classes[`session-status-${status}`] = true;
  return classes;
}

function sessionsForProject(
  projectId: string,
  fallback: SessionSummary[],
): SessionSummary[] {
  if (workspaceContextStore.focusedProject?.projectId !== projectId) return fallback;
  const hasForeignSession = chatStore.sessions.some(
    (session) => session.projectId && session.projectId !== projectId,
  );
  return hasForeignSession ? fallback : chatStore.sessions;
}

function buildLayoutRuntimeStatuses(
  nodes: ProjectExplorerNode[],
  sessionById: Map<string, SessionSummary>,
): Map<string, SessionTreeStatus | null> {
  const childrenByParent = new Map<string | null, ProjectExplorerNode[]>();
  for (const node of nodes) {
    const parentId = node.parentNodeId ?? null;
    const siblings = childrenByParent.get(parentId) ?? [];
    siblings.push(node);
    childrenByParent.set(parentId, siblings);
  }

  const statusByNodeId = new Map<string, SessionTreeStatus | null>();
  const visit = (node: ProjectExplorerNode): SessionTreeStatus | null => {
    const session = node.resourceKind === "session" && node.resourceId
      ? sessionById.get(node.resourceId)
      : undefined;
    let status = session
      ? sessionTreeStatusForSession(session, chatStore.streamingSessionIds)
      : null;
    for (const child of childrenByParent.get(node.nodeId) ?? []) {
      status = maxSessionTreeStatus(status, visit(child));
    }
    statusByNodeId.set(node.nodeId, status);
    return status;
  };
  for (const root of childrenByParent.get(null) ?? []) visit(root);
  return statusByNodeId;
}

function itemRuntimeStatus(item: DevelopmentTreeItem): SessionTreeStatus | null {
  return item.meta.runtimeStatus ?? null;
}

function itemSessionIsPending(item: DevelopmentTreeItem): boolean {
  return item.meta.kind === "session"
    && item.meta.session?.id === chatStore.pendingSelectionSessionId;
}

function isWorkspaceSessionSelected(projectId: string, sessionId: string): boolean {
  const pendingSessionId = chatStore.pendingSelectionSessionId;
  if (pendingSessionId) return pendingSessionId === sessionId;
  const resource = activeResource.value;
  if (resource) {
    return resource.kind === "session"
      && resource.projectId === projectId
      && resource.sessionId === sessionId;
  }
  return chatStore.activeSessionId === sessionId;
}

function checkoutBranchLabel(projectId: string, checkoutId?: string): string {
  if (!checkoutId) return "";
  const checkout = explorerStore.resources[projectId]?.collaboration?.checkouts.find(
    (candidate) => candidate.checkoutId === checkoutId,
  );
  const branchRef = checkout?.branchRef?.trim();
  if (branchRef) return branchRef.replace(/^refs\/heads\//, "");
  return checkout?.headOid?.trim().slice(0, 8) ?? "";
}

function isExpanded(key: string): boolean {
  return expanded.value.has(key);
}

function isCollaborationExpanded(projectId: string): boolean {
  const resource = activeResource.value;
  return resource?.projectId === projectId
    && (resource.kind === "collaboration" || resource.kind === "checkout");
}

function knowledgeFolderKind(nodeId: string): KnowledgeFolderKind | null {
  if (!nodeId.startsWith("knowledge-type:")) return null;
  const match = nodeId.match(/:(plan|memory|design|skill|reference)$/);
  return (match?.[1] as KnowledgeFolderKind | undefined) ?? null;
}

function isExplorerNodeVisible(
  node: ProjectExplorerNode,
  knowledge: ProjectKnowledgeDocument[],
): boolean {
  if ((node.resourceKind === "knowledge" && node.sourceKind !== "knowledge")
    || node.nodeId.startsWith("knowledge-type:")
    || node.nodeId.startsWith("knowledge-path:")) return false;
  if (node.hidden && !showHiddenNodes.value) return false;
  const kind = knowledgeFolderKind(node.nodeId);
  if (!kind) return true;
  if (!displaySettings.knowledgeFolderVisibility[kind]) return false;
  return kind !== "reference" || knowledge.some((document) => document.type === "reference");
}

function knowledgeDocumentName(document: ProjectKnowledgeDocument): string {
  const normalized = document.path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  const segments = normalized.split("/").filter(Boolean);
  return segments[segments.length - 1] || document.title;
}

function knowledgePathFromFolderId(
  projectId: string,
  nodeId: string,
): { type: KnowledgeFolderKind; path: string } | null {
  const projectPrefix = `knowledge-path:${encodeURIComponent(projectId)}:`;
  if (!nodeId.startsWith(projectPrefix)) return null;
  const remainder = nodeId.slice(projectPrefix.length);
  const separator = remainder.indexOf(":");
  if (separator < 0) return null;
  const type = remainder.slice(0, separator) as KnowledgeFolderKind;
  if (!KNOWLEDGE_ROOT_ORDER.includes(type)) return null;
  const path = remainder
    .slice(separator + 1)
    .split("/")
    .filter(Boolean)
    .map((segment) => decodeURIComponent(segment))
    .join("/");
  return { type, path };
}

function isSkillPackageFolder(
  projectId: string,
  node: ProjectExplorerNode,
  knowledge: ProjectKnowledgeDocument[],
): boolean {
  const folder = knowledgePathFromFolderId(projectId, node.nodeId);
  if (folder?.type !== "skill") return false;
  return knowledge.some((document) => {
    if (!isSkillPackageRootDocument(document)) return false;
    const normalized = document.path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
    return normalized.split("/").slice(0, -1).join("/") === folder.path;
  });
}

function compareKnowledgeTreeNodes(
  left: ProjectExplorerNode,
  right: ProjectExplorerNode,
  parentNodeId: string | null,
  knowledgeById: Map<string, ProjectKnowledgeDocument>,
): number {
  if (parentNodeId === null) {
    const leftType = knowledgeFolderKind(left.nodeId);
    const rightType = knowledgeFolderKind(right.nodeId);
    if (leftType || rightType) {
      if (!leftType) return 1;
      if (!rightType) return -1;
      return KNOWLEDGE_ROOT_ORDER.indexOf(leftType) - KNOWLEDGE_ROOT_ORDER.indexOf(rightType);
    }
  }
  const knowledgeParent = parentNodeId?.startsWith("knowledge-type:")
    || parentNodeId?.startsWith("knowledge-path:");
  if (!knowledgeParent) {
    return left.position - right.position || left.nodeId.localeCompare(right.nodeId);
  }
  const rank = (node: ProjectExplorerNode) => node.nodeKind === "folder" ? 0 : 1;
  const rankDelta = rank(left) - rank(right);
  if (rankDelta !== 0) return rankDelta;
  const name = (node: ProjectExplorerNode) => node.nodeKind === "folder"
    ? node.folderName ?? ""
    : knowledgeById.get(node.resourceId ?? "")
      ? knowledgeDocumentName(knowledgeById.get(node.resourceId ?? "")!)
      : "";
  return name(left).localeCompare(name(right), undefined, {
    sensitivity: "base",
    numeric: true,
  });
}

function makeRow(
  key: string,
  name: string,
  depth: number,
  kind: WorkspaceTreeRow["kind"],
  options: Partial<WorkspaceTreeRow> = {},
): WorkspaceTreeRow {
  return {
    key,
    name,
    depth,
    kind,
    ...options,
    classes: {
      ...(options.classes ?? {}),
      "drop-target": dropTargetKey.value === key,
      "is-drag-source": dragging.value?.key === key,
    },
  };
}

function appendDropPreview(
  items: DevelopmentTreeItem[],
  projectId: string,
  parentNodeId: string | null,
  depth: number,
): void {
  const preview = layoutDragPreview.value;
  if (!preview) return;
  items.push({
    key: `drop-preview:${projectId}:${parentNodeId ?? "root"}`,
    treeRow: makeRow(
      `drop-preview:${projectId}:${parentNodeId ?? "root"}`,
      dragPreviewLabel(preview),
      depth,
      preview.rowKind,
      {
        disabled: true,
        classes: { "is-drop-preview": true },
      },
    ),
    meta: { kind: "dropPreview", projectId, dropPreview: preview },
  });
}

function appendInlineCreate(
  items: DevelopmentTreeItem[],
  projectId: string,
  parentNodeId: string | null,
  depth: number,
): void {
  const draft = inlineCreate.value;
  if (
    !draft
    || draft.projectId !== projectId
    || draft.parentNodeId !== parentNodeId
  ) return;
  items.push({
    key: `inline-create:${projectId}:${parentNodeId ?? "root"}:${draft.kind}`,
    treeRow: null,
    meta: {
      kind: "inlineCreate",
      projectId,
      inlineCreate: draft,
      inlineCreateDepth: depth,
    },
  });
}

function mountedEntryKey(projectId: string, nodeId: string, relativePath: string): string {
  return `mounted:${projectId}:${nodeId}:${encodeURIComponent(relativePath)}`;
}

function appendMountedEntries(
  items: DevelopmentTreeItem[],
  projectId: string,
  mountNode: ProjectExplorerNode,
  depth: number,
): void {
  const listing = explorerStore.mountListing(projectId, mountNode.nodeId);
  if (!listing) return;
  const directoryPaths = new Set(
    listing.entries.filter((entry) => entry.isDir).map((entry) => entry.relativePath),
  );
  for (const entry of listing.entries) {
    const segments = entry.relativePath.split("/").filter(Boolean);
    const ancestorPaths = segments.slice(0, -1).map((_, index) => (
      segments.slice(0, index + 1).join("/")
    ));
    if (ancestorPaths.some((path) => (
      directoryPaths.has(path)
      && !isExpanded(mountedEntryKey(projectId, mountNode.nodeId, path))
    ))) continue;
    const key = mountedEntryKey(projectId, mountNode.nodeId, entry.relativePath);
    const selected = activeResource.value?.kind === "localFile"
      && activeResource.value.path === entry.absolutePath;
    items.push({
      key,
      treeRow: makeRow(key, entry.name, depth + entry.depth, entry.isDir ? "folder" : "file", {
        expandable: entry.isDir,
        expanded: entry.isDir ? isExpanded(key) : undefined,
        selected,
        title: entry.absolutePath,
        classes: {
          "is-open": selected,
          "is-mounted-entry": true,
        },
      }),
      meta: {
        kind: entry.isDir ? "mountedFolder" : "mountedFile",
        projectId,
        explorerNode: mountNode,
        mountEntry: entry,
      },
    });
  }
}

function appendLayoutChildren(
  items: DevelopmentTreeItem[],
  project: ProjectContextDescriptor,
  parentNodeId: string | null,
  depth: number,
  sessionById: Map<string, SessionSummary>,
  runtimeStatusByNodeId: Map<string, SessionTreeStatus | null>,
): void {
  const snapshot = explorerStore.snapshots[project.projectId];
  const projectResources = explorerStore.resources[project.projectId];
  if (!snapshot || !projectResources) return;
  const knowledgeById = new Map(projectResources.knowledge.map((document) => [document.id, document]));
  const renderedIntent = renderedLayoutDropIntent.value;
  const renderedSource = settlingLayoutDrop.value?.source ?? dragging.value;
  const internalSourceNodeId = (
    settlingLayoutDrop.value !== null || internalDrag.previewMode.value !== "floating"
  )
    && renderedSource?.meta.projectId === project.projectId
    && renderedIntent
    ? renderedSource.meta.explorerNode?.nodeId ?? null
    : null;
  const nodes = snapshot.nodes
    .filter((node) => (
      (node.parentNodeId ?? null) === parentNodeId
      && isExplorerNodeVisible(node, projectResources.knowledge)
      && node.nodeId !== internalSourceNodeId
    ))
    .sort((left, right) => compareKnowledgeTreeNodes(
      left,
      right,
      parentNodeId,
      knowledgeById,
    ));
  const previewIntent = renderedIntent?.projectId === project.projectId
    && renderedIntent.parentNodeId === parentNodeId
    ? renderedIntent
    : null;
  let previewInserted = false;
  for (const node of nodes) {
    if (previewIntent && !previewInserted && node.position >= previewIntent.position) {
      appendDropPreview(items, project.projectId, parentNodeId, depth);
      previewInserted = true;
    }
    if (node.nodeKind === "folder") {
      const key = `folder:${project.projectId}:${node.nodeId}`;
      const layoutChildren = snapshot.nodes.some((candidate) => {
        if (candidate.parentNodeId !== node.nodeId) return false;
        if (!isExplorerNodeVisible(candidate, projectResources.knowledge)) return false;
        if (candidate.nodeKind === "folder") return true;
        if (candidate.resourceKind === "session") {
          return sessionById.has(candidate.resourceId ?? "");
        }
        if (candidate.resourceKind === "knowledge") {
          return knowledgeById.has(candidate.resourceId ?? "");
        }
        if (candidate.resourceKind === SYSTEM_RESOURCE_KIND) return true;
        if (candidate.resourceKind === "local_file") return !!candidate.sourcePath;
        return false;
      });
      const mountedDirectory = !!node.sourcePath;
      const hasDropPreview = renderedIntent?.projectId === project.projectId
        && renderedIntent.parentNodeId === node.nodeId;
      const hasInlineCreate = inlineCreate.value?.projectId === project.projectId
        && inlineCreate.value.parentNodeId === node.nodeId;
      const children = layoutChildren || mountedDirectory || hasDropPreview || hasInlineCreate;
      const isKnowledgeRoot = knowledgeFolderKind(node.nodeId) !== null;
      const isPackage = isSkillPackageFolder(project.projectId, node, projectResources.knowledge);
      const selected = activeResource.value?.kind === "folder"
        && activeResource.value.nodeId === node.nodeId;
      const runtimeStatus = runtimeStatusByNodeId.get(node.nodeId) ?? null;
      items.push({
        key,
        treeRow: makeRow(key, node.folderName || t("development.untitledFolder"), depth, isPackage ? "package" : "folder", {
          expandable: true,
          expanded: isExpanded(key),
          selected,
          dragEnabled: !isKnowledgeRoot,
          classes: {
            ...runtimeStatusClasses(runtimeStatus, "folder"),
            "kx-folder": !isPackage,
            "kx-package": isPackage,
            "is-special-root": isKnowledgeRoot,
            "is-open": selected,
            "is-hidden-node": node.hidden,
            "is-mounted-root": mountedDirectory,
          },
        }),
        meta: {
          kind: "folder",
          projectId: project.projectId,
          explorerNode: node,
          runtimeStatus,
        },
      });
      if (isExpanded(key)) {
        if (children) {
          if (layoutChildren || hasDropPreview || hasInlineCreate) {
            appendLayoutChildren(
              items,
              project,
              node.nodeId,
              depth + 1,
              sessionById,
              runtimeStatusByNodeId,
            );
          }
          if (mountedDirectory) {
            appendMountedEntries(items, project.projectId, node, depth + 1);
          }
        } else {
          const emptyKey = `empty:${project.projectId}:${node.nodeId}`;
          items.push({
            key: emptyKey,
            treeRow: makeRow(emptyKey, t("development.emptyFolder"), depth + 1, "file", {
              disabled: true,
              classes: { "is-empty-folder-row": true },
            }),
            meta: {
              kind: "empty",
              projectId: project.projectId,
              dropParentNodeId: node.nodeId,
            },
          });
        }
      }
      continue;
    }
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === NEW_SESSION_SYSTEM_RESOURCE_ID
    ) {
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const key = `new-session:${project.projectId}`;
      items.push({
        key,
        treeRow: makeRow(key, t("chat.session.newSession"), depth, "file", {
          selected: activeResource.value?.kind === "newSession"
            && activeResource.value.projectId === project.projectId
            && chatStore.activeSessionId === null,
          dragEnabled: true,
          classes: { "is-hidden-node": node.hidden },
        }),
        meta: {
          kind: "newSession",
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      });
      continue;
    }
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID
    ) {
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const key = `knowledge-root:${project.projectId}`;
      const selected = activeResource.value?.kind === "knowledgeRoot"
        && activeResource.value.projectId === project.projectId;
      items.push({
        key,
        treeRow: makeRow(key, t("app.tab.knowledge"), depth, "folder", {
          selected,
          dragEnabled: true,
          classes: {
            "is-open": selected,
            "is-hidden-node": node.hidden,
          },
        }),
        meta: {
          kind: "knowledgeRoot",
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      });
      continue;
    }
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === COLLABORATION_SYSTEM_RESOURCE_ID
    ) {
      const key = `collaboration:${project.projectId}`;
      const collaborationExpanded = isCollaborationExpanded(project.projectId);
      items.push({
        key,
        treeRow: makeRow(key, t("app.tab.collab"), depth, "folder", {
          expandable: project.checkouts.length > 0,
          expanded: collaborationExpanded,
          selected: activeResource.value?.kind === "collaboration"
            && activeResource.value.projectId === project.projectId,
          dragEnabled: true,
          classes: { "is-hidden-node": node.hidden },
        }),
        meta: {
          kind: "collaboration",
          projectId: project.projectId,
          explorerNode: node,
        },
      });
      if (collaborationExpanded) {
        for (const checkout of project.checkouts) {
          const checkoutKey = `checkout:${checkout.checkoutId}`;
          items.push({
            key: checkoutKey,
            treeRow: makeRow(checkoutKey, shortPath(checkout.root), depth + 1, "folder", {
              selected: activeResource.value?.kind === "checkout"
                && activeResource.value.checkoutId === checkout.checkoutId,
              focused: workspaceContextStore.focusedCheckout?.checkoutId === checkout.checkoutId,
              title: checkout.root,
            }),
            meta: {
              kind: "checkout",
              projectId: project.projectId,
              checkoutId: checkout.checkoutId,
            },
          });
        }
      }
      continue;
    }
    if (node.resourceKind === "session" && node.resourceId) {
      const session = sessionById.get(node.resourceId);
      if (!session) continue;
      const key = `session:${project.projectId}:${session.id}`;
      const runtimeStatus = runtimeStatusByNodeId.get(node.nodeId) ?? null;
      const selected = isWorkspaceSessionSelected(project.projectId, session.id);
      const displayTitle = sessionTreeDisplayTitle(
        session.title,
        session.sessionType,
        Boolean(session.parentSessionId),
      ) || t("chat.session.newSession");
      items.push({
        key,
        treeRow: makeRow(key, displayTitle, depth, "file", {
          selected,
          dragEnabled: true,
          title: runtimeStatus
            ? `${session.title || displayTitle} — ${sessionStatusLabel(runtimeStatus)}`
            : session.title,
          classes: {
            ...runtimeStatusClasses(runtimeStatus, "session"),
            "is-open": selected,
            "is-session-pending": chatStore.pendingSelectionSessionId === session.id,
            "is-hidden-node": node.hidden,
          },
        }),
        meta: {
          kind: "session",
          projectId: project.projectId,
          explorerNode: node,
          session,
          runtimeStatus,
        },
      });
      continue;
    }
    if (node.resourceKind === "knowledge" && node.resourceId) {
      const knowledge = knowledgeById.get(node.resourceId);
      if (!knowledge) continue;
      const key = `knowledge:${project.projectId}:${knowledge.id}`;
      items.push({
        key,
        treeRow: makeRow(key, knowledgeDocumentName(knowledge), depth, "file", {
          selected: activeResource.value?.kind === "knowledge" && activeResource.value.documentId === knowledge.id,
          dragEnabled: true,
          title: `${knowledge.type}/${knowledge.path}`,
          classes: {
            "kx-leaf": true,
            "is-hidden-node": node.hidden,
            "is-open": activeResource.value?.kind === "knowledge"
              && activeResource.value.documentId === knowledge.id,
          },
        }),
        meta: { kind: "knowledge", projectId: project.projectId, explorerNode: node, knowledge },
      });
      continue;
    }
    if (node.resourceKind === "local_file" && node.sourcePath) {
      const key = `local-file:${project.projectId}:${node.nodeId}`;
      const selected = activeResource.value?.kind === "localFile"
        && activeResource.value.path === node.sourcePath;
      items.push({
        key,
        treeRow: makeRow(key, node.folderName || shortPath(node.sourcePath), depth, "file", {
          selected,
          dragEnabled: true,
          title: node.sourcePath,
          classes: {
            "is-open": selected,
            "is-hidden-node": node.hidden,
          },
        }),
        meta: { kind: "localFile", projectId: project.projectId, explorerNode: node },
      });
    }
  }
  if (previewIntent && !previewInserted) {
    appendDropPreview(items, project.projectId, parentNodeId, depth);
  }
  appendInlineCreate(items, project.projectId, parentNodeId, depth);
}

const treeItems = computed<DevelopmentTreeItem[]>(() => {
  const items: DevelopmentTreeItem[] = [];
  const showProjectNodes = displaySettings.workspaceDisplayMode === "multi";
  for (const project of visibleProjects.value) {
    const projectKey = `project:${project.projectId}`;
    const projectOpen = !showProjectNodes || isExpanded(projectKey);
    const resourceDepth = showProjectNodes ? 1 : 0;
    const projectResources = explorerStore.resources[project.projectId];
    const projectSessions = sessionsForProject(
      project.projectId,
      projectResources?.sessions ?? [],
    );
    const sessionById = new Map(projectSessions.map((session) => [session.id, session]));
    const runtimeStatusByNodeId = buildLayoutRuntimeStatuses(
      explorerStore.snapshots[project.projectId]?.nodes ?? [],
      sessionById,
    );
    if (showProjectNodes) {
      items.push({
        key: projectKey,
        treeRow: makeRow(projectKey, projectLabel(project), 0, "package", {
          expandable: true,
          expanded: projectOpen,
          selected: activeResource.value?.kind === "project" && activeResource.value.projectId === project.projectId,
          title: project.projectId,
        }),
        meta: { kind: "project", projectId: project.projectId },
      });
    }
    if (!projectOpen) continue;

    appendLayoutChildren(
      items,
      project,
      null,
      resourceDepth,
      sessionById,
      runtimeStatusByNodeId,
    );
  }
  return items;
});

function itemIcon(item: DevelopmentTreeItem) {
  switch (item.meta.kind) {
    case "project": {
      const project = workspaceContextStore.projectsById[item.meta.projectId];
      const projectIcon = projectIconForServices(project?.detectedServices ?? []);
      if (projectIcon) return projectIcon;
      return item.treeRow?.expanded ? FolderOpen : Folder;
    }
    case "collaboration": return GitMerge;
    case "newSession": return Plus;
    case "dropPreview": return item.meta.dropPreview?.icon ?? File;
    case "knowledgeRoot": return BookOpen;
    case "checkout": return GitBranch;
    case "session": return MessageSquare;
    case "knowledge": return unityAssetIconNodeForPath(item.meta.knowledge?.path ?? "document.md", {
      isFolder: false,
    });
    case "localFile": return unityAssetIconNodeForPath(
      item.meta.explorerNode?.sourcePath ?? "file",
      { isFolder: false },
    );
    case "mountedFile": return unityAssetIconNodeForPath(
      item.meta.mountEntry?.absolutePath ?? "file",
      { isFolder: false },
    );
    case "mountedFolder": return item.treeRow?.expanded ? FolderOpen : Folder;
    default:
      if (item.meta.explorerNode && knowledgeFolderKind(item.meta.explorerNode.nodeId)) {
        return BookOpen;
      }
      if (item.treeRow?.kind === "package") return Package;
      return item.treeRow?.expanded ? FolderOpen : Folder;
  }
}

function itemIconClass(item: DevelopmentTreeItem): string | undefined {
  if (item.meta.kind === "dropPreview") return item.meta.dropPreview?.iconClass;
  const path = item.meta.kind === "knowledge"
    ? item.meta.knowledge?.path
    : item.meta.kind === "localFile"
      ? item.meta.explorerNode?.sourcePath
      : item.meta.kind === "mountedFile"
        ? item.meta.mountEntry?.absolutePath
        : null;
  if (!path) return undefined;
  return unityAssetIconClassForPath(path, {
    isFolder: false,
  });
}

function dragPreviewLabel(preview: WorkspaceDragPreview): string {
  return preview.count > 1
    ? `${preview.name} +${preview.count - 1}`
    : preview.name;
}

const workspaceDragPreview = computed<WorkspaceDragPreview | null>(() => {
  const unityRefs = unityAssetWorkspaceDragRefs.value;
  const unityRef = unityRefs[0];
  if (unityRef) {
    return {
      name: unityRef.name || unityRef.path.split(/[\\/]/).pop() || unityRef.path,
      rowKind: "file",
      icon: unityAssetIconNodeForPath(unityRef.path, { isFolder: false }),
      iconClass: unityAssetIconClassForPath(unityRef.path, { isFolder: false }),
      count: unityRefs.length,
    };
  }

  if (locusFileWorkspaceDragActive.value) {
    return {
      name: t("development.draggedItems", Math.max(1, locusFileWorkspaceDragCount.value)),
      rowKind: "file",
      icon: File,
      count: 1,
    };
  }
  return null;
});

function workspaceDragPreviewForInternalSource(source: InternalDragSource): WorkspaceDragPreview {
  const preview = source.preview;
  const rowKind: WorkspaceTreeRow["kind"] = preview.kind === "folder"
    ? "folder"
    : preview.kind === "package"
      ? "package"
      : "file";
  return {
    name: preview.label,
    rowKind,
    icon: preview.icon ?? (rowKind === "folder" ? Folder : rowKind === "package" ? Package : File),
    iconClass: preview.iconClass,
    count: Math.max(1, preview.count ?? 1),
  };
}

const internalLayoutDragPreview = computed<WorkspaceDragPreview | null>(() => {
  if (!internalDrag.dragging.value || internalDrag.previewMode.value === "floating") return null;
  const source = internalDrag.source.value;
  if (!source || (
    source.payload.type !== WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
    && source.payload.type !== KNOWLEDGE_INTERNAL_DRAG_TYPE
  )) return null;
  return workspaceDragPreviewForInternalSource(source);
});

const layoutDragPreview = computed<WorkspaceDragPreview | null>(() => (
  internalLayoutDragPreview.value
  ?? settlingLayoutDrop.value?.preview
  ?? workspaceDragPreview.value
));

const workspaceDragFloatingStyle = computed(() => {
  const width = 228;
  const height = 34;
  const x = Math.max(8, Math.min(window.innerWidth - width - 8, workspaceDragPointer.value.x + 14));
  const y = Math.max(8, Math.min(window.innerHeight - height - 8, workspaceDragPointer.value.y + 12));
  return {
    transform: `translate3d(${Math.round(x)}px, ${Math.round(y)}px, 0)`,
  };
});

function updateWorkspaceDragPointer(clientX: number, clientY: number): void {
  if (!Number.isFinite(clientX) || !Number.isFinite(clientY)) return;
  if (clientX <= 0 && clientY <= 0) return;
  workspaceDragPointer.value = { x: clientX, y: clientY, visible: true };
}

function trackWorkspaceDragPointer(event: DragEvent): void {
  if (!workspaceDragPreview.value) return;
  updateWorkspaceDragPointer(event.clientX, event.clientY);
}

function handleWindowWorkspaceDrop(): void {
  if (!workspaceDragPreview.value) return;
  clearWorkspaceDragPointer();
}

function clearWorkspaceDragPointer(): void {
  if (!workspaceDragPointer.value.visible) return;
  workspaceDragPointer.value = {
    ...workspaceDragPointer.value,
    visible: false,
  };
}

async function ensureProjectCheckout(
  project: ProjectContextDescriptor,
  preferredCheckoutId?: string | null,
): Promise<WorkspaceCheckoutDescriptor | null> {
  const focused = workspaceContextStore.focusedCheckout;
  const checkout = focused?.projectId === project.projectId
    ? focused
    : project.checkouts.find((candidate) => candidate.checkoutId === preferredCheckoutId)
      ?? project.checkouts[0];
  if (!checkout) return null;
  if (workspaceContextStore.focusedCheckout?.checkoutId !== checkout.checkoutId) {
    await workspaceContextStore.focusCheckout(checkout.checkoutId);
    await refreshFocusedCheckoutServices();
  }
  return workspaceContextStore.checkoutsById[checkout.checkoutId] ?? checkout;
}

async function refreshFocusedCheckoutServices(): Promise<void> {
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (!workspaceRef) return;
  await Promise.all([
    chatStore.refreshSessions(),
    agentStore.loadWorkspaceAgents(workspaceRef),
    projectStore.checkUnityConnection(),
    projectStore.checkUnityPlugin(),
    projectStore.loadAssetDbStatus(),
  ]);
}

function toggleWorkspaceMenu(event: MouseEvent): void {
  if (displaySettings.workspaceDisplayMode !== "single") return;
  if (workspaceMenu.value) {
    workspaceMenu.value = null;
    return;
  }
  contextMenu.value = null;
  displayMenu.value = null;
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  workspaceMenu.value = { x: rect.left + 4, y: rect.bottom + 2 };
  void projectStore.loadRecentDirs().catch((error) => {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  });
}

function toggleDisplayMenu(event: MouseEvent): void {
  if (displayMenu.value) {
    displayMenu.value = null;
    return;
  }
  contextMenu.value = null;
  workspaceMenu.value = null;
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  displayMenu.value = { x: rect.right, y: rect.bottom + 2 };
}

async function switchWorkspaceTreePreset(presetId: string): Promise<void> {
  const projectId = presetProjectId.value;
  if (!projectId || !presetId || presetId === activePresetId.value) return;
  try {
    await explorerStore.switchPreset(projectId, presetId);
    expanded.value = new Set([
      ...(displaySettings.workspaceDisplayMode === "multi" ? [`project:${projectId}`] : []),
    ]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function beginPresetDialog(mode: PresetDialogState["mode"]): void {
  displayMenu.value = null;
  const projectId = presetProjectId.value;
  const snapshot = explorerStore.snapshots[projectId];
  if (!projectId || !snapshot) return;
  presetDialog.value = {
    mode,
    projectId,
    presetId: snapshot.presetId,
    value: mode === "create" ? "" : snapshot.presetName,
  };
  void nextTick(() => {
    if (mode === "delete") return;
    presetInput.value?.focus();
    if (mode === "rename") presetInput.value?.select();
  });
}

async function commitPresetDialog(): Promise<void> {
  const dialog = presetDialog.value;
  if (!dialog) return;
  try {
    if (dialog.mode === "create") {
      if (!dialog.value.trim()) return;
      await explorerStore.createPreset(dialog.projectId, dialog.value.trim());
    } else if (dialog.mode === "rename" && dialog.presetId) {
      if (!dialog.value.trim()) return;
      await explorerStore.renamePreset(dialog.projectId, dialog.presetId, dialog.value.trim());
    } else if (dialog.mode === "delete" && dialog.presetId) {
      await explorerStore.deletePreset(dialog.projectId, dialog.presetId);
    }
    presetDialog.value = null;
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function selectRecentWorkspace(path: string): Promise<void> {
  workspaceMenu.value = null;
  if (!path.trim()) return;
  try {
    await workspaceContextStore.openAndFocus(path);
    await refreshFocusedCheckoutServices();
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function revealPendingKnowledgeSelection(): Promise<void> {
  const pending = uiStore.pendingKnowledgeSelection;
  if (!pending) return;
  uiStore.setTab("chat");
  const project = workspaceContextStore.focusedProject ?? visibleProjects.value[0];
  if (!project) return;
  await explorerStore.loadProject(project.projectId, true);
  if (uiStore.pendingKnowledgeSelection?.id !== pending.id) return;
  const document = explorerStore.resources[project.projectId]?.knowledge.find(
    (candidate) => knowledgeDocumentMatchesPendingSelection(candidate, pending),
  );
  if (!document) return;
  await ensureProjectCheckout(project, document.sourceCheckoutId);
  const resourceNode = explorerStore.snapshots[project.projectId]?.nodes.find(
    (node) => node.nodeKind === "resource"
      && node.resourceKind === "knowledge"
      && node.resourceId === document.id,
  );
  const nextExpanded = new Set(expanded.value);
  if (displaySettings.workspaceDisplayMode === "multi") {
    nextExpanded.add(`project:${project.projectId}`);
  }
  let parentNodeId = resourceNode?.parentNodeId ?? null;
  const nodesById = new Map(
    (explorerStore.snapshots[project.projectId]?.nodes ?? []).map((node) => [node.nodeId, node]),
  );
  while (parentNodeId) {
    nextExpanded.add(`folder:${project.projectId}:${parentNodeId}`);
    parentNodeId = nodesById.get(parentNodeId)?.parentNodeId ?? null;
  }
  expanded.value = nextExpanded;
  explorerStore.selectedNodeKey = `knowledge:${project.projectId}:${document.id}`;
  activeResource.value = {
    kind: "knowledge",
    projectId: project.projectId,
    documentId: document.id,
    sourceCheckoutId: document.sourceCheckoutId,
  };
}

async function activateItem(raw: WorkspaceTreeItem, event?: MouseEvent): Promise<void> {
  const item = raw as DevelopmentTreeItem;
  const project = workspaceContextStore.projectsById[item.meta.projectId];
  if (!project) return;
  try {
    if (item.meta.kind === "project") {
      toggleItem(item);
      return;
    }
    explorerStore.selectedNodeKey = item.key;
    if (item.meta.kind === "folder") {
      if (item.treeRow?.expandable) toggleItem(item);
      if (item.meta.explorerNode?.sourcePath) {
        await explorerStore.loadMount(project.projectId, item.meta.explorerNode.nodeId);
      }
      if (item.meta.explorerNode) {
        activeResource.value = {
          kind: "folder",
          projectId: project.projectId,
          nodeId: item.meta.explorerNode.nodeId,
        };
      }
      return;
    }
    if (item.meta.kind === "mountedFolder") {
      toggleItem(item);
      return;
    }
    if (item.meta.kind === "newSession") {
      const checkout = await ensureProjectCheckout(project);
      if (!checkout) return;
      if (event?.ctrlKey || event?.metaKey) {
        const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
        if (workspaceRef) {
          await openNewChatSessionWindow(workspaceRef, t("chat.session.newSession"));
        }
        return;
      }
      chatStore.newChat();
      activeResource.value = {
        kind: "newSession",
        projectId: project.projectId,
        checkoutId: checkout.checkoutId,
      };
      return;
    }
    if (item.meta.kind === "knowledgeRoot") {
      const checkout = await ensureProjectCheckout(project, item.meta.checkoutId);
      if (!checkout) return;
      activeResource.value = {
        kind: "knowledgeRoot",
        projectId: project.projectId,
        checkoutId: checkout.checkoutId,
      };
      return;
    }
    if (item.meta.kind === "collaboration") {
      await ensureProjectCheckout(project);
      activeResource.value = { kind: "collaboration", projectId: project.projectId };
      return;
    }
    if (item.meta.kind === "checkout" && item.meta.checkoutId) {
      if (workspaceContextStore.focusedCheckout?.checkoutId !== item.meta.checkoutId) {
        await workspaceContextStore.focusCheckout(item.meta.checkoutId);
        await refreshFocusedCheckoutServices();
      }
      activeResource.value = {
        kind: "checkout",
        projectId: project.projectId,
        checkoutId: item.meta.checkoutId,
      };
      collabHeadFocusRequest.value = {
        id: ++collabHeadFocusRequestId,
        checkoutId: item.meta.checkoutId,
      };
      return;
    }
    if (item.meta.kind === "session" && item.meta.session) {
      if (event?.ctrlKey || event?.metaKey) {
        await openSessionWindow(item.meta.session);
        return;
      }
      const preferred = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout.checkoutId
        : item.meta.session.executionTarget?.checkoutId
          ?? item.meta.session.defaultCheckoutId;
      const checkout = await ensureProjectCheckout(project, preferred);
      if (!checkout) return;
      await chatStore.selectSession(item.meta.session.id);
      activeResource.value = {
        kind: "session",
        projectId: project.projectId,
        sessionId: item.meta.session.id,
        checkoutId: checkout.checkoutId,
      };
      return;
    }
    if (item.meta.kind === "knowledge" && item.meta.knowledge) {
      const checkout = await ensureProjectCheckout(project, item.meta.knowledge.sourceCheckoutId);
      if (!checkout) return;
      activeResource.value = {
        kind: "knowledge",
        projectId: project.projectId,
        documentId: item.meta.knowledge.id,
        sourceCheckoutId: checkout.checkoutId,
      };
      return;
    }
    if (
      (item.meta.kind === "localFile" || item.meta.kind === "mountedFile")
      && (item.meta.explorerNode?.sourcePath || item.meta.mountEntry?.absolutePath)
    ) {
      activeResource.value = {
        kind: "localFile",
        projectId: project.projectId,
        path: item.meta.mountEntry?.absolutePath ?? item.meta.explorerNode!.sourcePath!,
        nodeId: item.meta.explorerNode?.nodeId ?? item.meta.mountEntry!.nodeId,
      };
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function toggleItem(raw: WorkspaceTreeItem): void {
  const item = raw as DevelopmentTreeItem;
  const next = new Set(expanded.value);
  if (next.has(item.key)) next.delete(item.key);
  else next.add(item.key);
  expanded.value = next;
}

function contextSessionEntry(): {
  item: DevelopmentTreeItem;
  project: ProjectContextDescriptor;
  session: SessionSummary;
} | null {
  const item = contextMenu.value?.item;
  const session = item?.meta.kind === "session" ? item.meta.session : null;
  const project = item ? workspaceContextStore.projectsById[item.meta.projectId] : null;
  return item && session && project ? { item, project, session } : null;
}

async function openSessionWindow(session: SessionSummary): Promise<void> {
  try {
    await openChatSessionWindow({
      sessionId: session.id,
      title: session.title || session.id,
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "openChatSessionWindow",
      skipConsoleLog: true,
    });
  }
}

async function contextOpenSessionWindow(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (entry) await openSessionWindow(entry.session);
}

async function contextOpenSessionInUnity(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  try {
    const preferredCheckoutId = entry.session.executionTarget?.checkoutId
      ?? entry.session.defaultCheckoutId;
    const checkout = await ensureProjectCheckout(entry.project, preferredCheckoutId);
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!checkout || !workspaceRef) return;
    await openUnityEmbeddedSessionWindow(workspaceRef, {
      sessionId: entry.session.id,
      title: entry.session.title || entry.session.id,
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "openSessionInUnity",
      skipConsoleLog: true,
    });
  }
}

async function contextOpenNewSessionWindow(): Promise<void> {
  const item = contextMenu.value?.item;
  const project = item ? workspaceContextStore.projectsById[item.meta.projectId] : null;
  contextMenu.value = null;
  if (!project) return;
  try {
    const checkout = await ensureProjectCheckout(project);
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!checkout || !workspaceRef) return;
    await openNewChatSessionWindow(workspaceRef, t("chat.session.newSession"));
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "openNewChatSessionWindow",
      skipConsoleLog: true,
    });
  }
}

function beginRenameSession(): void {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  sessionDialog.value = {
    mode: "rename",
    projectId: entry.project.projectId,
    session: entry.session,
    value: entry.session.title || "",
  };
  void nextTick(() => sessionInput.value?.select());
}

function beginDeleteSession(): void {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  sessionDialog.value = {
    mode: "delete",
    projectId: entry.project.projectId,
    session: entry.session,
    value: entry.session.title || t("chat.session.newSession"),
  };
}

function resetActiveSessionResource(projectId: string): void {
  const checkout = workspaceContextStore.focusedCheckout?.projectId === projectId
    ? workspaceContextStore.focusedCheckout
    : workspaceContextStore.projectsById[projectId]?.checkouts[0];
  if (!checkout) {
    activeResource.value = null;
    return;
  }
  activeResource.value = {
    kind: "newSession",
    projectId,
    checkoutId: checkout.checkoutId,
  };
}

async function archiveSessionEntry(projectId: string, session: SessionSummary): Promise<void> {
  await chatStore.archiveSession(session.id);
  if (activeResource.value?.kind === "session"
    && activeResource.value.sessionId === session.id) {
    resetActiveSessionResource(projectId);
  }
}

async function archiveSessionItem(item: DevelopmentTreeItem): Promise<void> {
  if (item.meta.kind !== "session" || !item.meta.session) return;
  await archiveSessionEntry(item.meta.projectId, item.meta.session);
}

async function archiveContextSession(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  await archiveSessionEntry(entry.project.projectId, entry.session);
}

async function commitSessionDialog(): Promise<void> {
  const dialog = sessionDialog.value;
  if (!dialog) return;
  if (dialog.mode === "rename") {
    const title = dialog.value.trim();
    if (!title) return;
    await chatStore.renameSession(dialog.session.id, title);
  } else {
    await chatStore.deleteSession(dialog.session.id);
    if (activeResource.value?.kind === "session"
      && activeResource.value.sessionId === dialog.session.id) {
      resetActiveSessionResource(dialog.projectId);
    }
  }
  sessionDialog.value = null;
}

async function exportContextSession(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  try {
    const filePath = await save({
      defaultPath: sessionContextExportFileName(entry.session.id, entry.session.title || "untitled"),
      filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
    });
    if (!filePath) return;
    const result = await exportSessionContext(entry.session.id, filePath);
    notificationStore.addNotice("success", t("chat.contextExported", result.filePath), {
      operation: "exportSessionContext",
      replaceOperation: true,
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("app.saveFailed", normalized.message), {
      code: normalized.code,
      operation: "exportSessionContext",
      skipConsoleLog: true,
    });
  }
}

async function reviewContextSession(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  try {
    const preferredCheckoutId = entry.session.executionTarget?.checkoutId
      ?? entry.session.defaultCheckoutId;
    const checkout = await ensureProjectCheckout(entry.project, preferredCheckoutId);
    if (!checkout) return;
    await chatStore.selectSession(entry.session.id);
    activeResource.value = {
      kind: "session",
      projectId: entry.project.projectId,
      sessionId: entry.session.id,
      checkoutId: checkout.checkoutId,
    };
    await nextTick();
    await chatWorkspaceView.value?.reviewSessionContext({ sessionId: entry.session.id });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "reviewSessionContext",
      skipConsoleLog: true,
    });
  }
}

function openContextMenu(raw: WorkspaceTreeItem, event: MouseEvent): void {
  const item = raw as DevelopmentTreeItem;
  if (!(item.meta.kind === "project"
    || item.meta.kind === "newSession"
    || item.meta.kind === "folder"
    || item.meta.kind === "checkout"
    || item.meta.kind === "session"
    || item.meta.kind === "knowledge"
    || item.meta.kind === "localFile")) return;
  event.preventDefault();
  contextMenu.value = { x: event.clientX, y: event.clientY, item };
}

function openExplorerBackgroundContextMenu(event: MouseEvent): void {
  const target = event.target as HTMLElement | null;
  if (target?.closest("[data-tree-key]")) return;
  const projectId = activeResource.value?.projectId
    ?? workspaceContextStore.focusedProject?.projectId
    ?? visibleProjects.value[0]?.projectId;
  if (!projectId) return;
  event.preventDefault();
  contextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    item: {
      key: `root-context:${projectId}`,
      treeRow: null,
      meta: { kind: "project", projectId },
    },
  };
}

function contextCheckout(): WorkspaceCheckoutDescriptor | null {
  const item = contextMenu.value?.item;
  if (item?.meta.kind !== "checkout" || !item.meta.checkoutId) return null;
  return workspaceContextStore.checkoutsById[item.meta.checkoutId] ?? null;
}

function checkoutWorkspaceRef(checkout: WorkspaceCheckoutDescriptor): WorkspaceRef {
  return {
    checkoutId: checkout.checkoutId,
    expectedGeneration: checkout.runtime?.workspaceGeneration,
  };
}

async function copyCheckoutMcpArtifact(kind: "endpoint" | "claude" | "json"): Promise<void> {
  const checkout = contextCheckout();
  contextMenu.value = null;
  if (!checkout) return;
  try {
    const server = await mcpServerGetState();
    const artifacts = buildScopedMcpServerArtifacts(
      server.endpointUrl,
      server.settings.token,
      checkoutWorkspaceRef(checkout),
    );
    const content = kind === "endpoint"
      ? artifacts.endpointUrl
      : kind === "claude"
        ? artifacts.claudeCodeCommand
        : artifacts.jsonSnippet;
    await navigator.clipboard.writeText(content);
    notificationStore.addNotice("success", t("common.copied"));
  } catch (error) {
    notificationStore.addNotice("warning", normalizeAppError(error).message);
  }
}

async function openCheckoutInFileExplorer(): Promise<void> {
  const checkout = contextCheckout();
  contextMenu.value = null;
  if (!checkout) return;
  try {
    await projectStore.openDirInFileExplorer(checkout.root);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function configureCheckoutExtraWorkdirs(): Promise<void> {
  const checkout = contextCheckout();
  contextMenu.value = null;
  if (!checkout) return;
  try {
    const runtime = await openWorkspace(checkout.root);
    await openExtraWorkdirsWindow({
      workspacePath: runtime.root,
      workspaceRef: {
        checkoutId: runtime.checkoutId,
        expectedGeneration: runtime.workspaceGeneration,
      },
    });
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function isKnowledgeTypeFolder(item: DevelopmentTreeItem): boolean {
  return item.meta.explorerNode?.nodeId.startsWith("knowledge-type:") === true;
}

function beginCreateFolder(): void {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (!item) return;
  const parentNodeId = item.meta.kind === "folder"
    ? item.meta.explorerNode?.nodeId ?? null
    : item.meta.explorerNode?.parentNodeId ?? null;
  inlineCreate.value = {
    kind: "folder",
    projectId: item.meta.projectId,
    parentNodeId,
    name: "",
  };
  const nextExpanded = new Set(expanded.value);
  if (displaySettings.workspaceDisplayMode === "multi") {
    nextExpanded.add(`project:${item.meta.projectId}`);
  }
  if (item.meta.kind === "folder") nextExpanded.add(item.key);
  expanded.value = nextExpanded;
  void nextTick(() => {
    inlineCreateInput.value?.focus();
    inlineCreateInput.value?.select();
  });
}

function cancelInlineCreate(): void {
  inlineCreate.value = null;
}

async function submitInlineCreate(): Promise<void> {
  const draft = inlineCreate.value;
  const name = draft?.name.trim() ?? "";
  if (!draft || !name) return;
  inlineCreate.value = null;
  try {
    await explorerStore.applyOperations(draft.projectId, [{
      kind: "createFolder",
      parentNodeId: draft.parentNodeId,
      name,
      position: Number.MAX_SAFE_INTEGER,
    }]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
    if (!inlineCreate.value) {
      inlineCreate.value = { ...draft, name };
      await nextTick();
      inlineCreateInput.value?.focus();
      inlineCreateInput.value?.select();
    }
  }
}

function handleInlineCreatePointerDown(event: PointerEvent): void {
  const target = event.target;
  if (!inlineCreate.value || !(target instanceof Node)) return;
  if (inlineCreateRow.value?.contains(target)) return;
  if (inlineCreate.value.name.trim()) void submitInlineCreate();
  else cancelInlineCreate();
}

function beginRenameFolder(): void {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (item?.meta.kind !== "folder" || !item.meta.explorerNode || isKnowledgeTypeFolder(item)) return;
  folderDialog.value = {
    mode: "rename",
    projectId: item.meta.projectId,
    nodeId: item.meta.explorerNode.nodeId,
    value: item.meta.explorerNode.folderName ?? "",
  };
  void nextTick(() => folderInput.value?.select());
}

function beginDeleteFolder(): void {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (item?.meta.kind !== "folder" || !item.meta.explorerNode || isKnowledgeTypeFolder(item)) return;
  folderDialog.value = {
    mode: "delete",
    projectId: item.meta.projectId,
    nodeId: item.meta.explorerNode.nodeId,
    value: item.meta.explorerNode.folderName ?? "",
  };
}

async function commitFolderDialog(): Promise<void> {
  const dialog = folderDialog.value;
  if (!dialog) return;
  try {
    if (dialog.mode === "rename" && dialog.nodeId) {
      if (!dialog.value.trim()) return;
      await explorerStore.applyOperations(dialog.projectId, [{
        kind: "renameFolder",
        nodeId: dialog.nodeId,
        name: dialog.value.trim(),
      }]);
    } else if (dialog.mode === "delete" && dialog.nodeId) {
      await explorerStore.applyOperations(dialog.projectId, [{
        kind: "deleteFolder",
        nodeId: dialog.nodeId,
      }]);
    }
    folderDialog.value = null;
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function contextLayoutTarget(): { projectId: string; parentNodeId: string | null } | null {
  const item = contextMenu.value?.item;
  if (!item) return null;
  if (item.meta.kind === "folder" && item.meta.explorerNode) {
    return { projectId: item.meta.projectId, parentNodeId: item.meta.explorerNode.nodeId };
  }
  return { projectId: item.meta.projectId, parentNodeId: null };
}

async function mountPaths(
  projectId: string,
  parentNodeId: string | null,
  files: LocusFileDropRef[],
  forcedSourceKind?: "local" | "knowledge",
  position?: number,
): Promise<void> {
  const valid = files.filter((file) => file.path.trim());
  if (!valid.length) return;
  const snapshot = explorerStore.snapshots[projectId];
  const startPosition = position ?? snapshot?.nodes.filter(
    (node) => (node.parentNodeId ?? null) === parentNodeId,
  ).length ?? 0;
  await explorerStore.applyOperations(projectId, valid.map((file, index) => {
    const normalized = file.path.replace(/\\/g, "/");
    const sourceKind = forcedSourceKind
      ?? (/\/Locus\/knowledge(?:\/|$)/i.test(normalized) ? "knowledge" : "local");
    return {
      kind: "mountPath" as const,
      parentNodeId,
      path: file.path,
      sourceKind,
      name: file.name ?? null,
      position: startPosition + index,
    };
  }));
  for (const file of valid) {
    if (!file.isDir) continue;
    const node = explorerStore.snapshots[projectId]?.nodes.find((candidate) => (
      candidate.sourcePath?.replace(/\\/g, "/").toLocaleLowerCase()
        === file.path.replace(/\\/g, "/").toLocaleLowerCase()
    ));
    if (!node) continue;
    expanded.value = new Set([...expanded.value, `folder:${projectId}:${node.nodeId}`]);
    void explorerStore.loadMount(projectId, node.nodeId);
  }
}

async function addLocalFiles(): Promise<void> {
  const target = contextLayoutTarget();
  contextMenu.value = null;
  if (!target) return;
  const selected = await open({ multiple: true, directory: false });
  const paths = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
  try {
    await mountPaths(target.projectId, target.parentNodeId, paths.map((path) => ({
      path,
      isDir: false,
      source: "local",
    })));
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function mountKnowledgeFolder(): Promise<void> {
  const target = contextLayoutTarget();
  contextMenu.value = null;
  if (!target) return;
  const selected = await open({ multiple: false, directory: true });
  if (typeof selected !== "string" || !selected.trim()) return;
  try {
    await mountPaths(target.projectId, target.parentNodeId, [{
      path: selected,
      isDir: true,
      source: "knowledge",
    }], "knowledge");
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function setContextNodeHidden(hidden: boolean): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  const node = item?.meta.explorerNode;
  if (!item || !node) return;
  try {
    await explorerStore.applyOperations(item.meta.projectId, [{
      kind: "setNodeHidden",
      nodeId: node.nodeId,
      hidden,
    }]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function removeContextMountedNode(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  const node = item?.meta.explorerNode;
  if (!item || !node?.sourcePath) return;
  try {
    await explorerStore.applyOperations(item.meta.projectId, [{
      kind: "removeNode",
      nodeId: node.nodeId,
    }]);
    if (activeResource.value?.kind === "localFile" && activeResource.value.nodeId === node.nodeId) {
      activeResource.value = null;
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function developmentTreeItemAt(element: Element | null): DevelopmentTreeItem | null {
  const row = element?.closest<HTMLElement>("[data-tree-key]");
  const key = row?.dataset.treeKey;
  return key ? treeItems.value.find((candidate) => candidate.key === key) ?? null : null;
}

function nearestExternalDropTarget(element: Element | null): DevelopmentTreeItem | null {
  const item = developmentTreeItemAt(element);
  if (
    item?.meta.kind === "folder"
    || item?.meta.kind === "project"
    || item?.meta.kind === "newSession"
  ) return item;
  const projectId = item?.meta.projectId
    ?? activeResource.value?.projectId
    ?? presetProjectId.value;
  if (!projectId) return null;
  const parentNodeId = item?.meta.explorerNode?.parentNodeId;
  if (parentNodeId) {
    const parent = treeItems.value.find((candidate) => (
      candidate.meta.explorerNode?.nodeId === parentNodeId
    ));
    if (parent) return parent;
  }
  return treeItems.value.find((candidate) => (
    candidate.meta.kind === "project" && candidate.meta.projectId === projectId
  )) ?? {
    key: `root-context:${projectId}`,
    treeRow: null,
    meta: { kind: "project", projectId },
  };
}

function handleLocusFileDragState(payload: LocusFileDragStatePayload): void {
  locusFileWorkspaceDragActive.value = payload.active && payload.phase !== "leave";
  if (payload.fileCount > 0) locusFileWorkspaceDragCount.value = payload.fileCount;
  if (payload.active) updateWorkspaceDragPointer(payload.x, payload.y);
  if (payload.phase === "leave") {
    locusFileWorkspaceDragCount.value = 0;
    clearWorkspaceDragPointer();
    if (unityAssetWorkspaceDragRefs.value.length > 0) {
      window.clearTimeout(unityWorkspaceDragStateClearTimer);
      unityWorkspaceDragStateClearTimer = window.setTimeout(() => {
        unityWorkspaceDragStateClearTimer = 0;
        unityAssetWorkspaceDragActive.value = false;
        unityAssetWorkspaceDragRefs.value = [];
      }, UNITY_WORKSPACE_DRAG_STATE_TTL_MS);
    }
    externalDropTarget.value = null;
    dropTargetKey.value = null;
    layoutDropIntent.value = null;
    return;
  }
  const explorer = document.querySelector<HTMLElement>(".development-explorer");
  const bounds = explorer?.getBoundingClientRect();
  if (!bounds
    || payload.x < bounds.left
    || payload.x > bounds.right
    || payload.y < bounds.top
    || payload.y > bounds.bottom) {
    externalDropTarget.value = null;
    dropTargetKey.value = null;
    layoutDropIntent.value = null;
    return;
  }
  const hit = document.elementFromPoint(payload.x, payload.y);
  const exactTarget = developmentTreeItemAt(hit);
  if (exactTarget?.meta.kind === "dropPreview") return;
  const target = nearestExternalDropTarget(hit);
  externalDropTarget.value = target;
  if (exactTarget?.meta.kind === "newSession") {
    layoutDropIntent.value = null;
    dropTargetKey.value = exactTarget.key;
    externalDropTarget.value = exactTarget;
    return;
  }
  const row = hit?.closest<HTMLElement>("[data-tree-key]") ?? null;
  const intent = exactTarget
    ? resolveLayoutDropIntentAt(exactTarget, payload.y, row)
    : resolveExplorerRootDropIntent();
  if (intent) activateLayoutDropIntent(intent, exactTarget);
  else dropTargetKey.value = target?.key ?? null;
}

async function handleLocusFileDrop(payload: LocusFileDropPayload): Promise<void> {
  const target = externalDropTarget.value;
  const intent = layoutDropIntent.value;
  externalDropTarget.value = null;
  dropTargetKey.value = null;
  layoutDropIntent.value = null;
  locusFileWorkspaceDragActive.value = false;
  locusFileWorkspaceDragCount.value = 0;
  clearWorkspaceDragPointer();
  if ((!target && !intent) || !payload.files.length) return;
  if (target?.meta.kind === "newSession") {
    await createNewSessionWithAttachments(target, attachmentDraft({
      localFiles: payload.files,
    }));
    return;
  }
  const projectId = intent?.projectId ?? target!.meta.projectId;
  const parentNodeId = intent?.parentNodeId
    ?? (target?.meta.kind === "folder" ? target.meta.explorerNode?.nodeId ?? null : null);
  try {
    await mountPaths(projectId, parentNodeId, payload.files, undefined, intent?.position);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function handleWorkspaceUnityAssetDrop(
  payload: UnityEmbedAssetDropPayload,
): Promise<void> {
  const target = externalDropTarget.value;
  const intent = layoutDropIntent.value;
  externalDropTarget.value = null;
  dropTargetKey.value = null;
  layoutDropIntent.value = null;
  window.clearTimeout(unityWorkspaceDragStateClearTimer);
  unityWorkspaceDragStateClearTimer = 0;
  unityAssetWorkspaceDragActive.value = false;
  unityAssetWorkspaceDragRefs.value = [];
  clearWorkspaceDragPointer();
  if ((!target && !intent) || !payload.refs.length) return;
  if (target?.meta.kind === "newSession") {
    await createNewSessionWithAttachments(target, attachmentDraft({
      assetRefs: payload.refs,
    }));
    return;
  }
  const projectId = intent?.projectId ?? target!.meta.projectId;
  const project = workspaceContextStore.projectsById[projectId];
  const checkout = workspaceContextStore.focusedCheckout?.projectId === projectId
    ? workspaceContextStore.focusedCheckout
    : project?.checkouts[0];
  if (!checkout) return;
  const parentNodeId = intent?.parentNodeId
    ?? (target?.meta.kind === "folder" ? target.meta.explorerNode?.nodeId ?? null : null);
  const root = checkout.root.replace(/[\\/]+$/, "");
  try {
    await mountPaths(projectId, parentNodeId, payload.refs.map((asset) => ({
      path: `${root}/${asset.path.replace(/^\/+/, "")}`,
      name: asset.name,
      typeLabel: asset.typeLabel,
      isDir: false,
      source: "unity",
    })), undefined, intent?.position);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function onDragPointerDown(raw: WorkspaceTreeItem, event: PointerEvent): void {
  const item = raw as DevelopmentTreeItem;
  if (settlingLayoutDrop.value || !item.meta.explorerNode || isKnowledgeTypeFolder(item)) return;
  internalDrag.start(event, {
    id: `workspace-layout:${item.meta.projectId}:${item.meta.explorerNode.nodeId}`,
    payload: {
      type: WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE,
      data: { item } satisfies WorkspaceLayoutInternalDragData,
    },
    preview: {
      label: item.treeRow?.name ?? "",
      kind: item.treeRow?.kind ?? "item",
      icon: itemIcon(item),
      iconClass: itemIconClass(item),
    },
    allowedOperations: ["move"],
    onActivated: () => {
      contextMenu.value = null;
      displayMenu.value = null;
      workspaceMenu.value = null;
    },
    onFinished: () => {
      dropTargetKey.value = null;
      layoutDropIntent.value = null;
    },
  });
}

function resolveLayoutDropIntent(
  target: DevelopmentTreeItem,
  event: DragEvent,
): LayoutDropIntent | null {
  return resolveLayoutDropIntentAt(
    target,
    event.clientY,
    event.currentTarget as HTMLElement | null,
  );
}

function resolveLayoutDropIntentAt(
  target: DevelopmentTreeItem,
  clientY: number,
  rowElement: HTMLElement | null,
): LayoutDropIntent | null {
  if (target.meta.kind === "mountedFolder" || target.meta.kind === "mountedFile") return null;
  const snapshot = explorerStore.snapshots[target.meta.projectId];
  if (!snapshot) return null;
  if (target.meta.kind === "empty" && target.meta.dropParentNodeId) {
    return {
      projectId: target.meta.projectId,
      parentNodeId: target.meta.dropParentNodeId,
      position: snapshot.nodes.filter(
        (node) => node.parentNodeId === target.meta.dropParentNodeId,
      ).length,
      targetKey: target.key,
    };
  }
  if (!target.meta.explorerNode || target.meta.kind === "project") {
    return {
      projectId: target.meta.projectId,
      parentNodeId: null,
      position: snapshot.nodes.filter((node) => !node.parentNodeId).length,
      targetKey: target.key,
    };
  }
  const targetNode = target.meta.explorerNode;
  const bounds = rowElement?.getBoundingClientRect();
  const ratio = bounds && bounds.height > 0
    ? (clientY - bounds.top) / bounds.height
    : 0.5;
  if (target.meta.kind === "folder" && ratio >= 0.25 && ratio <= 0.75) {
    return {
      projectId: target.meta.projectId,
      parentNodeId: targetNode.nodeId,
      position: snapshot.nodes.filter((node) => node.parentNodeId === targetNode.nodeId).length,
      targetKey: target.key,
    };
  }
  const parentNodeId = targetNode.parentNodeId ?? null;
  const siblings = snapshot.nodes
    .filter((node) => (node.parentNodeId ?? null) === parentNodeId)
    .sort((left, right) => left.position - right.position);
  const targetIndex = Math.max(0, siblings.findIndex((node) => node.nodeId === targetNode.nodeId));
  return {
    projectId: target.meta.projectId,
    parentNodeId,
    position: targetIndex + (ratio > 0.5 ? 1 : 0),
    targetKey: target.key,
  };
}

function resolveExplorerRootDropIntent(): LayoutDropIntent | null {
  const projectId = presetProjectId.value;
  const snapshot = projectId ? explorerStore.snapshots[projectId] : null;
  if (!projectId || !snapshot) return null;
  return {
    projectId,
    parentNodeId: null,
    position: snapshot.nodes.filter((node) => !node.parentNodeId).length,
    targetKey: `explorer-root:${projectId}`,
  };
}

function canMoveExplorerNodeToIntent(
  source: DevelopmentTreeItem,
  intent: LayoutDropIntent,
): boolean {
  const sourceNode = source.meta.explorerNode;
  const snapshot = explorerStore.snapshots[source.meta.projectId];
  if (!sourceNode || !snapshot || source.meta.projectId !== intent.projectId) return false;
  let parentNodeId = intent.parentNodeId;
  while (parentNodeId) {
    if (parentNodeId === sourceNode.nodeId) return false;
    parentNodeId = snapshot.nodes.find((node) => node.nodeId === parentNodeId)?.parentNodeId ?? null;
  }
  return true;
}

function activateLayoutDropIntent(
  intent: LayoutDropIntent,
  target?: DevelopmentTreeItem | null,
): void {
  layoutDropIntent.value = intent;
  dropTargetKey.value = (
    target?.meta.kind === "folder"
    && target.meta.explorerNode?.nodeId === intent.parentNodeId
  )
    ? target.key
    : null;
  if (
    target?.meta.kind === "folder"
    && target.meta.explorerNode?.nodeId === intent.parentNodeId
  ) {
    expanded.value = new Set([...expanded.value, target.key]);
  }
}

function handleUnityAssetWorkspaceDragState(
  payload: UnityEmbedAssetDragStatePayload,
): void {
  const refs = Array.isArray(payload.refs) ? payload.refs : [];
  window.clearTimeout(unityWorkspaceDragStateClearTimer);
  unityWorkspaceDragStateClearTimer = 0;
  if (!payload.hasRefs || refs.length === 0) {
    unityAssetWorkspaceDragActive.value = false;
    unityAssetWorkspaceDragRefs.value = [];
    externalDropTarget.value = null;
    return;
  }
  unityAssetWorkspaceDragActive.value = true;
  unityAssetWorkspaceDragRefs.value = refs;
  unityWorkspaceDragStateClearTimer = window.setTimeout(() => {
    unityWorkspaceDragStateClearTimer = 0;
    if (locusFileWorkspaceDragActive.value) return;
    unityAssetWorkspaceDragActive.value = false;
    unityAssetWorkspaceDragRefs.value = [];
    if (!locusFileWorkspaceDragActive.value) externalDropTarget.value = null;
  }, UNITY_WORKSPACE_DRAG_STATE_TTL_MS);
}

function attachmentDraft(params: {
  assetRefs?: AssetRefAttachment[];
  localFiles?: LocusFileDropRef[];
}): UserMessageDraft {
  return {
    text: "",
    images: [],
    assetRefs: params.assetRefs ?? [],
    localFiles: (params.localFiles ?? []).map((file) => ({
      path: file.path,
      isDir: file.isDir,
      name: file.name,
      typeLabel: file.typeLabel,
      source: file.source,
    })),
    consoleTexts: [],
    intent: emptyComposerIntent(),
  };
}

function knowledgeDragAssetRefs(
  payload: KnowledgeWorkspaceDragPayload,
): AssetRefAttachment[] {
  return payload.entries.map((entry) => ({
    path: entry.path,
    kind: "knowledge" as const,
    name: entry.name,
    source: "manual" as const,
  }));
}

async function createNewSessionWithAttachments(
  target: DevelopmentTreeItem,
  draft: UserMessageDraft,
): Promise<void> {
  const project = workspaceContextStore.projectsById[target.meta.projectId];
  if (!project) return;
  const checkout = await ensureProjectCheckout(project, target.meta.checkoutId);
  if (!checkout) return;
  chatStore.newChat();
  activeResource.value = {
    kind: "newSession",
    projectId: project.projectId,
    checkoutId: checkout.checkoutId,
  };
  await nextTick();
  uiStore.stageChatDraftPrefill(draft, {
    sessionId: null,
    requireEmptyComposer: true,
  });
}

async function placeKnowledgeWorkspaceDrag(
  intent: LayoutDropIntent,
  payload: KnowledgeWorkspaceDragPayload,
): Promise<void> {
  const project = workspaceContextStore.projectsById[intent.projectId];
  const checkout = workspaceContextStore.focusedCheckout?.projectId === intent.projectId
    ? workspaceContextStore.focusedCheckout
    : project?.checkouts[0];
  if (!checkout) return;
  const root = checkout.root.replace(/[\\/]+$/, "");
  const operations = payload.entries.map((entry, index) => {
    if (entry.kind === "document") {
      return {
        kind: "placeResource" as const,
        resourceKind: "knowledge" as const,
        resourceId: entry.documentId!,
        sourceKind: "knowledge",
        parentNodeId: intent.parentNodeId,
        position: intent.position + index,
      };
    }
    const relativePath = (entry.relativePath ?? "").replace(/^\/+|\/+$/g, "");
    return {
      kind: "mountPath" as const,
      parentNodeId: intent.parentNodeId,
      path: `${root}/Locus/knowledge/${entry.type}/${relativePath}`,
      sourceKind: "knowledge" as const,
      name: entry.name,
      position: intent.position + index,
    };
  });
  const snapshot = await explorerStore.applyOperations(intent.projectId, operations);
  for (const operation of operations) {
    if (operation.kind !== "mountPath") continue;
    const normalized = operation.path.replace(/\\/g, "/").toLocaleLowerCase();
    const node = snapshot.nodes.find((candidate) => (
      candidate.sourcePath?.replace(/\\/g, "/").toLocaleLowerCase() === normalized
    ));
    if (!node) continue;
    expanded.value = new Set([...expanded.value, `folder:${intent.projectId}:${node.nodeId}`]);
    void explorerStore.loadMount(intent.projectId, node.nodeId);
  }
}

function developmentTreeItemFromHit(hit: Element): {
  item: DevelopmentTreeItem;
  rowElement: HTMLElement;
} | null {
  const rowElement = hit.closest<HTMLElement>(".workspace-tree-row-shell");
  if (!rowElement || !explorerRootRef.value?.contains(rowElement)) return null;
  const item = treeItems.value.find((candidate) => candidate.key === rowElement.dataset.treeKey);
  return item ? { item, rowElement } : null;
}

function resolveWorkbenchInternalDrop(
  context: InternalDropResolveContext<WorkspaceLayoutInternalDragData | KnowledgeInternalDragData>,
): InternalDropDecision<WorkbenchInternalDropIntent> | null {
  if (!explorerRootRef.value?.contains(context.hit)) return null;
  const rowHit = developmentTreeItemFromHit(context.hit);
  const sourceType = context.source.payload.type;
  if (rowHit?.item.meta.kind === "dropPreview") {
    const intent = layoutDropIntent.value;
    return intent ? {
      key: `layout:${intent.targetKey}:${intent.position}`,
      operation: sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE ? "move" : "copy",
      intent: { kind: "layout", layout: intent, target: null },
    } : null;
  }

  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    const source = (context.source.payload.data as WorkspaceLayoutInternalDragData).item;
    const intent = rowHit
      ? resolveLayoutDropIntentAt(rowHit.item, context.point.y, rowHit.rowElement)
      : resolveExplorerRootDropIntent();
    if (!intent || source.meta.projectId !== intent.projectId || !canMoveExplorerNodeToIntent(source, intent)) {
      return null;
    }
    return {
      key: `layout:${intent.targetKey}:${intent.position}`,
      operation: "move",
      intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
    };
  }

  if (sourceType !== KNOWLEDGE_INTERNAL_DRAG_TYPE) return null;
  if (rowHit?.item.meta.kind === "newSession") {
    return {
      key: `new-session:${rowHit.item.key}`,
      operation: "copy",
      intent: { kind: "newSession", target: rowHit.item },
    };
  }
  const intent = rowHit
    ? resolveLayoutDropIntentAt(rowHit.item, context.point.y, rowHit.rowElement)
    : resolveExplorerRootDropIntent();
  if (!intent) return null;
  return {
    key: `layout:${intent.targetKey}:${intent.position}`,
    operation: "copy",
    intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
  };
}

function handleWorkbenchInternalTargetChange(
  decision: InternalDropDecision<WorkbenchInternalDropIntent> | null,
): void {
  if (!decision) {
    layoutDropIntent.value = null;
    dropTargetKey.value = null;
    return;
  }
  if (decision.intent.kind === "newSession") {
    layoutDropIntent.value = null;
    dropTargetKey.value = decision.intent.target.key;
    return;
  }
  activateLayoutDropIntent(decision.intent.layout, decision.intent.target);
}

async function commitWorkbenchInternalDrop(
  sourceType: string,
  sourceData: WorkspaceLayoutInternalDragData | KnowledgeInternalDragData,
  intent: WorkbenchInternalDropIntent,
): Promise<void> {
  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    if (intent.kind === "layout") {
      await moveExplorerNodeToIntent((sourceData as WorkspaceLayoutInternalDragData).item, intent.layout);
    }
    return;
  }
  if (sourceType !== KNOWLEDGE_INTERNAL_DRAG_TYPE) return;
  const payload = (sourceData as KnowledgeInternalDragData).payload;
  if (intent.kind === "newSession") {
    await createNewSessionWithAttachments(intent.target, attachmentDraft({
      assetRefs: knowledgeDragAssetRefs(payload),
    }));
    return;
  }
  await placeKnowledgeWorkspaceDrag(intent.layout, payload);
}

const workbenchInternalDropTarget: InternalDropTargetRegistration<
  WorkspaceLayoutInternalDragData | KnowledgeInternalDragData,
  WorkbenchInternalDropIntent
> = {
  id: "development-workbench",
  root: () => workbenchRootRef.value,
  accepts: (source) => source.payload.type === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
    || source.payload.type === KNOWLEDGE_INTERNAL_DRAG_TYPE,
  resolve: resolveWorkbenchInternalDrop,
  onTargetChange: handleWorkbenchInternalTargetChange,
  drop: async ({ source, decision }) => {
    const settlingId = ++settlingLayoutDropId;
    if (
      source.payload.type === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      && decision.intent.kind === "layout"
    ) {
      settlingLayoutDrop.value = {
        id: settlingId,
        source: (source.payload.data as WorkspaceLayoutInternalDragData).item,
        intent: decision.intent.layout,
        preview: workspaceDragPreviewForInternalSource(source),
      };
    }
    try {
      await commitWorkbenchInternalDrop(source.payload.type, source.payload.data, decision.intent);
    } catch (error) {
      notificationStore.addNotice("error", normalizeAppError(error).message);
    } finally {
      if (settlingLayoutDrop.value?.id === settlingId) {
        settlingLayoutDrop.value = null;
      }
    }
  },
  previewMode: ({ hit }) => explorerRootRef.value?.contains(hit) ? "floating-with-gap" : "floating",
  priority: 10,
};

function onExternalRowDragOver(raw: WorkspaceTreeItem, event: DragEvent): void {
  const target = raw as DevelopmentTreeItem;
  const types = Array.from(event.dataTransfer?.types ?? []);
  if (!types.includes("Files") && !unityAssetWorkspaceDragActive.value) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
  externalDropTarget.value = (
    target.meta.kind === "folder"
    || target.meta.kind === "project"
    || target.meta.kind === "newSession"
  )
    ? target
    : nearestExternalDropTarget(event.currentTarget as Element | null);
  if (target.meta.kind === "newSession") {
    layoutDropIntent.value = null;
    dropTargetKey.value = target.key;
    return;
  }
  const intent = resolveLayoutDropIntent(target, event);
  if (intent) activateLayoutDropIntent(intent, target);
  else dropTargetKey.value = externalDropTarget.value?.key ?? null;
}

async function moveExplorerNodeToIntent(
  source: DevelopmentTreeItem,
  intent: LayoutDropIntent,
): Promise<void> {
  if (!source.meta.explorerNode || source.meta.projectId !== intent.projectId) return;
  const sourceParentNodeId = source.meta.explorerNode.parentNodeId ?? null;
  const position = sourceParentNodeId === intent.parentNodeId
    && source.meta.explorerNode.position < intent.position
    ? Math.max(0, intent.position - 1)
    : intent.position;
  try {
    await explorerStore.applyOperations(source.meta.projectId, [{
      kind: "moveNode",
      nodeId: source.meta.explorerNode.nodeId,
      parentNodeId: intent.parentNodeId,
      position,
    }]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function onExplorerDragOver(event: DragEvent): void {
  const types = Array.from(event.dataTransfer?.types ?? []);
  const externalAssetDrag = types.includes("Files") || unityAssetWorkspaceDragActive.value;
  if (!externalAssetDrag) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
  const target = event.target;
  if (target instanceof Element && target.closest(".workspace-tree-row-shell")) return;
  externalDropTarget.value = nearestExternalDropTarget(
    target instanceof Element ? target : null,
  );
  const intent = resolveExplorerRootDropIntent();
  if (intent) activateLayoutDropIntent(intent);
  else dropTargetKey.value = externalDropTarget.value?.key ?? null;
}

function onExplorerDragLeave(event: DragEvent): void {
  const current = event.currentTarget as HTMLElement | null;
  const related = event.relatedTarget as Node | null;
  if (current && related && current.contains(related)) return;
  const bounds = current?.getBoundingClientRect();
  if (
    bounds
    && event.clientX >= bounds.left
    && event.clientX <= bounds.right
    && event.clientY >= bounds.top
    && event.clientY <= bounds.bottom
  ) return;
  layoutDropIntent.value = null;
  dropTargetKey.value = null;
  externalDropTarget.value = null;
}

function onExplorerDrop(event: DragEvent): void {
  const types = Array.from(event.dataTransfer?.types ?? []);
  if (!types.includes("Files") && !unityAssetWorkspaceDragActive.value) return;
  event.preventDefault();
}

async function browseWorkspace(): Promise<void> {
  contextMenu.value = null;
  workspaceMenu.value = null;
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected !== "string" || !selected.trim()) return;
  try {
    await workspaceContextStore.openAndFocus(selected);
    await refreshFocusedCheckoutServices();
    const project = workspaceContextStore.focusedProject;
    if (project) {
      expanded.value = new Set([
        ...expanded.value,
        `project:${project.projectId}`,
      ]);
      await explorerStore.loadProject(project.projectId, true);
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function setWorkspaceMode(mode: "single" | "multi"): void {
  setDisplaySetting("workspaceDisplayMode", mode);
  displayMenu.value = null;
  workspaceMenu.value = null;
}

const activeProject = computed(() => {
  const projectId = activeResource.value?.projectId;
  return projectId ? workspaceContextStore.projectsById[projectId] ?? null : null;
});

const selectedKnowledge = computed(() => {
  if (activeResource.value?.kind !== "knowledge") return null;
  const resource = activeResource.value;
  return explorerStore.resources[resource.projectId]?.knowledge.find(
    (document) => document.id === resource.documentId,
  ) ?? null;
});

const showChat = computed(() => activeResource.value?.kind === "session"
  || activeResource.value?.kind === "newSession");
const showKnowledge = computed(() => activeResource.value?.kind === "knowledge"
  || activeResource.value?.kind === "knowledgeRoot");
const showCollab = computed(() => activeResource.value?.kind === "collaboration"
  || activeResource.value?.kind === "checkout");

async function activateCheckoutOverview(checkoutId: string): Promise<void> {
  const item = treeItems.value.find((candidate) => candidate.meta.checkoutId === checkoutId);
  if (item) await activateItem(item);
}

watch(
  [visibleProjects, () => workspaceContextStore.focusedProject?.projectId] as const,
  ([projects]) => {
    const next = new Set(expanded.value);
    for (const project of projects) {
      next.add(`project:${project.projectId}`);
      void explorerStore.loadProject(project.projectId);
    }
    expanded.value = next;
    if (!activeResource.value && projects[0]) {
      const checkout = projects[0].checkouts[0];
      if (checkout) {
        activeResource.value = {
          kind: "newSession",
          projectId: projects[0].projectId,
          checkoutId: checkout.checkoutId,
        };
      }
    }
  },
  { immediate: true },
);

watch(
  () => chatStore.sessions,
  () => {
    const projectId = workspaceContextStore.focusedProject?.projectId;
    if (!projectId) return;
    void explorerStore.refreshProjectSessions(projectId).catch((error) => {
      console.warn("[DevelopmentWorkbench] session catalog refresh failed", error);
    });
  },
);

watch(
  [
    () => chatStore.activeSessionId,
    () => workspaceContextStore.focusedProject?.projectId ?? null,
  ] as const,
  ([sessionId, projectId]) => {
    const current = activeResource.value;
    if (current && current.kind !== "session" && current.kind !== "newSession") return;
    if (!projectId) return;
    if (!sessionId) {
      if (current?.kind === "session") resetActiveSessionResource(projectId);
      return;
    }
    const session = chatStore.sessions.find((candidate) => candidate.id === sessionId);
    if (!session || (session.projectId && session.projectId !== projectId)) return;
    const project = workspaceContextStore.projectsById[projectId];
    const preferredCheckout = project?.checkouts.find((candidate) => (
      candidate.checkoutId === session.executionTarget?.checkoutId
      || candidate.checkoutId === session.defaultCheckoutId
    ));
    const checkout = preferredCheckout
      ?? (workspaceContextStore.focusedCheckout?.projectId === projectId
        ? workspaceContextStore.focusedCheckout
        : project?.checkouts[0]);
    if (!checkout) return;
    activeResource.value = {
      kind: "session",
      projectId,
      sessionId,
      checkoutId: checkout.checkoutId,
    };
  },
  { immediate: true },
);

watch(activeResource, (resource) => {
  if (resource?.kind !== "checkout") {
    collabHeadFocusRequest.value = null;
  }
});

onMounted(() => {
  unregisterWorkbenchInternalDropTarget = internalDrag.registerTarget(workbenchInternalDropTarget);
  document.addEventListener("pointerdown", handleInlineCreatePointerDown, true);
  window.addEventListener("drag", trackWorkspaceDragPointer, true);
  window.addEventListener("dragenter", trackWorkspaceDragPointer, true);
  window.addEventListener("dragover", trackWorkspaceDragPointer, true);
  window.addEventListener("drop", handleWindowWorkspaceDrop, true);
  window.addEventListener("dragend", clearWorkspaceDragPointer, true);
  for (const project of visibleProjects.value) void explorerStore.loadProject(project.projectId);
  void subscribeLocusFileDragState(handleLocusFileDragState).then((release) => {
    releaseLocusFileDragState = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] file drag subscription failed", error);
  });
  void subscribeLocusFileDrop((payload) => {
    void handleLocusFileDrop(payload);
  }).then((release) => {
    releaseLocusFileDrop = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] file drop subscription failed", error);
  });
  void subscribeUnityEmbedAssetDrop((payload) => {
    void handleWorkspaceUnityAssetDrop(payload);
  }).then((release) => {
    releaseUnityAssetDrop = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] Unity asset drop subscription failed", error);
  });
  void subscribeUnityEmbedAssetDragState(handleUnityAssetWorkspaceDragState)
    .then((release) => {
      releaseUnityAssetDragState = release;
    })
    .catch((error) => {
      console.warn("[DevelopmentWorkbench] Unity asset drag subscription failed", error);
    });
});

onUnmounted(() => {
  unregisterWorkbenchInternalDropTarget?.();
  unregisterWorkbenchInternalDropTarget = null;
  document.removeEventListener("pointerdown", handleInlineCreatePointerDown, true);
  window.removeEventListener("drag", trackWorkspaceDragPointer, true);
  window.removeEventListener("dragenter", trackWorkspaceDragPointer, true);
  window.removeEventListener("dragover", trackWorkspaceDragPointer, true);
  window.removeEventListener("drop", handleWindowWorkspaceDrop, true);
  window.removeEventListener("dragend", clearWorkspaceDragPointer, true);
  window.clearTimeout(unityWorkspaceDragStateClearTimer);
  unityWorkspaceDragStateClearTimer = 0;
  onExplorerResizeEnd();
  releaseLocusFileDragState?.();
  releaseLocusFileDragState = null;
  releaseLocusFileDrop?.();
  releaseLocusFileDrop = null;
  releaseUnityAssetDrop?.();
  releaseUnityAssetDrop = null;
  releaseUnityAssetDragState?.();
  releaseUnityAssetDragState = null;
});

watch(
  () => uiStore.pendingKnowledgeSelection?.id ?? null,
  (selectionId) => {
    if (selectionId) void revealPendingKnowledgeSelection();
  },
  { immediate: true },
);
</script>

<template>
  <div
    ref="workbenchRootRef"
    class="development-workbench"
    @dragover.capture="trackWorkspaceDragPointer"
    @dragend.capture="clearWorkspaceDragPointer"
    @drop.capture="clearWorkspaceDragPointer"
  >
    <Teleport to="body">
      <div
        v-if="workspaceDragPointer.visible && workspaceDragPreview"
        class="workspace-drag-floating-preview"
        :style="workspaceDragFloatingStyle"
        aria-hidden="true"
      >
        <LucideIcon
          class="workspace-drag-floating-icon"
          :class="workspaceDragPreview.iconClass"
          :icon="workspaceDragPreview.icon"
          :size="14"
          :stroke-width="2"
        />
        <span class="workspace-drag-floating-name">
          {{ dragPreviewLabel(workspaceDragPreview) }}
        </span>
      </div>
    </Teleport>
    <aside
      ref="explorerRootRef"
      class="development-explorer"
      :class="{ 'is-workspace-drop-target': workspaceDropAffordanceActive }"
      :style="{ width: `${explorerWidth}px` }"
      @contextmenu="openExplorerBackgroundContextMenu"
      @dragenter.capture="onExplorerDragOver"
      @dragover.capture="onExplorerDragOver"
      @dragleave="onExplorerDragLeave"
      @drop="onExplorerDrop"
    >
      <div class="development-explorer-toolbar">
        <button
          v-if="displaySettings.workspaceDisplayMode === 'single'"
          type="button"
          class="development-explorer-label development-workspace-trigger"
          :title="explorerHeaderTitle"
          @click="toggleWorkspaceMenu"
        >
          {{ explorerHeaderLabel }}
        </button>
        <span v-else class="development-explorer-label">{{ explorerHeaderLabel }}</span>
        <div class="development-explorer-actions">
          <button
            type="button"
            :title="t('common.more')"
            @click="toggleDisplayMenu"
          >
            <LucideIcon :icon="MoreHorizontal" :size="15" />
          </button>
        </div>
      </div>

      <WorkspaceTree
        class="development-tree"
        :items="treeItems"
        :row-height="30"
        :base-indent="12"
        :indent-size="14"
        @activate="activateItem"
        @contextmenu="openContextMenu"
        @drag-pointer-down="onDragPointerDown"
        @dragover="onExternalRowDragOver"
      >
        <template #icon="{ item }">
          <span
            v-if="(item as DevelopmentTreeItem).meta.kind === 'empty'"
            class="development-empty-folder-icon"
            aria-hidden="true"
          />
          <LucideIcon
            v-else
            :class="itemIconClass(item as DevelopmentTreeItem)"
            :icon="itemIcon(item as DevelopmentTreeItem)"
            :size="13"
            :stroke-width="2"
          />
        </template>
        <template #name="{ item, row }">
          <span
            v-if="(item as DevelopmentTreeItem).meta.kind === 'session'"
            class="development-session-title"
            :class="{ 'is-running': isAnimatedSessionStatus(itemRuntimeStatus(item as DevelopmentTreeItem)) }"
            :data-title="isAnimatedSessionStatus(itemRuntimeStatus(item as DevelopmentTreeItem)) ? row.name : undefined"
          >{{ row.name }}</span>
          <span v-else>{{ row.name }}</span>
        </template>
        <template #trailing="{ item }">
          <span
            v-if="itemSessionIsPending(item as DevelopmentTreeItem)"
            class="development-session-spinner"
            :title="t('common.loading')"
            aria-hidden="true"
          />
          <span
            v-else-if="itemRuntimeStatus(item as DevelopmentTreeItem) && !isAnimatedSessionStatus(itemRuntimeStatus(item as DevelopmentTreeItem))"
            class="development-session-dot"
            :class="`is-${itemRuntimeStatus(item as DevelopmentTreeItem)}`"
            :title="sessionStatusLabel(itemRuntimeStatus(item as DevelopmentTreeItem))"
            aria-hidden="true"
          />
          <span
            v-if="itemRuntimeStatus(item as DevelopmentTreeItem) && itemRuntimeStatus(item as DevelopmentTreeItem) !== 'running'"
            class="development-session-status"
            :class="`is-${itemRuntimeStatus(item as DevelopmentTreeItem)}`"
          >
            {{ sessionStatusLabel(itemRuntimeStatus(item as DevelopmentTreeItem)) }}
          </span>
          <span
            v-if="(item as DevelopmentTreeItem).meta.kind === 'session' && sessionBranchLabel((item as DevelopmentTreeItem).meta.session)"
            class="development-branch-label"
            :title="(item as DevelopmentTreeItem).meta.session?.executionTarget?.branchRef || (item as DevelopmentTreeItem).meta.session?.executionTarget?.headOid || undefined"
          >
            {{ sessionBranchLabel((item as DevelopmentTreeItem).meta.session) }}
          </span>
          <span
            v-else-if="(item as DevelopmentTreeItem).meta.kind === 'checkout' && checkoutBranchLabel((item as DevelopmentTreeItem).meta.projectId, (item as DevelopmentTreeItem).meta.checkoutId)"
            class="development-branch-label"
          >
            {{ checkoutBranchLabel((item as DevelopmentTreeItem).meta.projectId, (item as DevelopmentTreeItem).meta.checkoutId) }}
          </span>
          <button
            v-if="(item as DevelopmentTreeItem).meta.kind === 'session'"
            type="button"
            class="development-session-archive-button"
            :title="t('chat.session.archive')"
            :aria-label="t('chat.session.archive')"
            @pointerdown.stop
            @click.stop="archiveSessionItem(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="Archive" :size="12" :stroke-width="2" />
          </button>
        </template>
        <template #custom="{ item }">
          <div
            v-if="(item as DevelopmentTreeItem).meta.kind === 'inlineCreate'"
            ref="inlineCreateRow"
            class="development-inline-create-row"
            :style="{
              paddingLeft: `${12 + ((item as DevelopmentTreeItem).meta.inlineCreateDepth ?? 0) * 14}px`,
            }"
          >
            <span class="development-inline-create-bullet" aria-hidden="true" />
            <div class="development-inline-create-body">
              <input
                ref="inlineCreateInput"
                v-model="inlineCreate!.name"
                class="development-inline-create-input"
                :placeholder="t('knowledge.explorer.namePlaceholder')"
                :aria-label="t('development.newFolder')"
                @keydown.enter.prevent="submitInlineCreate"
                @keydown.esc.prevent.stop="cancelInlineCreate"
              />
              <div class="development-inline-create-actions">
                <BaseButton
                  class="development-inline-create-action"
                  type="button"
                  :title="t('common.confirm')"
                  :disabled="!inlineCreate?.name.trim()"
                  @click="submitInlineCreate"
                >
                  <LucideIcon :icon="Check" :size="12" :stroke-width="2.4" />
                </BaseButton>
                <BaseButton
                  class="development-inline-create-action"
                  type="button"
                  :title="t('common.cancel')"
                  @click="cancelInlineCreate"
                >
                  <LucideIcon :icon="X" :size="12" :stroke-width="2.4" />
                </BaseButton>
              </div>
            </div>
          </div>
        </template>
        <template #empty>
          <div class="development-tree-empty">{{ t("development.empty") }}</div>
        </template>
      </WorkspaceTree>
    </aside>
    <div
      class="development-explorer-resize"
      :class="{ active: resizingExplorer }"
      role="separator"
      aria-orientation="vertical"
      @mousedown="onExplorerResizeStart"
    />

    <main class="development-editor">
      <ChatWorkspaceView
        v-if="showChat"
        ref="chatWorkspaceView"
        :active="true"
        :show-session-navigation="false"
        :persist-session-selection="true"
        layout-mode="auto"
      />
      <template v-if="workspaceContextStore.focusedWorkspaceRef">
        <KnowledgeView
          v-show="showKnowledge"
          :embedded="activeResource?.kind === 'knowledge'"
          :selected-document-id="selectedKnowledge?.id ?? null"
          :working-dir="workspaceContextStore.focusedRoot"
          :workspace-ref="workspaceContextStore.focusedWorkspaceRef"
          :selected-model-id="modelStore.selectedModelId"
          :model-defaults="modelStore.modelDefaults"
        />
        <CollabView
          v-show="showCollab"
          :working-dir="workspaceContextStore.focusedRoot"
          :workspace-ref="workspaceContextStore.focusedWorkspaceRef"
          :is-active="showCollab"
          :selected-model-id="modelStore.selectedModelId"
          :selected-agent-id="agentStore.selectedAgentId"
          :models="modelStore.availableModels"
          :head-focus-request="collabHeadFocusRequest"
          @select-model="(id: string) => modelStore.selectModel(id)"
        />
      </template>
      <WorkspaceFilePreview
        v-if="activeResource?.kind === 'localFile'"
        :project-id="activeResource.projectId"
        :path="activeResource.path"
      />
      <div
        v-if="!showChat && !showKnowledge && activeResource?.kind !== 'localFile' && !showCollab"
        class="development-overview"
      >
        <template v-if="activeResource?.kind === 'checkout'">
          <div class="development-overview-title">
            {{ shortPath(workspaceContextStore.checkoutsById[activeResource.checkoutId]?.root || activeResource.checkoutId) }}
          </div>
          <div class="development-overview-path">
            {{ workspaceContextStore.checkoutsById[activeResource.checkoutId]?.root || activeResource.checkoutId }}
          </div>
        </template>
        <template v-else-if="activeProject">
          <div class="development-overview-title">{{ projectLabel(activeProject) }}</div>
          <button
            v-for="checkout in activeProject.checkouts"
            :key="checkout.checkoutId"
            type="button"
            class="development-worktree-row"
            @click="activateCheckoutOverview(checkout.checkoutId)"
          >
            <LucideIcon :icon="GitBranch" :size="13" />
            <span>{{ shortPath(checkout.root) }}</span>
            <span>{{ checkout.root }}</span>
          </button>
        </template>
      </div>
    </main>

    <BaseContextMenu
      v-if="contextMenu"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :min-width="164"
      @close="contextMenu = null"
    >
      <template v-if="contextMenu.item.meta.kind === 'checkout'">
        <button type="button" @click="copyCheckoutMcpArtifact('endpoint')">
          <LucideIcon :icon="Copy" :size="13" />
          {{ t("app.dir.copyMcpEndpoint") }}
        </button>
        <button type="button" @click="copyCheckoutMcpArtifact('claude')">
          <LucideIcon :icon="Copy" :size="13" />
          {{ t("app.dir.copyMcpClaudeCommand") }}
        </button>
        <button type="button" @click="copyCheckoutMcpArtifact('json')">
          <LucideIcon :icon="Copy" :size="13" />
          {{ t("app.dir.copyMcpJson") }}
        </button>
        <button type="button" @click="openCheckoutInFileExplorer">
          <LucideIcon :icon="FolderOpen" :size="13" />
          {{ t("common.openInFileExplorer") }}
        </button>
        <button type="button" @click="configureCheckoutExtraWorkdirs">
          <LucideIcon :icon="FolderCog" :size="13" />
          {{ t("app.dir.configureExtraWorkdirs") }}
        </button>
      </template>
      <template v-else-if="contextMenu.item.meta.kind === 'session'">
        <button type="button" @click="beginRenameSession">
          <LucideIcon :icon="PencilLine" :size="13" />
          {{ t("chat.session.rename") }}
        </button>
        <button type="button" @click="contextOpenSessionWindow">
          <LucideIcon :icon="AppWindow" :size="13" />
          {{ t("chat.session.openInWindow") }}
        </button>
        <button type="button" @click="contextOpenSessionInUnity">
          <LucideIcon :icon="Box" :size="13" />
          {{ t("chat.session.openInUnity") }}
        </button>
        <button type="button" @click="exportContextSession">
          <LucideIcon :icon="Save" :size="13" />
          {{ t("chat.exportContext") }}
        </button>
        <button type="button" @click="reviewContextSession">
          <LucideIcon :icon="FileSearch" :size="13" />
          {{ t("chat.reviewContext") }}
        </button>
        <div class="base-context-menu-separator" />
        <button
          v-if="contextMenu.item.meta.explorerNode && !contextMenu.item.meta.explorerNode.hidden"
          type="button"
          @click="setContextNodeHidden(true)"
        >
          <LucideIcon :icon="EyeOff" :size="13" />
          {{ t("development.hideNode") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode?.hidden"
          type="button"
          @click="setContextNodeHidden(false)"
        >
          <LucideIcon :icon="Eye" :size="13" />
          {{ t("development.showNode") }}
        </button>
        <div class="base-context-menu-separator" />
        <button type="button" @click="archiveContextSession">
          <LucideIcon :icon="Archive" :size="13" />
          {{ t("chat.session.archive") }}
        </button>
        <button type="button" class="danger" @click="beginDeleteSession">
          <LucideIcon :icon="Trash2" :size="13" />
          {{ t("chat.session.delete") }}
        </button>
      </template>
      <template v-else>
        <button
          v-if="contextMenu.item.meta.kind === 'newSession'"
          type="button"
          @click="contextOpenNewSessionWindow"
        >
          <LucideIcon :icon="AppWindow" :size="13" />
          {{ t("chat.session.openInWindow") }}
        </button>
        <div
          v-if="contextMenu.item.meta.kind === 'newSession'"
          class="base-context-menu-separator"
        />
        <button
          v-if="contextMenu.item.meta.kind === 'project' || contextMenu.item.meta.kind === 'newSession' || contextMenu.item.meta.kind === 'folder'"
          type="button"
          @click="beginCreateFolder"
        >
          <LucideIcon :icon="FolderPlus" :size="13" />
          {{ t("development.newFolder") }}
        </button>
        <button
          v-if="contextMenu.item.meta.kind === 'project' || contextMenu.item.meta.kind === 'newSession' || contextMenu.item.meta.kind === 'folder'"
          type="button"
          @click="addLocalFiles"
        >
          <LucideIcon :icon="File" :size="13" />
          {{ t("development.addFiles") }}
        </button>
        <button
          v-if="contextMenu.item.meta.kind === 'project' || contextMenu.item.meta.kind === 'newSession' || contextMenu.item.meta.kind === 'folder'"
          type="button"
          @click="mountKnowledgeFolder"
        >
          <LucideIcon :icon="FolderPlus" :size="13" />
          {{ t("development.mountKnowledgeFolder") }}
        </button>
        <button
          v-if="contextMenu.item.meta.kind === 'folder' && !isKnowledgeTypeFolder(contextMenu.item)"
          type="button"
          @click="beginRenameFolder"
        >
          <LucideIcon :icon="Folder" :size="13" />
          {{ t("common.rename") }}
        </button>
        <button
          v-if="contextMenu.item.meta.kind === 'folder' && !isKnowledgeTypeFolder(contextMenu.item) && !contextMenu.item.meta.explorerNode?.sourcePath"
          type="button"
          @click="beginDeleteFolder"
        >
          <LucideIcon :icon="Trash2" :size="13" />
          {{ t("common.delete") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode?.sourcePath"
          type="button"
          @click="removeContextMountedNode"
        >
          <LucideIcon :icon="Trash2" :size="13" />
          {{ t("development.removeMount") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode && !contextMenu.item.meta.explorerNode.hidden"
          type="button"
          @click="setContextNodeHidden(true)"
        >
          <LucideIcon :icon="EyeOff" :size="13" />
          {{ t("development.hideNode") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode?.hidden"
          type="button"
          @click="setContextNodeHidden(false)"
        >
          <LucideIcon :icon="Eye" :size="13" />
          {{ t("development.showNode") }}
        </button>
        <button type="button" @click="browseWorkspace">
          <LucideIcon :icon="Plus" :size="13" />
          {{ t("development.openWorkspace") }}
        </button>
      </template>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="displayMenu"
      :x="displayMenu.x"
      :y="displayMenu.y"
      :min-width="176"
      @close="displayMenu = null"
    >
      <button
        v-for="preset in explorerStore.snapshots[presetProjectId]?.presets || []"
        :key="preset.presetId"
        type="button"
        :title="preset.filePath"
        @click="displayMenu = null; switchWorkspaceTreePreset(preset.presetId)"
      >
        <LucideIcon :icon="preset.presetId === activePresetId ? Check : ChevronRight" :size="13" />
        {{ preset.name }}
      </button>
      <div
        v-if="(explorerStore.snapshots[presetProjectId]?.presets.length || 0) > 0"
        class="base-context-menu-separator"
      />
      <button type="button" @click="setWorkspaceMode('single')">
        <LucideIcon :icon="displaySettings.workspaceDisplayMode === 'single' ? Check : ChevronRight" :size="13" />
        {{ t("settings.display.workspaceModeSingle") }}
      </button>
      <button type="button" @click="setWorkspaceMode('multi')">
        <LucideIcon :icon="displaySettings.workspaceDisplayMode === 'multi' ? Check : ChevronRight" :size="13" />
        {{ t("settings.display.workspaceModeMulti") }}
      </button>
      <div class="base-context-menu-separator" />
      <button type="button" @click="showHiddenNodes = !showHiddenNodes; displayMenu = null">
        <LucideIcon :icon="showHiddenNodes ? EyeOff : Eye" :size="13" />
        {{ showHiddenNodes ? t("development.hideHiddenNodes") : t("development.showHiddenNodes") }}
      </button>
      <div class="base-context-menu-separator" />
      <button type="button" @click="beginPresetDialog('create')">
        <LucideIcon :icon="Plus" :size="13" />
        {{ t("development.newPreset") }}
      </button>
      <button type="button" @click="beginPresetDialog('rename')">
        <LucideIcon :icon="Folder" :size="13" />
        {{ t("development.renamePreset") }}
      </button>
      <button
        type="button"
        :disabled="(explorerStore.snapshots[presetProjectId]?.presets.length || 0) <= 1"
        @click="beginPresetDialog('delete')"
      >
        <LucideIcon :icon="Trash2" :size="13" />
        {{ t("development.deletePreset") }}
      </button>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="workspaceMenu"
      :x="workspaceMenu.x"
      :y="workspaceMenu.y"
      :min-width="420"
      class="development-workspace-menu"
      @close="workspaceMenu = null"
    >
      <button
        v-for="path in projectStore.recentDirs"
        :key="path"
        type="button"
        class="development-recent-workspace"
        :class="{ active: isCurrentWorkspacePath(path) }"
        :title="path"
        @click="selectRecentWorkspace(path)"
      >
        <LucideIcon :icon="Folder" :size="13" />
        <span class="development-recent-workspace-text">
          <span>{{ shortPath(path) }}</span>
          <span>{{ parentPath(path) }}</span>
        </span>
        <LucideIcon v-if="isCurrentWorkspacePath(path)" :icon="Check" :size="13" />
      </button>
      <div v-if="projectStore.recentDirs.length === 0" class="development-recent-workspace-empty">
        {{ t("app.dir.noRecords") }}
      </div>
      <div class="base-context-menu-separator" />
      <button type="button" class="development-open-workspace" @click="browseWorkspace">
        <LucideIcon :icon="FolderOpen" :size="13" />
        {{ t("development.openWorkspace") }}
      </button>
    </BaseContextMenu>

    <div v-if="folderDialog" class="development-dialog-backdrop" @click.self="folderDialog = null">
      <form class="development-dialog" @submit.prevent="commitFolderDialog">
        <div class="development-dialog-title">
          {{ folderDialog.mode === 'rename'
            ? t('common.rename')
            : t('common.confirmDelete') }}
        </div>
        <input
          v-if="folderDialog.mode !== 'delete'"
          ref="folderInput"
          v-model="folderDialog.value"
          class="development-dialog-input"
          @keydown.esc.prevent="folderDialog = null"
        />
        <div v-else class="development-dialog-message">{{ folderDialog.value }}</div>
        <div class="development-dialog-actions">
          <button type="button" @click="folderDialog = null">{{ t("common.cancel") }}</button>
          <button type="submit">{{ folderDialog.mode === 'delete' ? t("common.delete") : t("common.confirm") }}</button>
        </div>
      </form>
    </div>
    <div v-if="presetDialog" class="development-dialog-backdrop" @click.self="presetDialog = null">
      <form class="development-dialog" @submit.prevent="commitPresetDialog">
        <div class="development-dialog-title">
          {{ presetDialog.mode === 'create'
            ? t('development.newPreset')
            : presetDialog.mode === 'rename'
              ? t('development.renamePreset')
              : t('development.deletePreset') }}
        </div>
        <input
          v-if="presetDialog.mode !== 'delete'"
          ref="presetInput"
          v-model="presetDialog.value"
          class="development-dialog-input"
          @keydown.esc.prevent="presetDialog = null"
        />
        <div v-else class="development-dialog-message">
          {{ t('development.deletePresetConfirm', presetDialog.value) }}
        </div>
        <div class="development-dialog-actions">
          <button type="button" @click="presetDialog = null">{{ t("common.cancel") }}</button>
          <button type="submit">
            {{ presetDialog.mode === 'delete' ? t("common.delete") : t("common.confirm") }}
          </button>
        </div>
      </form>
    </div>
    <div v-if="sessionDialog" class="development-dialog-backdrop" @click.self="sessionDialog = null">
      <form class="development-dialog" @submit.prevent="commitSessionDialog">
        <div class="development-dialog-title">
          {{ sessionDialog.mode === 'rename'
            ? t('chat.session.rename')
            : t('chat.session.delete') }}
        </div>
        <input
          v-if="sessionDialog.mode === 'rename'"
          ref="sessionInput"
          v-model="sessionDialog.value"
          class="development-dialog-input"
          @keydown.esc.prevent="sessionDialog = null"
        />
        <div v-else class="development-dialog-message">
          {{ t('chat.session.deleteConfirm') }}
        </div>
        <div class="development-dialog-actions">
          <button type="button" @click="sessionDialog = null">{{ t("common.cancel") }}</button>
          <button
            type="submit"
            :class="{ danger: sessionDialog.mode === 'delete' }"
          >
            {{ sessionDialog.mode === 'delete' ? t("common.delete") : t("common.confirm") }}
          </button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.development-workbench {
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: var(--panel-bg);
}

.workspace-drag-floating-preview {
  position: fixed;
  inset: 0 auto auto 0;
  z-index: 240;
  width: 228px;
  min-height: 34px;
  padding: 6px 10px;
  display: flex;
  align-items: center;
  gap: 7px;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 96%, var(--accent-soft) 4%);
  box-shadow: 0 8px 22px color-mix(in srgb, var(--text-color) 16%, transparent);
  color: var(--text-color);
  pointer-events: none;
  will-change: transform;
}

.workspace-drag-floating-icon {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.workspace-drag-floating-name {
  min-width: 0;
  overflow: hidden;
  font-family: var(--font-ui);
  font-size: 12px;
  font-weight: 500;
  line-height: 1.4;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-explorer {
  position: relative;
  width: 300px;
  min-width: 220px;
  max-width: 520px;
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
}

.development-explorer.is-workspace-drop-target {
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 64%, transparent);
}

.development-explorer-resize {
  position: relative;
  z-index: 12;
  width: 0;
  flex: 0 0 auto;
  cursor: col-resize;
}

.development-explorer-resize::before {
  content: "";
  position: absolute;
  inset: 0 auto 0 -3px;
  width: 6px;
}

.development-explorer-resize::after {
  content: "";
  position: absolute;
  inset: 0 auto 0 -1px;
  width: 2px;
  background: transparent;
  transition: background 0.12s ease;
}

.development-explorer-resize:hover::after,
.development-explorer-resize.active::after {
  background: color-mix(in srgb, var(--accent-color) 42%, transparent);
}

.development-explorer-toolbar {
  min-height: 34px;
  padding: 0 7px 0 12px;
  display: flex;
  align-items: center;
  border-bottom: 1px solid var(--border-color);
}

.development-explorer-label {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-workspace-trigger {
  min-width: 0;
  max-width: calc(100% - 34px);
  padding: 0;
  overflow: hidden;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  font-weight: 600;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}

.development-workspace-trigger:hover,
.development-workspace-trigger:focus-visible {
  color: var(--text-color);
  outline: none;
}

.development-explorer-actions {
  margin-left: auto;
  display: flex;
  gap: 2px;
}

.development-explorer-actions button {
  width: 26px;
  height: 26px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
}

.development-explorer-actions button:hover,
.development-explorer-actions button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.development-tree {
  flex: 1;
  min-height: 0;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.development-tree :deep(.workspace-tree-row) {
  gap: 6px;
}

.development-tree :deep(.workspace-tree-row-shell.is-open),
.development-tree :deep(.workspace-tree-row-shell.is-open:hover) {
  background: var(--active-bg);
  box-shadow: inset 2px 0 0 var(--accent-color);
}

.development-tree :deep(.workspace-tree-row-shell.is-empty-folder-row),
.development-tree :deep(.workspace-tree-row-shell.is-empty-folder-row:hover) {
  background: transparent;
}

.development-tree :deep(.workspace-tree-row-shell.is-empty-folder-row .workspace-tree-row) {
  cursor: default;
}

.development-tree :deep(.workspace-tree-row-shell.is-session-row .workspace-tree-row.drag-enabled) {
  cursor: pointer;
}

.development-tree :deep(.workspace-tree-icon.kind-folder) {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.development-tree :deep(.workspace-tree-icon.kind-package) {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.development-tree :deep(.workspace-tree-row-shell.is-special-root .workspace-tree-name) {
  font-weight: 600;
}

.development-tree :deep(.workspace-tree-row-shell.drop-target) {
  background: color-mix(in srgb, var(--accent-soft) 42%, transparent);
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 64%, transparent);
}

.development-tree :deep(.workspace-tree-row-shell.is-drop-preview) {
  background: color-mix(in srgb, var(--accent-soft) 12%, transparent);
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 36%, transparent);
}

.development-tree :deep(.workspace-tree-row-shell.is-drop-preview::before) {
  content: "";
  position: absolute;
  top: 0;
  left: var(--workspace-tree-row-indent, 4px);
  right: 12px;
  z-index: 1;
  height: 2px;
  border-radius: 2px;
  background: var(--accent-color);
  pointer-events: none;
}

.development-tree :deep(.workspace-tree-row-shell.is-drop-preview .workspace-tree-row.disabled) {
  opacity: 0;
  transition: none;
}

.development-tree :deep(.workspace-tree-row-shell.is-drop-preview .workspace-tree-name) {
  color: var(--text-color);
  font-size: 12px;
  font-weight: 500;
}

.development-tree :deep(.workspace-tree-row-shell.is-drop-preview .workspace-tree-icon) {
  color: var(--accent-color);
}

.development-tree :deep(.workspace-tree-row-shell.is-drag-source) {
  opacity: 0.38;
}

.development-tree :deep(.workspace-tree-row-shell.is-drag-source .workspace-tree-row) {
  cursor: grabbing;
}

.development-tree :deep(.workspace-tree-row-shell.is-hidden-node) {
  opacity: 0.52;
}

.development-tree :deep(.workspace-tree-row-shell.has-active-session:not(.is-open)) {
  background: color-mix(in srgb, var(--accent-color) 5%, transparent);
}

.development-tree :deep(.workspace-tree-row-shell.has-active-session .workspace-tree-icon) {
  color: color-mix(in srgb, var(--accent-color) 72%, var(--text-secondary) 28%);
}

.development-session-archive-button {
  position: absolute;
  top: 50%;
  right: 14px;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  opacity: 0;
  pointer-events: none;
  transform: translateY(-50%);
  transition: opacity 0.1s ease, background 0.1s ease, border-color 0.1s ease, color 0.1s ease;
}

.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-archive-button),
.development-session-archive-button:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-dot),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-spinner),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-status),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-branch-label),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:focus-within .development-session-dot),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:focus-within .development-session-spinner),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:focus-within .development-session-status),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:focus-within .development-branch-label) {
  opacity: 0;
}

.development-session-archive-button:hover,
.development-session-archive-button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.development-inline-create-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 30px;
  padding: 2px 12px;
  background: color-mix(in srgb, var(--active-bg) 78%, transparent);
}

.development-inline-create-bullet {
  position: relative;
  display: inline-block;
  width: 14px;
  min-width: 14px;
  height: 16px;
}

.development-inline-create-bullet::before {
  content: "";
  position: absolute;
  top: 50%;
  left: 50%;
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--text-secondary);
  opacity: 0.5;
  transform: translate(-50%, -50%);
}

.development-inline-create-body {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-width: 0;
}

.development-inline-create-input {
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

.development-inline-create-input:focus {
  border-color: var(--accent-color);
  outline: none;
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.development-inline-create-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}

.development-inline-create-action {
  width: 24px;
  min-width: 24px;
  height: 24px;
  padding: 0;
}

.development-session-title {
  position: relative;
  display: block;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-session-title.is-running {
  color: color-mix(in srgb, var(--text-color) 62%, var(--text-secondary) 38%);
  user-select: none;
}

.development-session-title.is-running::after {
  content: attr(data-title);
  position: absolute;
  inset: 0;
  overflow: hidden;
  color: var(--text-color);
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
  -webkit-mask-image: linear-gradient(90deg, transparent 40%, currentColor 50%, transparent 60%);
  mask-image: linear-gradient(90deg, transparent 40%, currentColor 50%, transparent 60%);
  -webkit-mask-size: 220% 100%;
  mask-size: 220% 100%;
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  animation: development-session-title-scan 2s ease-in-out infinite;
}

.development-session-dot {
  width: 6px;
  height: 6px;
  flex: 0 0 auto;
  border-radius: 999px;
  background: var(--text-secondary);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text-secondary) 24%, transparent);
}

.development-session-dot.is-waiting_input {
  background: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent);
}

.development-session-dot.is-queued,
.development-session-dot.is-starting,
.development-session-dot.is-cancelling {
  background: var(--status-warn-fg, var(--text-color));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-warn-border, var(--border-color)) 58%, transparent);
}

.development-session-dot.is-error {
  background: var(--status-danger-fg);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-danger-border) 60%, transparent);
}

.development-session-status {
  max-width: 58px;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-session-status.is-finishing,
.development-session-status.is-waiting_input {
  color: var(--accent-color);
}

.development-session-status.is-queued,
.development-session-status.is-starting,
.development-session-status.is-cancelling {
  color: var(--status-warn-fg, var(--text-color));
}

.development-session-status.is-error {
  color: var(--status-danger-fg);
}

.development-session-spinner {
  width: 10px;
  height: 10px;
  flex: 0 0 auto;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 34%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 999px;
  animation: development-session-spin 0.8s linear infinite;
}

@keyframes development-session-title-scan {
  0% {
    -webkit-mask-position: 100% 0;
    mask-position: 100% 0;
  }
  100% {
    -webkit-mask-position: 0 0;
    mask-position: 0 0;
  }
}

@keyframes development-session-spin {
  to { transform: rotate(360deg); }
}

.development-branch-label {
  align-self: center;
  max-width: 88px;
  margin-right: 8px;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
}

.development-tree-empty {
  padding: 14px 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.development-recent-workspace {
  min-height: 43px !important;
  padding-top: 5px !important;
  padding-bottom: 5px !important;
}

.development-recent-workspace.active {
  background: var(--active-bg);
}

.development-recent-workspace-text {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
  overflow: hidden;
}

.development-recent-workspace-text > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-recent-workspace-text > span:first-child {
  color: var(--text-color);
  font-size: 12px;
  font-weight: 500;
  line-height: 16px;
}

.development-recent-workspace-text > span:last-child {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 10px;
  line-height: 14px;
}

.development-recent-workspace-empty {
  padding: 8px 10px;
  color: var(--text-secondary);
  font-size: 11px;
  text-align: center;
}

.development-open-workspace {
  color: var(--text-secondary) !important;
}

.development-open-workspace:hover,
.development-open-workspace:focus-visible {
  color: var(--text-color) !important;
}

.development-editor {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  background: var(--panel-bg);
}

.development-overview {
  flex: 1;
  min-width: 0;
  padding: 18px 22px;
  overflow: auto;
}

.development-overview-title {
  margin-bottom: 6px;
  font-size: 15px;
  font-weight: 600;
}

.development-overview-path {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.development-worktree-row {
  width: 100%;
  min-height: 34px;
  display: grid;
  grid-template-columns: 18px minmax(120px, 220px) minmax(0, 1fr);
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  border: none;
  border-bottom: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  text-align: left;
}

.development-worktree-row:hover {
  background: var(--hover-bg);
}

.development-worktree-row > span:last-child {
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-dialog-backdrop {
  position: fixed;
  inset: 0;
  z-index: 280;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--bg-color) 44%, transparent);
}

.development-dialog {
  width: min(360px, calc(100vw - 32px));
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--elevated-bg, var(--panel-bg));
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.3);
}

.development-dialog-title {
  margin-bottom: 12px;
  font-size: 13px;
  font-weight: 600;
}

.development-dialog-input {
  box-sizing: border-box;
  width: 100%;
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-color);
  outline: none;
}

.development-dialog-input:focus {
  border-color: var(--accent-color);
}

.development-dialog-message {
  min-height: 30px;
  color: var(--text-secondary);
  font-size: 12px;
}

.development-dialog-actions {
  margin-top: 14px;
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.development-dialog-actions button {
  min-height: 28px;
  padding: 0 11px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: transparent;
  color: var(--text-color);
}

.development-dialog-actions button:hover {
  background: var(--hover-bg);
}

.development-dialog-actions button.danger {
  border-color: var(--status-danger-border);
  color: var(--status-danger-fg);
}

.development-dialog-actions button.danger:hover {
  background: var(--status-danger-bg);
}
</style>
