<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, type Window as TauriWindowHandle } from "@tauri-apps/api/window";
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
  subscribeUnitySendToLocus,
  type LocusFileDragStatePayload,
  type LocusFileDropPayload,
  type LocusFileDropRef,
  type UnityEmbedAssetDragStatePayload,
  type UnityEmbedAssetDropPayload,
  type UnitySendToLocusEventPayload,
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
import { useSkills } from "../../composables/useSkills";
import {
  buildContextReviewDraft,
  contextReviewAttachmentName,
} from "../../composables/sessionContextReview";
import { normalizeAppError } from "../../services/errors";
import {
  clearLastFocusedComposer,
  readLastFocusedComposer,
  writeLastFocusedComposer,
} from "../../services/unitySendToLocusFocus";
import {
  WORKBENCH_INSPECTOR_OPEN_EVENT,
  locusAssetInspectorTabTitle,
  type WorkbenchInspectorOpenPayload,
} from "../../services/locusAssetInspector";
import { viewSetTabHost } from "../../services/view";
import {
  VIEW_WORKBENCH_OPEN_EVENT,
  type ViewWorkbenchOpenPayload,
} from "../../services/viewWorkbench";
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
import type { ExtraWorkdirStatus } from "../../services/extraWorkdirs";
import { openExtraWorkdirsWindow } from "../../services/extraWorkdirsWindow";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useProjectStore } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import {
  createWorkbenchEditorInput,
  isWorkbenchMarkdownPath,
  shouldShowWorkbenchTabStrip,
  workbenchResourceKey,
  useWorkbenchStore,
} from "../../stores/workbench";
import type {
  DevelopmentResourceRef,
  ProjectExplorerMountEntry,
  ProjectExplorerNode,
  ProjectExplorerOperation,
  ProjectKnowledgeDocument,
  WorkbenchDropDirection,
  WorkbenchEditorGroup,
  WorkbenchEditorInput,
  WorkbenchEditorTransferRecord,
  WorkbenchEditorTransferSnapshot,
  WorkbenchWindowDropIntent,
} from "../../types/workbench";
import type { AssetRefAttachment, KnowledgeDocumentType, SessionSummary } from "../../types";
import type { UserMessageDraft } from "../../composables/chatMessageDraft";
import { clearSharedComposerDraft } from "../../composables/chatComposerDraftMemory";
import { emptyComposerIntent } from "../../composables/chatInputIntents";
import CollabView from "../CollabView.vue";
import AssetView from "../AssetView.vue";
import KnowledgeView from "../KnowledgeView.vue";
import ViewPackageView from "../ViewPackageView.vue";
import WorkbenchEditorTabs from "./WorkbenchEditorTabs.vue";
import WorkbenchSessionEditor from "./WorkbenchSessionEditor.vue";
import WorkbenchAssetEditor from "./WorkbenchAssetEditor.vue";
import WorkbenchSplitHost from "./WorkbenchSplitHost.vue";
import WorkspaceDirectoryPreview from "./WorkspaceDirectoryPreview.vue";
import WorkspaceFilePreview from "./WorkspaceFilePreview.vue";
import WorkbenchViewEditor from "./WorkbenchViewEditor.vue";
import WorkbenchArchivedSessionsEditor from "./WorkbenchArchivedSessionsEditor.vue";
import {
  WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
  type WorkbenchEditorTabInternalDragData,
} from "./workbenchDrag";
import {
  workbenchSplitDirectionAtPoint,
  workbenchTabInsertionIndexAtPoint,
} from "./workbenchDropGeometry";
import { workbenchComposerFileAttachment } from "./workbenchComposerDrop";
import {
  WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE,
  type WorkbenchReferenceDragData,
  type WorkbenchReferenceDragEntry,
} from "./workbenchReferenceDrag";
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
  isAnimatedSessionTreeStatus,
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
import {
  useWorkbenchWindowTabDrag,
  type WorkbenchWindowTabDragItem,
} from "../../composables/useWorkbenchWindowTabDrag";
import type { BaseTabStripItem } from "../ui/BaseTabStrip.vue";
import {
  WORKBENCH_WINDOW_TRANSFER_ACK_EVENT,
  WORKBENCH_WINDOW_TRANSFER_CANCEL_EVENT,
  WORKBENCH_WINDOW_TRANSFER_PREPARE_EVENT,
  WORKBENCH_TRANSFER_TIMEOUT_MS,
  createInMemoryWorkbenchTransferRecord,
  persistWorkbenchTransferRecord,
  readWorkbenchTransferRecord,
  recordWorkbenchWindowMetric,
  removeWorkbenchTransferRecord,
  type WorkbenchWindowTransferAckPayload,
  type WorkbenchWindowTransferCancelPayload,
  type WorkbenchWindowTransferPreparePayload,
} from "../../services/workbenchWindow";
import {
  createSharedDetachedWorkbenchWindow,
  removeSharedWorkbenchWindowHost,
} from "../../services/sharedWorkbenchWindow";
import {
  cancelSharedWorkbenchTransfer,
  dispatchSharedWorkbenchTransfer,
  hasSharedWorkbenchTransferTarget,
  registerSharedWorkbenchTransferTarget,
} from "../../services/sharedWorkbenchTransfer";
import {
  resolveWorkspaceSessionContextIds,
  resolveWorkspaceSessionSelection,
} from "./workspaceSessionSelection";
import {
  workbenchNewSessionShortcutAction,
  workbenchSessionNavigationMode,
  type WorkbenchNewSessionShortcutAction,
  type WorkbenchSessionNavigationMode,
} from "./workbenchSessionNavigation";

type ItemKind =
  | "project"
  | "newSession"
  | "knowledgeRoot"
  | "collaboration"
  | "assetsRoot"
  | "viewsRoot"
  | "archivedRoot"
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
  items?: DevelopmentTreeItem[];
}

type WorkbenchInternalDragData =
  | WorkspaceLayoutInternalDragData
  | KnowledgeInternalDragData
  | WorkbenchReferenceDragData
  | WorkbenchEditorTabInternalDragData;

type WorkbenchInternalDropIntent =
  | { kind: "layout"; layout: LayoutDropIntent; target: DevelopmentTreeItem | null }
  | { kind: "newSession"; target: DevelopmentTreeItem }
  | { kind: "composer"; paneId: string; editorId: string }
  | {
      kind: "editor";
      paneId: string;
      direction: WorkbenchDropDirection;
      index?: number;
    };

const props = withDefaults(defineProps<{
  windowId?: string;
  auxiliary?: boolean;
  showExplorer?: boolean;
  initialTransferToken?: string;
  nativeWindow?: TauriWindowHandle;
  ownerWindow?: Window;
  prewarm?: boolean;
  initialSessionId?: string;
  fixedWorkspaceRef?: WorkspaceRef | null;
}>(), {
  windowId: "main",
  auxiliary: false,
  showExplorer: true,
  initialTransferToken: "",
  prewarm: false,
  initialSessionId: "",
  fixedWorkspaceRef: null,
});

const emit = defineEmits<{
  (event: "ready"): void;
  (event: "transfer-ready", token: string, startedAt: number): void;
  (event: "empty"): void;
}>();

const WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE = "locus/workspace-layout";
const ownerWindow = props.ownerWindow ?? window;
const ownerDocument = ownerWindow.document;

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

interface DevelopmentSessionTarget {
  item: DevelopmentTreeItem;
  projectId: string;
  session: SessionSummary;
}

interface DevelopmentContextMenuState {
  x: number;
  y: number;
  item: DevelopmentTreeItem;
  sessionTargets?: DevelopmentSessionTarget[];
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

interface SessionDeleteDialogState {
  targets: DevelopmentSessionTarget[];
}

interface SessionInlineRenameState {
  sessionId: string;
  originalTitle: string;
  value: string;
}

interface DirtyEditorCloseDialogState {
  paneId: string;
  editorId: string;
  title: string;
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

const workspaceContextBaseStore = useWorkspaceContextStore();
const explorerStore = useWorkspaceExplorerStore();
const workbenchStore = useWorkbenchStore();
const chatStore = useChatStore();
const modelStore = useModelStore();
const agentStore = useAgentStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const uiStore = useUiStore();
const { skillItems } = useSkills();
const { state: displaySettings, set: setDisplaySetting } = useDisplaySettings();

const WORKBENCH_WINDOW_ID = props.windowId;
let initialWorkspaceFallbackActive = !!props.fixedWorkspaceRef;

function initialWorkspaceCheckout(): WorkspaceCheckoutDescriptor | null {
  if (!initialWorkspaceFallbackActive || !props.fixedWorkspaceRef) return null;
  const checkout = workspaceContextBaseStore.checkoutsById[props.fixedWorkspaceRef.checkoutId];
  if (!checkout?.runtime) return null;
  if (
    props.fixedWorkspaceRef.expectedGeneration != null
    && checkout.runtime.workspaceGeneration !== props.fixedWorkspaceRef.expectedGeneration
  ) return null;
  return checkout;
}

function scopedWorkspacePaneId(): string {
  return workbenchStore.ensureWindow(WORKBENCH_WINDOW_ID).focusedPaneId;
}

const workspaceContextStore = new Proxy(workspaceContextBaseStore, {
  get(target, property, receiver) {
    const paneId = scopedWorkspacePaneId();
    const paneContext = target.paneContextAt(WORKBENCH_WINDOW_ID, paneId);
    const checkout = paneContext?.focusedCheckoutId
      ? target.checkoutsById[paneContext.focusedCheckoutId] ?? null
      : initialWorkspaceCheckout();
    switch (property) {
      case "focusedPaneContext":
        return paneContext;
      case "focusedCheckout":
        return checkout;
      case "focusedRuntime":
        return checkout?.runtime ?? null;
      case "focusedWorkspaceRef":
        return checkout?.runtime ? {
          checkoutId: checkout.checkoutId,
          expectedGeneration: checkout.runtime.workspaceGeneration,
        } : null;
      case "focusedRoot":
        return checkout?.root ?? "";
      case "focusedProject":
        return checkout ? target.projectsById[checkout.projectId] ?? null : null;
      case "focusCheckout":
        return async (checkoutOrId: string | WorkspaceCheckoutDescriptor) => {
          const checkoutId = typeof checkoutOrId === "string"
            ? checkoutOrId
            : checkoutOrId.checkoutId;
          if (usesCheckoutScopedWorkbench()) {
            if (!await activateCheckoutScopedWorkbench(checkoutId)) return null;
            return target.paneContextAt(WORKBENCH_WINDOW_ID, scopedWorkspacePaneId());
          }
          const context = await target.focusCheckoutInPane(
            checkoutOrId,
            WORKBENCH_WINDOW_ID,
            scopedWorkspacePaneId(),
          );
          return context;
        };
      case "openAndFocus":
        return async (path: string) => {
          const normalizedPath = normalizeExternalProjectPath(path);
          const existingCheckout = Object.values(target.checkoutsById).find(
            (checkout) => normalizeExternalProjectPath(checkout.root) === normalizedPath,
          );
          if (existingCheckout && usesCheckoutScopedWorkbench()) {
            if (!await activateCheckoutScopedWorkbench(existingCheckout.checkoutId)) return null;
            return target.paneContextAt(WORKBENCH_WINDOW_ID, scopedWorkspacePaneId());
          }
          const context = await target.openAndFocusInPane(
            path,
            WORKBENCH_WINDOW_ID,
            scopedWorkspacePaneId(),
          );
          if (context) await adoptWorkbenchWorkspaceContext(context.focusedCheckoutId);
          return context;
        };
      default:
        return Reflect.get(target, property, receiver);
    }
  },
}) as typeof workspaceContextBaseStore;
const initialWorkbenchWorkspaceScopeId = props.fixedWorkspaceRef?.checkoutId
  ?? (!props.auxiliary && displaySettings.workspaceDisplayMode === "single"
    ? workspaceContextStore.focusedCheckout?.checkoutId ?? null
    : null);
const singleWorkspaceScopeId = ref<string | null>(initialWorkbenchWorkspaceScopeId);
workbenchStore.switchWorkspaceScope(WORKBENCH_WINDOW_ID, initialWorkbenchWorkspaceScopeId);
const workbenchWindow = computed(() => workbenchStore.ensureWindow(WORKBENCH_WINDOW_ID));
const workbenchWorkspaceScopeId = computed(() => (
  props.fixedWorkspaceRef?.checkoutId
    ?? (!props.auxiliary && displaySettings.workspaceDisplayMode === "single"
      ? singleWorkspaceScopeId.value
        ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID)
        ?? workspaceContextStore.focusedCheckout?.checkoutId
        ?? null
      : null)
));

const expanded = ref<Set<string>>(new Set());
const collapsedSessionParents = ref<Set<string>>(new Set());
const activeResource = ref<DevelopmentResourceRef | null>(
  workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.resource ?? null,
);
const selectedSessionIds = ref<Set<string>>(new Set());
const lastSessionSelectionAnchorId = ref<string | null>(null);
const contextMenu = ref<DevelopmentContextMenuState | null>(null);
const displayMenu = ref<{ x: number; y: number } | null>(null);
const specialNodesMenu = ref<{ x: number; y: number } | null>(null);
const workspaceMenu = ref<{ x: number; y: number } | null>(null);
const folderDialog = ref<FolderDialogState | null>(null);
const folderInput = ref<HTMLInputElement | null>(null);
const inlineCreate = ref<WorkspaceInlineCreateState | null>(null);
const inlineCreateInput = ref<HTMLInputElement | null>(null);
const inlineCreateRow = ref<HTMLElement | null>(null);
const presetDialog = ref<PresetDialogState | null>(null);
const presetInput = ref<HTMLInputElement | null>(null);
const sessionDeleteDialog = ref<SessionDeleteDialogState | null>(null);
const sessionInlineRename = ref<SessionInlineRenameState | null>(null);
const sessionRenameInput = ref<HTMLInputElement | null>(null);
const sessionEditorRefs = new Map<string, InstanceType<typeof WorkbenchSessionEditor>>();
const replacedWorkspaceSessionDrafts = new Map<string, UserMessageDraft>();
const workspaceFileEditorRefs = new Map<string, InstanceType<typeof WorkspaceFilePreview>>();
const workbenchAssetEditorRefs = new Map<string, InstanceType<typeof WorkbenchAssetEditor>>();
const workbenchViewEditorRefs = new Map<string, InstanceType<typeof WorkbenchViewEditor>>();
const editorWorkspaceRefs = new Map<string, WorkspaceRef>();
let lastRefreshedCheckoutServicesScopeKey: string | null = null;
const pendingCheckoutServicesRefreshes = new Map<string, Promise<void>>();
const dirtyEditorCloseDialog = ref<DirtyEditorCloseDialogState | null>(null);
const queuedWorkbenchEditorCloses = ref<Array<{ paneId: string; editorId: string }>>([]);
interface OutgoingWorkbenchTransfer {
  targetLabel: string;
  resolve: (payload: WorkbenchWindowTransferAckPayload) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}
interface AcceptedWorkbenchTransfer {
  paneId: string;
  editorId: string;
  inserted: boolean;
}
const outgoingWorkbenchTransfers = new Map<string, OutgoingWorkbenchTransfer>();
const acceptedWorkbenchTransfers = new Map<string, AcceptedWorkbenchTransfer>();
let unlistenWorkbenchTransferPrepare: UnlistenFn | null = null;
let unlistenWorkbenchTransferAck: UnlistenFn | null = null;
let unlistenWorkbenchTransferCancel: UnlistenFn | null = null;
let unlistenViewWorkbenchOpen: UnlistenFn | null = null;
let unlistenWorkbenchInspectorOpen: UnlistenFn | null = null;
let transferHostReady = false;
let appliedInitialTransferToken = "";
let unregisterSharedTransferTarget: (() => void) | null = null;
const internalDrag = useInternalDragController();
let appWindow: TauriWindowHandle | null = props.nativeWindow ?? null;
try {
  appWindow ??= getCurrentWindow();
} catch {
  appWindow = null;
}
const workbenchWindowTabDrag = useWorkbenchWindowTabDrag({
  windowLabel: WORKBENCH_WINDOW_ID,
  ownerWindow,
  resolveClientPoint: (x, y) => nativeWorkbenchTabDropIntentAt(x, y),
});
const workbenchRootRef = ref<HTMLElement | null>(null);
const explorerRootRef = ref<HTMLElement | null>(null);
const workspaceTreeTabAttentionTimers = new Set<number>();
const workspaceTreeTabAttentionSequence = new WeakMap<HTMLElement, number>();
let nextWorkspaceTreeTabAttentionSequence = 0;
const dragging = computed<DevelopmentTreeItem | null>(() => {
  if (!internalDrag.dragging.value) return null;
  const source = internalDrag.source.value;
  if (source?.payload.type !== WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) return null;
  return (source.payload.data as WorkspaceLayoutInternalDragData).item;
});
const dropTargetKey = ref<string | null>(null);
const layoutDropIntent = ref<LayoutDropIntent | null>(null);
const editorDropIntent = ref<Extract<WorkbenchInternalDropIntent, { kind: "editor" }> | null>(null);
const renderedEditorDropIntent = computed(() => (
  editorDropIntent.value
  ?? (workbenchWindowTabDrag.dropTarget.value ? {
    kind: "editor" as const,
    paneId: workbenchWindowTabDrag.dropTarget.value.paneId,
    direction: workbenchWindowTabDrag.dropTarget.value.direction,
    index: workbenchWindowTabDrag.dropTarget.value.index,
  } : null)
));
const composerDropTarget = ref<
  Extract<WorkbenchInternalDropIntent, { kind: "composer" }> | null
>(null);
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
const locusFileWorkspaceTabEligible = ref(false);
const unityAssetWorkspaceDragActive = ref(false);
const unityAssetWorkspaceDragRefs = ref<AssetRefAttachment[]>([]);
const workspaceDragPointer = ref({ x: 0, y: 0, visible: false });
const workspaceDropAffordanceActive = computed(() => (
  internalDrag.isDraggingType(KNOWLEDGE_INTERNAL_DRAG_TYPE)
  || internalDrag.isDraggingType(WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE)
  || internalDrag.isDraggingType(WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE)
  || locusFileWorkspaceDragActive.value
  || unityAssetWorkspaceDragActive.value
  || explorerRootDropActive.value
));
const activeEditorDropKey = computed(() => {
  const intent = renderedEditorDropIntent.value;
  return intent ? `editor:${intent.paneId}:${intent.direction}` : null;
});
const UNITY_WORKSPACE_DRAG_STATE_TTL_MS = 1200;
const externalDropTarget = ref<DevelopmentTreeItem | null>(null);
const WORKSPACE_EXPLORER_WIDTH_KEY = "locus:developmentExplorerWidth";
const explorerWidth = ref((() => {
  const saved = Number(ownerWindow.localStorage.getItem(WORKSPACE_EXPLORER_WIDTH_KEY));
  const versioned = workbenchWindow.value.sidebar.width;
  if (Number.isFinite(versioned) && versioned !== 300) return versioned;
  return Number.isFinite(saved) ? Math.min(520, Math.max(220, saved)) : versioned;
})());
const resizingExplorer = ref(false);
let explorerResizeStartX = 0;
let explorerResizeStartWidth = 0;
let unityWorkspaceDragStateClearTimer = 0;
let releaseLocusFileDrop: (() => void) | null = null;
let releaseLocusFileDragState: (() => void) | null = null;
let releaseUnityAssetDragState: (() => void) | null = null;
let releaseUnityAssetDrop: (() => void) | null = null;
let releaseUnitySendToLocus: (() => void) | null = null;
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
const ASSETS_SYSTEM_RESOURCE_ID = "assets";
const VIEWS_SYSTEM_RESOURCE_ID = "views";
const ARCHIVED_SYSTEM_RESOURCE_ID = "archived";
const WORKSPACE_SPECIAL_NODE_DEFINITIONS: ReadonlyArray<{
  resourceId: string;
  labelKey: string;
  icon: IconNode;
}> = [
  { resourceId: NEW_SESSION_SYSTEM_RESOURCE_ID, labelKey: "chat.session.newSession", icon: Plus },
  { resourceId: COLLABORATION_SYSTEM_RESOURCE_ID, labelKey: "app.tab.collab", icon: GitMerge },
  { resourceId: KNOWLEDGE_SYSTEM_RESOURCE_ID, labelKey: "app.tab.knowledge", icon: BookOpen },
  { resourceId: ASSETS_SYSTEM_RESOURCE_ID, labelKey: "app.tab.asset", icon: Folder },
  { resourceId: VIEWS_SYSTEM_RESOURCE_ID, labelKey: "app.tab.views", icon: Eye },
  { resourceId: ARCHIVED_SYSTEM_RESOURCE_ID, labelKey: "app.tab.archived", icon: Archive },
];
const collabHeadFocusRequest = ref<CollabHeadFocusRequest | null>(null);
let collabHeadFocusRequestId = 0;

const visibleProjects = computed<ProjectContextDescriptor[]>(() => {
  if (props.fixedWorkspaceRef) {
    const checkout = workspaceContextStore.checkoutsById[props.fixedWorkspaceRef.checkoutId];
    const project = checkout ? workspaceContextStore.projectsById[checkout.projectId] : null;
    return project ? [project] : [];
  }
  if (displaySettings.workspaceDisplayMode === "multi") return workspaceContextStore.projects;
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  const scopedCheckout = scopedCheckoutId
    ? workspaceContextStore.checkoutsById[scopedCheckoutId]
    : null;
  const focused = scopedCheckout
    ? workspaceContextStore.projectsById[scopedCheckout.projectId] ?? null
    : workspaceContextStore.focusedProject;
  return focused ? [focused] : workspaceContextStore.projects.slice(0, 1);
});

const explorerHeaderLabel = computed(() => {
  if (!props.fixedWorkspaceRef && displaySettings.workspaceDisplayMode === "multi") {
    return t("development.explorer");
  }
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  const root = (scopedCheckoutId ? workspaceContextStore.checkoutsById[scopedCheckoutId]?.root : null)
    ?? workspaceContextStore.focusedCheckout?.root
    ?? visibleProjects.value[0]?.checkouts[0]?.root
    ?? "";
  return root ? shortPath(root) : t("development.explorer");
});

const explorerHeaderTitle = computed(() => {
  if (!props.fixedWorkspaceRef && displaySettings.workspaceDisplayMode === "multi") return undefined;
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  return (scopedCheckoutId ? workspaceContextStore.checkoutsById[scopedCheckoutId]?.root : null)
    ?? workspaceContextStore.focusedCheckout?.root
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

const specialNodeVisibilityItems = computed(() => {
  const nodes = explorerStore.snapshots[presetProjectId.value]?.nodes ?? [];
  return WORKSPACE_SPECIAL_NODE_DEFINITIONS.flatMap((definition) => {
    const node = nodes.find((candidate) => (
      candidate.resourceKind === SYSTEM_RESOURCE_KIND
      && candidate.resourceId === definition.resourceId
    ));
    return node ? [{ ...definition, node }] : [];
  });
});

const specialNodeVisibilityBusy = ref<Set<string>>(new Set());

watch(displayMenu, (menu) => {
  if (!menu) specialNodesMenu.value = null;
});

function onExplorerResizeStart(event: MouseEvent): void {
  if (event.button !== 0) return;
  event.preventDefault();
  resizingExplorer.value = true;
  explorerResizeStartX = event.clientX;
  explorerResizeStartWidth = explorerWidth.value;
  ownerDocument.addEventListener("mousemove", onExplorerResizeMove);
  ownerDocument.addEventListener("mouseup", onExplorerResizeEnd);
  ownerDocument.body.style.cursor = "col-resize";
  ownerDocument.body.classList.add("is-dragging-select-lock");
}

function onExplorerResizeMove(event: MouseEvent): void {
  if (!resizingExplorer.value) return;
  const viewportMax = Math.max(220, Math.min(520, ownerWindow.innerWidth - 360));
  explorerWidth.value = Math.min(
    viewportMax,
    Math.max(220, explorerResizeStartWidth + event.clientX - explorerResizeStartX),
  );
}

function onExplorerResizeEnd(): void {
  if (!resizingExplorer.value) return;
  resizingExplorer.value = false;
  ownerDocument.removeEventListener("mousemove", onExplorerResizeMove);
  ownerDocument.removeEventListener("mouseup", onExplorerResizeEnd);
  ownerDocument.body.style.cursor = "";
  ownerDocument.body.classList.remove("is-dragging-select-lock");
  ownerWindow.localStorage.setItem(WORKSPACE_EXPLORER_WIDTH_KEY, String(Math.round(explorerWidth.value)));
  workbenchStore.setSidebarWidth(WORKBENCH_WINDOW_ID, Math.round(explorerWidth.value));
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

function normalizedWorkspacePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLocaleLowerCase();
}

function isCurrentWorkspacePath(path: string): boolean {
  const current = workspaceContextStore.focusedCheckout?.root ?? "";
  return normalizedWorkspacePath(path) === normalizedWorkspacePath(current);
}

function extraWorkdirsFor(path: string): ExtraWorkdirStatus[] {
  const direct = projectStore.extraWorkdirs[path];
  if (direct) return direct;
  const pathKey = normalizedWorkspacePath(path);
  return Object.entries(projectStore.extraWorkdirs).find(
    ([workspacePath]) => normalizedWorkspacePath(workspacePath) === pathKey,
  )?.[1] ?? [];
}

function extraWorkdirTooltip(extra: ExtraWorkdirStatus): string {
  return [extra.path, extra.readOnly ? t("extraWorkdirs.readOnly") : "", extra.comment]
    .filter(Boolean)
    .join(" — ");
}

function normalizeKnowledgeSelectionPath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLocaleLowerCase();
}

function normalizeExternalProjectPath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLocaleLowerCase();
}

function externalScriptWorkspacePath(projectPath: string, assetPath: string): string {
  const root = projectPath.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  const candidate = assetPath.trim().replace(/\\/g, "/");
  const prefix = `${root}/`;
  if (candidate.toLocaleLowerCase().startsWith(prefix.toLocaleLowerCase())) {
    return candidate.slice(prefix.length).replace(/^\/+/, "");
  }
  return candidate.replace(/^\.\//, "").replace(/^\/+/, "");
}

let externalScriptOpenEpoch = 0;

async function revealPendingExternalScriptOpen(): Promise<void> {
  const pending = uiStore.pendingExternalScriptOpen;
  if (!pending?.assetPath.trim() || !pending.projectPath.trim()) return;
  const epoch = ++externalScriptOpenEpoch;
  const requestId = pending.id;
  const paneId = workbenchWindow.value.focusedPaneId;
  try {
    const normalizedProjectPath = normalizeExternalProjectPath(pending.projectPath);
    const existingCheckout = Object.values(workspaceContextStore.checkoutsById).find(
      (checkout) => normalizeExternalProjectPath(checkout.root) === normalizedProjectPath,
    );
    const context = existingCheckout
      ? await workspaceContextStore.focusCheckoutInPane(
        existingCheckout,
        WORKBENCH_WINDOW_ID,
        paneId,
      )
      : await workspaceContextStore.openAndFocusInPane(
        pending.projectPath,
        WORKBENCH_WINDOW_ID,
        paneId,
      );
    if (
      !context
      || epoch !== externalScriptOpenEpoch
      || uiStore.pendingExternalScriptOpen?.id !== requestId
    ) return;

    const checkout = workspaceContextStore.checkoutsById[context.focusedCheckoutId];
    if (!checkout) return;
    if (displaySettings.workspaceDisplayMode === "single") {
      await adoptWorkbenchWorkspaceContext(checkout.checkoutId);
    }
    if (
      epoch !== externalScriptOpenEpoch
      || uiStore.pendingExternalScriptOpen?.id !== requestId
    ) return;

    await explorerStore.loadProject(checkout.projectId);
    const path = externalScriptWorkspacePath(pending.projectPath, pending.assetPath);
    const editor = await openWorkbenchResource({
      resource: {
        kind: "workspaceFile",
        projectId: checkout.projectId,
        path,
      },
      title: shortPath(path),
      checkoutId: checkout.checkoutId,
    }, {
      paneId: workbenchWindow.value.focusedPaneId,
      preview: false,
      pinned: true,
    });
    await nextTick();
    await workspaceFileEditorRefs.get(editor.editorId)?.revealPosition(
      pending.line,
      pending.column,
    );
    uiStore.clearPendingExternalScriptOpen(requestId);
  } catch (error) {
    if (uiStore.pendingExternalScriptOpen?.id !== requestId) return;
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
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
  return isAnimatedSessionTreeStatus(status);
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

function isRenamingSessionItem(item: DevelopmentTreeItem): boolean {
  return item.meta.kind === "session"
    && item.meta.session?.id === sessionInlineRename.value?.sessionId;
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

function isWorkspaceSessionMultiSelected(sessionId: string): boolean {
  return selectedSessionIds.value.has(sessionId);
}

function isWorkspaceSessionContextSelected(sessionId: string): boolean {
  return contextMenu.value?.sessionTargets?.some((target) => target.session.id === sessionId) === true;
}

function clearSessionMultiSelection(): void {
  if (selectedSessionIds.value.size > 0) selectedSessionIds.value = new Set();
}

function resetSessionMultiSelection(): void {
  clearSessionMultiSelection();
  lastSessionSelectionAnchorId.value = null;
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

interface TreeEditorDescriptor {
  resource: DevelopmentResourceRef;
  title: string;
  checkoutId?: string | null;
  sourcePath?: string | null;
}

function workbenchGroup(paneId: string): WorkbenchEditorGroup | null {
  return workbenchWindow.value.groups[paneId] ?? null;
}

function editorForPane(paneId: string): WorkbenchEditorInput | null {
  return workbenchStore.activeEditor(WORKBENCH_WINDOW_ID, paneId);
}

function projectCheckout(
  projectId: string,
  checkoutId?: string | null,
): WorkspaceCheckoutDescriptor | null {
  const project = workspaceContextStore.projectsById[projectId];
  if (!project) return null;
  return project.checkouts.find((checkout) => checkout.checkoutId === checkoutId)
    ?? project.checkouts[0]
    ?? null;
}

function preferredCheckoutIdForResource(
  resource: DevelopmentResourceRef,
  paneId: string,
  explicitCheckoutId?: string | null,
): string | null {
  if (explicitCheckoutId && workspaceContextStore.checkoutsById[explicitCheckoutId]) {
    return explicitCheckoutId;
  }
  if (resource.kind === "checkout") return resource.checkoutId;
  const currentGroupCheckoutId = workbenchGroup(paneId)?.focusedCheckoutId;
  const currentGroupCheckout = currentGroupCheckoutId
    ? workspaceContextStore.checkoutsById[currentGroupCheckoutId]
    : null;
  if (resource.kind === "session") {
    const session = explorerStore.resources[resource.projectId]?.sessions.find(
      (candidate) => candidate.id === resource.sessionId,
    ) ?? chatStore.sessions.find((candidate) => candidate.id === resource.sessionId);
    return session?.executionTarget?.checkoutId
      ?? session?.defaultCheckoutId
      ?? (currentGroupCheckout?.projectId === resource.projectId ? currentGroupCheckout.checkoutId : null)
      ?? projectCheckout(resource.projectId)?.checkoutId
      ?? null;
  }
  if (resource.kind === "knowledge") {
    const document = explorerStore.resources[resource.projectId]?.knowledge.find(
      (candidate) => candidate.id === resource.documentId,
    );
    if (
      currentGroupCheckout?.projectId === resource.projectId
      && document?.availableCheckoutIds.includes(currentGroupCheckout.checkoutId)
    ) return currentGroupCheckout.checkoutId;
    return document?.sourceCheckoutId
      ?? projectCheckout(resource.projectId)?.checkoutId
      ?? null;
  }
  return currentGroupCheckout?.projectId === resource.projectId
    ? currentGroupCheckout.checkoutId
    : projectCheckout(resource.projectId)?.checkoutId ?? null;
}

function titleForResource(resource: DevelopmentResourceRef, sourcePath?: string | null): string {
  switch (resource.kind) {
    case "project": {
      const project = workspaceContextStore.projectsById[resource.projectId];
      return project ? projectLabel(project) : resource.projectId;
    }
    case "newSession": return t("chat.session.newSession");
    case "checkout": return shortPath(
      workspaceContextStore.checkoutsById[resource.checkoutId]?.root ?? resource.checkoutId,
    );
    case "section": {
      const key = resource.section === "sessions"
        ? "chat"
        : resource.section === "assets"
          ? "asset"
          : resource.section === "collab"
            ? "collab"
            : resource.section;
      return t(`app.tab.${key}`);
    }
    case "knowledgeRoot": return t("app.tab.knowledge");
    case "collaboration": return t("app.tab.collab");
    case "folder": {
      const node = explorerStore.snapshots[resource.projectId]?.nodes.find(
        (candidate) => candidate.nodeId === resource.nodeId,
      );
      return node?.folderName || t("development.untitledFolder");
    }
    case "session": {
      const session = explorerStore.resources[resource.projectId]?.sessions.find(
        (candidate) => candidate.id === resource.sessionId,
      ) ?? chatStore.sessions.find((candidate) => candidate.id === resource.sessionId);
      return session?.title || t("chat.session.newSession");
    }
    case "knowledge": {
      const document = explorerStore.resources[resource.projectId]?.knowledge.find(
        (candidate) => candidate.id === resource.documentId,
      );
      return document ? knowledgeDocumentName(document) : resource.documentId;
    }
    case "workspaceFile": return shortPath(resource.path);
    case "asset": return shortPath(resource.path);
    case "sceneObject": return shortPath(resource.objectPath);
    case "view": return resource.viewId;
    case "localDirectory": return sourcePath ? shortPath(sourcePath) : resource.relativePath
      ? shortPath(resource.relativePath)
      : resource.nodeId;
    case "localFile": return sourcePath ? shortPath(sourcePath) : resource.relativePath
      ? shortPath(resource.relativePath)
      : resource.nodeId;
  }
}

function createEditorForResource(
  resource: DevelopmentResourceRef,
  options: {
    paneId?: string;
    title?: string;
    checkoutId?: string | null;
    sourcePath?: string | null;
    preview?: boolean;
    pinned?: boolean;
  } = {},
): WorkbenchEditorInput {
  const paneId = options.paneId ?? workbenchWindow.value.focusedPaneId;
  const checkoutId = preferredCheckoutIdForResource(resource, paneId, options.checkoutId);
  const checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : null;
  return createWorkbenchEditorInput(resource, options.title ?? titleForResource(resource, options.sourcePath), {
    preview: options.preview ?? true,
    pinned: options.pinned ?? false,
    checkoutBinding: checkoutId
      ? {
          checkoutId,
          expectedGeneration: checkout?.runtime?.workspaceGeneration ?? null,
        }
      : null,
    sourcePath: options.sourcePath ?? null,
    capabilities: resource.kind === "view"
      ? { split: false, detach: true, duplicate: false }
      : undefined,
  });
}

function treeEditorDescriptor(item: DevelopmentTreeItem): TreeEditorDescriptor | null {
  switch (item.meta.kind) {
    case "project":
      return {
        resource: { kind: "project", projectId: item.meta.projectId },
        title: item.treeRow?.name ?? item.meta.projectId,
      };
    case "newSession":
      return {
        resource: { kind: "newSession", projectId: item.meta.projectId },
        title: item.treeRow?.name ?? t("chat.session.newSession"),
        checkoutId: item.meta.checkoutId,
      };
    case "knowledgeRoot":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "knowledge" },
        title: item.treeRow?.name ?? t("app.tab.knowledge"),
        checkoutId: item.meta.checkoutId,
      };
    case "collaboration":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "collab" },
        title: item.treeRow?.name ?? t("app.tab.collab"),
      };
    case "assetsRoot":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "assets" },
        title: item.treeRow?.name ?? t("app.tab.asset"),
        checkoutId: item.meta.checkoutId,
      };
    case "viewsRoot":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "views" },
        title: item.treeRow?.name ?? t("app.tab.views"),
        checkoutId: item.meta.checkoutId,
      };
    case "archivedRoot":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "archived" },
        title: item.treeRow?.name ?? t("app.tab.archived"),
        checkoutId: item.meta.checkoutId,
      };
    case "checkout":
      return item.meta.checkoutId ? {
        resource: {
          kind: "checkout",
          projectId: item.meta.projectId,
          checkoutId: item.meta.checkoutId,
        },
        title: item.treeRow?.name ?? item.meta.checkoutId,
        checkoutId: item.meta.checkoutId,
      } : null;
    case "folder":
      if (!item.meta.explorerNode) return null;
      return item.meta.explorerNode.sourcePath ? {
        resource: {
          kind: "localDirectory",
          projectId: item.meta.projectId,
          nodeId: item.meta.explorerNode.nodeId,
        },
        title: item.treeRow?.name ?? shortPath(item.meta.explorerNode.sourcePath),
        sourcePath: item.meta.explorerNode.sourcePath,
      } : {
        resource: {
          kind: "folder",
          projectId: item.meta.projectId,
          nodeId: item.meta.explorerNode.nodeId,
        },
        title: item.treeRow?.name ?? t("development.untitledFolder"),
      };
    case "session":
      return item.meta.session ? {
        resource: {
          kind: "session",
          projectId: item.meta.projectId,
          sessionId: item.meta.session.id,
        },
        title: item.treeRow?.name ?? item.meta.session.title,
        checkoutId: item.meta.session.executionTarget?.checkoutId
          ?? item.meta.session.defaultCheckoutId,
      } : null;
    case "knowledge":
      return item.meta.knowledge ? {
        resource: {
          kind: "knowledge",
          projectId: item.meta.projectId,
          documentId: item.meta.knowledge.id,
        },
        title: item.treeRow?.name ?? knowledgeDocumentName(item.meta.knowledge),
        checkoutId: item.meta.knowledge.sourceCheckoutId,
      } : null;
    case "localFile":
      return item.meta.explorerNode?.sourcePath ? {
        resource: {
          kind: "localFile",
          projectId: item.meta.projectId,
          nodeId: item.meta.explorerNode.nodeId,
        },
        title: item.treeRow?.name ?? shortPath(item.meta.explorerNode.sourcePath),
        sourcePath: item.meta.explorerNode.sourcePath,
      } : null;
    case "mountedFile":
      return item.meta.mountEntry ? {
        resource: {
          kind: "localFile",
          projectId: item.meta.projectId,
          nodeId: item.meta.mountEntry.nodeId,
          relativePath: item.meta.mountEntry.relativePath,
        },
        title: item.treeRow?.name ?? item.meta.mountEntry.name,
        sourcePath: item.meta.mountEntry.absolutePath,
      } : null;
    case "mountedFolder":
      return item.meta.mountEntry ? {
        resource: {
          kind: "localDirectory",
          projectId: item.meta.projectId,
          nodeId: item.meta.mountEntry.nodeId,
          relativePath: item.meta.mountEntry.relativePath,
        },
        title: item.treeRow?.name ?? item.meta.mountEntry.name,
        sourcePath: item.meta.mountEntry.absolutePath,
      } : null;
    default:
      return null;
  }
}

async function focusWorkbenchEditor(
  paneId: string,
  editorId: string,
  options: { refreshServices?: boolean; focusPane?: boolean } = {},
): Promise<void> {
  const focusPane = options.focusPane !== false;
  const paneWasFocused = workbenchWindow.value.focusedPaneId === paneId;
  if (!workbenchStore.activateEditor(
    WORKBENCH_WINDOW_ID,
    paneId,
    editorId,
    { focusPane },
  )) return;
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === editorId,
  );
  if (!editor) return;
  if (focusPane || paneWasFocused) activeResource.value = editor.resource;
  const binding = editor.checkoutBinding;
  if (!binding?.checkoutId) {
    if (focusPane) workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, paneId);
    return;
  }
  const existingContext = workspaceContextStore.paneContextAt(WORKBENCH_WINDOW_ID, paneId);
  const expectedGeneration = workspaceContextStore.checkoutsById[binding.checkoutId]
    ?.runtime?.workspaceGeneration ?? binding.expectedGeneration ?? null;
  const context = existingContext?.focusedCheckoutId === binding.checkoutId
    && (expectedGeneration === null || existingContext.workspaceGeneration === expectedGeneration)
    ? existingContext
    : await workspaceContextStore.focusCheckoutInPane(
         binding.checkoutId,
         WORKBENCH_WINDOW_ID,
         paneId,
         { activate: focusPane },
       );
  if (!context) return;
  if (focusPane) workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, paneId);
  if (
    binding.checkoutId !== context.focusedCheckoutId
    || binding.expectedGeneration !== context.workspaceGeneration
  ) {
    workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editorId, {
      checkoutBinding: {
        checkoutId: context.focusedCheckoutId,
        expectedGeneration: context.workspaceGeneration,
      },
    });
  }
  const activeSessionId = editor.resource.kind === "session" ? editor.resource.sessionId : null;
  if ((context.activeSessionId ?? null) !== activeSessionId) {
    await workspaceContextStore.setActiveSessionInPane(
      activeSessionId,
      WORKBENCH_WINDOW_ID,
      paneId,
      { activate: focusPane },
    );
  }
  if (
    options.refreshServices !== false
    && workspaceRefScopeKey(workspaceContextStore.focusedWorkspaceRef)
      !== lastRefreshedCheckoutServicesScopeKey
    && workbenchWindow.value.focusedPaneId === paneId
    && workbenchWindow.value.groups[paneId]?.activeEditorId === editorId
  ) await refreshFocusedCheckoutServices();
}

async function openWorkbenchResource(
  descriptor: TreeEditorDescriptor,
  options: {
    paneId?: string;
    preview?: boolean;
    pinned?: boolean;
    focus?: boolean;
    replacePreview?: boolean;
    allowDuplicate?: boolean;
  } = {},
): Promise<WorkbenchEditorInput> {
  let paneId = options.paneId ?? workbenchWindow.value.focusedPaneId;
  let input = createEditorForResource(descriptor.resource, {
    paneId,
    title: descriptor.title,
    checkoutId: descriptor.checkoutId,
    sourcePath: descriptor.sourcePath,
    preview: options.preview,
    pinned: options.pinned,
  });
  const inputCheckoutId = input.checkoutBinding?.checkoutId;
  if (
    inputCheckoutId
    && usesCheckoutScopedWorkbench()
    && workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== inputCheckoutId
  ) {
    if (!await activateCheckoutScopedWorkbench(inputCheckoutId)) {
      throw new Error(`Workbench checkout switch was superseded: ${inputCheckoutId}`);
    }
    paneId = workbenchWindow.value.focusedPaneId;
    input = createEditorForResource(descriptor.resource, {
      paneId,
      title: descriptor.title,
      checkoutId: inputCheckoutId,
      sourcePath: descriptor.sourcePath,
      preview: options.preview,
      pinned: options.pinned,
    });
  }
  const editor = workbenchStore.openEditor(WORKBENCH_WINDOW_ID, input, {
    paneId,
    preview: options.preview,
    pinned: options.pinned,
    replacePreview: options.replacePreview,
    allowDuplicate: options.allowDuplicate,
  });
  activeResource.value = editor.resource;
  if (options.focus !== false) await focusWorkbenchEditor(paneId, editor.editorId);
  await refreshWorkbenchFileEditor(editor.editorId);
  return editor;
}

let initialSessionApplied = false;

async function openInitialSessionIfRequested(): Promise<void> {
  const sessionId = props.initialSessionId.trim();
  if (initialSessionApplied || !sessionId) return;
  initialSessionApplied = true;

  const session = chatStore.sessions.find((candidate) => candidate.id === sessionId) ?? null;
  const checkout = initialWorkspaceCheckout() ?? workspaceContextStore.focusedCheckout;
  const projectId = session?.projectId ?? checkout?.projectId;
  if (!projectId || !checkout) {
    throw new Error("The embedded Unity session workspace is unavailable.");
  }

  await openWorkbenchResource({
    resource: { kind: "session", projectId, sessionId },
    title: session?.title?.trim() || sessionId,
    checkoutId: checkout.checkoutId,
  }, {
    preview: false,
    pinned: true,
  });
}

function matchingWorkbenchEditors(resource: DevelopmentResourceRef): Array<{
  paneId: string;
  editor: WorkbenchEditorInput;
}> {
  const resourceKey = workbenchResourceKey(resource);
  return Object.values(workbenchWindow.value.groups).flatMap((group) => (
    group.tabs
      .filter((editor) => workbenchResourceKey(editor.resource) === resourceKey)
      .map((editor) => ({ paneId: group.paneId, editor }))
  ));
}

function flashWorkspaceTreeEditorTabs(editors: readonly WorkbenchEditorInput[]): void {
  const root = workbenchRootRef.value;
  if (!root) return;
  const attentionSequence = ++nextWorkspaceTreeTabAttentionSequence;
  const attentionClass = attentionSequence % 2 === 0
    ? "workspace-tree-attention-a"
    : "workspace-tree-attention-b";
  for (const editor of editors) {
    const tab = root.querySelector<HTMLElement>(
      `[data-workbench-tab-id="${CSS.escape(editor.editorId)}"]`,
    );
    if (!tab) continue;
    const shell = tab.closest<HTMLElement>("[data-locus-tab-shell]");
    if (!shell) continue;
    shell.classList.remove("workspace-tree-attention-a", "workspace-tree-attention-b");
    void shell.offsetWidth;
    shell.classList.add(attentionClass);
    workspaceTreeTabAttentionSequence.set(shell, attentionSequence);
    tab.scrollIntoView({ block: "nearest", inline: "nearest" });
    const timer = ownerWindow.setTimeout(() => {
      workspaceTreeTabAttentionTimers.delete(timer);
      if (workspaceTreeTabAttentionSequence.get(shell) !== attentionSequence) return;
      shell.classList.remove("workspace-tree-attention-a", "workspace-tree-attention-b");
      workspaceTreeTabAttentionSequence.delete(shell);
    }, 520);
    workspaceTreeTabAttentionTimers.add(timer);
  }
}

async function openWorkbenchResourceFromWorkspaceTree(
  descriptor: TreeEditorDescriptor,
  options: {
    preview?: boolean;
    pinned?: boolean;
  } = {},
): Promise<WorkbenchEditorInput> {
  const matches = matchingWorkbenchEditors(descriptor.resource);
  if (matches.length === 0) return openWorkbenchResource(descriptor, options);
  if (options.pinned) {
    for (const match of matches) {
      workbenchStore.pinEditor(WORKBENCH_WINDOW_ID, match.paneId, match.editor.editorId);
    }
  }
  const focusedPaneId = workbenchWindow.value.focusedPaneId;
  const target = matches.find((match) => (
    match.paneId === focusedPaneId
    && workbenchWindow.value.groups[match.paneId]?.activeEditorId === match.editor.editorId
  ))
    ?? matches.find((match) => (
      workbenchWindow.value.groups[match.paneId]?.activeEditorId === match.editor.editorId
    ))
    ?? matches.find((match) => match.paneId === focusedPaneId)
    ?? matches[0]!;
  const alreadyForeground = workbenchWindow.value.groups[target.paneId]?.activeEditorId
    === target.editor.editorId;
  await nextTick();
  flashWorkspaceTreeEditorTabs(matches.map((match) => match.editor));
  await focusWorkbenchEditor(target.paneId, target.editor.editorId, {
    focusPane: alreadyForeground,
  });
  await refreshWorkbenchFileEditor(target.editor.editorId);
  return target.editor;
}

function currentWorkbenchEditorIsProtected(): boolean {
  const paneId = workbenchWindow.value.focusedPaneId;
  const editor = editorForPane(paneId);
  return editor?.dirty === true;
}

function userMessageDraftHasContent(draft: UserMessageDraft): boolean {
  return !!draft.text.trim()
    || draft.images.length > 0
    || draft.assetRefs.length > 0
    || draft.localFiles.length > 0
    || draft.consoleTexts.length > 0
    || !!draft.intent.mode
    || draft.intent.skills.length > 0;
}

function preserveReplacedWorkspaceSessionDraft(editor: WorkbenchEditorInput): void {
  if (editor.resource.kind !== "session" && editor.resource.kind !== "newSession") return;
  const snapshot = sessionEditorRefs.get(editor.editorId)?.exportTransferSnapshot();
  const draft = snapshot?.composerDraft as UserMessageDraft | null | undefined;
  if (!draft || !userMessageDraftHasContent(draft)) return;
  replacedWorkspaceSessionDrafts.set(workbenchResourceKey(editor.resource), draft);
}

async function restoreReplacedWorkspaceSessionDraft(
  resource: DevelopmentResourceRef,
  editorId: string,
): Promise<void> {
  if (resource.kind !== "session" && resource.kind !== "newSession") return;
  const key = workbenchResourceKey(resource);
  const draft = replacedWorkspaceSessionDrafts.get(key);
  if (!draft) return;
  await nextTick();
  const editor = sessionEditorRefs.get(editorId);
  if (!editor) return;
  await editor.applyDraftPrefill(draft);
  replacedWorkspaceSessionDrafts.delete(key);
}

function workspaceSessionNavigationMode(
  item: DevelopmentTreeItem,
): WorkbenchSessionNavigationMode {
  const group = workbenchGroup(workbenchWindow.value.focusedPaneId);
  const targetOpen = item.meta.kind === "session"
    && !!item.meta.session
    && matchingWorkbenchEditors({
      kind: "session",
      projectId: item.meta.projectId,
      sessionId: item.meta.session.id,
    }).length > 0;
  return workbenchSessionNavigationMode({
    targetOpen,
    splitLayout: workbenchWindow.value.layout.kind === "split",
    focusedGroupTabCount: group?.tabs.length ?? 0,
    currentEditorProtected: currentWorkbenchEditorIsProtected(),
  });
}

async function replaceFocusedWorkbenchResource(
  descriptor: TreeEditorDescriptor,
): Promise<WorkbenchEditorInput> {
  const paneId = workbenchWindow.value.focusedPaneId;
  const current = editorForPane(paneId);
  if (!current) {
    return openWorkbenchResource(descriptor, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: descriptor.resource.kind === "newSession",
    });
  }
  const input = createEditorForResource(descriptor.resource, {
    paneId,
    title: descriptor.title,
    checkoutId: descriptor.checkoutId,
    sourcePath: descriptor.sourcePath,
    preview: false,
    pinned: true,
  });
  preserveReplacedWorkspaceSessionDraft(current);
  const editor = workbenchStore.replaceEditor(
    WORKBENCH_WINDOW_ID,
    paneId,
    current.editorId,
    input,
  );
  if (!editor) {
    return openWorkbenchResource(descriptor, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: descriptor.resource.kind === "newSession",
    });
  }
  activeResource.value = editor.resource;
  await focusWorkbenchEditor(paneId, editor.editorId);
  return editor;
}

async function openWorkspaceSessionDescriptor(
  descriptor: TreeEditorDescriptor,
  mode: WorkbenchSessionNavigationMode,
): Promise<WorkbenchEditorInput | null> {
  if (descriptor.resource.kind === "session") {
    const matches = matchingWorkbenchEditors(descriptor.resource);
    if (matches.length > 0) {
      return openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: false,
        pinned: true,
      });
    }
  }

  let resolvedMode = mode;
  if (resolvedMode === "reuse" && currentWorkbenchEditorIsProtected()) {
    resolvedMode = "newTab";
  }
  const paneId = workbenchWindow.value.focusedPaneId;
  const current = editorForPane(paneId);
  let editor: WorkbenchEditorInput;
  const reusesCurrentNewSession = resolvedMode === "reuse"
    && descriptor.resource.kind === "newSession"
    && current?.resource.kind === "newSession"
    && current.resource.projectId === descriptor.resource.projectId;
  if (reusesCurrentNewSession && current) {
    await focusWorkbenchEditor(paneId, current.editorId);
    editor = current;
  } else if (resolvedMode === "reuse") {
    editor = await replaceFocusedWorkbenchResource(descriptor);
  } else {
    editor = await openWorkbenchResource(descriptor, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: descriptor.resource.kind === "newSession",
    });
  }
  await nextTick();
  await restoreReplacedWorkspaceSessionDraft(descriptor.resource, editor.editorId);
  flashWorkspaceTreeEditorTabs([editor]);
  return editor;
}

async function executeWorkspaceSessionTreeNavigation(
  item: DevelopmentTreeItem,
  mode: WorkbenchSessionNavigationMode,
): Promise<WorkbenchEditorInput | null> {
  try {
    if (mode === "activate") {
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) return await openWorkspaceSessionDescriptor(descriptor, mode);
    }
    const project = workspaceContextStore.projectsById[item.meta.projectId];
    if (!project) return null;
    if (item.meta.kind === "newSession") {
      const checkout = await ensureProjectCheckout(project, item.meta.checkoutId, {
        refreshServices: false,
      });
      const descriptor = treeEditorDescriptor(item);
      if (!checkout || !descriptor) return null;
      return await openWorkspaceSessionDescriptor({
        ...descriptor,
        checkoutId: checkout.checkoutId,
      }, mode);
    }
    if (item.meta.kind !== "session" || !item.meta.session) return null;
    const preferred = workspaceContextStore.focusedCheckout?.projectId === project.projectId
      ? workspaceContextStore.focusedCheckout.checkoutId
      : item.meta.session.executionTarget?.checkoutId
        ?? item.meta.session.defaultCheckoutId;
    const checkout = await ensureProjectCheckout(project, preferred, {
      refreshServices: false,
    });
    const descriptor = treeEditorDescriptor(item);
    if (!checkout || !descriptor) return null;
    return await openWorkspaceSessionDescriptor({
      ...descriptor,
      checkoutId: checkout.checkoutId,
    }, mode);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
    return null;
  }
}

function activateWorkspaceSessionItem(item: DevelopmentTreeItem, event?: MouseEvent): void {
  if ((event?.detail ?? 1) > 1) return;
  void executeWorkspaceSessionTreeNavigation(item, workspaceSessionNavigationMode(item));
}

async function focusWorkbenchPane(paneId: string): Promise<void> {
  if (!workbenchStore.focusPane(WORKBENCH_WINDOW_ID, paneId)) return;
  const editor = editorForPane(paneId);
  if (!editor) {
    activeResource.value = null;
    workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, paneId);
    return;
  }
  await focusWorkbenchEditor(paneId, editor.editorId);
}

function editorMatchesWorkbenchScope(
  editor: WorkbenchEditorInput,
  projectId: string,
  checkoutId?: string | null,
): boolean {
  return checkoutId
    ? editor.checkoutBinding?.checkoutId === checkoutId
    : editor.resource.projectId === projectId;
}

function findWorkbenchScopeFallback(
  projectId: string,
  checkoutId: string | null,
  preferredPaneId: string,
): { paneId: string; editor: WorkbenchEditorInput } | null {
  const groups = Object.values(workbenchWindow.value.groups);
  const orderedGroups = [
    ...groups.filter((group) => group.paneId === preferredPaneId),
    ...groups.filter((group) => group.paneId !== preferredPaneId),
  ];
  for (const group of orderedGroups) {
    const active = group.tabs.find((editor) => editor.editorId === group.activeEditorId);
    if (active && editorMatchesWorkbenchScope(active, projectId, checkoutId)) {
      return { paneId: group.paneId, editor: active };
    }
    const editor = group.tabs.find((candidate) => (
      editorMatchesWorkbenchScope(candidate, projectId, checkoutId)
    ));
    if (editor) return { paneId: group.paneId, editor };
  }
  return null;
}

function openWorkbenchScopeFallback(
  projectId: string,
  checkoutId: string | null,
  paneId: string,
): { paneId: string; editor: WorkbenchEditorInput } {
  const input = createEditorForResource({ kind: "newSession", projectId }, {
    paneId,
    title: t("chat.session.newSession"),
    checkoutId,
    preview: true,
    pinned: false,
  });
  const editor = workbenchStore.openEditor(WORKBENCH_WINDOW_ID, input, {
    paneId,
    preview: true,
    pinned: false,
    replacePreview: false,
  });
  return { paneId, editor };
}

async function focusEmptyWorkbenchScope(
  projectId: string,
  checkoutId: string | null,
  paneId: string,
): Promise<void> {
  activeResource.value = { kind: "newSession", projectId };
  if (!checkoutId) {
    workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, paneId);
    return;
  }
  const context = await workspaceContextStore.focusCheckoutInPane(
    checkoutId,
    WORKBENCH_WINDOW_ID,
    paneId,
  );
  if (!context) return;
  await workspaceContextStore.setActiveSessionInPane(
    null,
    WORKBENCH_WINDOW_ID,
    paneId,
  );
  await refreshFocusedCheckoutServices();
}

async function disposeWorkbenchPaneContext(paneId: string): Promise<void> {
  await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, paneId);
}

async function closeWorkbenchEditor(
  paneId: string,
  editorId: string,
  force = false,
): Promise<void> {
  const groupBefore = workbenchWindow.value.groups[paneId];
  const closingEditor = groupBefore?.tabs.find((editor) => editor.editorId === editorId) ?? null;
  if (closingEditor?.dirty && !force) {
    dirtyEditorCloseDialog.value = {
      paneId,
      editorId,
      title: closingEditor.title,
    };
    return;
  }
  const wasActive = groupBefore?.activeEditorId === editorId;
  const paneCountBefore = Object.keys(workbenchWindow.value.groups).length;
  if (!workbenchStore.closeEditor(WORKBENCH_WINDOW_ID, paneId, editorId)) return;
  if (
    closingEditor?.resource.kind === "session"
    || closingEditor?.resource.kind === "newSession"
  ) clearSharedComposerDraft(`workbench:${editorId}`);
  const paneRemoved = paneCountBefore > Object.keys(workbenchWindow.value.groups).length;
  const focusedPaneId = workbenchWindow.value.focusedPaneId;

  if (props.auxiliary) {
    if (paneRemoved) await disposeWorkbenchPaneContext(paneId);
    if (!workbenchStore.hasEditors(WORKBENCH_WINDOW_ID)) {
      emit("empty");
      return;
    }
    const fallback = editorForPane(focusedPaneId);
    if (fallback) await focusWorkbenchEditor(focusedPaneId, fallback.editorId);
    return;
  }

  if (wasActive && closingEditor) {
    const projectId = closingEditor.resource.projectId;
    const checkoutId = closingEditor.checkoutBinding?.checkoutId ?? null;
    let fallback = findWorkbenchScopeFallback(projectId, checkoutId, focusedPaneId);
    if (!fallback && closingEditor.resource.kind !== "newSession") {
      fallback = openWorkbenchScopeFallback(projectId, checkoutId, focusedPaneId);
    }
    if (fallback) {
      await focusWorkbenchEditor(fallback.paneId, fallback.editor.editorId);
    } else {
      await focusEmptyWorkbenchScope(projectId, checkoutId, focusedPaneId);
    }
  } else if (paneRemoved) {
    await focusWorkbenchPane(focusedPaneId);
  }

  if (paneRemoved) await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, paneId);
}

async function continueQueuedWorkbenchEditorCloses(): Promise<void> {
  while (queuedWorkbenchEditorCloses.value.length > 0) {
    const target = queuedWorkbenchEditorCloses.value.shift();
    if (!target) return;
    const editor = workbenchWindow.value.groups[target.paneId]?.tabs.find(
      (candidate) => candidate.editorId === target.editorId,
    );
    if (!editor) continue;
    if (editor.dirty) {
      dirtyEditorCloseDialog.value = {
        paneId: target.paneId,
        editorId: target.editorId,
        title: editor.title,
      };
      return;
    }
    await closeWorkbenchEditor(target.paneId, target.editorId, true);
  }
}

async function closeWorkbenchEditors(paneId: string, editorIds: string[]): Promise<void> {
  const group = workbenchWindow.value.groups[paneId];
  if (!group) return;
  const requested = new Set(editorIds);
  const orderedEditors = group.tabs
    .filter((editor) => requested.has(editor.editorId))
    .sort((left, right) => {
      const leftRank = (left.dirty ? 0 : 2) + (left.editorId === group.activeEditorId ? 1 : 0);
      const rightRank = (right.dirty ? 0 : 2) + (right.editorId === group.activeEditorId ? 1 : 0);
      return leftRank - rightRank;
    });
  queuedWorkbenchEditorCloses.value = orderedEditors.map((editor) => ({
    paneId,
    editorId: editor.editorId,
  }));
  await continueQueuedWorkbenchEditorCloses();
}

function cancelDirtyEditorClose(): void {
  dirtyEditorCloseDialog.value = null;
  queuedWorkbenchEditorCloses.value = [];
}

async function saveAndCloseDirtyEditor(): Promise<void> {
  const dialog = dirtyEditorCloseDialog.value;
  if (!dialog) return;
  const saved = await workspaceFileEditorRefs.get(dialog.editorId)?.saveFile();
  if (!saved) return;
  dirtyEditorCloseDialog.value = null;
  await closeWorkbenchEditor(dialog.paneId, dialog.editorId, true);
  await continueQueuedWorkbenchEditorCloses();
}

async function discardAndCloseDirtyEditor(): Promise<void> {
  const dialog = dirtyEditorCloseDialog.value;
  if (!dialog) return;
  dirtyEditorCloseDialog.value = null;
  await closeWorkbenchEditor(dialog.paneId, dialog.editorId, true);
  await continueQueuedWorkbenchEditorCloses();
}

function pinWorkbenchEditor(paneId: string, editorId: string): void {
  workbenchStore.pinEditor(WORKBENCH_WINDOW_ID, paneId, editorId);
}

function handleWorkbenchComposerDraftChange(
  paneId: string,
  payload: { editorId: string; hasDraft: boolean },
): void {
  if (!payload.hasDraft) return;
  workbenchStore.pinEditor(WORKBENCH_WINDOW_ID, paneId, payload.editorId);
}

function resizeWorkbenchSplit(splitId: string, ratio: number, commit: boolean): void {
  workbenchStore.updateSplitRatio(WORKBENCH_WINDOW_ID, splitId, ratio, {
    persist: commit,
  });
}

async function splitFocusedWorkbenchEditor(
  direction: Exclude<WorkbenchDropDirection, "center"> = "right",
): Promise<void> {
  const paneId = workbenchWindow.value.focusedPaneId;
  const editor = editorForPane(paneId);
  if (!editor?.capabilities.split) return;
  const duplicate = createEditorForResource(editor.resource, {
    paneId,
    title: editor.title,
    checkoutId: editor.checkoutBinding?.checkoutId,
    sourcePath: editor.sourcePath,
    preview: false,
    pinned: true,
  });
  const newPaneId = workbenchStore.splitPane(
    WORKBENCH_WINDOW_ID,
    paneId,
    direction,
    duplicate,
  );
  if (newPaneId) await focusWorkbenchEditor(newPaneId, duplicate.editorId);
}

async function cycleFocusedEditor(delta: number): Promise<void> {
  const group = workbenchStore.activeGroup(WORKBENCH_WINDOW_ID);
  if (group.tabs.length < 2) return;
  const activeIndex = Math.max(0, group.tabs.findIndex(
    (editor) => editor.editorId === group.activeEditorId,
  ));
  const nextIndex = (activeIndex + delta + group.tabs.length) % group.tabs.length;
  const editor = group.tabs[nextIndex];
  if (editor) await focusWorkbenchEditor(group.paneId, editor.editorId);
}

function handleWorkbenchKeydown(event: KeyboardEvent): void {
  const primary = event.ctrlKey || event.metaKey;
  if (!primary || event.altKey) return;
  if (event.key === "\\") {
    event.preventDefault();
    void splitFocusedWorkbenchEditor("right");
    return;
  }
  if (event.key === "Tab") {
    event.preventDefault();
    void cycleFocusedEditor(event.shiftKey ? -1 : 1);
    return;
  }
  if (event.key.toLocaleLowerCase() === "w") {
    const group = workbenchStore.activeGroup(WORKBENCH_WINDOW_ID);
    if (!group.activeEditorId) return;
    event.preventDefault();
    void closeWorkbenchEditor(group.paneId, group.activeEditorId);
  }
}

function editorWorkspaceRef(editor: WorkbenchEditorInput): WorkspaceRef | null {
  const checkoutId = editor.checkoutBinding?.checkoutId;
  if (!checkoutId) return null;
  const runtime = workspaceContextStore.checkoutsById[checkoutId]?.runtime;
  const expectedGeneration = runtime?.workspaceGeneration
    ?? editor.checkoutBinding?.expectedGeneration
    ?? undefined;
  const cached = editorWorkspaceRefs.get(checkoutId);
  if (cached && cached.expectedGeneration === expectedGeneration) return cached;
  const workspaceRef: WorkspaceRef = {
    checkoutId,
    expectedGeneration,
  };
  editorWorkspaceRefs.set(checkoutId, workspaceRef);
  return workspaceRef;
}

function editorWorkingDir(editor: WorkbenchEditorInput): string {
  const checkoutId = editor.checkoutBinding?.checkoutId;
  return checkoutId ? workspaceContextStore.checkoutsById[checkoutId]?.root ?? "" : "";
}

function editorKnowledgeDocument(editor: WorkbenchEditorInput): ProjectKnowledgeDocument | null {
  const resource = editor.resource;
  if (resource.kind !== "knowledge") return null;
  return explorerStore.resources[resource.projectId]?.knowledge.find(
    (document) => document.id === resource.documentId,
  ) ?? null;
}

function editorProject(editor: WorkbenchEditorInput): ProjectContextDescriptor | null {
  return workspaceContextStore.projectsById[editor.resource.projectId] ?? null;
}

function setSessionEditorRef(
  editorId: string,
  value: unknown,
): void {
  if (value && typeof value === "object" && "applyDraftPrefill" in value) {
    sessionEditorRefs.set(editorId, value as InstanceType<typeof WorkbenchSessionEditor>);
  }
  else sessionEditorRefs.delete(editorId);
}

function setWorkspaceFileEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "saveFile" in value) {
    workspaceFileEditorRefs.set(editorId, value as InstanceType<typeof WorkspaceFilePreview>);
  } else {
    workspaceFileEditorRefs.delete(editorId);
  }
}

function setWorkbenchAssetEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "refreshIfChanged" in value) {
    workbenchAssetEditorRefs.set(editorId, value as InstanceType<typeof WorkbenchAssetEditor>);
  } else {
    workbenchAssetEditorRefs.delete(editorId);
  }
}

async function refreshWorkbenchFileEditor(editorId: string): Promise<void> {
  await nextTick();
  const editor = workspaceFileEditorRefs.get(editorId)
    ?? workbenchAssetEditorRefs.get(editorId);
  await editor?.refreshIfChanged("manual");
}

function setWorkbenchViewEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "ensureMounted" in value) {
    workbenchViewEditorRefs.set(editorId, value as InstanceType<typeof WorkbenchViewEditor>);
  } else {
    workbenchViewEditorRefs.delete(editorId);
  }
}

async function ensureWorkbenchViewEditorReady(editorId: string): Promise<void> {
  const editor = Object.values(workbenchWindow.value.groups)
    .flatMap((group) => group.tabs)
    .find((candidate) => candidate.editorId === editorId);
  if (editor?.resource.kind !== "view") return;
  const workspaceRef = editorWorkspaceRef(editor);
  if (!workspaceRef || !appWindow) throw new Error(t("workbench.unavailable.checkout"));
  await nextTick();
  const viewEditor = workbenchViewEditorRefs.get(editorId);
  if (!viewEditor) throw new Error("Workbench View editor did not mount.");
  await viewEditor.ensureMounted();
  await viewSetTabHost(workspaceRef, {
    hostLabel: appWindow.label,
    viewIds: [editor.resource.viewId],
    keepExistingForHost: true,
  });
}

async function openViewInWorkbench(payload: ViewWorkbenchOpenPayload): Promise<void> {
  if (payload.targetLabel && payload.targetLabel !== WORKBENCH_WINDOW_ID) return;
  const checkout = workspaceContextStore.checkoutsById[payload.workspaceRef.checkoutId];
  if (!checkout?.runtime) throw new Error(t("workbench.unavailable.checkout"));
  if (checkout.runtime.workspaceGeneration !== payload.workspaceRef.expectedGeneration) {
    throw new Error(t("workbench.unavailable.checkout"));
  }
  if (!await activateCheckoutScopedWorkbench(checkout.checkoutId)) return;
  if (WORKBENCH_WINDOW_ID === "main") uiStore.setPage("development");
  const existing = Object.values(workbenchWindow.value.groups).flatMap((group) => (
    group.tabs
      .filter((editor) => editor.resource.kind === "view"
        && editor.resource.projectId === checkout.projectId
        && editor.resource.viewId === payload.viewId
        && editor.checkoutBinding?.checkoutId === checkout.checkoutId)
      .map((editor) => ({ paneId: group.paneId, editor }))
  ))[0];
  if (existing) {
    await focusWorkbenchEditor(existing.paneId, existing.editor.editorId);
    await ensureWorkbenchViewEditorReady(existing.editor.editorId);
    await appWindow?.setFocus().catch(() => undefined);
    return;
  }
  const paneId = workbenchWindow.value.focusedPaneId;
  const editor = await openWorkbenchResource({
    resource: {
      kind: "view",
      projectId: checkout.projectId,
      viewId: payload.viewId,
    },
    title: payload.title || payload.viewId,
    checkoutId: checkout.checkoutId,
  }, {
    paneId,
    preview: false,
    pinned: true,
  });
  await ensureWorkbenchViewEditorReady(editor.editorId);
  await appWindow?.setFocus().catch(() => undefined);
}

async function openInspectorInWorkbench(payload: WorkbenchInspectorOpenPayload): Promise<void> {
  if (payload.targetLabel && payload.targetLabel !== WORKBENCH_WINDOW_ID) return;
  const checkout = workspaceContextStore.checkoutsById[payload.workspaceRef.checkoutId];
  if (!checkout) throw new Error(t("workbench.unavailable.checkout"));
  const expectedGeneration = payload.workspaceRef.expectedGeneration;
  if (
    expectedGeneration !== undefined
    && checkout.runtime?.workspaceGeneration !== expectedGeneration
  ) throw new Error(t("workbench.unavailable.checkout"));
  if (!await activateCheckoutScopedWorkbench(checkout.checkoutId)) return;
  const resource: DevelopmentResourceRef = payload.inspector.kind === "sceneObject"
    ? {
        kind: "sceneObject",
        projectId: checkout.projectId,
        scenePath: payload.inspector.scenePath ?? "",
        objectPath: payload.inspector.objectPath ?? "",
      }
    : {
        kind: "asset",
        projectId: checkout.projectId,
        path: payload.inspector.assetPath ?? "",
      };
  if (WORKBENCH_WINDOW_ID === "main") uiStore.setPage("development");
  const existing = Object.values(workbenchWindow.value.groups).flatMap((group) => (
    group.tabs
      .filter((editor) => workbenchResourceKey(editor.resource) === workbenchResourceKey(resource)
        && editor.checkoutBinding?.checkoutId === checkout.checkoutId)
      .map((editor) => ({ paneId: group.paneId, editor }))
  ))[0];
  if (existing) {
    await focusWorkbenchEditor(existing.paneId, existing.editor.editorId);
  } else {
    await openWorkbenchResource({
      resource,
      title: locusAssetInspectorTabTitle(payload.inspector),
      checkoutId: checkout.checkoutId,
    }, {
      paneId: workbenchWindow.value.focusedPaneId,
      preview: false,
      pinned: true,
    });
  }
  await appWindow?.setFocus().catch(() => undefined);
}

async function exportWorkbenchEditorTransferSnapshot(
  editor: WorkbenchEditorInput,
): Promise<WorkbenchEditorTransferSnapshot> {
  if (editor.resource.kind === "session" || editor.resource.kind === "newSession") {
    return sessionEditorRefs.get(editor.editorId)?.exportTransferSnapshot()
      ?? { kind: "session" };
  }
  if (
    editor.resource.kind === "workspaceFile"
    || editor.resource.kind === "localFile"
    || (editor.resource.kind === "asset" && isWorkbenchMarkdownPath(editor.resource.path))
  ) {
    return workspaceFileEditorRefs.get(editor.editorId)?.exportTransferSnapshot()
      ?? { kind: "resource" };
  }
  return { kind: "resource" };
}

async function applyWorkbenchEditorTransferSnapshot(
  editorId: string,
  snapshot: WorkbenchEditorTransferSnapshot | null | undefined,
): Promise<boolean> {
  if (!snapshot || snapshot.kind === "resource") return true;
  await nextTick();
  if (snapshot.kind === "session") {
    const draft = snapshot.composerDraft as UserMessageDraft | null | undefined;
    if (draft) await sessionEditorRefs.get(editorId)?.applyDraftPrefill(draft);
    return true;
  }
  return await workspaceFileEditorRefs.get(editorId)?.applyTransferSnapshot(snapshot) ?? false;
}

function waitForWorkbenchTransferAck(token: string): Promise<WorkbenchWindowTransferAckPayload> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      outgoingWorkbenchTransfers.delete(token);
      reject(new Error(t("workbench.window.transferTimeout")));
    }, WORKBENCH_TRANSFER_TIMEOUT_MS);
    outgoingWorkbenchTransfers.set(token, {
      targetLabel: "",
      resolve,
      reject,
      timer,
    });
  });
}

function handleWorkbenchTransferAck(payload: WorkbenchWindowTransferAckPayload): void {
  const pending = outgoingWorkbenchTransfers.get(payload.token);
  if (!pending) return;
  window.clearTimeout(pending.timer);
  outgoingWorkbenchTransfers.delete(payload.token);
  pending.resolve(payload);
}

async function finalizeTransferredSourceEditor(
  paneId: string,
  editorId: string,
): Promise<void> {
  const stateBefore = workbenchWindow.value;
  const groupBefore = stateBefore.groups[paneId];
  const wasActive = groupBefore?.activeEditorId === editorId;
  const paneCountBefore = Object.keys(stateBefore.groups).length;
  if (!workbenchStore.closeEditor(WORKBENCH_WINDOW_ID, paneId, editorId)) return;
  const paneRemoved = paneCountBefore > Object.keys(workbenchWindow.value.groups).length;
  if (paneRemoved) await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, paneId);
  const focusedPaneId = workbenchWindow.value.focusedPaneId;
  const active = editorForPane(focusedPaneId);
  if (active) {
    await focusWorkbenchEditor(focusedPaneId, active.editorId);
  } else if (wasActive || paneRemoved) {
    activeResource.value = null;
    workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, focusedPaneId);
    await workspaceContextStore.setActiveSessionInPane(
      null,
      WORKBENCH_WINDOW_ID,
      focusedPaneId,
    ).catch(() => null);
  }
  if (!workbenchStore.hasEditors(WORKBENCH_WINDOW_ID) && props.auxiliary) emit("empty");
}

async function transferWorkbenchEditor(
  paneId: string,
  editorId: string,
  options: {
    target?: WorkbenchWindowDropIntent;
    point?: { x: number; y: number };
    anchor?: { x: number; y: number };
  },
): Promise<void> {
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === editorId,
  );
  if (!editor?.capabilities.detach) return;
  const dragStartedAt = Date.now();
  const snapshot = await exportWorkbenchEditorTransferSnapshot(editor);
  const record = createInMemoryWorkbenchTransferRecord({
    sourceWindowId: WORKBENCH_WINDOW_ID,
    sourcePaneId: paneId,
    sourceEditorId: editorId,
    editor: {
      ...editor,
      resource: { ...editor.resource } as WorkbenchEditorInput["resource"],
      capabilities: { ...editor.capabilities },
      checkoutBinding: editor.checkoutBinding ? { ...editor.checkoutBinding } : null,
    },
    snapshot,
    target: options.target ? {
      windowId: options.target.windowId,
      paneId: options.target.paneId,
      direction: options.target.direction,
      index: options.target.index,
    } : null,
    dragStartedAt,
  });
  let targetLabel = options.target?.windowId ?? "";
  let persistedFallback = false;
  let detachedTargetCreated = false;
  recordWorkbenchWindowMetric("source-transfer-started", {
    token: record.token,
    startedAt: dragStartedAt,
    detail: {
      sourceWindowId: WORKBENCH_WINDOW_ID,
      targetWindowId: targetLabel || undefined,
      directDetach: !!options.point,
    },
  });
  try {
    let ack: WorkbenchWindowTransferAckPayload | null = null;
    if (options.target) {
      if (hasSharedWorkbenchTransferTarget(targetLabel)) {
        ack = await dispatchSharedWorkbenchTransfer(targetLabel, record, options.target);
      } else {
        await persistWorkbenchTransferRecord(record);
        persistedFallback = true;
        const ackPromise = waitForWorkbenchTransferAck(record.token);
        const pending = outgoingWorkbenchTransfers.get(record.token);
        if (pending) pending.targetLabel = targetLabel;
        await emitTo<WorkbenchWindowTransferPreparePayload>(
          targetLabel,
          WORKBENCH_WINDOW_TRANSFER_PREPARE_EVENT,
          { token: record.token, target: options.target },
        );
        ack = await ackPromise;
      }
    } else if (options.point) {
      const created = await createSharedDetachedWorkbenchWindow(
        record.token,
        options.point,
        dragStartedAt,
        options.anchor,
      );
      targetLabel = created.label;
      detachedTargetCreated = true;
      recordWorkbenchWindowMetric("detach-window-dispatched", {
        token: record.token,
        startedAt: dragStartedAt,
        detail: { targetLabel, pooled: created.pooled },
      });
      ack = await dispatchSharedWorkbenchTransfer(targetLabel, record, null);
    } else {
      throw new Error(t("workbench.window.targetUnavailable"));
    }

    if (!ack || ack.error || !ack.paneId || !ack.editorId) {
      throw new Error(ack?.error || t("workbench.window.targetUnavailable"));
    }
    if (editor.resource.kind === "view") {
      workbenchViewEditorRefs.get(editorId)?.relinquish();
    }
    await finalizeTransferredSourceEditor(paneId, editorId);
    recordWorkbenchWindowMetric("source-transfer-committed", {
      token: record.token,
      startedAt: dragStartedAt,
      detail: {
        sourceWindowId: WORKBENCH_WINDOW_ID,
        targetWindowId: ack.targetWindowId,
        paneId: ack.paneId,
      },
    });
  } catch (error) {
    recordWorkbenchWindowMetric("source-transfer-failed", {
      token: record.token,
      startedAt: dragStartedAt,
      detail: {
        sourceWindowId: WORKBENCH_WINDOW_ID,
        targetWindowId: targetLabel || undefined,
        error: normalizeAppError(error).message,
      },
    });
    const pending = outgoingWorkbenchTransfers.get(record.token);
    if (pending) {
      window.clearTimeout(pending.timer);
      outgoingWorkbenchTransfers.delete(record.token);
    }
    if (targetLabel) {
      void cancelSharedWorkbenchTransfer(targetLabel, record.token).catch(() => undefined);
      void emitTo<WorkbenchWindowTransferCancelPayload>(
        targetLabel,
        WORKBENCH_WINDOW_TRANSFER_CANCEL_EVENT,
        { token: record.token },
      ).catch(() => undefined);
    }
    if (detachedTargetCreated) {
      void removeSharedWorkbenchWindowHost(targetLabel).catch(() => undefined);
    }
    notificationStore.addNotice("error", normalizeAppError(error).message);
  } finally {
    if (persistedFallback) await removeWorkbenchTransferRecord(record.token);
  }
}

async function acceptWorkbenchTransferRecord(
  record: WorkbenchEditorTransferRecord,
  requestedTarget?: WorkbenchWindowDropIntent | null,
  emitAcknowledgement = false,
): Promise<WorkbenchWindowTransferAckPayload> {
  const token = record.token;
  recordWorkbenchWindowMetric("target-transfer-started", {
    token,
    startedAt: record.dragStartedAt,
    detail: { windowId: WORKBENCH_WINDOW_ID },
  });
  let target = requestedTarget?.windowId === WORKBENCH_WINDOW_ID
    ? requestedTarget
    : record.target?.windowId === WORKBENCH_WINDOW_ID
      ? record.target
      : {
          windowId: WORKBENCH_WINDOW_ID,
          paneId: workbenchWindow.value.focusedPaneId,
          direction: "center" as const,
        };
  let accepted: AcceptedWorkbenchTransfer | null = null;
  try {
    if (usesCheckoutScopedWorkbench()) {
      const checkoutId = record.editor.checkoutBinding?.checkoutId;
      if (!checkoutId) throw new Error(t("workbench.unavailable.checkout"));
      if (!await activateCheckoutScopedWorkbench(checkoutId)) {
        throw new Error(`Workbench checkout switch was superseded: ${checkoutId}`);
      }
      target = {
        ...target,
        paneId: workbenchWindow.value.focusedPaneId,
      };
    }
    const result = workbenchStore.acceptTransferredEditor(
      WORKBENCH_WINDOW_ID,
      record.editor,
      target.paneId,
      {
        direction: target.direction,
        index: target.index,
        allowDuplicate: record.allowDuplicate,
      },
    );
    if (!result) throw new Error(t("workbench.window.targetUnavailable"));
    accepted = result;
    acceptedWorkbenchTransfers.set(token, result);
    window.setTimeout(() => acceptedWorkbenchTransfers.delete(token), 30_000);
    await focusWorkbenchEditor(result.paneId, result.editorId, { refreshServices: false });
    await nextTick();
    if (!await applyWorkbenchEditorTransferSnapshot(result.editorId, record.snapshot)) {
      throw new Error(t("workbench.window.snapshotRestoreFailed"));
    }
    await ensureWorkbenchViewEditorReady(result.editorId);
    const acknowledgement: WorkbenchWindowTransferAckPayload = {
      token,
      targetWindowId: WORKBENCH_WINDOW_ID,
      paneId: result.paneId,
      editorId: result.editorId,
      inserted: result.inserted,
      readyAt: Date.now(),
    };
    if (emitAcknowledgement) {
      await emitTo<WorkbenchWindowTransferAckPayload>(
        record.sourceWindowId,
        WORKBENCH_WINDOW_TRANSFER_ACK_EVENT,
        acknowledgement,
      );
    }
    recordWorkbenchWindowMetric("target-editor-ready", {
      token,
      startedAt: record.dragStartedAt,
      detail: { windowId: WORKBENCH_WINDOW_ID, paneId: result.paneId },
    });
    emit("transfer-ready", token, record.dragStartedAt);
    void refreshFocusedCheckoutServices().catch((error) => {
      console.warn("[DevelopmentWorkbench] deferred transfer service refresh failed", error);
    });
    return acknowledgement;
  } catch (error) {
    recordWorkbenchWindowMetric("target-transfer-failed", {
      token,
      startedAt: record.dragStartedAt,
      detail: {
        windowId: WORKBENCH_WINDOW_ID,
        error: normalizeAppError(error).message,
      },
    });
    if (accepted?.inserted) {
      const paneCountBefore = Object.keys(workbenchWindow.value.groups).length;
      workbenchStore.closeEditor(WORKBENCH_WINDOW_ID, accepted.paneId, accepted.editorId);
      if (paneCountBefore > Object.keys(workbenchWindow.value.groups).length) {
        await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, accepted.paneId)
          .catch(() => false);
      }
    }
    acceptedWorkbenchTransfers.delete(token);
    const acknowledgement: WorkbenchWindowTransferAckPayload = {
      token,
      targetWindowId: WORKBENCH_WINDOW_ID,
      readyAt: Date.now(),
      error: normalizeAppError(error).message,
    };
    if (emitAcknowledgement) {
      await emitTo<WorkbenchWindowTransferAckPayload>(
        record.sourceWindowId,
        WORKBENCH_WINDOW_TRANSFER_ACK_EVENT,
        acknowledgement,
      ).catch(() => undefined);
    }
    if (!workbenchStore.hasEditors(WORKBENCH_WINDOW_ID) && props.auxiliary) emit("empty");
    return acknowledgement;
  }
}

async function acceptWorkbenchTransfer(
  token: string,
  requestedTarget?: WorkbenchWindowDropIntent | null,
): Promise<void> {
  const record = await readWorkbenchTransferRecord(token);
  if (record) await acceptWorkbenchTransferRecord(record, requestedTarget, true);
}

async function applyInitialWorkbenchTransfer(token: string): Promise<void> {
  const normalized = token.trim();
  if (!transferHostReady || !normalized || normalized === appliedInitialTransferToken) return;
  appliedInitialTransferToken = normalized;
  await acceptWorkbenchTransfer(normalized);
}

async function cancelAcceptedWorkbenchTransfer(token: string): Promise<void> {
  const accepted = acceptedWorkbenchTransfers.get(token);
  acceptedWorkbenchTransfers.delete(token);
  if (!accepted?.inserted) return;
  const paneCountBefore = Object.keys(workbenchWindow.value.groups).length;
  workbenchStore.closeEditor(WORKBENCH_WINDOW_ID, accepted.paneId, accepted.editorId);
  if (paneCountBefore > Object.keys(workbenchWindow.value.groups).length) {
    await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, accepted.paneId)
      .catch(() => false);
  }
  if (!workbenchStore.hasEditors(WORKBENCH_WINDOW_ID) && props.auxiliary) emit("empty");
}

function nativeWorkbenchTabDropIntentAt(x: number, y: number): WorkbenchWindowDropIntent | null {
  const hit = ownerDocument.elementFromPoint(x, y);
  if (hit?.nodeType !== 1 || !workbenchRootRef.value?.contains(hit)) return null;
  const tabStrip = hit.closest<HTMLElement>(
    ".workbench-editor-tabs[data-workbench-pane-id]",
  );
  if (tabStrip) {
    const paneId = tabStrip.dataset.workbenchPaneId ?? "";
    if (paneId && workbenchWindow.value.groups[paneId]) {
      const tabBounds = [...tabStrip.querySelectorAll<HTMLElement>("[data-workbench-tab-id]")]
        .map((tab) => tab.getBoundingClientRect());
      return {
        windowId: WORKBENCH_WINDOW_ID,
        paneId,
        direction: "center",
        index: workbenchTabInsertionIndexAtPoint(x, tabBounds),
      };
    }
  }
  const editorGroup = hit.closest<HTMLElement>(
    ".workbench-editor-group[data-workbench-pane-id]",
  );
  const paneId = editorGroup?.dataset.workbenchPaneId ?? "";
  const group = workbenchWindow.value.groups[paneId];
  if (!editorGroup || !group) return null;
  const bounds = editorGroup.getBoundingClientRect();
  const renderedTabStrip = editorGroup.querySelector<HTMLElement>(".workbench-editor-tabs");
  const direction = group.tabs.length === 0
    ? "center"
    : workbenchSplitDirectionAtPoint({ x, y }, {
        left: bounds.left,
        right: bounds.right,
        top: renderedTabStrip?.getBoundingClientRect().bottom ?? bounds.top,
        bottom: bounds.bottom,
      });
  return {
    windowId: WORKBENCH_WINDOW_ID,
    paneId,
    direction,
    index: group.tabs.length === 0 ? 0 : undefined,
  };
}

async function moveWorkbenchEditorInWindow(
  sourcePaneId: string,
  editorId: string,
  target: WorkbenchWindowDropIntent,
): Promise<void> {
  const movingEditor = workbenchWindow.value.groups[sourcePaneId]?.tabs.find(
    (editor) => editor.editorId === editorId,
  );
  if (movingEditor?.resource.kind === "view") {
    workbenchViewEditorRefs.get(editorId)?.relinquish();
  }
  const paneIdsBefore = new Set(Object.keys(workbenchWindow.value.groups));
  const destinationPaneId = workbenchStore.moveEditor(
    WORKBENCH_WINDOW_ID,
    sourcePaneId,
    editorId,
    target.paneId,
    { direction: target.direction, index: target.index },
  );
  for (const paneId of paneIdsBefore) {
    if (!workbenchWindow.value.groups[paneId]) {
      await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, paneId);
    }
  }
  if (destinationPaneId) {
    await focusWorkbenchPane(destinationPaneId);
    await ensureWorkbenchViewEditorReady(editorId);
  }
}

function workbenchWindowDragItem(
  tabId: string,
  anchor: { x: number; y: number },
): WorkbenchWindowTabDragItem | null {
  const source = Object.values(workbenchWindow.value.groups).find(
    (group) => group.tabs.some((editor) => editor.editorId === tabId),
  );
  const editor = source?.tabs.find((candidate) => candidate.editorId === tabId);
  if (!source || !editor?.capabilities.detach) return null;
  return {
    id: editor.editorId,
    title: editor.title,
    sourceWindowId: WORKBENCH_WINDOW_ID,
    sourcePaneId: source.paneId,
    anchor,
    move: (target) => moveWorkbenchEditorInWindow(source.paneId, editor.editorId, target),
    transfer: (target) => transferWorkbenchEditor(source.paneId, editor.editorId, { target }),
    detach: (point, detachAnchor) => transferWorkbenchEditor(
      source.paneId,
      editor.editorId,
      { point, anchor: detachAnchor },
    ),
  };
}

function handleWorkbenchTabExternalize(tab: BaseTabStripItem): void {
  const source = Object.values(workbenchWindow.value.groups).find(
    (group) => group.tabs.some((editor) => editor.editorId === tab.id),
  );
  const editor = source?.tabs.find((candidate) => candidate.editorId === tab.id);
  if (!source || !editor?.capabilities.detach) return;
  if (editor.dirty && editor.resource.kind === "knowledge") {
    notificationStore.addNotice("error", t("workbench.window.saveKnowledgeBeforeDetach"));
    return;
  }
  const anchor = { ...internalDrag.previewAnchor.value };
  const item = workbenchWindowDragItem(tab.id, anchor);
  if (item) workbenchWindowTabDrag.externalize(item, anchor);
}

function workspaceTreeTransferEditor(descriptor: TreeEditorDescriptor): WorkbenchEditorInput {
  return createEditorForResource(descriptor.resource, {
    title: descriptor.title,
    checkoutId: descriptor.checkoutId,
    sourcePath: descriptor.sourcePath,
    preview: false,
    pinned: true,
  });
}

function workspaceTreeTransferSnapshot(
  editor: WorkbenchEditorInput,
): WorkbenchEditorTransferSnapshot {
  return editor.resource.kind === "session" || editor.resource.kind === "newSession"
    ? { kind: "session" }
    : { kind: "resource" };
}

async function transferWorkspaceTreeEditors(
  descriptors: readonly TreeEditorDescriptor[],
  options: {
    target?: WorkbenchWindowDropIntent;
    point?: { x: number; y: number };
    anchor?: { x: number; y: number };
  },
): Promise<void> {
  if (descriptors.length === 0) return;
  const dragStartedAt = Date.now();
  let target = options.target ? { ...options.target } : null;
  let nextCenterIndex = target?.direction === "center" && target.index !== undefined
    ? target.index
    : undefined;

  for (const [descriptorIndex, descriptor] of descriptors.entries()) {
    const editor = workspaceTreeTransferEditor(descriptor);
    const record = createInMemoryWorkbenchTransferRecord({
      sourceWindowId: WORKBENCH_WINDOW_ID,
      sourcePaneId: workbenchWindow.value.focusedPaneId,
      sourceEditorId: editor.editorId,
      editor: {
        ...editor,
        resource: { ...editor.resource } as WorkbenchEditorInput["resource"],
        capabilities: { ...editor.capabilities },
        checkoutBinding: editor.checkoutBinding ? { ...editor.checkoutBinding } : null,
      },
      snapshot: workspaceTreeTransferSnapshot(editor),
      target,
      allowDuplicate: descriptor.resource.kind === "session"
        || descriptor.resource.kind === "newSession",
      dragStartedAt,
    });
    let targetLabel = target?.windowId ?? "";
    let persistedFallback = false;
    let detachedTargetCreated = false;
    recordWorkbenchWindowMetric("workspace-tree-transfer-started", {
      token: record.token,
      startedAt: dragStartedAt,
      detail: {
        sourceWindowId: WORKBENCH_WINDOW_ID,
        targetWindowId: targetLabel || undefined,
        descriptorIndex,
        descriptorCount: descriptors.length,
      },
    });

    try {
      let acknowledgement: WorkbenchWindowTransferAckPayload | null = null;
      if (target) {
        if (hasSharedWorkbenchTransferTarget(targetLabel)) {
          acknowledgement = await dispatchSharedWorkbenchTransfer(targetLabel, record, target);
        } else {
          await persistWorkbenchTransferRecord(record);
          persistedFallback = true;
          const acknowledgementPromise = waitForWorkbenchTransferAck(record.token);
          const pending = outgoingWorkbenchTransfers.get(record.token);
          if (pending) pending.targetLabel = targetLabel;
          await emitTo<WorkbenchWindowTransferPreparePayload>(
            targetLabel,
            WORKBENCH_WINDOW_TRANSFER_PREPARE_EVENT,
            { token: record.token, target },
          );
          acknowledgement = await acknowledgementPromise;
        }
      } else if (descriptorIndex === 0 && options.point) {
        const created = await createSharedDetachedWorkbenchWindow(
          record.token,
          options.point,
          dragStartedAt,
          options.anchor,
        );
        targetLabel = created.label;
        detachedTargetCreated = true;
        acknowledgement = await dispatchSharedWorkbenchTransfer(targetLabel, record, null);
      } else {
        throw new Error(t("workbench.window.targetUnavailable"));
      }

      if (
        !acknowledgement
        || acknowledgement.error
        || !acknowledgement.paneId
        || !acknowledgement.editorId
      ) {
        throw new Error(acknowledgement?.error || t("workbench.window.targetUnavailable"));
      }

      if (nextCenterIndex !== undefined) nextCenterIndex += 1;
      target = {
        windowId: acknowledgement.targetWindowId,
        paneId: acknowledgement.paneId,
        direction: "center",
        index: nextCenterIndex,
      };
      recordWorkbenchWindowMetric("workspace-tree-transfer-committed", {
        token: record.token,
        startedAt: dragStartedAt,
        detail: {
          targetWindowId: acknowledgement.targetWindowId,
          paneId: acknowledgement.paneId,
          descriptorIndex,
        },
      });
    } catch (error) {
      const pending = outgoingWorkbenchTransfers.get(record.token);
      if (pending) {
        window.clearTimeout(pending.timer);
        outgoingWorkbenchTransfers.delete(record.token);
      }
      if (targetLabel) {
        void cancelSharedWorkbenchTransfer(targetLabel, record.token).catch(() => undefined);
        void emitTo<WorkbenchWindowTransferCancelPayload>(
          targetLabel,
          WORKBENCH_WINDOW_TRANSFER_CANCEL_EVENT,
          { token: record.token },
        ).catch(() => undefined);
      }
      if (detachedTargetCreated) {
        void removeSharedWorkbenchWindowHost(targetLabel).catch(() => undefined);
      }
      recordWorkbenchWindowMetric("workspace-tree-transfer-failed", {
        token: record.token,
        startedAt: dragStartedAt,
        detail: {
          targetWindowId: targetLabel || undefined,
          descriptorIndex,
          error: normalizeAppError(error).message,
        },
      });
      notificationStore.addNotice("error", normalizeAppError(error).message);
      return;
    } finally {
      if (persistedFallback) await removeWorkbenchTransferRecord(record.token);
    }
  }
}

function workspaceTreeWindowDragItem(
  data: WorkspaceLayoutInternalDragData,
  anchor: { x: number; y: number },
): WorkbenchWindowTabDragItem | null {
  const sourceItems = data.items?.length ? data.items : [data.item];
  const descriptors = sourceItems
    .map(treeEditorDescriptor)
    .filter((descriptor): descriptor is TreeEditorDescriptor => descriptor !== null);
  const first = descriptors[0];
  if (!first) return null;
  const title = descriptors.length > 1 ? `${first.title} (${descriptors.length})` : first.title;
  return {
    id: `workspace-tree:${data.item.key}`,
    title,
    sourceWindowId: WORKBENCH_WINDOW_ID,
    sourcePaneId: workbenchWindow.value.focusedPaneId,
    anchor,
    move: (target) => transferWorkspaceTreeEditors(descriptors, { target }),
    transfer: (target) => transferWorkspaceTreeEditors(descriptors, { target }),
    detach: (point, detachAnchor) => transferWorkspaceTreeEditors(
      descriptors,
      { point, anchor: detachAnchor },
    ),
  };
}

function handleWorkspaceTreeExternalize(data: WorkspaceLayoutInternalDragData): void {
  const anchor = { ...internalDrag.previewAnchor.value };
  const item = workspaceTreeWindowDragItem(data, anchor);
  if (item) workbenchWindowTabDrag.externalize(item, anchor);
}

function setWorkspaceFileEditorDirty(
  paneId: string,
  editorId: string,
  dirty: boolean,
): void {
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editorId, { dirty });
}

async function handleWorkbenchSessionCreated(
  paneId: string,
  payload: { editorId: string; sessionId: string },
): Promise<void> {
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!editor || editor.resource.kind !== "newSession") return;
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, payload.editorId, {
    resource: {
      kind: "session",
      projectId: editor.resource.projectId,
      sessionId: payload.sessionId,
    },
    preview: false,
    pinned: true,
  });
  await workspaceContextStore.setActiveSessionInPane(
    payload.sessionId,
    WORKBENCH_WINDOW_ID,
    paneId,
  );
  await explorerStore.refreshProjectSessions(editor.resource.projectId);
}

async function handleWorkbenchSessionForked(
  paneId: string,
  payload: {
    editorId: string;
    sourceSessionId: string;
    forkedSessionId: string;
  },
): Promise<void> {
  const sourceEditor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!sourceEditor || sourceEditor.resource.kind !== "session") return;
  if (sourceEditor.resource.sessionId !== payload.sourceSessionId) return;
  const projectId = sourceEditor.resource.projectId;
  try {
    await explorerStore.refreshProjectSessions(projectId);
    const forkedSession = explorerStore.resources[projectId]?.sessions.find(
      (session) => session.id === payload.forkedSessionId,
    );
    await openWorkbenchResource({
      resource: {
        kind: "session",
        projectId,
        sessionId: payload.forkedSessionId,
      },
      title: forkedSession?.title || t("chat.session.forkTitle", sourceEditor.title),
      checkoutId: forkedSession?.executionTarget?.checkoutId
        ?? forkedSession?.defaultCheckoutId
        ?? sourceEditor.checkoutBinding?.checkoutId
        ?? undefined,
    }, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: true,
    });
    notificationStore.addNotice("success", t("chat.session.forked"), {
      operation: "forkSession",
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("chat.session.forkFailed", normalized.message), {
      code: normalized.code,
      operation: "forkSession",
      skipConsoleLog: true,
    });
  }
}

async function handleWorkbenchSessionExport(
  paneId: string,
  payload: { editorId: string; request: { sessionId: string } },
): Promise<void> {
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!editor || editor.resource.kind !== "session") return;
  if (editor.resource.sessionId !== payload.request.sessionId) return;
  await exportSessionContextToFile(payload.request.sessionId, editor.title);
}

async function reviewSessionInWorkbench(
  paneId: string,
  source: {
    projectId: string;
    checkoutId?: string | null;
    sessionId: string;
    title: string;
  },
): Promise<void> {
  const sourceSessionId = source.sessionId;
  const sourceTitle = source.title || sourceSessionId.slice(0, 8);

  try {
    const result = await exportSessionContext(sourceSessionId, null);
    const loadingName = sessionContextExportFileName(sourceSessionId, sourceTitle);
    const draft = buildContextReviewDraft(skillItems.value, t("chat.contextReviewPrompt"));
    draft.localFiles.push({
      name: contextReviewAttachmentName(result.filePath, loadingName),
      typeLabel: "YAML",
      path: result.filePath,
      isDir: false,
      source: "context-review",
    });
    const reviewEditor = await openWorkbenchResource({
      resource: {
        kind: "newSession",
        projectId: source.projectId,
      },
      title: t("chat.contextReviewTitle", sourceTitle),
      checkoutId: source.checkoutId ?? undefined,
    }, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: true,
    });
    await nextTick();
    const reviewSessionEditor = sessionEditorRefs.get(reviewEditor.editorId);
    if (!reviewSessionEditor) throw new Error("Workbench session editor did not mount.");
    await reviewSessionEditor.applyDraftPrefill(draft);
    await reviewSessionEditor.focusComposerInput();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("chat.contextReviewFailed", normalized.message), {
      code: normalized.code,
      operation: "reviewSessionContext",
      skipConsoleLog: true,
    });
  }
}

async function handleWorkbenchSessionReview(
  paneId: string,
  payload: { editorId: string; request: { sessionId: string } },
): Promise<void> {
  const sourceEditor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!sourceEditor || sourceEditor.resource.kind !== "session") return;
  if (sourceEditor.resource.sessionId !== payload.request.sessionId) return;
  const sourceSessionId = payload.request.sessionId;
  const sourceTitle = explorerStore.resources[sourceEditor.resource.projectId]?.sessions.find(
    (session) => session.id === sourceSessionId,
  )?.title || sourceEditor.title;
  await reviewSessionInWorkbench(paneId, {
    projectId: sourceEditor.resource.projectId,
    checkoutId: sourceEditor.checkoutBinding?.checkoutId,
    sessionId: sourceSessionId,
    title: sourceTitle,
  });
}

function normalizeKnowledgeReferencePath(path: string, docType: KnowledgeDocumentType): string {
  const normalized = path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  const prefix = `${docType}/`;
  return normalized.startsWith(prefix) ? normalized.slice(prefix.length) : normalized;
}

async function handleWorkbenchKnowledgeDocument(
  paneId: string,
  payload: {
    editorId: string;
    target: "editor" | "knowledge";
    request: { docType: KnowledgeDocumentType; path: string; workspaceRef: WorkspaceRef };
  },
): Promise<void> {
  const sourceEditor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!sourceEditor) return;
  const sourceWorkspaceRef = editorWorkspaceRef(sourceEditor);
  if (
    !sourceWorkspaceRef
    || sourceWorkspaceRef.checkoutId !== payload.request.workspaceRef.checkoutId
    || (
      payload.request.workspaceRef.expectedGeneration != null
      && sourceWorkspaceRef.expectedGeneration !== payload.request.workspaceRef.expectedGeneration
    )
  ) return;
  const projectId = sourceEditor.resource.projectId;
  try {
    const targetPath = normalizeKnowledgeReferencePath(
      payload.request.path,
      payload.request.docType,
    );
    let document = explorerStore.resources[projectId]?.knowledge.find((candidate) => (
      candidate.type === payload.request.docType
      && normalizeKnowledgeReferencePath(candidate.path, candidate.type) === targetPath
    ));
    if (!document) {
      await explorerStore.loadProject(projectId, true);
      document = explorerStore.resources[projectId]?.knowledge.find((candidate) => (
        candidate.type === payload.request.docType
        && normalizeKnowledgeReferencePath(candidate.path, candidate.type) === targetPath
      ));
    }
    if (!document) {
      notificationStore.addNotice("warning", t("workbench.unavailable.knowledge"), {
        operation: "openKnowledgeDocument",
        replaceOperation: true,
      });
      return;
    }
    await openWorkbenchResource({
      resource: { kind: "knowledge", projectId, documentId: document.id },
      title: knowledgeDocumentName(document),
      checkoutId: payload.request.workspaceRef.checkoutId,
    }, {
      paneId,
      preview: payload.target === "editor",
      pinned: payload.target === "knowledge",
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "openKnowledgeDocument",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  }
}

function newSessionShortcutAction(
  group: WorkbenchEditorGroup,
  editor: WorkbenchEditorInput,
): WorkbenchNewSessionShortcutAction {
  return workbenchNewSessionShortcutAction({
    currentIsNewSession: editor.resource.kind === "newSession",
    tabStripVisible: shouldShowWorkbenchTabStrip(
      group,
      props.auxiliary || workbenchWindow.value.layout.kind === "split",
    ),
  });
}

async function handleWorkbenchNewSessionRequested(
  paneId: string,
  payload: { editorId: string; source: "control" | "shortcut" },
): Promise<void> {
  const group = workbenchWindow.value.groups[paneId];
  const editor = group?.tabs.find((candidate) => candidate.editorId === payload.editorId);
  if (!editor || (
    editor.resource.kind !== "session"
    && editor.resource.kind !== "newSession"
  )) return;

  if (payload.source === "shortcut" && group) {
    const action = newSessionShortcutAction(group, editor);
    if (action === "keepCurrent") return;
    if (action === "newTab") {
      const newEditor = await openWorkbenchResource({
        resource: {
          kind: "newSession",
          projectId: editor.resource.projectId,
        },
        title: t("chat.session.newSession"),
        checkoutId: editor.checkoutBinding?.checkoutId ?? undefined,
      }, {
        paneId,
        preview: false,
        pinned: true,
        replacePreview: false,
        allowDuplicate: true,
      });
      await nextTick();
      await sessionEditorRefs.get(newEditor.editorId)?.focusComposerInput();
      return;
    }
  }

  const resource = {
    kind: "newSession" as const,
    projectId: editor.resource.projectId,
  };
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, payload.editorId, {
    resource,
    title: t("chat.session.newSession"),
  });
  if (group?.activeEditorId === payload.editorId) activeResource.value = resource;
  void workspaceContextStore.setActiveSessionInPane(
    null,
    WORKBENCH_WINDOW_ID,
    paneId,
  ).catch((error) => {
    console.warn("[DevelopmentWorkbench] failed to clear active session", error);
  });
}

let workbenchReconcileEpoch = 0;

async function reconcileRestoredWorkbenchEditors(
  expectedWorkspaceScopeId = workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID),
): Promise<void> {
  const epoch = ++workbenchReconcileEpoch;
  const projectIds = new Set(
    Object.values(workbenchWindow.value.groups).flatMap((group) => (
      group.tabs.map((editor) => editor.resource.projectId)
    )),
  );
  await Promise.all([...projectIds]
    .filter((projectId) => !!workspaceContextStore.projectsById[projectId])
    .map((projectId) => explorerStore.loadProject(projectId)));
  if (
    epoch !== workbenchReconcileEpoch
    || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId
  ) return;

  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (
        expectedWorkspaceScopeId
        && editor.checkoutBinding?.checkoutId !== expectedWorkspaceScopeId
      ) continue;
      const resource = editor.resource;
      const project = workspaceContextStore.projectsById[resource.projectId];
      let available = !!project;
      let sourcePath = editor.sourcePath ?? null;
      let reason: string | null = project ? null : t("workbench.unavailable.project");

      if (available) {
        switch (resource.kind) {
          case "checkout":
            available = !!workspaceContextStore.checkoutsById[resource.checkoutId];
            reason = available ? null : t("workbench.unavailable.checkout");
            break;
          case "folder":
            available = explorerStore.snapshots[resource.projectId]?.nodes.some(
              (node) => node.nodeId === resource.nodeId,
            ) === true;
            reason = available ? null : t("workbench.unavailable.removed");
            break;
          case "session":
            available = explorerStore.resources[resource.projectId]?.sessions.some(
              (session) => session.id === resource.sessionId,
            ) === true;
            reason = available ? null : t("workbench.unavailable.session");
            break;
          case "knowledge":
            available = explorerStore.resources[resource.projectId]?.knowledge.some(
              (document) => document.id === resource.documentId,
            ) === true;
            reason = available ? null : t("workbench.unavailable.knowledge");
            break;
          case "workspaceFile":
          case "asset":
          case "sceneObject": {
            const checkoutId = preferredCheckoutIdForResource(
              resource,
              group.paneId,
              editor.checkoutBinding?.checkoutId,
            );
            available = !!checkoutId && !!workspaceContextStore.checkoutsById[checkoutId];
            reason = available ? null : t("workbench.unavailable.file");
            break;
          }
          case "localDirectory":
          case "localFile": {
            const node = explorerStore.snapshots[resource.projectId]?.nodes.find(
              (candidate) => candidate.nodeId === resource.nodeId,
            );
            if (node && resource.relativePath) {
              try {
                await explorerStore.loadMount(resource.projectId, resource.nodeId);
              } catch {
                // The editor becomes unavailable below while the persisted tab stays closeable.
              }
              sourcePath = explorerStore.mountListing(
                resource.projectId,
                resource.nodeId,
              )?.entries.find(
                (entry) => entry.relativePath === resource.relativePath
                  && entry.isDir === (resource.kind === "localDirectory"),
              )?.absolutePath ?? sourcePath;
            } else if (node?.sourcePath) {
              sourcePath = node.sourcePath;
            }
            available = !!node && !!sourcePath;
            reason = available ? null : t("workbench.unavailable.file");
            break;
          }
          default:
            break;
        }
      }

      const checkoutId = preferredCheckoutIdForResource(
        editor.resource,
        group.paneId,
        editor.checkoutBinding?.checkoutId,
      );
      const checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : null;
      if (
        epoch !== workbenchReconcileEpoch
        || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId
      ) return;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        title: titleForResource(editor.resource, sourcePath),
        sourcePath,
        availability: available ? "available" : "unavailable",
        unavailableReason: reason,
        checkoutBinding: checkoutId
          ? {
              checkoutId,
              expectedGeneration: checkout?.runtime?.workspaceGeneration ?? null,
            }
          : null,
      });
    }
  }
}

async function restoreWorkbenchPaneContexts(
  expectedWorkspaceScopeId = workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID),
): Promise<void> {
  const focusedPaneId = workbenchWindow.value.focusedPaneId;
  for (const group of Object.values(workbenchWindow.value.groups)) {
    if (workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId) return;
    const editor = group.tabs.find((candidate) => candidate.editorId === group.activeEditorId);
    const checkoutId = editor?.checkoutBinding?.checkoutId;
    if (expectedWorkspaceScopeId && checkoutId && checkoutId !== expectedWorkspaceScopeId) {
      console.error(
        `[DevelopmentWorkbench] skipped foreign checkout ${checkoutId} in scope ${expectedWorkspaceScopeId}`,
      );
      continue;
    }
    if (!editor || editor.availability === "unavailable" || !checkoutId) {
      await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, group.paneId).catch(() => false);
      continue;
    }
    const context = await workspaceContextStore.focusCheckoutInPane(
      checkoutId,
      WORKBENCH_WINDOW_ID,
      group.paneId,
      { activate: false },
    );
    if (workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId) return;
    if (!context) continue;
    await workspaceContextStore.setActiveSessionInPane(
      editor.resource.kind === "session" ? editor.resource.sessionId : null,
      WORKBENCH_WINDOW_ID,
      group.paneId,
      { activate: false },
    );
  }
  if (workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId) return;
  await focusWorkbenchPane(focusedPaneId);
}

function isExpanded(key: string): boolean {
  return expanded.value.has(key);
}

function isSessionParentExpanded(key: string): boolean {
  return !collapsedSessionParents.value.has(key);
}

function isCollaborationExpanded(projectId: string): boolean {
  const resource = activeResource.value;
  return resource?.projectId === projectId
    && (
      resource.kind === "checkout"
      || resource.kind === "collaboration"
      || (resource.kind === "section" && resource.section === "collab")
    );
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
  if (node.resourceKind === SYSTEM_RESOURCE_KIND) {
    if (
      node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID
      && !displaySettings.workspaceSectionVisibility.knowledge
    ) return false;
    if (
      node.resourceId === COLLABORATION_SYSTEM_RESOURCE_ID
      && !displaySettings.workspaceSectionVisibility.collab
    ) return false;
    if (
      node.resourceId === ASSETS_SYSTEM_RESOURCE_ID
      && !displaySettings.workspaceSectionVisibility.assets
    ) return false;
    if (
      node.resourceId === VIEWS_SYSTEM_RESOURCE_ID
      && !displaySettings.workspaceSectionVisibility.views
    ) return false;
  }
  if ((node.resourceKind === "knowledge" && node.sourceKind !== "knowledge")
    || node.nodeId.startsWith("knowledge-type:")
    || node.nodeId.startsWith("knowledge-path:")) return false;
  if (node.hidden) return false;
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

function isKnowledgeFolderPlacement(item: DevelopmentTreeItem): boolean {
  const node = item.meta.explorerNode;
  return item.meta.kind === "folder"
    && node?.nodeKind === "folder"
    && node.sourceKind === "knowledge"
    && !!node.sourcePath;
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

function isActiveLocalFile(
  projectId: string,
  nodeId: string,
  relativePath?: string | null,
): boolean {
  const resource = activeResource.value;
  return resource?.kind === "localFile"
    && resource.projectId === projectId
    && resource.nodeId === nodeId
    && (resource.relativePath ?? null) === (relativePath ?? null);
}

function isActiveLocalDirectory(
  projectId: string,
  nodeId: string,
  relativePath?: string | null,
): boolean {
  const resource = activeResource.value;
  return resource?.kind === "localDirectory"
    && resource.projectId === projectId
    && resource.nodeId === nodeId
    && (resource.relativePath ?? null) === (relativePath ?? null);
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
    const selected = entry.isDir
      ? isActiveLocalDirectory(projectId, mountNode.nodeId, entry.relativePath)
      : isActiveLocalFile(projectId, mountNode.nodeId, entry.relativePath);
    items.push({
      key,
      treeRow: makeRow(key, entry.name, depth + entry.depth, entry.isDir ? "folder" : "file", {
        expandable: entry.isDir,
        expanded: entry.isDir ? isExpanded(key) : undefined,
        selected,
        dragEnabled: !entry.isDir,
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
      const selected = mountedDirectory
        ? isActiveLocalDirectory(project.projectId, node.nodeId)
        : activeResource.value?.kind === "folder"
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
            "is-knowledge-row": node.sourceKind === "knowledge" && mountedDirectory,
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
      const dropTarget: DevelopmentTreeItem = {
        key,
        treeRow: null,
        meta: {
          kind: "newSession",
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      };
      const dropAvailable = isNewSessionDropAvailable(dropTarget);
      items.push({
        ...dropTarget,
        treeRow: makeRow(
          key,
          dropAvailable
            ? t("development.dropToCreateSession")
            : t("chat.session.newSession"),
          depth,
          "file",
          {
            selected: activeResource.value?.kind === "newSession"
              && activeResource.value.projectId === project.projectId
              && chatStore.activeSessionId === null,
            dragEnabled: true,
            title: dropAvailable ? t("development.dropToCreateSession") : undefined,
            classes: {
              "is-new-session-row": true,
              "is-new-session-drop-zone": dropAvailable,
              "is-hidden-node": node.hidden,
            },
          },
        ),
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
      const selected = activeResource.value?.projectId === project.projectId
        && (
          activeResource.value.kind === "knowledgeRoot"
          || (activeResource.value.kind === "section" && activeResource.value.section === "knowledge")
        );
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
          selected: activeResource.value?.projectId === project.projectId
            && (
              activeResource.value.kind === "collaboration"
              || (activeResource.value.kind === "section" && activeResource.value.section === "collab")
            ),
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
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && (node.resourceId === ASSETS_SYSTEM_RESOURCE_ID || node.resourceId === VIEWS_SYSTEM_RESOURCE_ID)
    ) {
      const section = node.resourceId === ASSETS_SYSTEM_RESOURCE_ID ? "assets" as const : "views" as const;
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const kind = section === "assets" ? "assetsRoot" as const : "viewsRoot" as const;
      const key = `${section}:${project.projectId}`;
      const selected = activeResource.value?.kind === "section"
        && activeResource.value.projectId === project.projectId
        && activeResource.value.section === section;
      items.push({
        key,
        treeRow: makeRow(
          key,
          section === "assets" ? t("app.tab.asset") : t("app.tab.views"),
          depth,
          section === "assets" ? "folder" : "package",
          {
            selected,
            dragEnabled: true,
            classes: {
              "is-open": selected,
              "is-hidden-node": node.hidden,
            },
          },
        ),
        meta: {
          kind,
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      });
      continue;
    }
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === ARCHIVED_SYSTEM_RESOURCE_ID
    ) {
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const key = `archived:${project.projectId}`;
      const selected = activeResource.value?.kind === "section"
        && activeResource.value.projectId === project.projectId
        && activeResource.value.section === "archived";
      items.push({
        key,
        treeRow: makeRow(key, t("app.tab.archived"), depth, "folder", {
          selected,
          dragEnabled: true,
          classes: {
            "is-open": selected,
            "is-hidden-node": node.hidden,
          },
        }),
        meta: {
          kind: "archivedRoot",
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      });
      continue;
    }
    if (node.resourceKind === "session" && node.resourceId) {
      const session = sessionById.get(node.resourceId);
      if (!session) continue;
      const key = `session:${project.projectId}:${session.id}`;
      const layoutChildren = snapshot.nodes.some((candidate) => (
        candidate.parentNodeId === node.nodeId
        && candidate.resourceKind === "session"
        && sessionById.has(candidate.resourceId ?? "")
      ));
      const hasDropPreview = renderedIntent?.projectId === project.projectId
        && renderedIntent.parentNodeId === node.nodeId;
      const hasChildren = layoutChildren || hasDropPreview;
      const sessionExpanded = hasChildren && isSessionParentExpanded(key);
      const runtimeStatus = runtimeStatusByNodeId.get(node.nodeId) ?? null;
      const selected = isWorkspaceSessionSelected(project.projectId, session.id);
      const multiSelected = isWorkspaceSessionMultiSelected(session.id);
      const contextSelected = isWorkspaceSessionContextSelected(session.id);
      const displayTitle = sessionTreeDisplayTitle(
        session.title,
        session.sessionType,
        Boolean(session.parentSessionId),
      ) || t("chat.session.newSession");
      items.push({
        key,
        treeRow: makeRow(key, displayTitle, depth, "file", {
          expandable: hasChildren,
          expanded: sessionExpanded,
          selected: selected || multiSelected || contextSelected,
          editing: sessionInlineRename.value?.sessionId === session.id,
          dragEnabled: true,
          title: runtimeStatus
            ? `${session.title || displayTitle} — ${sessionStatusLabel(runtimeStatus)}`
            : session.title,
          classes: {
            ...runtimeStatusClasses(runtimeStatus, "session"),
            "is-open": selected,
            "is-multi-selected": multiSelected,
            "is-context-selected": contextSelected,
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
      if (sessionExpanded) {
        appendLayoutChildren(
          items,
          project,
          node.nodeId,
          depth + 1,
          sessionById,
          runtimeStatusByNodeId,
        );
      }
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
            "is-knowledge-row": true,
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
      const selected = isActiveLocalFile(project.projectId, node.nodeId);
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

function visibleWorkspaceSessionTargets(): DevelopmentSessionTarget[] {
  return treeItems.value.flatMap((item) => (
    item.meta.kind === "session" && item.meta.session
      ? [{ item, projectId: item.meta.projectId, session: item.meta.session }]
      : []
  ));
}

function activeWorkspaceSessionId(): string | null {
  const resource = activeResource.value;
  if (resource?.kind === "session") return resource.sessionId;
  return chatStore.activeSessionId;
}

function resolveSessionClickSelection(item: DevelopmentTreeItem, event?: MouseEvent): boolean {
  const sessionId = item.meta.session?.id;
  if (!sessionId) return false;
  const visibleSessionIds = visibleWorkspaceSessionTargets().map((target) => target.session.id);
  const result = resolveWorkspaceSessionSelection({
    visibleSessionIds,
    selectedSessionIds: selectedSessionIds.value,
    anchorSessionId: lastSessionSelectionAnchorId.value,
    activeSessionId: activeWorkspaceSessionId(),
    clickedSessionId: sessionId,
    shiftKey: event?.shiftKey ?? false,
    ctrlKey: event?.ctrlKey ?? false,
    metaKey: event?.metaKey ?? false,
  });
  selectedSessionIds.value = result.nextSelectedSessionIds;
  lastSessionSelectionAnchorId.value = result.nextAnchorSessionId;
  return result.shouldActivateSession;
}

function itemIcon(item: DevelopmentTreeItem) {
  switch (item.meta.kind) {
    case "project": {
      const project = workspaceContextStore.projectsById[item.meta.projectId];
      const projectIcon = projectIconForServices(project?.detectedServices ?? []);
      if (projectIcon) return projectIcon;
      return item.treeRow?.expanded ? FolderOpen : Folder;
    }
    case "collaboration": return GitMerge;
    case "assetsRoot": return Folder;
    case "viewsRoot": return Eye;
    case "archivedRoot": return Archive;
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

// Native file drags already carry an OS drag image. Keep the WebView fallback
// for semantic Unity drags that arrive without a native file payload.
const showWorkspaceDragFloatingPreview = computed(() => (
  workspaceDragPointer.value.visible
  && workspaceDragPreview.value !== null
  && !locusFileWorkspaceDragActive.value
));

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
    && source.payload.type !== WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
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
  const x = Math.max(8, Math.min(ownerWindow.innerWidth - width - 8, workspaceDragPointer.value.x + 14));
  const y = Math.max(8, Math.min(ownerWindow.innerHeight - height - 8, workspaceDragPointer.value.y + 12));
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
  options: { refreshServices?: boolean } = {},
): Promise<WorkspaceCheckoutDescriptor | null> {
  const focused = workspaceContextStore.focusedCheckout;
  const checkout = focused?.projectId === project.projectId
    ? focused
    : project.checkouts.find((candidate) => candidate.checkoutId === preferredCheckoutId)
      ?? project.checkouts[0];
  if (!checkout) return null;
  if (workspaceContextStore.focusedCheckout?.checkoutId !== checkout.checkoutId) {
    const context = await workspaceContextStore.focusCheckout(checkout.checkoutId);
    if (!context) return null;
    if (options.refreshServices !== false) await refreshFocusedCheckoutServices();
  }
  return workspaceContextStore.checkoutsById[checkout.checkoutId] ?? checkout;
}

function workspaceRefScopeKey(workspaceRef: WorkspaceRef | null): string | null {
  return workspaceRef
    ? `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}`
    : null;
}

async function refreshFocusedCheckoutServices(): Promise<void> {
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (!workspaceRef) return;
  const scopeKey = workspaceRefScopeKey(workspaceRef)!;
  const existingRefresh = pendingCheckoutServicesRefreshes.get(scopeKey);
  if (existingRefresh) {
    await existingRefresh;
    return;
  }
  const promise = Promise.all([
    chatStore.refreshSessions(),
    agentStore.loadWorkspaceAgents(workspaceRef),
    projectStore.checkUnityConnection(),
    projectStore.checkUnityPlugin(),
    projectStore.loadAssetDbStatus(),
  ]).then(() => {
    if (workspaceRefScopeKey(workspaceContextStore.focusedWorkspaceRef) === scopeKey) {
      lastRefreshedCheckoutServicesScopeKey = scopeKey;
    }
  });
  pendingCheckoutServicesRefreshes.set(scopeKey, promise);
  try {
    await promise;
  } finally {
    if (pendingCheckoutServicesRefreshes.get(scopeKey) === promise) {
      pendingCheckoutServicesRefreshes.delete(scopeKey);
    }
  }
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

function openSpecialNodesMenu(event: MouseEvent | FocusEvent): void {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  specialNodesMenu.value = { x: rect.right + 2, y: rect.top };
}

async function toggleSpecialNodeVisibility(node: ProjectExplorerNode): Promise<void> {
  const projectId = presetProjectId.value;
  if (!projectId || specialNodeVisibilityBusy.value.has(node.nodeId)) return;
  specialNodeVisibilityBusy.value = new Set(specialNodeVisibilityBusy.value).add(node.nodeId);
  try {
    await explorerStore.applyOperations(projectId, [{
      kind: "setNodeHidden",
      nodeId: node.nodeId,
      hidden: !node.hidden,
    }]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  } finally {
    const next = new Set(specialNodeVisibilityBusy.value);
    next.delete(node.nodeId);
    specialNodeVisibilityBusy.value = next;
  }
}

async function switchWorkspaceTreePreset(presetId: string): Promise<void> {
  const projectId = presetProjectId.value;
  if (!projectId || !presetId || presetId === activePresetId.value) return;
  try {
    await explorerStore.switchPreset(projectId, presetId);
    expanded.value = new Set([
      ...(displaySettings.workspaceDisplayMode === "multi" ? [`project:${projectId}`] : []),
    ]);
    collapsedSessionParents.value = new Set();
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
  await openWorkbenchResource({
    resource: {
      kind: "knowledge",
      projectId: project.projectId,
      documentId: document.id,
    },
    title: knowledgeDocumentName(document),
    checkoutId: document.sourceCheckoutId,
  }, { preview: true });
}

async function activateDirectoryPreviewEntry(
  paneId: string,
  editor: WorkbenchEditorInput,
  entry: ProjectExplorerMountEntry,
): Promise<void> {
  if (editor.resource.kind !== "localDirectory") return;
  await openWorkbenchResource({
    resource: entry.isDir ? {
      kind: "localDirectory",
      projectId: editor.resource.projectId,
      nodeId: editor.resource.nodeId,
      relativePath: entry.relativePath,
    } : {
      kind: "localFile",
      projectId: editor.resource.projectId,
      nodeId: editor.resource.nodeId,
      relativePath: entry.relativePath,
    },
    title: entry.name,
    checkoutId: editor.checkoutBinding?.checkoutId,
    sourcePath: entry.absolutePath,
  }, {
    paneId,
    preview: true,
  });
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
    const pinEditor = (event?.detail ?? 1) >= 2;
    explorerStore.selectedNodeKey = item.key;
    if (item.meta.kind === "folder") {
      resetSessionMultiSelection();
      if (item.treeRow?.expandable) toggleItem(item);
      if (item.meta.explorerNode?.sourcePath) {
        await explorerStore.loadMount(project.projectId, item.meta.explorerNode.nodeId);
      }
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (item.meta.kind === "mountedFolder") {
      toggleItem(item);
      resetSessionMultiSelection();
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (item.meta.kind === "newSession") {
      if (event?.ctrlKey || event?.metaKey) {
        const checkout = await ensureProjectCheckout(project, item.meta.checkoutId);
        if (!checkout) return;
        const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
        if (workspaceRef) {
          await openNewChatSessionWindow(workspaceRef, t("chat.session.newSession"));
        }
        return;
      }
      resetSessionMultiSelection();
      activateWorkspaceSessionItem(item, event);
      return;
    }
    if (item.meta.kind === "knowledgeRoot") {
      resetSessionMultiSelection();
      const checkout = await ensureProjectCheckout(project, item.meta.checkoutId);
      if (!checkout) return;
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree({
        ...descriptor,
        checkoutId: checkout.checkoutId,
      }, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (item.meta.kind === "collaboration") {
      resetSessionMultiSelection();
      const checkout = await ensureProjectCheckout(project);
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree({
        ...descriptor,
        checkoutId: checkout?.checkoutId,
      }, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (
      item.meta.kind === "assetsRoot"
      || item.meta.kind === "viewsRoot"
      || item.meta.kind === "archivedRoot"
    ) {
      resetSessionMultiSelection();
      const checkout = await ensureProjectCheckout(project, item.meta.checkoutId);
      const descriptor = treeEditorDescriptor(item);
      if (checkout && descriptor) await openWorkbenchResourceFromWorkspaceTree({
        ...descriptor,
        checkoutId: checkout.checkoutId,
      }, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (item.meta.kind === "checkout" && item.meta.checkoutId) {
      resetSessionMultiSelection();
      if (workspaceContextStore.focusedCheckout?.checkoutId !== item.meta.checkoutId) {
        await workspaceContextStore.focusCheckout(item.meta.checkoutId);
        await refreshFocusedCheckoutServices();
      }
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      collabHeadFocusRequest.value = {
        id: ++collabHeadFocusRequestId,
        checkoutId: item.meta.checkoutId,
      };
      return;
    }
    if (item.meta.kind === "session" && item.meta.session) {
      if (!resolveSessionClickSelection(item, event)) return;
      if (item.treeRow?.expandable) toggleItem(item);
      activateWorkspaceSessionItem(item, event);
      return;
    }
    if (item.meta.kind === "knowledge" && item.meta.knowledge) {
      resetSessionMultiSelection();
      const checkout = await ensureProjectCheckout(project, item.meta.knowledge.sourceCheckoutId);
      if (!checkout) return;
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree({
        ...descriptor,
        checkoutId: checkout.checkoutId,
      }, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
      return;
    }
    if (
      (item.meta.kind === "localFile" || item.meta.kind === "mountedFile")
      && (item.meta.explorerNode?.sourcePath || item.meta.mountEntry?.absolutePath)
    ) {
      resetSessionMultiSelection();
      const descriptor = treeEditorDescriptor(item);
      if (descriptor) await openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: !pinEditor,
        pinned: pinEditor,
      });
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function toggleItem(raw: WorkspaceTreeItem): void {
  const item = raw as DevelopmentTreeItem;
  if (item.meta.kind === "session") {
    const next = new Set(collapsedSessionParents.value);
    if (next.has(item.key)) next.delete(item.key);
    else next.add(item.key);
    collapsedSessionParents.value = next;
    return;
  }
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
  const targets = contextMenu.value?.sessionTargets ?? [];
  const target = targets.length === 1 ? targets[0] : null;
  const project = target ? workspaceContextStore.projectsById[target.projectId] : null;
  return target && project
    ? { item: target.item, project, session: target.session }
    : null;
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
  sessionInlineRename.value = {
    sessionId: entry.session.id,
    originalTitle: entry.session.title || "",
    value: entry.session.title || "",
  };
  void nextTick(() => {
    sessionRenameInput.value?.focus();
    sessionRenameInput.value?.select();
  });
}

function cancelSessionRename(): void {
  sessionInlineRename.value = null;
}

async function submitSessionRename(): Promise<void> {
  const draft = sessionInlineRename.value;
  if (!draft) return;
  const title = draft.value.trim();
  sessionInlineRename.value = null;
  if (!title || title === draft.originalTitle.trim()) return;
  await chatStore.renameSession(draft.sessionId, title);
}

function beginDeleteSession(): void {
  const targets = [...(contextMenu.value?.sessionTargets ?? [])];
  contextMenu.value = null;
  if (targets.length === 0) return;
  sessionDeleteDialog.value = {
    targets,
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
  void openWorkbenchResource({
    resource: { kind: "newSession", projectId },
    title: t("chat.session.newSession"),
    checkoutId: checkout.checkoutId,
  }, { preview: true });
}

function markWorkbenchSessionUnavailable(sessionId: string): void {
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (editor.resource.kind !== "session" || editor.resource.sessionId !== sessionId) continue;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        availability: "unavailable",
        unavailableReason: t("workbench.unavailable.session"),
      });
    }
  }
}

async function archiveSessionEntry(target: DevelopmentSessionTarget): Promise<void> {
  await chatStore.archiveSession(target.session.id);
  await explorerStore.refreshProjectSessions(target.projectId);
  markWorkbenchSessionUnavailable(target.session.id);
  if (activeResource.value?.kind === "session"
    && activeResource.value.sessionId === target.session.id) {
    resetActiveSessionResource(target.projectId);
  }
}

async function archiveSessionItem(item: DevelopmentTreeItem): Promise<void> {
  if (item.meta.kind !== "session" || !item.meta.session) return;
  const nextSelectedIds = new Set(selectedSessionIds.value);
  nextSelectedIds.delete(item.meta.session.id);
  selectedSessionIds.value = nextSelectedIds;
  if (lastSessionSelectionAnchorId.value === item.meta.session.id) {
    lastSessionSelectionAnchorId.value = null;
  }
  await archiveSessionEntry({
    item,
    projectId: item.meta.projectId,
    session: item.meta.session,
  });
}

async function archiveContextSession(): Promise<void> {
  const targets = [...(contextMenu.value?.sessionTargets ?? [])];
  contextMenu.value = null;
  if (targets.length === 0) return;
  resetSessionMultiSelection();
  for (const target of targets) await archiveSessionEntry(target);
}

async function commitSessionDeleteDialog(): Promise<void> {
  const dialog = sessionDeleteDialog.value;
  if (!dialog) return;
  resetSessionMultiSelection();
  for (const target of dialog.targets) {
    await chatStore.deleteSession(target.session.id);
    markWorkbenchSessionUnavailable(target.session.id);
    if (activeResource.value?.kind === "session"
      && activeResource.value.sessionId === target.session.id) {
      resetActiveSessionResource(target.projectId);
    }
  }
  sessionDeleteDialog.value = null;
}

async function exportSessionContextToFile(sessionId: string, title: string): Promise<void> {
  try {
    const filePath = await save({
      defaultPath: sessionContextExportFileName(sessionId, title || "untitled"),
      filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
    });
    if (!filePath) return;
    const result = await exportSessionContext(sessionId, filePath);
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

async function exportContextSession(): Promise<void> {
  const entry = contextSessionEntry();
  contextMenu.value = null;
  if (!entry) return;
  await exportSessionContextToFile(entry.session.id, entry.session.title || "untitled");
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
    await reviewSessionInWorkbench(workbenchWindow.value.focusedPaneId, {
      projectId: entry.project.projectId,
      checkoutId: checkout.checkoutId,
      sessionId: entry.session.id,
      title: entry.session.title,
    });
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
  if (item.meta.kind === "session" && item.meta.session) {
    const visibleTargets = visibleWorkspaceSessionTargets();
    const targetIds = resolveWorkspaceSessionContextIds({
      visibleSessionIds: visibleTargets.map((target) => target.session.id),
      selectedSessionIds: selectedSessionIds.value,
      targetSessionId: item.meta.session.id,
    });
    if (targetIds.length === 1 && !selectedSessionIds.value.has(item.meta.session.id)) {
      clearSessionMultiSelection();
    }
    const targetIdSet = new Set(targetIds);
    contextMenu.value = {
      x: event.clientX,
      y: event.clientY,
      item,
      sessionTargets: visibleTargets.filter((target) => targetIdSet.has(target.session.id)),
    };
    return;
  }
  resetSessionMultiSelection();
  contextMenu.value = { x: event.clientX, y: event.clientY, item };
}

async function removeContextWorkspace(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (props.fixedWorkspaceRef || item?.meta.kind !== "project") return;
  const project = workspaceContextStore.projectsById[item.meta.projectId];
  if (!project) return;

  const approved = await confirm(
    t("development.deleteWorkspaceConfirm", projectLabel(project)),
    {
      title: t("development.deleteWorkspace"),
      kind: "warning",
      okLabel: t("development.deleteWorkspaceAction"),
      cancelLabel: t("common.cancel"),
    },
  );
  if (!approved) return;

  const checkoutIds = new Set(project.checkouts.map((checkout) => checkout.checkoutId));
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  const removesCurrentScope = !!scopedCheckoutId && checkoutIds.has(scopedCheckoutId);
  const removesFocusedCheckout = !!workspaceContextStore.focusedCheckout
    && checkoutIds.has(workspaceContextStore.focusedCheckout.checkoutId);
  const fallbackCheckout = workspaceContextStore.projects
    .find((candidate) => candidate.projectId !== project.projectId)
    ?.checkouts[0] ?? null;

  try {
    const removed = await workspaceContextBaseStore.removeProject(project.projectId);
    if (!removed) return;
    await projectStore.loadRecentDirs();

    if (removesCurrentScope || removesFocusedCheckout) {
      if (fallbackCheckout) {
        await workspaceContextStore.focusCheckout(fallbackCheckout);
        await refreshFocusedCheckoutServices();
      } else {
        singleWorkspaceScopeId.value = null;
        await syncWorkbenchWorkspaceScope(null);
        await workspaceContextBaseStore.disposePane(
          WORKBENCH_WINDOW_ID,
          scopedWorkspacePaneId(),
        );
      }
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function openExplorerBackgroundContextMenu(event: MouseEvent): void {
  const target = event.target as HTMLElement | null;
  if (target?.closest("[data-tree-key]")) return;
  const projectId = activeResource.value?.projectId
    ?? workspaceContextStore.focusedProject?.projectId
    ?? visibleProjects.value[0]?.projectId;
  if (!projectId) return;
  event.preventDefault();
  resetSessionMultiSelection();
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

async function configureCurrentWorkspaceExtraWorkdirs(): Promise<void> {
  const runtime = workspaceContextStore.focusedRuntime;
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  workspaceMenu.value = null;
  if (!runtime || !workspaceRef) return;
  try {
    await openExtraWorkdirsWindow({
      workspacePath: runtime.root,
      workspaceRef,
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
  const target = event.target as Node | null;
  if (!inlineCreate.value || !target) return;
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
  if (!item || !node || node.resourceKind !== SYSTEM_RESOURCE_KIND) return;
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

async function removeKnowledgeItemFromWorkspace(item: DevelopmentTreeItem): Promise<void> {
  if (item.meta.kind !== "knowledge" || !item.meta.knowledge) return;
  try {
    await explorerStore.applyOperations(item.meta.projectId, [{
      kind: "removeResourcePlacement",
      resourceKind: "knowledge",
      resourceId: item.meta.knowledge.id,
    }]);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function removeContextKnowledgeItemFromWorkspace(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (!item) return;
  await removeKnowledgeItemFromWorkspace(item);
}

async function removeMountedNodeFromWorkspace(item: DevelopmentTreeItem): Promise<void> {
  const node = item?.meta.explorerNode;
  if (!node?.sourcePath) return;
  try {
    await explorerStore.applyOperations(item.meta.projectId, [{
      kind: "removeNode",
      nodeId: node.nodeId,
    }]);
    if (
      (activeResource.value?.kind === "localFile" || activeResource.value?.kind === "localDirectory")
      && activeResource.value.nodeId === node.nodeId
    ) {
      for (const group of Object.values(workbenchWindow.value.groups)) {
        for (const editor of group.tabs) {
          if (
            (editor.resource.kind !== "localFile" && editor.resource.kind !== "localDirectory")
            || editor.resource.nodeId !== node.nodeId
          ) continue;
          workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
            availability: "unavailable",
            unavailableReason: t("workbench.unavailable.removed"),
          });
        }
      }
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function removeKnowledgeFolderFromWorkspace(item: DevelopmentTreeItem): Promise<void> {
  if (!isKnowledgeFolderPlacement(item)) return;
  await removeMountedNodeFromWorkspace(item);
}

async function removeContextMountedNode(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (!item) return;
  await removeMountedNodeFromWorkspace(item);
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

function nativeWorkbenchDropDecisionAt(
  x: number,
  y: number,
): InternalDropDecision<WorkbenchInternalDropIntent> | null {
  const hit = ownerDocument.elementFromPoint(x, y);
  if (hit?.nodeType !== 1 || !workbenchRootRef.value?.contains(hit)) return null;
  const nativeFileCanOpenEditor = locusFileWorkspaceDragActive.value
    && locusFileWorkspaceTabEligible.value
    && !unityAssetWorkspaceDragActive.value;

  const composer = hit.closest<HTMLElement>(".chat-composer");
  const composerGroup = composer?.closest<HTMLElement>(
    ".workbench-editor-group[data-workbench-pane-id]",
  );
  if (composer && composerGroup) {
    const paneId = composerGroup.dataset.workbenchPaneId ?? "";
    const editor = paneId ? editorForPane(paneId) : null;
    if (paneId && editor && (
      editor.resource.kind === "session" || editor.resource.kind === "newSession"
    )) {
      return {
        key: `composer:${paneId}:${editor.editorId}`,
        operation: "copy",
        intent: { kind: "composer", paneId, editorId: editor.editorId },
      };
    }
  }

  const tabStrip = hit.closest<HTMLElement>(
    ".workbench-editor-tabs[data-workbench-pane-id]",
  );
  if (tabStrip && nativeFileCanOpenEditor) {
    const paneId = tabStrip.dataset.workbenchPaneId ?? "";
    const group = workbenchWindow.value.groups[paneId];
    if (paneId && group) {
      const tabBounds = [...tabStrip.querySelectorAll<HTMLElement>("[data-workbench-tab-id]")]
        .map((tab) => tab.getBoundingClientRect());
      const index = workbenchTabInsertionIndexAtPoint(x, tabBounds);
      return {
        key: `editor:${paneId}:center:${index}`,
        operation: "copy",
        intent: { kind: "editor", paneId, direction: "center", index },
      };
    }
  }

  const editorGroup = hit.closest<HTMLElement>(
    ".workbench-editor-group[data-workbench-pane-id]",
  );
  if (editorGroup && nativeFileCanOpenEditor) {
    const paneId = editorGroup.dataset.workbenchPaneId ?? "";
    if (paneId && workbenchWindow.value.groups[paneId]) {
      const bounds = editorGroup.getBoundingClientRect();
      const tab = editorGroup.querySelector<HTMLElement>(".workbench-editor-tabs");
      const direction = workbenchSplitDirectionAtPoint({ x, y }, {
        left: bounds.left,
        right: bounds.right,
        top: tab?.getBoundingClientRect().bottom ?? bounds.top,
        bottom: bounds.bottom,
      });
      return {
        key: `editor:${paneId}:${direction}`,
        operation: "copy",
        intent: { kind: "editor", paneId, direction },
      };
    }
  }

  if (!explorerRootRef.value?.contains(hit)) return null;
  const rowHit = developmentTreeItemFromHit(hit);
  const target = rowHit?.item ?? nearestExternalDropTarget(hit);
  externalDropTarget.value = target;
  if (target?.meta.kind === "newSession") {
    return {
      key: `new-session:${target.key}`,
      operation: "copy",
      intent: { kind: "newSession", target },
    };
  }
  const intent = rowHit
    ? resolveLayoutDropIntentAt(rowHit.item, y, rowHit.rowElement)
    : resolveExplorerRootDropIntent();
  return intent ? {
    key: `layout:${intent.targetKey}:${intent.position}`,
    operation: "copy",
    intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
  } : null;
}

function updateNativeWorkbenchDropTarget(x: number, y: number): void {
  const decision = nativeWorkbenchDropDecisionAt(x, y);
  handleWorkbenchInternalTargetChange(decision);
  if (!decision) externalDropTarget.value = null;
}

function clearNativeWorkbenchDropTarget(): void {
  externalDropTarget.value = null;
  layoutDropIntent.value = null;
  dropTargetKey.value = null;
  editorDropIntent.value = null;
  composerDropTarget.value = null;
}

function currentNativeWorkbenchDropIntent(): WorkbenchInternalDropIntent | null {
  if (composerDropTarget.value) return composerDropTarget.value;
  if (editorDropIntent.value) return editorDropIntent.value;
  if (externalDropTarget.value?.meta.kind === "newSession") {
    return { kind: "newSession", target: externalDropTarget.value };
  }
  if (layoutDropIntent.value) {
    return {
      kind: "layout",
      layout: layoutDropIntent.value,
      target: externalDropTarget.value,
    };
  }
  return null;
}

function nativeDropScope(intent: WorkbenchInternalDropIntent): WorkbenchReferenceDragData["origin"] | null {
  const paneId = intent.kind === "composer" || intent.kind === "editor" ? intent.paneId : null;
  const editor = paneId ? editorForPane(paneId) : null;
  const projectId = editor?.resource.projectId
    ?? (intent.kind === "newSession" ? intent.target.meta.projectId : null)
    ?? (intent.kind === "layout" ? intent.layout.projectId : null);
  if (!projectId) return null;
  const checkoutId = editor?.checkoutBinding?.checkoutId
    ?? (
      workspaceContextStore.focusedCheckout?.projectId === projectId
        ? workspaceContextStore.focusedCheckout.checkoutId
        : workspaceContextStore.projectsById[projectId]?.checkouts[0]?.checkoutId
    );
  const checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : null;
  if (!checkout) return null;
  return {
    projectId,
    workspaceRef: {
      checkoutId: checkout.checkoutId,
      expectedGeneration: checkout.runtime?.workspaceGeneration,
    },
    workspaceRoot: checkout.root,
  };
}

function nativeFileReferenceData(
  payload: LocusFileDropPayload,
  intent: WorkbenchInternalDropIntent,
): WorkbenchReferenceDragData | null {
  const origin = nativeDropScope(intent);
  if (!origin) return null;
  return {
    version: 1,
    origin,
    entries: payload.files.map((file) => ({
      kind: "file" as const,
      path: file.path,
      isDir: file.isDir,
      name: file.name,
      typeLabel: file.typeLabel,
    })),
  };
}

function nativeAssetReferenceData(
  payload: UnityEmbedAssetDropPayload,
  intent: WorkbenchInternalDropIntent,
): WorkbenchReferenceDragData | null {
  const origin = nativeDropScope(intent);
  if (!origin) return null;
  const entries = payload.refs.flatMap((ref): WorkbenchReferenceDragEntry[] => {
    if (ref.kind === "sceneObject") {
      const match = normalizeReferencePath(ref.path).match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
      return match ? [{
        kind: "sceneObject",
        scenePath: match[1]!,
        objectPath: match[2]!,
        name: ref.name,
        typeLabel: ref.typeLabel,
      }] : [];
    }
    if (ref.kind === "knowledge") {
      const match = normalizeReferencePath(ref.path).match(
        /^(?:Locus\/knowledge\/)?(design|plan|memory|skill|reference)\/(.+\.md)$/i,
      );
      if (!match) return [];
      return [{
        kind: "knowledge",
        type: match[1]!.toLocaleLowerCase() as KnowledgeDocumentType,
        path: `${match[1]!.toLocaleLowerCase()}/${match[2]}`,
        name: ref.name,
      } as WorkbenchReferenceDragEntry];
    }
    return [{
      kind: "asset",
      path: ref.path,
      name: ref.name,
      typeLabel: ref.typeLabel,
    }];
  });
  return entries.length > 0 ? { version: 1, origin, entries } : null;
}

function handleLocusFileDragState(payload: LocusFileDragStatePayload): void {
  locusFileWorkspaceDragActive.value = payload.active && payload.phase !== "leave";
  if (payload.fileCount > 0) locusFileWorkspaceDragCount.value = payload.fileCount;
  if (payload.phase === "enter" || payload.phase === "drop") {
    locusFileWorkspaceTabEligible.value = payload.tabEligible;
  }
  if (payload.active) {
    updateWorkspaceDragPointer(payload.x, payload.y);
    updateNativeWorkbenchDropTarget(payload.x, payload.y);
  }
  if (payload.phase === "leave") {
    locusFileWorkspaceDragCount.value = 0;
    locusFileWorkspaceTabEligible.value = false;
    clearWorkspaceDragPointer();
    if (unityAssetWorkspaceDragRefs.value.length > 0) {
      window.clearTimeout(unityWorkspaceDragStateClearTimer);
      unityWorkspaceDragStateClearTimer = window.setTimeout(() => {
        unityWorkspaceDragStateClearTimer = 0;
        unityAssetWorkspaceDragActive.value = false;
        unityAssetWorkspaceDragRefs.value = [];
      }, UNITY_WORKSPACE_DRAG_STATE_TTL_MS);
    }
    clearNativeWorkbenchDropTarget();
  }
}

async function handleLocusFileDrop(payload: LocusFileDropPayload): Promise<void> {
  const intent = currentNativeWorkbenchDropIntent();
  const data = intent ? nativeFileReferenceData(payload, intent) : null;
  clearNativeWorkbenchDropTarget();
  locusFileWorkspaceDragActive.value = false;
  locusFileWorkspaceDragCount.value = 0;
  locusFileWorkspaceTabEligible.value = false;
  clearWorkspaceDragPointer();
  if (!intent || !data || data.entries.length === 0) return;
  try {
    await commitWorkbenchInternalDrop(WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE, data, intent);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function handleWorkspaceUnityAssetDrop(
  payload: UnityEmbedAssetDropPayload,
): Promise<void> {
  const intent = currentNativeWorkbenchDropIntent();
  const data = intent ? nativeAssetReferenceData(payload, intent) : null;
  clearNativeWorkbenchDropTarget();
  window.clearTimeout(unityWorkspaceDragStateClearTimer);
  unityWorkspaceDragStateClearTimer = 0;
  unityAssetWorkspaceDragActive.value = false;
  unityAssetWorkspaceDragRefs.value = [];
  locusFileWorkspaceTabEligible.value = false;
  clearWorkspaceDragPointer();
  if (!intent || !data || data.entries.length === 0) return;
  try {
    await commitWorkbenchInternalDrop(WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE, data, intent);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function onDragPointerDown(raw: WorkspaceTreeItem, event: PointerEvent): void {
  const item = raw as DevelopmentTreeItem;
  if (settlingLayoutDrop.value || !item.meta.explorerNode || isKnowledgeTypeFolder(item)) return;
  const items = item.meta.kind === "session" && item.meta.session && selectedSessionIds.value.has(item.meta.session.id)
    ? treeItems.value.filter((candidate) => (
        candidate.meta.kind === "session"
        && candidate.meta.session
        && selectedSessionIds.value.has(candidate.meta.session.id)
      ))
    : [item];
  const dragData = { item, items } satisfies WorkspaceLayoutInternalDragData;
  const canExternalize = items.some((candidate) => treeEditorDescriptor(candidate) !== null);
  internalDrag.start(event, {
    id: `workspace-layout:${item.meta.projectId}:${item.meta.explorerNode.nodeId}`,
    payload: {
      type: WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE,
      data: dragData,
    },
    preview: {
      label: item.treeRow?.name ?? "",
      kind: item.treeRow?.kind ?? "item",
      icon: itemIcon(item),
      iconClass: itemIconClass(item),
      count: items.length,
    },
    allowedOperations: item.meta.kind === "mountedFile" ? ["copy"] : ["move", "copy"],
    cancelOnWindowBlur: canExternalize ? false : undefined,
    externalize: canExternalize ? () => handleWorkspaceTreeExternalize(dragData) : undefined,
    onActivated: () => {
      contextMenu.value = null;
      displayMenu.value = null;
      workspaceMenu.value = null;
    },
    onFinished: () => {
      dropTargetKey.value = null;
      layoutDropIntent.value = null;
      editorDropIntent.value = null;
      composerDropTarget.value = null;
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
  if (parentNodeId) {
    const parentNode = snapshot.nodes.find((node) => node.nodeId === parentNodeId);
    if (
      !parentNode
      || parentNode.nodeKind !== "folder"
    ) return null;
  }
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
  if (intent.parentNodeId) {
    const parentNode = snapshot.nodes.find((node) => node.nodeId === intent.parentNodeId);
    if (!parentNode) return false;
    if (parentNode.nodeKind !== "folder") return false;
  }
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
    if (!locusFileWorkspaceDragActive.value) clearNativeWorkbenchDropTarget();
    return;
  }
  unityAssetWorkspaceDragActive.value = true;
  unityAssetWorkspaceDragRefs.value = refs;
  unityWorkspaceDragStateClearTimer = window.setTimeout(() => {
    unityWorkspaceDragStateClearTimer = 0;
    if (locusFileWorkspaceDragActive.value) return;
    unityAssetWorkspaceDragActive.value = false;
    unityAssetWorkspaceDragRefs.value = [];
    if (!locusFileWorkspaceDragActive.value) clearNativeWorkbenchDropTarget();
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

function sendToLocusCheckout(workspaceRef: WorkspaceRef): WorkspaceCheckoutDescriptor | null {
  const checkout = workspaceContextStore.checkoutsById[workspaceRef.checkoutId];
  if (!checkout) return null;
  if (
    workspaceRef.expectedGeneration != null
    && checkout.runtime?.workspaceGeneration != null
    && checkout.runtime.workspaceGeneration !== workspaceRef.expectedGeneration
  ) return null;
  return checkout;
}

function clearLastFocusedComposerForWindow(): void {
  clearLastFocusedComposer(
    { surface: "workbench", windowId: WORKBENCH_WINDOW_ID },
    ownerWindow.localStorage,
  );
}

function handleWorkbenchComposerFocus(
  paneId: string,
  payload: { editorId: string },
): void {
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  const checkoutId = editor?.checkoutBinding?.checkoutId;
  if (!editor || !checkoutId) return;
  writeLastFocusedComposer({
    surface: "workbench",
    windowId: WORKBENCH_WINDOW_ID,
    paneId,
    editorId: editor.editorId,
    checkoutId,
  }, ownerWindow.localStorage);
}

function lastFocusedSendToLocusSessionEditor(checkoutId: string): {
  paneId: string;
  editor: WorkbenchEditorInput;
} | null {
  const target = readLastFocusedComposer(ownerWindow.localStorage);
  if (
    !target
    || target.surface !== "workbench"
    || target.windowId !== WORKBENCH_WINDOW_ID
    || target.checkoutId !== checkoutId
  ) return null;
  const editor = workbenchWindow.value.groups[target.paneId]?.tabs.find(
    (candidate) => candidate.editorId === target.editorId,
  );
  if (
    !editor
    || (editor.resource.kind !== "session" && editor.resource.kind !== "newSession")
    || editor.checkoutBinding?.checkoutId !== checkoutId
    || !sessionEditorRefs.has(editor.editorId)
  ) {
    clearLastFocusedComposerForWindow();
    return null;
  }
  return { paneId: target.paneId, editor };
}

async function handleUnitySendToLocus(
  payload: UnitySendToLocusEventPayload,
): Promise<void> {
  const focusTarget = readLastFocusedComposer(ownerWindow.localStorage);
  if (!focusTarget) {
    if (WORKBENCH_WINDOW_ID === "main") {
      notificationStore.addNotice("warning", t("workbench.sendToLocus.noFocusedComposer"));
    }
    return;
  }
  if (
    focusTarget.surface !== "workbench"
    || focusTarget.windowId !== WORKBENCH_WINDOW_ID
  ) return;

  const checkout = sendToLocusCheckout(payload.workspaceRef);
  if (!checkout || focusTarget.checkoutId !== checkout.checkoutId) return;
  const target = lastFocusedSendToLocusSessionEditor(checkout.checkoutId);
  if (!target) {
    notificationStore.addNotice("warning", t("workbench.sendToLocus.noFocusedComposer"));
    return;
  }

  const draft = attachmentDraft({
    assetRefs: payload.assetRefs,
    localFiles: payload.files,
  });
  if (draft.assetRefs.length === 0 && draft.localFiles.length === 0) return;

  await focusWorkbenchEditor(target.paneId, target.editor.editorId);
  await nextTick();
  await sessionEditorRefs.get(target.editor.editorId)?.appendComposerDraft(draft);
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

function normalizeReferencePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function isAbsoluteReferencePath(path: string): boolean {
  return /^[A-Za-z]:\//.test(path) || path.startsWith("//") || path.startsWith("/");
}

function absoluteReferencePath(path: string, workspaceRoot: string): string {
  const normalized = normalizeReferencePath(path);
  if (isAbsoluteReferencePath(normalized)) return normalized;
  return `${normalizeReferencePath(workspaceRoot)}/${normalized.replace(/^\/+/, "")}`;
}

function workspaceRelativeReferencePath(path: string, workspaceRoot: string): string | null {
  const normalized = normalizeReferencePath(path);
  if (!normalized) return null;
  if (!isAbsoluteReferencePath(normalized)) {
    if (normalized.split("/").some((segment) => segment === "..")) return null;
    return normalized.replace(/^\.\//, "");
  }
  const root = normalizeReferencePath(workspaceRoot);
  const prefix = `${root}/`;
  if (!root || !normalized.toLocaleLowerCase().startsWith(prefix.toLocaleLowerCase())) return null;
  return normalized.slice(prefix.length);
}

function knowledgeDocumentForReference(
  data: WorkbenchReferenceDragData,
  entry: Extract<WorkbenchReferenceDragEntry, { kind: "knowledge" }>,
): ProjectKnowledgeDocument | null {
  const documents = explorerStore.resources[data.origin.projectId]?.knowledge ?? [];
  if (entry.documentId) {
    const byId = documents.find((document) => document.id === entry.documentId);
    if (byId) return byId;
  }
  const key = normalizeReferencePath(entry.path).replace(/^Locus\/knowledge\//i, "").toLocaleLowerCase();
  return documents.find((document) => (
    `${document.type}/${normalizeReferencePath(document.path)}`.toLocaleLowerCase() === key
  )) ?? null;
}

function referenceAttachmentDraft(data: WorkbenchReferenceDragData): UserMessageDraft | null {
  const assetRefs: AssetRefAttachment[] = [];
  const localFiles: LocusFileDropRef[] = [];
  for (const entry of data.entries) {
    if (entry.kind === "knowledge") {
      assetRefs.push({
        kind: "knowledge",
        path: entry.path,
        name: entry.name,
        source: "manual",
      });
      continue;
    }
    if (entry.kind === "asset") {
      assetRefs.push({
        kind: "asset",
        path: entry.path,
        name: entry.name,
        typeLabel: entry.typeLabel,
        source: "manual",
      });
      continue;
    }
    if (entry.kind === "sceneObject") {
      assetRefs.push({
        kind: "sceneObject",
        path: `${entry.scenePath}/${entry.objectPath}`,
        name: entry.name,
        typeLabel: entry.typeLabel,
        source: "manual",
      });
      continue;
    }
    const attachment = workbenchComposerFileAttachment({
      absolutePath: absoluteReferencePath(entry.path, data.origin.workspaceRoot),
      workspaceRoot: data.origin.workspaceRoot,
      relativePath: workspaceRelativeReferencePath(entry.path, data.origin.workspaceRoot),
      name: entry.name,
      typeLabel: entry.typeLabel,
      source: "locus",
    });
    if (attachment?.assetRef) assetRefs.push(attachment.assetRef);
    if (attachment?.localFile) localFiles.push(attachment.localFile);
  }
  return assetRefs.length || localFiles.length
    ? attachmentDraft({ assetRefs, localFiles })
    : null;
}

async function mountedReferenceDescriptor(
  data: WorkbenchReferenceDragData,
  entry: Extract<WorkbenchReferenceDragEntry, { kind: "file" }>,
): Promise<TreeEditorDescriptor | null> {
  const absolutePath = absoluteReferencePath(entry.path, data.origin.workspaceRoot);
  await mountPaths(data.origin.projectId, null, [{
    path: absolutePath,
    isDir: entry.isDir,
    name: entry.name,
    typeLabel: entry.typeLabel,
    source: "local",
  }]);
  const normalized = normalizeReferencePath(absolutePath).toLocaleLowerCase();
  const node = explorerStore.snapshots[data.origin.projectId]?.nodes.find((candidate) => (
    normalizeReferencePath(candidate.sourcePath ?? "").toLocaleLowerCase() === normalized
  ));
  if (!node) return null;
  return {
    resource: {
      kind: entry.isDir ? "localDirectory" : "localFile",
      projectId: data.origin.projectId,
      nodeId: node.nodeId,
    },
    title: entry.name || shortPath(absolutePath),
    checkoutId: data.origin.workspaceRef.checkoutId,
    sourcePath: absolutePath,
  };
}

async function referenceEditorDescriptors(
  data: WorkbenchReferenceDragData,
): Promise<TreeEditorDescriptor[]> {
  await explorerStore.loadProject(data.origin.projectId);
  const descriptors: TreeEditorDescriptor[] = [];
  for (const entry of data.entries) {
    if (entry.kind === "knowledge") {
      const document = knowledgeDocumentForReference(data, entry);
      if (!document) continue;
      descriptors.push({
        resource: {
          kind: "knowledge",
          projectId: data.origin.projectId,
          documentId: document.id,
        },
        title: entry.name || knowledgeDocumentName(document),
        checkoutId: document.sourceCheckoutId || data.origin.workspaceRef.checkoutId,
      });
      continue;
    }
    if (entry.kind === "asset") {
      descriptors.push({
        resource: { kind: "asset", projectId: data.origin.projectId, path: entry.path },
        title: entry.name || shortPath(entry.path),
        checkoutId: data.origin.workspaceRef.checkoutId,
      });
      continue;
    }
    if (entry.kind === "sceneObject") {
      descriptors.push({
        resource: {
          kind: "sceneObject",
          projectId: data.origin.projectId,
          scenePath: entry.scenePath,
          objectPath: entry.objectPath,
        },
        title: entry.name || shortPath(entry.objectPath),
        checkoutId: data.origin.workspaceRef.checkoutId,
      });
      continue;
    }
    const mounted = await mountedReferenceDescriptor(data, entry);
    if (mounted) descriptors.push(mounted);
  }
  return descriptors;
}

async function placeWorkbenchReferenceDrag(
  intent: LayoutDropIntent,
  data: WorkbenchReferenceDragData,
): Promise<void> {
  if (intent.projectId !== data.origin.projectId) return;
  await explorerStore.loadProject(data.origin.projectId);
  const operations = data.entries.flatMap<ProjectExplorerOperation>((entry, index) => {
    const position = intent.position + index;
    if (entry.kind === "knowledge") {
      const document = knowledgeDocumentForReference(data, entry);
      if (document) {
        return [{
          kind: "placeResource" as const,
          resourceKind: "knowledge" as const,
          resourceId: document.id,
          sourceKind: "knowledge",
          parentNodeId: intent.parentNodeId,
          position,
        }];
      }
      return [{
        kind: "mountPath" as const,
        parentNodeId: intent.parentNodeId,
        path: absoluteReferencePath(`Locus/knowledge/${entry.path}`, data.origin.workspaceRoot),
        sourceKind: "knowledge" as const,
        name: entry.name,
        position,
      }];
    }
    const referencePath = entry.kind === "sceneObject" ? entry.scenePath : entry.path;
    return [{
      kind: "mountPath" as const,
      parentNodeId: intent.parentNodeId,
      path: absoluteReferencePath(referencePath, data.origin.workspaceRoot),
      sourceKind: "local" as const,
      name: entry.name,
      position,
    }];
  });
  if (operations.length === 0) return;
  const snapshot = await explorerStore.applyOperations(intent.projectId, operations);
  for (const entry of data.entries) {
    if (entry.kind !== "file" || !entry.isDir) continue;
    const absolutePath = absoluteReferencePath(entry.path, data.origin.workspaceRoot);
    const normalized = normalizeReferencePath(absolutePath).toLocaleLowerCase();
    const node = snapshot.nodes.find((candidate) => (
      normalizeReferencePath(candidate.sourcePath ?? "").toLocaleLowerCase() === normalized
    ));
    if (!node) continue;
    expanded.value = new Set([...expanded.value, `folder:${intent.projectId}:${node.nodeId}`]);
    void explorerStore.loadMount(intent.projectId, node.nodeId);
  }
}

function workspaceLayoutAttachmentDraft(
  data: WorkspaceLayoutInternalDragData,
  workspaceRoot: string,
): UserMessageDraft | null {
  if (!workspaceRoot) return null;
  const assetRefs: AssetRefAttachment[] = [];
  const localFiles: LocusFileDropRef[] = [];
  const items = data.items?.length ? data.items : [data.item];

  for (const item of items) {
    if (item.meta.kind === "knowledge" && item.meta.knowledge) {
      const path = item.meta.knowledge.path
        .replace(/\\/g, "/")
        .replace(/^\/+/, "");
      assetRefs.push({
        path: `${item.meta.knowledge.type}/${path}`,
        kind: "knowledge",
        name: item.treeRow?.name ?? knowledgeDocumentName(item.meta.knowledge),
        source: "manual",
      });
      continue;
    }

    const mountEntry = item.meta.kind === "mountedFile" ? item.meta.mountEntry : null;
    const explorerNode = item.meta.explorerNode;
    const absolutePath = mountEntry?.absolutePath
      ?? (item.meta.kind === "localFile" ? explorerNode?.sourcePath : null);
    if (!absolutePath) continue;
    const attachment = workbenchComposerFileAttachment({
      absolutePath,
      workspaceRoot,
      relativePath: mountEntry?.relativePath,
      name: item.treeRow?.name ?? mountEntry?.name,
      source: explorerNode?.sourceKind,
      knowledgeSource: explorerNode?.sourceKind === "knowledge",
    });
    if (attachment?.assetRef) assetRefs.push(attachment.assetRef);
    else if (attachment?.localFile) localFiles.push(attachment.localFile);
  }

  if (assetRefs.length === 0 && localFiles.length === 0) return null;
  return attachmentDraft({ assetRefs, localFiles });
}

function workspaceLayoutComposerDraft(
  data: WorkspaceLayoutInternalDragData,
  paneId: string,
  editorId?: string,
): UserMessageDraft | null {
  const targetEditor = editorId
    ? workbenchGroup(paneId)?.tabs.find((editor) => editor.editorId === editorId) ?? null
    : editorForPane(paneId);
  return targetEditor
    ? workspaceLayoutAttachmentDraft(data, editorWorkingDir(targetEditor))
    : null;
}

function workspaceLayoutNewSessionDraft(
  data: WorkspaceLayoutInternalDragData,
  target: DevelopmentTreeItem,
): UserMessageDraft | null {
  const checkoutId = target.meta.checkoutId;
  const workspaceRoot = checkoutId
    ? workspaceContextStore.checkoutsById[checkoutId]?.root ?? ""
    : "";
  return workspaceLayoutAttachmentDraft(data, workspaceRoot);
}

function newSessionDropDraft(
  sourceType: string,
  sourceData: WorkbenchInternalDragData,
  target: DevelopmentTreeItem,
): UserMessageDraft | null {
  if (target.meta.kind !== "newSession") return null;
  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    return workspaceLayoutNewSessionDraft(
      sourceData as WorkspaceLayoutInternalDragData,
      target,
    );
  }
  if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
    return referenceAttachmentDraft(sourceData as WorkbenchReferenceDragData);
  }
  if (sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE) {
    const assetRefs = knowledgeDragAssetRefs(
      (sourceData as KnowledgeInternalDragData).payload,
    );
    return assetRefs.length > 0 ? attachmentDraft({ assetRefs }) : null;
  }
  return null;
}

function isNewSessionDropAvailable(target: DevelopmentTreeItem): boolean {
  if (locusFileWorkspaceDragActive.value || unityAssetWorkspaceDragRefs.value.length > 0) {
    return true;
  }
  if (!internalDrag.dragging.value) return false;
  const source = internalDrag.source.value;
  return !!source && newSessionDropDraft(
    source.payload.type,
    source.payload.data as WorkbenchInternalDragData,
    target,
  ) !== null;
}

function composerDraftForInternalDrop(
  sourceType: string,
  sourceData: WorkbenchInternalDragData,
  paneId: string,
  editorId?: string,
): UserMessageDraft | null {
  if (sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE) {
    const refs = knowledgeDragAssetRefs(
      (sourceData as KnowledgeInternalDragData).payload,
    );
    return refs.length > 0 ? attachmentDraft({ assetRefs: refs }) : null;
  }
  if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
    return referenceAttachmentDraft(sourceData as WorkbenchReferenceDragData);
  }
  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    return workspaceLayoutComposerDraft(
      sourceData as WorkspaceLayoutInternalDragData,
      paneId,
      editorId,
    );
  }
  return null;
}

function composerAcceptsCurrentDrag(
  paneId: string,
  editor: WorkbenchEditorInput,
): boolean {
  if (editor.resource.kind !== "session" && editor.resource.kind !== "newSession") return false;
  if (internalDrag.dragging.value) {
    const source = internalDrag.source.value;
    if (!source) return false;
    return composerDraftForInternalDrop(
      source.payload.type,
      source.payload.data as WorkbenchInternalDragData,
      paneId,
      editor.editorId,
    ) !== null;
  }
  return locusFileWorkspaceDragActive.value
    || (unityAssetWorkspaceDragActive.value && unityAssetWorkspaceDragRefs.value.length > 0);
}

async function createNewSessionWithAttachments(
  target: DevelopmentTreeItem,
  draft: UserMessageDraft,
): Promise<void> {
  const project = workspaceContextStore.projectsById[target.meta.projectId];
  if (!project) return;
  const checkout = await ensureProjectCheckout(project, target.meta.checkoutId);
  if (!checkout) return;
  await createNewSessionWithAttachmentsForCheckout(checkout, draft);
}

async function createNewSessionWithAttachmentsForCheckout(
  checkout: WorkspaceCheckoutDescriptor,
  draft: UserMessageDraft,
): Promise<void> {
  const project = workspaceContextStore.projectsById[checkout.projectId];
  if (!project) return;
  let checkoutChanged = false;
  if (workspaceContextStore.focusedCheckout?.checkoutId !== checkout.checkoutId) {
    const paneId = workbenchWindow.value.focusedPaneId;
    const context = await workspaceContextStore.focusCheckoutInPane(
      checkout.checkoutId,
      WORKBENCH_WINDOW_ID,
      paneId,
    );
    if (!context) return;
    checkoutChanged = true;
  }
  if (displaySettings.workspaceDisplayMode === "single") {
    await adoptWorkbenchWorkspaceContext(checkout.checkoutId);
  }
  if (checkoutChanged) await refreshFocusedCheckoutServices();
  const focusedCheckout = workspaceContextStore.checkoutsById[checkout.checkoutId] ?? checkout;
  const editor = await openWorkbenchResource({
    resource: { kind: "newSession", projectId: project.projectId },
    title: t("chat.session.newSession"),
    checkoutId: focusedCheckout.checkoutId,
  }, {
    preview: false,
    pinned: true,
    replacePreview: false,
    allowDuplicate: true,
  });
  await nextTick();
  await sessionEditorRefs.get(editor.editorId)?.applyDraftPrefill(draft);
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
  context: InternalDropResolveContext<WorkbenchInternalDragData>,
): InternalDropDecision<WorkbenchInternalDropIntent> | null {
  const sourceType = context.source.payload.type;
  const sourceData = context.source.payload.data as WorkbenchInternalDragData;
  const sourceCanOpenEditor = sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE
    || (
      sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      && treeEditorDescriptor((sourceData as WorkspaceLayoutInternalDragData).item) !== null
    )
    || (
      sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE
      && (sourceData as KnowledgeInternalDragData).payload.entries.some(
        (entry) => entry.kind === "document" && !!entry.documentId,
      )
    );
  const referenceSourceCanOpenEditor = sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    && (sourceData as WorkbenchReferenceDragData).entries.length > 0;
  const canOpenEditor = sourceCanOpenEditor || referenceSourceCanOpenEditor;

  const composer = context.hit.closest<HTMLElement>(".chat-composer");
  const composerGroup = composer?.closest<HTMLElement>(
    ".workbench-editor-group[data-workbench-pane-id]",
  );
  if (composer && composerGroup && workbenchRootRef.value?.contains(composer)) {
    const paneId = composerGroup.dataset.workbenchPaneId ?? "";
    const editor = paneId ? editorForPane(paneId) : null;
    const acceptsComposerDrop = editor?.resource.kind === "session"
      || editor?.resource.kind === "newSession";
    if (
      paneId
      && editor
      && acceptsComposerDrop
      && composerDraftForInternalDrop(sourceType, sourceData, paneId, editor.editorId)
    ) {
      return {
        key: `composer:${paneId}:${editor.editorId}`,
        operation: "copy",
        intent: { kind: "composer", paneId, editorId: editor.editorId },
        previewMode: "inline",
      };
    }
  }

  const tabStrip = context.hit.closest<HTMLElement>(".workbench-editor-tabs[data-workbench-pane-id]");
  if (tabStrip && workbenchRootRef.value?.contains(tabStrip) && canOpenEditor) {
    const paneId = tabStrip.dataset.workbenchPaneId ?? "";
    const targetGroup = workbenchWindow.value.groups[paneId];
    if (paneId && targetGroup) {
      const tabBounds = [...tabStrip.querySelectorAll<HTMLElement>("[data-workbench-tab-id]")]
        .map((tab) => tab.getBoundingClientRect());
      const index = workbenchTabInsertionIndexAtPoint(context.point.x, tabBounds);
      return {
        key: `editor:${paneId}:center:${index}`,
        operation: sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE ? "move" : "copy",
        intent: { kind: "editor", paneId, direction: "center", index },
      };
    }
  }

  const editorGroup = context.hit.closest<HTMLElement>(
    ".workbench-editor-group[data-workbench-pane-id]",
  );
  if (editorGroup && workbenchRootRef.value?.contains(editorGroup) && canOpenEditor) {
    const paneId = editorGroup.dataset.workbenchPaneId ?? "";
    if (paneId && workbenchWindow.value.groups[paneId]) {
      const groupBounds = editorGroup.getBoundingClientRect();
      const renderedTabStrip = editorGroup.querySelector<HTMLElement>(".workbench-editor-tabs");
      const contentTop = renderedTabStrip?.getBoundingClientRect().bottom ?? groupBounds.top;
      const direction = workbenchSplitDirectionAtPoint(context.point, {
        left: groupBounds.left,
        right: groupBounds.right,
        top: contentTop,
        bottom: groupBounds.bottom,
      });
      return {
        key: `editor:${paneId}:${direction}`,
        operation: sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE ? "move" : "copy",
        intent: { kind: "editor", paneId, direction },
      };
    }
  }
  if (!explorerRootRef.value?.contains(context.hit)) return null;
  const rowHit = developmentTreeItemFromHit(context.hit);
  if (rowHit?.item.meta.kind === "dropPreview") {
    const intent = layoutDropIntent.value;
    return intent ? {
      key: `layout:${intent.targetKey}:${intent.position}`,
      operation: sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE ? "move" : "copy",
      intent: { kind: "layout", layout: intent, target: null },
    } : null;
  }

  if (
    rowHit?.item.meta.kind === "newSession"
    && newSessionDropDraft(sourceType, sourceData, rowHit.item)
  ) {
    return {
      key: `new-session:${rowHit.item.key}`,
      operation: "copy",
      intent: { kind: "newSession", target: rowHit.item },
    };
  }

  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    const source = (context.source.payload.data as WorkspaceLayoutInternalDragData).item;
    if (source.meta.kind === "mountedFile" || source.meta.kind === "mountedFolder") return null;
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

  if (
    sourceType !== KNOWLEDGE_INTERNAL_DRAG_TYPE
    && sourceType !== WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
  ) return null;
  const intent = rowHit
    ? resolveLayoutDropIntentAt(rowHit.item, context.point.y, rowHit.rowElement)
    : resolveExplorerRootDropIntent();
  if (!intent) return null;
  if (
    sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    && (sourceData as WorkbenchReferenceDragData).origin.projectId !== intent.projectId
  ) return null;
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
    editorDropIntent.value = null;
    composerDropTarget.value = null;
    return;
  }
  if (decision.intent.kind === "composer") {
    layoutDropIntent.value = null;
    dropTargetKey.value = null;
    editorDropIntent.value = null;
    composerDropTarget.value = decision.intent;
    return;
  }
  if (decision.intent.kind === "editor") {
    layoutDropIntent.value = null;
    dropTargetKey.value = null;
    editorDropIntent.value = decision.intent;
    composerDropTarget.value = null;
    return;
  }
  editorDropIntent.value = null;
  composerDropTarget.value = null;
  if (decision.intent.kind === "newSession") {
    layoutDropIntent.value = null;
    dropTargetKey.value = decision.intent.target.key;
    return;
  }
  activateLayoutDropIntent(decision.intent.layout, decision.intent.target);
}

async function commitWorkbenchInternalDrop(
  sourceType: string,
  sourceData: WorkbenchInternalDragData,
  intent: WorkbenchInternalDropIntent,
): Promise<void> {
  if (intent.kind === "composer") {
    const editor = workbenchGroup(intent.paneId)?.tabs.find(
      (candidate) => candidate.editorId === intent.editorId,
    );
    if (!editor || (
      editor.resource.kind !== "session"
      && editor.resource.kind !== "newSession"
    )) return;
    const draft = composerDraftForInternalDrop(
      sourceType,
      sourceData,
      intent.paneId,
      intent.editorId,
    );
    if (!draft) return;
    await focusWorkbenchEditor(intent.paneId, intent.editorId);
    await nextTick();
    await sessionEditorRefs.get(intent.editorId)?.appendComposerDraft(draft);
    return;
  }
  if (intent.kind === "editor") {
    if (sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE) {
      const data = sourceData as WorkbenchEditorTabInternalDragData;
      const movingEditor = workbenchWindow.value.groups[data.paneId]?.tabs.find(
        (editor) => editor.editorId === data.editorId,
      );
      if (movingEditor?.resource.kind === "view") {
        workbenchViewEditorRefs.get(data.editorId)?.relinquish();
      }
      const paneIdsBefore = new Set(Object.keys(workbenchWindow.value.groups));
      const destinationPaneId = workbenchStore.moveEditor(
        data.windowId,
        data.paneId,
        data.editorId,
        intent.paneId,
        { direction: intent.direction, index: intent.index },
      );
      for (const paneId of paneIdsBefore) {
        if (!workbenchWindow.value.groups[paneId]) {
          await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, paneId);
        }
      }
      if (destinationPaneId) {
        await focusWorkbenchPane(destinationPaneId);
        await ensureWorkbenchViewEditorReady(data.editorId);
      }
      return;
    }

    let descriptors: TreeEditorDescriptor[] = [];
    if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
      const data = sourceData as WorkspaceLayoutInternalDragData;
      descriptors = (data.items?.length ? data.items : [data.item])
        .map(treeEditorDescriptor)
        .filter((descriptor): descriptor is TreeEditorDescriptor => descriptor !== null);
    } else if (sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE) {
      const targetEditor = editorForPane(intent.paneId);
      const projectId = targetEditor?.resource.projectId ?? presetProjectId.value;
      descriptors = (sourceData as KnowledgeInternalDragData).payload.entries.flatMap((entry) => {
        if (entry.kind !== "document" || !entry.documentId || !projectId) return [];
        const document = explorerStore.resources[projectId]?.knowledge.find(
          (candidate) => candidate.id === entry.documentId,
        );
        return [{
          resource: { kind: "knowledge", projectId, documentId: entry.documentId } as DevelopmentResourceRef,
          title: document ? knowledgeDocumentName(document) : entry.name,
          checkoutId: document?.sourceCheckoutId,
        }];
      });
    } else if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
      descriptors = await referenceEditorDescriptors(sourceData as WorkbenchReferenceDragData);
    }
    if (descriptors.length === 0) return;

    let destinationPaneId = intent.paneId;
    if (usesCheckoutScopedWorkbench()) {
      const checkoutIds = new Set(
        descriptors
          .map((descriptor) => descriptor.checkoutId?.trim())
          .filter((checkoutId): checkoutId is string => !!checkoutId),
      );
      if (checkoutIds.size > 1) {
        throw new Error("A single-workspace drop cannot mix checkout bindings.");
      }
      const checkoutId = checkoutIds.values().next().value
        ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
      if (!checkoutId) throw new Error(t("workbench.unavailable.checkout"));
      if (!await activateCheckoutScopedWorkbench(checkoutId)) return;
      destinationPaneId = workbenchWindow.value.focusedPaneId;
    }
    if (intent.direction === "center") {
      for (const descriptor of descriptors) {
        await openWorkbenchResource(descriptor, {
          paneId: destinationPaneId,
          preview: false,
          pinned: true,
          replacePreview: false,
          allowDuplicate: descriptor.resource.kind === "session"
            || descriptor.resource.kind === "newSession",
        });
      }
      return;
    }

    const firstInput = createEditorForResource(descriptors[0]!.resource, {
      paneId: destinationPaneId,
      title: descriptors[0]!.title,
      checkoutId: descriptors[0]!.checkoutId,
      sourcePath: descriptors[0]!.sourcePath,
      preview: false,
      pinned: true,
    });
    destinationPaneId = workbenchStore.splitPane(
      WORKBENCH_WINDOW_ID,
      destinationPaneId,
      intent.direction,
      firstInput,
    ) ?? destinationPaneId;
    for (const descriptor of descriptors.slice(1)) {
      await openWorkbenchResource(descriptor, {
        paneId: destinationPaneId,
        preview: false,
        pinned: true,
        focus: false,
      });
    }
    await focusWorkbenchEditor(destinationPaneId, firstInput.editorId);
    return;
  }
  if (intent.kind === "newSession") {
    const draft = newSessionDropDraft(sourceType, sourceData, intent.target);
    if (draft) await createNewSessionWithAttachments(intent.target, draft);
    return;
  }
  if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
    if (intent.kind === "layout") {
      await moveExplorerNodeToIntent((sourceData as WorkspaceLayoutInternalDragData).item, intent.layout);
    }
    return;
  }
  if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
    const data = sourceData as WorkbenchReferenceDragData;
    if (intent.kind === "layout") await placeWorkbenchReferenceDrag(intent.layout, data);
    return;
  }
  if (sourceType !== KNOWLEDGE_INTERNAL_DRAG_TYPE) return;
  const payload = (sourceData as KnowledgeInternalDragData).payload;
  if (intent.kind === "layout") await placeKnowledgeWorkspaceDrag(intent.layout, payload);
}

const workbenchInternalDropTarget: InternalDropTargetRegistration<
  WorkbenchInternalDragData,
  WorkbenchInternalDropIntent
> = {
  id: `development-workbench:${WORKBENCH_WINDOW_ID}`,
  root: () => workbenchRootRef.value,
  accepts: (source) => source.payload.type === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
    || source.payload.type === KNOWLEDGE_INTERNAL_DRAG_TYPE
    || source.payload.type === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    || source.payload.type === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
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
      editorDropIntent.value = null;
      composerDropTarget.value = null;
    }
  },
  previewMode: ({ hit }) => {
    if (hit.closest(".workspace-tree-row-shell.is-new-session-row")) return "inline";
    return explorerRootRef.value?.contains(hit) ? "floating-with-gap" : "floating";
  },
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
  const target = event.target as Node | null;
  if (target?.nodeType === 1 && (target as Element).closest(".workspace-tree-row-shell")) return;
  externalDropTarget.value = nearestExternalDropTarget(
    target?.nodeType === 1 ? target as Element : null,
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
  if (mode === "single") {
    singleWorkspaceScopeId.value = workspaceContextStore.focusedCheckout?.checkoutId
      ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID)
      ?? null;
  }
  setDisplaySetting("workspaceDisplayMode", mode);
  displayMenu.value = null;
  workspaceMenu.value = null;
}

let workbenchWorkspaceScopeEpoch = 0;
let activeWorkbenchWorkspaceScopeSync: {
  scopeId: string | null;
  promise: Promise<void>;
} | null = null;

function usesCheckoutScopedWorkbench(): boolean {
  return !!props.fixedWorkspaceRef
    || (!props.auxiliary && displaySettings.workspaceDisplayMode === "single");
}

function assertFixedWorkspaceScope(checkoutId: string): void {
  if (props.fixedWorkspaceRef && props.fixedWorkspaceRef.checkoutId !== checkoutId) {
    throw new Error(
      `Workbench ${WORKBENCH_WINDOW_ID} is fixed to checkout ${props.fixedWorkspaceRef.checkoutId}.`,
    );
  }
}

async function adoptWorkbenchWorkspaceContext(checkoutId: string): Promise<void> {
  if (!usesCheckoutScopedWorkbench()) return;
  assertFixedWorkspaceScope(checkoutId);
  if (!props.fixedWorkspaceRef) singleWorkspaceScopeId.value = checkoutId;
  await syncWorkbenchWorkspaceScope(checkoutId);
  if (!props.fixedWorkspaceRef && singleWorkspaceScopeId.value !== checkoutId) return;
  if (workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== checkoutId) {
    throw new Error(`Workbench checkout switch was superseded: ${checkoutId}`);
  }
}

async function activateCheckoutScopedWorkbench(checkoutId: string): Promise<boolean> {
  if (!usesCheckoutScopedWorkbench()) return true;
  assertFixedWorkspaceScope(checkoutId);
  if (!workspaceContextBaseStore.checkoutsById[checkoutId]) {
    throw new Error(t("workbench.unavailable.checkout"));
  }
  await adoptWorkbenchWorkspaceContext(checkoutId);
  if (
    workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== checkoutId
    || workbenchWorkspaceScopeId.value !== checkoutId
  ) return false;
  const paneId = workbenchWindow.value.focusedPaneId;
  const paneContext = workspaceContextBaseStore.paneContextAt(WORKBENCH_WINDOW_ID, paneId);
  if (paneContext?.focusedCheckoutId !== checkoutId) {
    const context = await workspaceContextBaseStore.focusCheckoutInPane(
      checkoutId,
      WORKBENCH_WINDOW_ID,
      paneId,
    );
    if (!context) return false;
  }
  return workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) === checkoutId
    && workbenchWorkspaceScopeId.value === checkoutId;
}

function syncWorkbenchWorkspaceScope(nextWorkspaceScopeId: string | null): Promise<void> {
  const currentSync = activeWorkbenchWorkspaceScopeSync;
  if (currentSync?.scopeId === nextWorkspaceScopeId) return currentSync.promise;
  if (
    workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) === nextWorkspaceScopeId
    && workbenchWorkspaceScopeId.value === nextWorkspaceScopeId
  ) return Promise.resolve();

  const epoch = ++workbenchWorkspaceScopeEpoch;
  const isCurrent = () => (
    epoch === workbenchWorkspaceScopeEpoch
    && workbenchWorkspaceScopeId.value === nextWorkspaceScopeId
  );
  const promise = (async () => {
    if (!isCurrent()) return;
    const state = workbenchStore.switchWorkspaceScope(
      WORKBENCH_WINDOW_ID,
      nextWorkspaceScopeId,
    );
    if (
      !isCurrent()
      || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== nextWorkspaceScopeId
    ) return;
    const activeEditor = workbenchStore.activeEditor(WORKBENCH_WINDOW_ID);
    activeResource.value = activeEditor?.resource ?? null;
    resetSessionMultiSelection();
    collabHeadFocusRequest.value = null;

    const checkout = nextWorkspaceScopeId
      ? workspaceContextStore.checkoutsById[nextWorkspaceScopeId] ?? null
      : null;
    const hasOpenTabs = Object.values(state.groups).some((group) => group.tabs.length > 0);
    if (checkout && !hasOpenTabs && !props.auxiliary) {
      if (!isCurrent()) return;
      await openWorkbenchResource({
        resource: { kind: "newSession", projectId: checkout.projectId },
        title: t("chat.session.newSession"),
        checkoutId: checkout.checkoutId,
      }, {
        preview: true,
        focus: false,
      });
      if (!isCurrent()) return;
    }

    if (checkout) await explorerStore.loadProject(checkout.projectId);
    if (
      !isCurrent()
      || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== nextWorkspaceScopeId
    ) return;
    await reconcileRestoredWorkbenchEditors(nextWorkspaceScopeId);
    if (
      !isCurrent()
      || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== nextWorkspaceScopeId
    ) return;
    await restoreWorkbenchPaneContexts(nextWorkspaceScopeId);
  })();
  activeWorkbenchWorkspaceScopeSync = { scopeId: nextWorkspaceScopeId, promise };
  return promise.finally(() => {
    if (activeWorkbenchWorkspaceScopeSync?.promise === promise) {
      activeWorkbenchWorkspaceScopeSync = null;
    }
  });
}

async function activateCheckoutOverview(checkoutId: string): Promise<void> {
  const item = treeItems.value.find((candidate) => candidate.meta.checkoutId === checkoutId);
  if (item) await activateItem(item);
}

watch(
  workbenchWorkspaceScopeId,
  (workspaceScopeId) => {
    void syncWorkbenchWorkspaceScope(workspaceScopeId).catch((error) => {
      console.warn("[DevelopmentWorkbench] workspace layout switch failed", error);
    });
  },
  { flush: "sync" },
);

watch(
  () => props.initialTransferToken,
  (token) => {
    if (token) void applyInitialWorkbenchTransfer(token);
  },
);

watch(
  [visibleProjects, () => workspaceContextStore.focusedProject?.projectId] as const,
  ([projects]) => {
    const next = new Set(expanded.value);
    for (const project of projects) {
      next.add(`project:${project.projectId}`);
      void explorerStore.loadProject(project.projectId);
    }
    expanded.value = next;
    if (!props.auxiliary && !activeResource.value && projects[0]) {
      const checkout = workspaceContextStore.focusedCheckout?.projectId === projects[0].projectId
        ? workspaceContextStore.focusedCheckout
        : projects[0].checkouts[0];
      if (checkout) {
        void openWorkbenchResource({
          resource: { kind: "newSession", projectId: projects[0].projectId },
          title: t("chat.session.newSession"),
          checkoutId: checkout.checkoutId,
        }, { preview: true });
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
    void explorerStore.refreshProjectSessions(projectId)
      .then(() => reconcileRestoredWorkbenchEditors())
      .catch((error) => {
        console.warn("[DevelopmentWorkbench] session catalog refresh failed", error);
      });
  },
);

watch(
  () => Object.entries(explorerStore.resources).map(([projectId, resources]) => [
    projectId,
    resources.sessions.map((session) => `${session.id}:${session.title}`).join(","),
    resources.knowledge.map((document) => `${document.id}:${document.title}:${document.path}`).join(","),
  ].join("|")).join("\n"),
  () => {
    for (const group of Object.values(workbenchWindow.value.groups)) {
      for (const editor of group.tabs) {
        const title = titleForResource(editor.resource, editor.sourcePath);
        if (title !== editor.title) {
          workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, { title });
        }
      }
    }
    void reconcileRestoredWorkbenchEditors().catch((error) => {
      console.warn("[DevelopmentWorkbench] editor reconciliation failed", error);
    });
  },
);

watch(activeResource, (resource) => {
  if (resource?.kind !== "checkout") {
    collabHeadFocusRequest.value = null;
  }
});

onMounted(() => {
  unregisterWorkbenchInternalDropTarget = internalDrag.registerTarget(workbenchInternalDropTarget);
  ownerDocument.addEventListener("pointerdown", handleInlineCreatePointerDown, true);
  ownerWindow.addEventListener("drag", trackWorkspaceDragPointer, true);
  ownerWindow.addEventListener("dragenter", trackWorkspaceDragPointer, true);
  ownerWindow.addEventListener("dragover", trackWorkspaceDragPointer, true);
  ownerWindow.addEventListener("drop", handleWindowWorkspaceDrop, true);
  ownerWindow.addEventListener("dragend", clearWorkspaceDragPointer, true);
  const restorePromise = Promise.all(
    visibleProjects.value.map((project) => explorerStore.loadProject(project.projectId)),
  ).then(async () => {
    await reconcileRestoredWorkbenchEditors();
    await restoreWorkbenchPaneContexts();
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] workbench restore failed", error);
  });
  void (async () => {
    if (appWindow) {
      unlistenWorkbenchTransferPrepare = await appWindow.listen<WorkbenchWindowTransferPreparePayload>(
        WORKBENCH_WINDOW_TRANSFER_PREPARE_EVENT,
        (event) => void acceptWorkbenchTransfer(event.payload.token, event.payload.target),
      );
      unlistenWorkbenchTransferAck = await appWindow.listen<WorkbenchWindowTransferAckPayload>(
        WORKBENCH_WINDOW_TRANSFER_ACK_EVENT,
        (event) => handleWorkbenchTransferAck(event.payload),
      );
      unlistenWorkbenchTransferCancel = await appWindow.listen<WorkbenchWindowTransferCancelPayload>(
        WORKBENCH_WINDOW_TRANSFER_CANCEL_EVENT,
        (event) => void cancelAcceptedWorkbenchTransfer(event.payload.token),
      );
      unlistenViewWorkbenchOpen = await appWindow.listen<ViewWorkbenchOpenPayload>(
        VIEW_WORKBENCH_OPEN_EVENT,
        (event) => {
          void openViewInWorkbench(event.payload).catch((error) => {
            notificationStore.addNotice("error", normalizeAppError(error).message);
          });
        },
      );
      unlistenWorkbenchInspectorOpen = await appWindow.listen<WorkbenchInspectorOpenPayload>(
        WORKBENCH_INSPECTOR_OPEN_EVENT,
        (event) => {
          void openInspectorInWorkbench(event.payload).catch((error) => {
            notificationStore.addNotice("error", normalizeAppError(error).message);
          });
        },
      );
    }
    await restorePromise;
    await openInitialSessionIfRequested();
    initialWorkspaceFallbackActive = false;
    await nextTick();
    for (const group of Object.values(workbenchWindow.value.groups)) {
      const editor = group.tabs.find((candidate) => candidate.editorId === group.activeEditorId);
      if (editor?.resource.kind === "view") {
        await ensureWorkbenchViewEditorReady(editor.editorId).catch((error) => {
          console.warn("[DevelopmentWorkbench] failed to restore View editor", error);
        });
      }
    }
    if (props.prewarm && !workbenchStore.hasEditors(WORKBENCH_WINDOW_ID)) {
      const checkout = workspaceContextBaseStore.focusedCheckout;
      if (checkout) {
        await workspaceContextBaseStore.focusCheckoutInPane(
          checkout,
          WORKBENCH_WINDOW_ID,
          workbenchWindow.value.focusedPaneId,
          { activate: false },
        );
      }
    }
    transferHostReady = true;
    unregisterSharedTransferTarget = registerSharedWorkbenchTransferTarget(
      WORKBENCH_WINDOW_ID,
      {
        accept: (record, target) => acceptWorkbenchTransferRecord(record, target),
        cancel: cancelAcceptedWorkbenchTransfer,
      },
    );
    if (props.initialTransferToken) {
      await applyInitialWorkbenchTransfer(props.initialTransferToken);
    } else {
      emit("ready");
    }
  })().catch((error) => {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  });
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
  void subscribeUnitySendToLocus((payload) => {
    void handleUnitySendToLocus(payload);
  }).then((release) => {
    releaseUnitySendToLocus = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] Send to Locus subscription failed", error);
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
  transferHostReady = false;
  unregisterSharedTransferTarget?.();
  unregisterSharedTransferTarget = null;
  workbenchWindowTabDrag.dispose();
  unlistenWorkbenchTransferPrepare?.();
  unlistenWorkbenchTransferPrepare = null;
  unlistenWorkbenchTransferAck?.();
  unlistenWorkbenchTransferAck = null;
  unlistenWorkbenchTransferCancel?.();
  unlistenWorkbenchTransferCancel = null;
  unlistenViewWorkbenchOpen?.();
  unlistenViewWorkbenchOpen = null;
  unlistenWorkbenchInspectorOpen?.();
  unlistenWorkbenchInspectorOpen = null;
  for (const pending of outgoingWorkbenchTransfers.values()) {
    window.clearTimeout(pending.timer);
    pending.reject(new Error(t("workbench.window.targetUnavailable")));
  }
  outgoingWorkbenchTransfers.clear();
  unregisterWorkbenchInternalDropTarget?.();
  unregisterWorkbenchInternalDropTarget = null;
  ownerDocument.removeEventListener("pointerdown", handleInlineCreatePointerDown, true);
  ownerWindow.removeEventListener("drag", trackWorkspaceDragPointer, true);
  ownerWindow.removeEventListener("dragenter", trackWorkspaceDragPointer, true);
  ownerWindow.removeEventListener("dragover", trackWorkspaceDragPointer, true);
  ownerWindow.removeEventListener("drop", handleWindowWorkspaceDrop, true);
  ownerWindow.removeEventListener("dragend", clearWorkspaceDragPointer, true);
  for (const timer of workspaceTreeTabAttentionTimers) ownerWindow.clearTimeout(timer);
  workspaceTreeTabAttentionTimers.clear();
  window.clearTimeout(unityWorkspaceDragStateClearTimer);
  unityWorkspaceDragStateClearTimer = 0;
  onExplorerResizeEnd();
  releaseLocusFileDragState?.();
  releaseLocusFileDragState = null;
  releaseLocusFileDrop?.();
  releaseLocusFileDrop = null;
  releaseUnityAssetDrop?.();
  releaseUnityAssetDrop = null;
  releaseUnitySendToLocus?.();
  releaseUnitySendToLocus = null;
  clearLastFocusedComposerForWindow();
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

watch(
  () => uiStore.pendingExternalScriptOpen?.id ?? null,
  (requestId) => {
    if (requestId) void revealPendingExternalScriptOpen();
  },
  { immediate: true },
);
</script>

<template>
  <div
    ref="workbenchRootRef"
    class="development-workbench"
    :class="{
      'is-auxiliary': props.auxiliary,
      'is-external-tab-drop-target': !!workbenchWindowTabDrag.dropTarget.value,
    }"
    @keydown.capture="handleWorkbenchKeydown"
    @dragover.capture="trackWorkspaceDragPointer"
    @dragend.capture="clearWorkspaceDragPointer"
    @drop.capture="clearWorkspaceDragPointer"
  >
    <Teleport to="body">
      <div
        v-if="showWorkspaceDragFloatingPreview && workspaceDragPreview"
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
      v-if="props.showExplorer"
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
          v-if="!props.fixedWorkspaceRef && displaySettings.workspaceDisplayMode === 'single'"
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
        <template #editor="{ item }">
          <input
            v-if="isRenamingSessionItem(item as DevelopmentTreeItem)"
            ref="sessionRenameInput"
            v-model="sessionInlineRename!.value"
            class="development-session-rename-input"
            :aria-label="t('chat.session.rename')"
            autocomplete="off"
            @pointerdown.stop
            @click.stop
            @keydown.enter.prevent="submitSessionRename"
            @keydown.esc.prevent.stop="cancelSessionRename"
            @blur="submitSessionRename"
          />
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
          <button
            v-else-if="(item as DevelopmentTreeItem).meta.kind === 'knowledge'"
            type="button"
            class="development-knowledge-remove-button"
            :title="t('development.removeFromWorkspace')"
            :aria-label="t('development.removeFromWorkspace')"
            @pointerdown.stop
            @click.stop="removeKnowledgeItemFromWorkspace(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="X" :size="12" :stroke-width="2" />
          </button>
          <button
            v-else-if="isKnowledgeFolderPlacement(item as DevelopmentTreeItem)"
            type="button"
            class="development-knowledge-remove-button"
            :title="t('development.removeFromWorkspace')"
            :aria-label="t('development.removeFromWorkspace')"
            @pointerdown.stop
            @click.stop="removeKnowledgeFolderFromWorkspace(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="X" :size="12" :stroke-width="2" />
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
      v-if="props.showExplorer"
      class="development-explorer-resize"
      :class="{ active: resizingExplorer }"
      role="separator"
      aria-orientation="vertical"
      @mousedown="onExplorerResizeStart"
    />

    <main class="development-editor">
      <WorkbenchSplitHost
        :node="workbenchWindow.layout"
        :groups="workbenchWindow.groups"
        :focused-pane-id="workbenchWindow.focusedPaneId"
        :active-drop-key="activeEditorDropKey"
        :show-single-tabs="props.auxiliary || workbenchWindow.layout.kind === 'split'"
        @focus-pane="focusWorkbenchPane"
        @resize="resizeWorkbenchSplit"
      >
        <template #group="{ group, paneId, focused }">
          <template v-if="group">
            <WorkbenchEditorTabs
              :window-id="WORKBENCH_WINDOW_ID"
              :group="group"
              :show-single-tab="props.auxiliary || workbenchWindow.layout.kind === 'split'"
              :drop-active="renderedEditorDropIntent?.paneId === paneId && renderedEditorDropIntent.direction === 'center'"
              :drop-index="renderedEditorDropIntent?.paneId === paneId && renderedEditorDropIntent.direction === 'center' ? renderedEditorDropIntent.index : undefined"
              @activate="focusWorkbenchEditor(paneId, $event)"
              @close="closeWorkbenchEditor(paneId, $event)"
              @close-many="closeWorkbenchEditors(paneId, $event)"
              @pin="pinWorkbenchEditor(paneId, $event)"
              @drag-externalize="handleWorkbenchTabExternalize"
            />
            <div class="workbench-editor-stack">
              <div
                v-for="editor in group.tabs"
                :key="editor.editorId"
                v-show="group.activeEditorId === editor.editorId"
                class="workbench-editor-instance"
                :data-editor-id="editor.editorId"
              >
                <div
                  v-if="editor.availability === 'unavailable'"
                  class="workbench-unavailable-editor"
                >
                  <div class="workbench-unavailable-title">{{ editor.title }}</div>
                  <div class="workbench-unavailable-reason">
                    {{ editor.unavailableReason || t('workbench.unavailable.default') }}
                  </div>
                  <BaseButton size="sm" @click="closeWorkbenchEditor(paneId, editor.editorId)">
                    {{ t('common.close') }}
                  </BaseButton>
                </div>
                <WorkbenchSessionEditor
                  v-else-if="editor.resource.kind === 'session' || editor.resource.kind === 'newSession'"
                  :ref="(value) => setSessionEditorRef(editor.editorId, value)"
                  :editor="editor"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :reference-drop-available="composerAcceptsCurrentDrag(paneId, editor)"
                  :reference-drop-active="
                    composerDropTarget?.paneId === paneId
                      && composerDropTarget.editorId === editor.editorId
                  "
                  :shortcut-active="focused && group.activeEditorId === editor.editorId"
                  :new-chat-shortcut-action="newSessionShortcutAction(group, editor)"
                   @session-created="handleWorkbenchSessionCreated(paneId, $event)"
                   @session-forked="handleWorkbenchSessionForked(paneId, $event)"
                   @export-session-context="handleWorkbenchSessionExport(paneId, $event)"
                   @review-session-context="handleWorkbenchSessionReview(paneId, $event)"
                   @open-knowledge-document="handleWorkbenchKnowledgeDocument(paneId, $event)"
                   @new-session-requested="handleWorkbenchNewSessionRequested(paneId, $event)"
                  @composer-draft-change="handleWorkbenchComposerDraftChange(paneId, $event)"
                  @composer-focus="handleWorkbenchComposerFocus(paneId, $event)"
                />
                <KnowledgeView
                  v-else-if="editor.resource.kind === 'knowledge' || editor.resource.kind === 'knowledgeRoot' || (editor.resource.kind === 'section' && editor.resource.section === 'knowledge')"
                  :embedded="editor.resource.kind === 'knowledge'"
                  :active="group.activeEditorId === editor.editorId"
                  :selected-document-id="editorKnowledgeDocument(editor)?.id ?? null"
                  :selected-document-target="editorKnowledgeDocument(editor)"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :selected-model-id="modelStore.selectedModelId"
                  :model-defaults="modelStore.modelDefaults"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                />
                <CollabView
                  v-else-if="editor.resource.kind === 'collaboration' || editor.resource.kind === 'checkout' || (editor.resource.kind === 'section' && editor.resource.section === 'collab')"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :is-active="focused && group.activeEditorId === editor.editorId"
                  :selected-model-id="modelStore.selectedModelId"
                  :selected-agent-id="agentStore.selectedAgentId"
                  :models="modelStore.availableModels"
                  :head-focus-request="collabHeadFocusRequest"
                  @select-model="(id: string) => modelStore.selectModel(id)"
                />
                <AssetView
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'assets'"
                  :project-id="editor.resource.projectId"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <WorkspaceFilePreview
                  v-else-if="editor.resource.kind === 'workspaceFile' || (editor.resource.kind === 'asset' && isWorkbenchMarkdownPath(editor.resource.path))"
                  :ref="(value) => setWorkspaceFileEditorRef(editor.editorId, value)"
                  :project-id="editor.resource.projectId"
                  :path="editor.resource.path"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                />
                <WorkbenchAssetEditor
                  v-else-if="editor.resource.kind === 'asset' || editor.resource.kind === 'sceneObject'"
                  :ref="(value) => setWorkbenchAssetEditorRef(editor.editorId, value)"
                  :editor="editor"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <WorkbenchViewEditor
                  v-else-if="editor.resource.kind === 'view'"
                  :ref="(value) => setWorkbenchViewEditorRef(editor.editorId, value)"
                  :view-id="editor.resource.viewId"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                  :native-window="appWindow"
                  :owner-window="ownerWindow"
                />
                <ViewPackageView
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'views'"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                />
                <WorkbenchArchivedSessionsEditor
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'archived'"
                  :project-id="editor.resource.projectId"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <WorkspaceDirectoryPreview
                  v-else-if="editor.resource.kind === 'localDirectory' && editor.sourcePath"
                  :project-id="editor.resource.projectId"
                  :node-id="editor.resource.nodeId"
                  :relative-path="editor.resource.relativePath"
                  :path="editor.sourcePath"
                  :title="editor.title"
                  @activate="activateDirectoryPreviewEntry(paneId, editor, $event)"
                />
                <WorkspaceFilePreview
                  v-else-if="editor.resource.kind === 'localFile' && editor.sourcePath"
                  :ref="(value) => setWorkspaceFileEditorRef(editor.editorId, value)"
                  :project-id="editor.resource.projectId"
                  :path="editor.sourcePath"
                  :active="group.activeEditorId === editor.editorId"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                />
                <div v-else class="development-overview">
                  <template v-if="editor.resource.kind === 'folder'">
                    <div class="development-overview-title">{{ editor.title }}</div>
                    <div v-if="editor.sourcePath" class="development-overview-path">
                      {{ editor.sourcePath }}
                    </div>
                  </template>
                  <template v-else-if="editorProject(editor)">
                    <div class="development-overview-title">
                      {{ projectLabel(editorProject(editor)!) }}
                    </div>
                    <button
                      v-for="checkout in editorProject(editor)!.checkouts"
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
              </div>
              <div v-if="group.tabs.length === 0" class="workbench-editor-empty">
                {{ t('workbench.empty') }}
              </div>
            </div>
          </template>
        </template>
      </WorkbenchSplitHost>
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
        <template v-if="(contextMenu.sessionTargets?.length ?? 0) <= 1">
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
          <div class="base-context-menu-separator" />
        </template>
        <button type="button" @click="beginCreateFolder">
          <LucideIcon :icon="FolderPlus" :size="13" />
          {{ t("development.newFolder") }}
        </button>
        <div class="base-context-menu-separator" />
        <template v-if="(contextMenu.sessionTargets?.length ?? 0) <= 1">
          <button type="button" @click="exportContextSession">
            <LucideIcon :icon="Save" :size="13" />
            {{ t("chat.exportContext") }}
          </button>
          <button type="button" @click="reviewContextSession">
            <LucideIcon :icon="FileSearch" :size="13" />
            {{ t("chat.reviewContext") }}
          </button>
          <div class="base-context-menu-separator" />
        </template>
        <button type="button" @click="archiveContextSession">
          <LucideIcon :icon="Archive" :size="13" />
          <template v-if="(contextMenu.sessionTargets?.length ?? 0) <= 1">
            {{ t("chat.session.archive") }}
          </template>
          <template v-else>
            {{ t("chat.session.archiveMany", contextMenu.sessionTargets?.length ?? 0) }}
          </template>
        </button>
        <button type="button" class="danger" @click="beginDeleteSession">
          <LucideIcon :icon="Trash2" :size="13" />
          <template v-if="(contextMenu.sessionTargets?.length ?? 0) <= 1">
            {{ t("chat.session.delete") }}
          </template>
          <template v-else>
            {{ t("chat.session.deleteMany", contextMenu.sessionTargets?.length ?? 0) }}
          </template>
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
          v-if="contextMenu.item.meta.kind === 'knowledge'"
          type="button"
          @click="removeContextKnowledgeItemFromWorkspace"
        >
          <LucideIcon :icon="X" :size="13" />
          {{ t("development.removeFromWorkspace") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND && !contextMenu.item.meta.explorerNode.hidden"
          type="button"
          @click="setContextNodeHidden(true)"
        >
          <LucideIcon :icon="EyeOff" :size="13" />
          {{ t("development.hideNode") }}
        </button>
        <button
          v-if="contextMenu.item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND && contextMenu.item.meta.explorerNode.hidden"
          type="button"
          @click="setContextNodeHidden(false)"
        >
          <LucideIcon :icon="Eye" :size="13" />
          {{ t("development.showNode") }}
        </button>
        <button v-if="!props.fixedWorkspaceRef" type="button" @click="browseWorkspace">
          <LucideIcon :icon="Plus" :size="13" />
          {{ t("development.openWorkspace") }}
        </button>
        <div
          v-if="contextMenu.item.meta.kind === 'project' && !props.fixedWorkspaceRef"
          class="base-context-menu-separator"
        />
        <button
          v-if="contextMenu.item.meta.kind === 'project' && !props.fixedWorkspaceRef"
          type="button"
          class="danger"
          @click="removeContextWorkspace"
        >
          <LucideIcon :icon="Trash2" :size="13" />
          {{ t("development.deleteWorkspace") }}
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
      <template v-if="!props.fixedWorkspaceRef">
        <button type="button" @click="setWorkspaceMode('single')">
          <LucideIcon :icon="displaySettings.workspaceDisplayMode === 'single' ? Check : ChevronRight" :size="13" />
          {{ t("settings.display.workspaceModeSingle") }}
        </button>
        <button type="button" @click="setWorkspaceMode('multi')">
          <LucideIcon :icon="displaySettings.workspaceDisplayMode === 'multi' ? Check : ChevronRight" :size="13" />
          {{ t("settings.display.workspaceModeMulti") }}
        </button>
        <div class="base-context-menu-separator" />
      </template>
      <button
        type="button"
        class="development-submenu-trigger"
        aria-haspopup="menu"
        :aria-expanded="!!specialNodesMenu"
        :disabled="specialNodeVisibilityItems.length === 0"
        @click="openSpecialNodesMenu"
        @focus="openSpecialNodesMenu"
        @mouseenter="openSpecialNodesMenu"
      >
        <LucideIcon :icon="Eye" :size="13" />
        {{ t("development.specialNodeVisibility") }}
        <LucideIcon class="development-submenu-chevron" :icon="ChevronRight" :size="13" />
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
      v-if="displayMenu && specialNodesMenu"
      :x="specialNodesMenu.x"
      :y="specialNodesMenu.y"
      :min-width="168"
      :z-index="10001"
      :show-backdrop="false"
      :aria-label="t('development.specialNodeVisibility')"
      @close="specialNodesMenu = null"
    >
      <button
        v-for="item in specialNodeVisibilityItems"
        :key="item.resourceId"
        type="button"
        role="menuitemcheckbox"
        :aria-checked="!item.node.hidden"
        :disabled="specialNodeVisibilityBusy.has(item.node.nodeId)"
        @click="toggleSpecialNodeVisibility(item.node)"
      >
        <LucideIcon :icon="item.icon" :size="13" />
        <span class="development-special-node-label">{{ t(item.labelKey) }}</span>
        <LucideIcon
          class="development-menu-check"
          :class="{ visible: !item.node.hidden }"
          :icon="Check"
          :size="13"
        />
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
      <template v-for="path in projectStore.recentDirs" :key="path">
        <button
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
        <div
          v-if="extraWorkdirsFor(path).length > 0"
          class="development-workspace-extra-list"
        >
          <div
            v-for="extra in extraWorkdirsFor(path)"
            :key="extra.path"
            class="development-workspace-extra"
            :class="{ missing: !extra.exists }"
            :title="extraWorkdirTooltip(extra)"
          >
            <LucideIcon :icon="Folder" :size="11" />
            <span class="development-workspace-extra-name">{{ shortPath(extra.path) }}</span>
            <span v-if="extra.comment" class="development-workspace-extra-comment">
              {{ extra.comment }}
            </span>
            <span v-if="extra.readOnly" class="development-workspace-extra-state">
              {{ t("extraWorkdirs.readOnly") }}
            </span>
            <span v-if="!extra.exists" class="development-workspace-extra-state">
              {{ t("extraWorkdirs.missingBadge") }}
            </span>
          </div>
        </div>
      </template>
      <div v-if="projectStore.recentDirs.length === 0" class="development-recent-workspace-empty">
        {{ t("app.dir.noRecords") }}
      </div>
      <div class="base-context-menu-separator" />
      <button
        v-if="workspaceContextStore.focusedWorkspaceRef"
        type="button"
        @click="configureCurrentWorkspaceExtraWorkdirs"
      >
        <LucideIcon :icon="FolderCog" :size="13" />
        {{ t("app.dir.configureExtraWorkdirs") }}
      </button>
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
    <div
      v-if="sessionDeleteDialog"
      class="development-dialog-backdrop"
      @click.self="sessionDeleteDialog = null"
    >
      <form class="development-dialog" @submit.prevent="commitSessionDeleteDialog">
        <div class="development-dialog-title">
          {{ sessionDeleteDialog.targets.length > 1
            ? t('chat.session.deleteMany', sessionDeleteDialog.targets.length)
            : t('chat.session.delete') }}
        </div>
        <div class="development-dialog-message">
          {{ sessionDeleteDialog.targets.length > 1
            ? t('chat.session.deleteManyConfirm', sessionDeleteDialog.targets.length)
            : t('chat.session.deleteConfirm') }}
        </div>
        <div class="development-dialog-actions">
          <button type="button" @click="sessionDeleteDialog = null">{{ t("common.cancel") }}</button>
          <button type="submit" class="danger">{{ t("common.delete") }}</button>
        </div>
      </form>
    </div>
    <div
      v-if="dirtyEditorCloseDialog"
      class="development-dialog-backdrop"
      @click.self="cancelDirtyEditorClose"
    >
      <form class="development-dialog" @submit.prevent="saveAndCloseDirtyEditor">
        <div class="development-dialog-title">
          {{ t("development.editor.discardTitle") }}
        </div>
        <div class="development-dialog-message">
          {{ t("development.editor.discardMessage", dirtyEditorCloseDialog.title) }}
        </div>
        <div class="development-dialog-actions">
          <button type="button" @click="cancelDirtyEditorClose">
            {{ t("common.cancel") }}
          </button>
          <button type="button" class="danger" @click="discardAndCloseDirtyEditor">
            {{ t("development.editor.discard") }}
          </button>
          <button type="submit">{{ t("common.save") }}</button>
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

.development-workbench.is-auxiliary {
  border: 0;
}

.development-workbench.is-external-tab-drop-target .development-editor {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 34%, transparent);
}

.development-workbench :deep(.base-tab-shell.workspace-tree-attention-a),
.development-workbench :deep(.base-tab-shell.workspace-tree-attention-b) {
  z-index: 2;
  outline: 1px solid transparent;
  outline-offset: -1px;
}

.development-workbench :deep(.base-tab-shell.workspace-tree-attention-a) {
  animation: workspace-tree-tab-attention-a 420ms ease-out;
}

.development-workbench :deep(.base-tab-shell.workspace-tree-attention-b) {
  animation: workspace-tree-tab-attention-b 420ms ease-out;
}

@keyframes workspace-tree-tab-attention-a {
  0%, 100% {
    outline-color: transparent;
    box-shadow: inset 0 0 0 1px transparent;
  }
  22% {
    outline-color: var(--accent-color);
    box-shadow: inset 0 0 0 1px var(--accent-color);
  }
}

@keyframes workspace-tree-tab-attention-b {
  0%, 100% {
    outline-color: transparent;
    box-shadow: inset 0 0 0 1px transparent;
  }
  22% {
    outline-color: var(--accent-color);
    box-shadow: inset 0 0 0 1px var(--accent-color);
  }
}

@media (prefers-reduced-motion: reduce) {
  .development-workbench :deep(.base-tab-shell.workspace-tree-attention-a),
  .development-workbench :deep(.base-tab-shell.workspace-tree-attention-b) {
    animation: none;
    outline-color: var(--accent-color);
    box-shadow: inset 0 0 0 1px var(--accent-color);
  }
}

.development-submenu-trigger .development-submenu-chevron {
  margin-left: auto;
}

.development-special-node-label {
  flex: 1;
  margin-left: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.development-menu-check {
  margin-left: auto;
  visibility: hidden;
}

.development-menu-check.visible {
  visibility: visible;
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
  cursor: default;
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

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone),
.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone:hover),
.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone.drop-target) {
  background: transparent;
  box-shadow: none;
}

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone .workspace-tree-row) {
  min-height: 26px;
  margin: 2px 6px;
  padding: 1px 8px;
  justify-content: center;
  border: 1px dashed var(--border-strong);
  border-radius: 6px;
  color: var(--text-secondary);
}

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone .workspace-tree-icon) {
  display: none;
}

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone .workspace-tree-name) {
  flex: 0 1 auto;
  text-align: center;
}

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone.drop-target .workspace-tree-row) {
  border-color: color-mix(in srgb, var(--accent-color) 72%, var(--border-strong));
  background: color-mix(in srgb, var(--accent-soft) 42%, transparent);
  color: var(--text-color);
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

.development-tree :deep(.workspace-tree-row-shell.editing .workspace-tree-trailing) {
  display: none;
}

:where(.development-session-archive-button, .development-knowledge-remove-button) {
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

.development-session-archive-button {
  right: 14px;
}

.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-archive-button),
.development-session-archive-button:focus-visible,
.development-tree :deep(.workspace-tree-row-shell.is-knowledge-row:hover .development-knowledge-remove-button),
.development-knowledge-remove-button:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-dot),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-spinner),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-session-status),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:hover .development-branch-label),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:has(.development-session-archive-button:focus-visible) .development-session-dot),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:has(.development-session-archive-button:focus-visible) .development-session-spinner),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:has(.development-session-archive-button:focus-visible) .development-session-status),
.development-tree :deep(.workspace-tree-row-shell.is-session-row:has(.development-session-archive-button:focus-visible) .development-branch-label) {
  opacity: 0;
}

:where(.development-session-archive-button, .development-knowledge-remove-button):hover,
:where(.development-session-archive-button, .development-knowledge-remove-button):focus-visible {
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

.development-session-rename-input {
  width: 100%;
  height: 22px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-ui);
  font-size: 12px;
}

.development-session-rename-input:focus {
  border-color: var(--accent-color);
  outline: none;
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
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

.development-workspace-extra-list {
  margin: -1px 8px 2px 31px;
  padding-left: 9px;
  display: flex;
  min-width: 0;
  flex-direction: column;
  border-left: 1px solid var(--border-color);
}

.development-workspace-extra {
  min-height: 22px;
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: 11px;
}

.development-workspace-extra > svg {
  flex: 0 0 auto;
}

.development-workspace-extra-name {
  flex: 0 1 auto;
  max-width: 150px;
  overflow: hidden;
  color: var(--text-color);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-workspace-extra-comment {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-workspace-extra-state {
  flex: 0 0 auto;
  color: var(--text-secondary);
  font-size: 10px;
}

.development-workspace-extra.missing,
.development-workspace-extra.missing .development-workspace-extra-name,
.development-workspace-extra.missing .development-workspace-extra-state {
  color: var(--status-danger-fg);
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
  overflow: hidden;
  background: var(--panel-bg);
}

.development-editor :deep(> .workbench-split),
.development-editor :deep(> .workbench-editor-group) {
  flex: 1 1 0;
}

.workbench-editor-stack,
.workbench-editor-instance {
  display: flex;
  flex: 1 1 0;
  width: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.workbench-editor-instance > :deep(*) {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
}

.workbench-editor-empty,
.workbench-unavailable-editor {
  display: flex;
  flex: 1 1 0;
  align-items: center;
  justify-content: center;
  min-width: 0;
  min-height: 0;
  color: var(--text-secondary);
  font-size: 12px;
}

.workbench-unavailable-editor {
  flex-direction: column;
  gap: 8px;
}

.workbench-unavailable-title {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.workbench-unavailable-reason {
  max-width: 420px;
  text-align: center;
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
