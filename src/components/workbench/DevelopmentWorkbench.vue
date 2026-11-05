<script setup lang="ts">
import { requestWorkspaceSetup } from "../../composables/useWorkspaceSetup";
import { openWorkspaceWindow, WORKSPACE_WINDOW_TITLE_BAR_SETTING } from "../../services/workspaceWindow";
import { openWorkspacePageWindow } from "../../services/workspacePageWindow";
import { registerWorkStateProvider, savedWindowState, updateWindowWorkState, scheduleWorkStateCapture } from "../../services/workState";
import { unarchiveSession } from "../../services/session";
import { registerFrontendWorkbench } from "../../services/frontendWorkbench";
import { isSessionUnread, sessionAttention } from "../../services/sessionAttention";
import WorktreeManager from "./WorktreeManager.vue";
import WorkbenchEditorStack from "./WorkbenchEditorStack.vue";
import { isWorkbenchSessionEditorInput } from "./workbenchEditorMounting";
import { resolveWorkbenchSession } from "./workbenchSessionAvailability";
import { workbenchViewIconValue } from "./workbenchViewIcon";
import { sessionTabWorkspaceLabels } from "./workbenchTabWorkspace";
import type { ManagedWorktree } from "../../services/worktrees";
import { beginWorkspaceGitHeadObservation, differingSessionBranchLabel, rememberWorkspaceGitHead, workspaceGitHead } from "../../services/workspaceGitHead";
import { computed, nextTick, onMounted, onUnmounted, provide, reactive, ref, shallowReactive, toRaw, watch } from "vue";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, type Window as TauriWindowHandle } from "@tauri-apps/api/window";
import {
  AppWindow,
  Bot,
  Archive,
  Box,
  Check,
  BookOpen,
  ChevronDown,
  ChevronRight,
  Copy,
  Eye,
  EyeOff,
  File,
  FilePlus,
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
  PanelTop,
  Star,
  Pin,
  PinOff,
  PencilLine,
  Plus,
  Puzzle,
  Save,
  Settings,
  Table2,
  Trash2,
  X,
} from "lucide";
import { t } from "../../i18n";
import { formatWorkspaceUpdatedTime } from "../../composables/useFormatters";
import { preloadEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
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
  showInFolder,
  type LocusFileDragStatePayload,
  type LocusFileDropPayload,
  type LocusFileDropRef,
  type UnityEmbedAssetDragStatePayload,
  type UnityEmbedAssetDropPayload,
  type UnitySendToLocusEventPayload,
} from "../../services/unity";
import { knowledgeRead, knowledgeRevealTarget } from "../../services/knowledge";
import GlobalSearch from "./GlobalSearch.vue";
import type { GlobalSearchResult, GlobalSearchTarget } from "../../services/globalSearch";
import {
  openChatSessionWindow,
  openNewChatSessionWindow,
} from "../../services/chatSessionWindow";
import { exportSessionContext } from "../../services/session";
import { sessionContextExportFileName } from "../../composables/sessionContextExport";
import {
  useDisplaySettings,
  type KnowledgeFolderKind,
  type WorkspaceDisplayMode,
} from "../../composables/useDisplaySettings";
import { useSkills } from "../../composables/useSkills";
import { matchesShortcut, useKeyboardShortcuts } from "../../composables/useKeyboardShortcuts";
import { useWorkbenchPaneLifecycle } from "../../composables/useWorkbenchPaneLifecycle";
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
import { viewRead, viewSetTabHost } from "../../services/view";
import { VIEW_OPEN_WINDOW_KEY } from "../../composables/useOpenView";
import { openViewPackage } from "../../services/viewOpening";
import { resolveLocusViewIcon } from "../icons/locusViewIcons";
import { VIEW_TREE_INTERNAL_DRAG_TYPE, type ViewWorkspaceDragPayload, type WorkspaceViewReference } from "../view/viewWorkspaceDrag";
import {
  WORKBENCH_FILE_OPEN_EVENT,
  WORKBENCH_FILE_OPEN_KEY,
  type WorkbenchFileOpenRequest,
} from "../../services/workbenchFile";
import {
  VIEW_WORKBENCH_OPEN_EVENT,
  type ViewWorkbenchOpenPayload,
} from "../../services/viewWorkbench";
import {
  type ProjectContextDescriptor,
  type WorkspaceCheckoutDescriptor,
  type WorkspaceRef,
} from "../../services/project";
import {
  buildScopedMcpServerArtifacts,
  mcpServerGetState,
} from "../../services/mcpServer";
import type { ExtraWorkdirStatus } from "../../services/extraWorkdirs";
import { broadcastExtraWorkdirsUpdated } from "../../services/extraWorkdirsWindow";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useProjectStore } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { isManagedWorktreeCheckout, projectEntryCheckout, workspaceMaterializationMatches } from "../../services/project";
import { workspaceRefForEditorBinding, editorBindingMatchesRuntime, sessionEditorBinding } from "./editorWorkspaceScope";
import { lastSessionWorktree, rememberSessionWorktree } from "./sessionWorktreePreference";
import { resolveWorkbenchFileTarget } from "./workbenchFileTarget";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import { useWorkspaceNavigationMode, type WorkspaceNavigationMode } from "../../composables/useWorkspaceNavigationMode";
import { createNativeWorkspaceFileDrop, useWorkspaceFileDrop } from "../../composables/useWorkspaceFileDrop";
import { workspaceFileParent, workspaceMovedPath, workspaceRelativeEntryPath, type WorkspaceFileDropScope } from "../explorer/workspaceFileMove";
import { workspaceCreateDocument, workspaceCreateFolder } from "../../services/workspaceExplorer";
import { useWorkspaceFileOrderDrop, type WorkspaceOrderDragItem } from "../../composables/useWorkspaceFileOrderDrop";
import { orderWorkspaceFileRows, workspaceFileOrderKey, workspaceNodeOrderKey, workspaceOrderDirectory } from "../explorer/workspaceFileOrder";
import WorkspaceFileTreeSource from "../explorer/WorkspaceFileTreeSource.vue";
import WorkspaceArchivedFiles from "../explorer/WorkspaceArchivedFiles.vue";
import { useWorkspaceFileVisibility } from "../../composables/useWorkspaceFileVisibility";
import { WORKSPACE_FILE_ROW_HEIGHT, type WorkspaceFileEntry, type WorkspaceFileTreeState } from "../explorer/workspaceFileTree";
import { assetWorkspaceReferenceDragData } from "../asset/assetWorkspaceDrag";
import { workbenchReferenceInternalDragSource } from "./workbenchReferenceDrag";
import { useWorkspaceFileModifiedTimes, type WorkspaceFileTimeTarget } from "../../composables/useWorkspaceFileModifiedTimes";
import {
  createWorkbenchEditorInput,
  isWorkbenchDocumentPath,
  shouldShowWorkbenchTabStrip,
  workbenchPaneRightOf,
  workbenchResourceKey,
  useWorkbenchStore,
} from "../../stores/workbench";
import type {
  DevelopmentResourceRef,
  EditorCheckoutBinding,
  KnowledgeWorkbenchPage,
  ProjectExplorerMountEntry,
  ProjectExplorerNode,
  ProjectExplorerOperation,
  ProjectExplorerSnapshot,
  ProjectExplorerItemRef,
  ProjectKnowledgeDocument,
  WorkbenchDropDirection,
  WorkbenchEditorGroup,
  WorkbenchEditorInput,
  WorkbenchEditorTransferRecord,
  WorkbenchEditorTransferSnapshot,
  WorkbenchWindowDropIntent,
} from "../../types/workbench";
import type { AssetRefAttachment, KnowledgeDocumentSummary, KnowledgeDocumentType, SessionSummary } from "../../types";
import type { UserMessageDraft, UserMessageDraftKnowledgeQuote } from "../../composables/chatMessageDraft";
import {
  KNOWLEDGE_QUOTE_SELECTION_KEY,
  type KnowledgeQuoteSelection,
} from "../../services/knowledgeSelection";
import {
  createDocumentSelectionHub,
  DOCUMENT_QUOTE_IN_NEW_SESSION_KEY,
  DOCUMENT_SELECTION_HUB_KEY,
} from "../../composables/documentSelection";
import { clearSharedComposerDraft } from "../../composables/chatComposerDraftMemory";
import { emptyComposerIntent } from "../../composables/chatInputIntents";
import AssetView from "../AssetView.vue";
import AgentView from "../AgentView.vue";
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
import WorkbenchSecondarySidebar from "./WorkbenchSecondarySidebar.vue";
import { cancelWorkbenchSidebarMotion, enterWorkbenchSidebar, leaveWorkbenchSidebar } from "./workbenchSidebarMotion";
import {
  isWorkspaceSecondarySection,
  workspaceOverlaySection,
  workspaceSecondarySection,
  workspaceSecondaryResourceSection,
  type WorkspaceOverlaySection,
  type WorkspaceSecondarySection,
} from "./workspaceSecondaryNavigation";
import WorkbenchCollabPage from "./WorkbenchCollabPage.vue";
import {
  WORKSPACE_TOP_BAR_NODES_SETTING,
  isTopBarNodeId,
  isTopBarPageId,
  placeTopBarNode,
  removeTopBarNode,
  sameTopBarLayout,
  topBarInsertionBefore,
  topBarNodeIds,
  type TopBarEntryId,
  type TopBarNodeId,
} from "./workspaceTopBarNodes";
import { saveSetting, settingValue } from "../../composables/useSettingsStore";
import { findFileHistorySidebarOwner } from "./fileHistorySidebarOwner";
import type { FileHistoryTarget } from "../../services/fileHistory";
import FileHistoryPanel from "../explorer/FileHistoryPanel.vue";
import {
  WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
  type WorkbenchEditorTabInternalDragData,
} from "./workbenchDrag";
import {
  workbenchSplitDirectionAtPoint,
  workbenchTabInsertionIndexAtPoint,
} from "./workbenchDropGeometry";
import {
  workbenchComposerEditorAttachment,
  workbenchComposerFileAttachment,
  workbenchComposerTreeFileAttachment,
} from "./workbenchComposerDrop";
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
import ExplorerResourceActions from "../explorer/ExplorerResourceActions.vue";
import ResourceFileMenuItems from "../explorer/ResourceFileMenuItems.vue";
import ViewResourceMenuItems from "../view/ViewResourceMenuItems.vue";
import ViewResourceActions from "../view/ViewResourceActions.vue";
import type { ViewResourceAction, ViewResourceTarget } from "../view/viewResourceActions";
import { resourcePathKey, resourceRelativePath, validResourceName, type ExplorerResourceAction, type ExplorerResourceTarget } from "../explorer/explorerResourceActions";
import { explorerFileKey, explorerFilePath, useExplorerPathDisplay } from "../../composables/useExplorerPathDisplay";
import { knowledgeResourcePath } from "../knowledge/knowledgeResourceActions";
import BaseButton from "../ui/BaseButton.vue";
import WorkspaceTree, {
  type WorkspaceTreeItem,
  type WorkspaceTreeRow,
} from "../explorer/WorkspaceTree.vue";
import { mountedEntryInPinnedSubtree, pinnedInsertionBefore, sameWorkspaceTreeItem, workspaceTreeItemState } from "../explorer/workspaceTreeItemState";
import { explorerChildNodes } from "../explorer/explorerSnapshotIndex";
import { previewWorkspaceTreeMove, previewWorkspaceTreePin, workspaceTreeMoveOperation, workspaceTreeMoveOperations, workspaceTreePinOperations } from "../explorer/workspaceTreeDropPreview";
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
  | "sessionsRoot"
  | "workspaceFile"
  | "agentsRoot"
  | "viewsRoot"
  | "archivedRoot"
  | "archivedFilesRoot"
  | "checkout"
  | "folder"
  | "empty"
  | "session"
  | "knowledge"
  | "view"
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
  | ViewWorkspaceDragPayload
  | WorkbenchEditorTabInternalDragData;

type WorkbenchInternalDropIntent =
  | { kind: "pin"; projectId: string; insertion?: PinnedDropInsertion }
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
  toolbarTarget?: HTMLElement | null;
  /** App top tab bar; while set, single-workspace system nodes can move into it. */
  topBarTarget?: HTMLElement | null;
  /** A workspace window whose page tab bar the menu can turn on or off. */
  titleBarOptional?: boolean;
  /** A window opened for one workspace keeps showing only it in multi-workspace mode. */
  singleWorkspaceOnly?: boolean;
  titlebarTarget?: HTMLElement | null;
  singleTabTitleTarget?: HTMLElement | null;
  initialTransferToken?: string;
  nativeWindow?: Pick<TauriWindowHandle, "label" | "listen" | "setFocus">;
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
const treeTimeNow = ref(Date.now() / 1000);
let treeTimeRefreshTimer: number | undefined;
const treeVisibleRange = ref({ start: 0, end: 40 });
const workspaceTreeRef = ref<InstanceType<typeof WorkspaceTree> | null>(null);
const treePointerPressed = ref(false);
// Resting on or pressing a session row preloads that session, so its first
// frame is ready by the time the click opens it.
const SESSION_ROW_PRELOAD_HOVER_MS = 60;
let sessionRowPreloadTimer: ReturnType<typeof setTimeout> | null = null;
let sessionRowPreloadTarget = "";

function preloadableSessionRowId(target: EventTarget | null): string {
  const row = target instanceof Element
    ? target.closest<HTMLElement>(".workspace-tree-row-shell.is-session-row")
    : null;
  // A session with a live runtime streams into its editor; it is loaded at open time.
  if (!row || row.classList.contains("has-session-runtime")) return "";
  const key = row.dataset.treeKey ?? "";
  return key.startsWith("session:") ? key.slice(key.lastIndexOf(":") + 1) : "";
}

function cancelSessionRowPreload(): void {
  if (sessionRowPreloadTimer) clearTimeout(sessionRowPreloadTimer);
  sessionRowPreloadTimer = null;
  sessionRowPreloadTarget = "";
}

function handleSessionRowPointerOver(event: PointerEvent): void {
  const sessionId = preloadableSessionRowId(event.target);
  if (sessionId === sessionRowPreloadTarget) return;
  cancelSessionRowPreload();
  if (!sessionId) return;
  sessionRowPreloadTarget = sessionId;
  sessionRowPreloadTimer = setTimeout(() => {
    sessionRowPreloadTimer = null;
    preloadEmbeddedChatSession(sessionId, workspaceContextStore.focusedWorkspaceRef);
  }, SESSION_ROW_PRELOAD_HOVER_MS);
}

function handleTreePointerDown(event: PointerEvent): void {
  treePointerPressed.value = true;
  if (event.button !== 0) return;
  const sessionId = preloadableSessionRowId(event.target);
  if (sessionId) preloadEmbeddedChatSession(sessionId, workspaceContextStore.focusedWorkspaceRef);
}
const lastRevealedUnreadSessionId = ref<string | null>(null);

interface DevelopmentTreeItem extends WorkspaceTreeItem {
  meta: {
    kind: ItemKind;
    projectId: string;
    checkoutId?: string;
    explorerNode?: ProjectExplorerNode;
    session?: SessionSummary;
    archived?: boolean;
    runtimeStatus?: SessionTreeStatus | null;
    knowledge?: ProjectKnowledgeDocument;
    view?: WorkspaceViewReference;
    mountEntry?: ProjectExplorerMountEntry;
    workspaceFile?: WorkspaceFileEntry;
    archivedFile?: boolean;
    pinnedRoot?: ProjectExplorerItemRef;
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
  /** Set only when a file-mode menu applies to more than one selected row. */
  fileTargets?: DevelopmentTreeItem[];
}

interface FolderDialogState {
  mode: "rename" | "delete";
  projectId: string;
  nodeId?: string;
  value: string;
}

interface WorkspaceInlineCreateState {
  kind: "folder" | "document" | "csv";
  projectId: string;
  parentNodeId: string | null;
  name: string;
  fileTarget?: WorkspaceFileDropScope;
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

interface PinnedDropInsertion {
  before: ProjectExplorerItemRef | null;
  lineKey: string;
  side: "before" | "after";
}

interface SettlingLayoutDrop {
  id: number;
  snapshot: ProjectExplorerSnapshot;
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
/** This window's tree projection: the setting, except in windows fixed to one workspace. */
const workspaceDisplayMode = computed<WorkspaceDisplayMode>(() => (
  props.singleWorkspaceOnly ? "single" : displaySettings.workspaceDisplayMode
));

const WORKBENCH_WINDOW_ID = props.windowId;
provide(VIEW_OPEN_WINDOW_KEY, WORKBENCH_WINDOW_ID);
let initialWorkspaceFallbackActive = !props.auxiliary || !!props.fixedWorkspaceRef;

function initialWorkspaceCheckout(): WorkspaceCheckoutDescriptor | null {
  if (!initialWorkspaceFallbackActive) return null;
  if (!props.fixedWorkspaceRef) {
    return workspaceContextBaseStore.windowId === WORKBENCH_WINDOW_ID
      ? workspaceContextBaseStore.focusedCheckout : null;
  }
  const checkout = workspaceContextBaseStore.checkoutsById[props.fixedWorkspaceRef.checkoutId];
  if (!checkout?.runtime || checkout.available === false) return null;
  if (!workspaceMaterializationMatches(props.fixedWorkspaceRef.expectedMaterializationEpoch, checkout.runtime.materializationEpoch)) return null;
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
      ? target.checkoutForPane(WORKBENCH_WINDOW_ID, paneId)
      : initialWorkspaceCheckout();
    switch (property) {
      case "focusedPaneContext":
        return paneContext;
      case "focusedCheckout":
        return checkout;
      case "focusedRuntime":
        return workspaceMaterializationMatches(paneContext?.materializationEpoch ?? props.fixedWorkspaceRef?.expectedMaterializationEpoch,checkout?.runtime?.materializationEpoch) ? checkout?.runtime ?? null : null;
      case "focusedWorkspaceRef":
        return checkout?.runtime ? {
          checkoutId: checkout.checkoutId,
          expectedGeneration: checkout.runtime.workspaceGeneration,
          expectedMaterializationEpoch: paneContext?.materializationEpoch ?? props.fixedWorkspaceRef?.expectedMaterializationEpoch,
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
          // Explicitly opening a directory must revalidate it on disk, even
          // when its path is already in the cached checkout catalog.
          const normalizedPath = normalizeExternalProjectPath(path);
          const existingCheckout = Object.values(target.checkoutsById).find(
            (checkout) => normalizeExternalProjectPath(checkout.root) === normalizedPath,
          );
          if (existingCheckout && usesCheckoutScopedWorkbench()) {
            await target.openCheckout(path);
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
  ?? (!props.auxiliary && workspaceDisplayMode.value === "single"
    ? workspaceContextStore.focusedCheckout?.checkoutId ?? savedWindowState(WORKBENCH_WINDOW_ID)?.activeWorkspaceId ?? null
    : null);
const singleWorkspaceScopeId = ref<string | null>(initialWorkbenchWorkspaceScopeId);
workbenchStore.switchWorkspaceScope(WORKBENCH_WINDOW_ID, initialWorkbenchWorkspaceScopeId);
const workbenchWindow = computed(() => workbenchStore.ensureWindow(WORKBENCH_WINDOW_ID));
const workbenchWorkspaceScopeId = computed(() => (
  props.fixedWorkspaceRef?.checkoutId
    ?? (!props.auxiliary && workspaceDisplayMode.value === "single"
      ? singleWorkspaceScopeId.value
        ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID)
        ?? workspaceContextStore.focusedCheckout?.checkoutId
        ?? null
      : null)
));

const restoredNavigation = savedWindowState(WORKBENCH_WINDOW_ID)?.navigation?.[initialWorkbenchWorkspaceScopeId ?? ""];
const expanded = ref<Set<string>>(new Set(restoredNavigation?.expanded ?? []));
const workspaceNavigation = useWorkspaceNavigationMode();
const workspaceFileStates = shallowReactive<Record<string, WorkspaceFileTreeState>>({});
const workspaceFileVisibility = useWorkspaceFileVisibility();
const sessionsTreeRef = ref<InstanceType<typeof WorkspaceTree> | null>(null);
const secondaryAssetViewRef = ref<InstanceType<typeof AssetView> | null>(null);
const secondaryNavigation = ref<{
  section: WorkspaceSecondarySection;
  projectId: string;
  checkoutId: string;
  editorId?: string;
} | null>(restoredNavigation?.secondary && isWorkspaceSecondarySection(restoredNavigation.secondary.section)
  ? restoredNavigation.secondary
  : null);

interface WorkspaceOverlayTarget {
  section: WorkspaceOverlaySection;
  projectId: string;
  checkoutId: string;
}
// Collaboration and Agent pages cover the secondary list and editor tabs
// without changing them, so closing the overlay restores the previous layout.
const workspaceOverlay = ref<WorkspaceOverlayTarget | null>(null);
// The last page of each section stays mounted while hidden to keep its state.
const workspaceOverlayPages = ref<Partial<Record<WorkspaceOverlaySection, WorkspaceOverlayTarget>>>({});
const collabOverlayToolbarTarget = ref<HTMLElement | null>(null);
let workspaceOverlayRequestId = 0;

function workspaceOverlayPage(section: WorkspaceOverlaySection) {
  const target = workspaceOverlayPages.value[section];
  const checkout = target ? workspaceContextBaseStore.checkoutsById[target.checkoutId] : null;
  if (!target || checkout?.available === false || !checkout?.runtime) return null;
  const workspaceRef: WorkspaceRef = {
    checkoutId: checkout.checkoutId,
    expectedGeneration: checkout.runtime.workspaceGeneration,
    expectedMaterializationEpoch: checkout.runtime.materializationEpoch,
  };
  return { target, root: checkout.root, workspaceRef };
}
const collabOverlayPage = computed(() => workspaceOverlayPage("collab"));
const agentOverlayPage = computed(() => workspaceOverlayPage("agents"));
const workspaceOverlayOpen = computed(() => {
  const overlay = workspaceOverlay.value;
  if (!overlay || !props.showExplorer) return false;
  const page = overlay.section === "collab" ? collabOverlayPage.value : agentOverlayPage.value;
  return page?.target.checkoutId === overlay.checkoutId;
});
const collabOverlayActive = computed(() => workspaceOverlayOpen.value && workspaceOverlay.value?.section === "collab");
const agentOverlayActive = computed(() => workspaceOverlayOpen.value && workspaceOverlay.value?.section === "agents");
const workspaceOverlayTitle = computed(() => (
  workspaceOverlay.value?.section === "agents" ? t("app.tab.agent") : t("app.tab.collab")
));
const workspaceOverlaySubtitle = computed(() => {
  const overlay = workspaceOverlay.value;
  if (!overlay) return null;
  const project = workspaceContextStore.projectsById[overlay.projectId];
  const root = workspaceContextBaseStore.checkoutsById[overlay.checkoutId]?.root;
  if (!project || !root) return null;
  // Name the checkout only when the explorer header does not already identify it.
  return project.checkouts.length > 1 || workspaceDisplayMode.value === "multi"
    ? { label: shortPath(root), title: root }
    : null;
});

function workspaceOverlayIsOpen(projectId: string, section: WorkspaceOverlaySection): boolean {
  const overlay = workspaceOverlay.value;
  return workspaceOverlayOpen.value && overlay?.projectId === projectId && overlay.section === section;
}

function closeWorkspaceOverlay(): void {
  workspaceOverlayRequestId += 1;
  workspaceOverlay.value = null;
}

async function showWorkspaceOverlay(
  section: WorkspaceOverlaySection,
  projectId: string,
  checkout: WorkspaceCheckoutDescriptor,
): Promise<void> {
  const requestId = ++workspaceOverlayRequestId;
  const target: WorkspaceOverlayTarget = { section, projectId, checkoutId: checkout.checkoutId };
  // Record the intent first so a second click can cancel a cold checkout open.
  workspaceOverlay.value = target;
  try {
    if (!workspaceContextBaseStore.checkoutsById[checkout.checkoutId]?.runtime) {
      await workspaceContextBaseStore.openCheckout(checkout.root);
    }
  } catch (error) {
    if (requestId === workspaceOverlayRequestId) closeWorkspaceOverlay();
    throw error;
  }
  if (requestId !== workspaceOverlayRequestId) return;
  workspaceOverlayPages.value = { ...workspaceOverlayPages.value, [section]: target };
}

const fileHistorySidebarTarget = ref<HTMLElement | null>(null);
const fileHistorySidebarToolbarTarget = ref<HTMLElement | null>(null);
const fileHistorySidebarOwner = computed(() => {
  const target = secondaryNavigation.value;
  if (target?.section !== "fileHistory") return null;
  const owner = findFileHistorySidebarOwner(workbenchWindow.value.groups, target);
  const runtime = workspaceContextBaseStore.checkoutsById[target.checkoutId]?.runtime;
  return owner && editorBindingMatchesRuntime(owner.editor.checkoutBinding, runtime) ? owner : null;
});

function showFileHistorySecondarySidebar(editor: WorkbenchEditorInput): void {
  if (editor.resource.kind !== "fileHistory" || !editor.checkoutBinding || editor.availability === "unavailable") return;
  secondaryNavigationRequestId += 1;
  secondaryNavigation.value = {
    section: "fileHistory", projectId: editor.resource.projectId,
    checkoutId: editor.checkoutBinding.checkoutId, editorId: editor.editorId,
  };
}

watch(fileHistorySidebarOwner, owner => {
  if (secondaryNavigation.value?.section === "fileHistory" && !owner) closeSecondaryNavigation();
});

const fileHistoryEditorTargets = new WeakMap<object, FileHistoryTarget>();
/** Same object while unchanged: a new target per stack render re-renders the panel. */
function fileHistoryEditorTarget(editor: WorkbenchEditorInput): FileHistoryTarget {
  if (editor.resource.kind !== "fileHistory") throw new Error("Expected a file history editor");
  const projectId = editor.resource.projectId;
  const path = editor.resource.path;
  const workspaceRef = editor.resource.workspaceScoped ? editorWorkspaceRef(editor) : null;
  const cached = fileHistoryEditorTargets.get(toRaw(editor));
  if (cached?.projectId === projectId && cached.path === path && cached.workspaceRef === workspaceRef) return cached;
  const target: FileHistoryTarget = { projectId, path, workspaceRef };
  fileHistoryEditorTargets.set(toRaw(editor), target);
  return target;
}

// Editor stack slot props are rebuilt on every stack render, i.e. on every tab
// switch. A function, object or array created per render changes the vnode
// props and updates every mounted editor, so per-editor callbacks are created
// once per editor object (emit listeners are exempt: Vue skips declared emits).
const EMPTY_SESSIONS: SessionSummary[] = [];
const editorSlotCallbacks = new WeakMap<object, Map<string, unknown>>();

function editorSlotCallback<T>(editor: WorkbenchEditorInput, key: string, create: () => T): T {
  const owner = toRaw(editor);
  let callbacks = editorSlotCallbacks.get(owner);
  if (!callbacks) {
    callbacks = new Map();
    editorSlotCallbacks.set(owner, callbacks);
  }
  if (!callbacks.has(key)) callbacks.set(key, create());
  return callbacks.get(key) as T;
}

type EditorRefKind = "session" | "agent" | "knowledge" | "workspaceFile" | "asset" | "view";

/**
 * Template ref callbacks are part of the vnode props. The stack's keyed slot
 * children are patched unoptimized, comparing every prop, so a new inline ref
 * function per render updated every mounted editor on every tab switch.
 */
function editorRefCallback(editor: WorkbenchEditorInput, kind: EditorRefKind) {
  return editorSlotCallback(editor, `ref:${kind}`, () => (value: unknown) => {
    const editorId = editor.editorId;
    if (kind === "session") setSessionEditorRef(editorId, value);
    else if (kind === "agent") setAgentEditorRef(editorId, value);
    else if (kind === "knowledge") {
      if (editor.resource.kind === "knowledge") setWorkspaceFileEditorRef(editorId, value);
    } else if (kind === "workspaceFile") setWorkspaceFileEditorRef(editorId, value);
    else if (kind === "asset") setWorkbenchAssetEditorRef(editorId, value);
    else setWorkbenchViewEditorRef(editorId, value);
  });
}

function sessionWorktreeSelector(paneId: string, editor: WorkbenchEditorInput) {
  return editorSlotCallback(editor, `select-worktree:${paneId}`, () => (
    (item: ManagedWorktree) => selectSessionWorktree(paneId, editor.editorId, item)
  ));
}

function editorActivator(paneId: string, editor: WorkbenchEditorInput) {
  return editorSlotCallback(editor, `activate:${paneId}`, () => (
    () => focusWorkbenchEditor(paneId, editor.editorId)
  ));
}

/**
 * Only shown (or incoming) session editors render their session picker. Hidden
 * ones get a stable empty list, so replacing `chatStore.sessions` (any session
 * title or status change) does not re-render every mounted session tab.
 */
function sessionEditorSessions(editor: WorkbenchEditorInput, contentActive: boolean): SessionSummary[] {
  if (!contentActive) return EMPTY_SESSIONS;
  const projectId = editor.resource.projectId;
  return sessionsForProject(projectId, explorerStore.resources[projectId]?.sessions ?? EMPTY_SESSIONS);
}

async function openFileHistory(target: FileHistoryTarget): Promise<void> {
  try {
    const editor = await openWorkbenchResource({
      resource: { kind: "fileHistory", projectId: target.projectId, path: target.path, workspaceScoped: !!target.workspaceRef },
      title: `${shortPath(target.path)} · ${t("fileHistory.title")}`,
      checkoutId: target.workspaceRef?.checkoutId,
    }, { preview: false, pinned: true, replacePreview: false });
    showFileHistorySecondarySidebar(editor);
  } catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
}
let secondaryNavigationRequestId = 0;
const secondaryCheckout = computed(() => {
  const target = secondaryNavigation.value;
  const checkout = target ? workspaceContextBaseStore.checkoutsById[target.checkoutId] : null;
  return checkout?.available !== false && checkout?.runtime ? checkout : null;
});
const secondaryWorkspaceRef = computed<WorkspaceRef | null>(() => {
  const checkout = secondaryCheckout.value;
  return checkout?.runtime ? {
    checkoutId: checkout.checkoutId,
    expectedGeneration: checkout.runtime.workspaceGeneration,
    expectedMaterializationEpoch: checkout.runtime.materializationEpoch,
  } : null;
});
const secondaryTitle = computed(() => {
  switch (secondaryNavigation.value?.section) {
    case "fileHistory": return t("fileHistory.title");
    case "knowledge": return t("app.tab.knowledge");
    case "assets": return t("app.tab.asset");
    case "sessions": return t("development.sessions");
    case "views": return t("app.tab.views");
    case "archived": return t("app.tab.archived");
    case "archivedFiles": return t("development.archivedFiles");
    default: return "";
  }
});
const secondaryDocuments = ref<Record<string, ProjectKnowledgeDocument>>({});
const secondaryKnowledgeSelection = ref<{
  requestId: number;
  document: ProjectKnowledgeDocument;
} | null>(null);

function closeSecondaryNavigation(): void {
  secondaryNavigationRequestId += 1;
  secondaryNavigation.value = null;
  secondaryKnowledgeSelection.value = null;
}

function secondarySectionIsOpen(projectId: string, section: WorkspaceSecondarySection): boolean {
  return secondaryNavigation.value?.projectId === projectId
    && secondaryNavigation.value.section === section;
}

watch(workbenchWorkspaceScopeId, (scopeId) => {
  if (scopeId && secondaryNavigation.value?.checkoutId !== scopeId) closeSecondaryNavigation();
  if (scopeId && workspaceOverlay.value && workspaceOverlay.value.checkoutId !== scopeId) closeWorkspaceOverlay();
});

// Opening another list belongs to the covered layout, so reveal it.
watch(secondaryNavigation, (navigation) => {
  if (navigation) closeWorkspaceOverlay();
});

watch(() => [displaySettings.workspaceSectionVisibility.collab, displaySettings.showAgentTab] as const, ([collab, agents]) => {
  const section = workspaceOverlay.value?.section;
  if ((section === "collab" && !collab) || (section === "agents" && !agents)) closeWorkspaceOverlay();
});

async function showSecondaryNavigation(
  item: DevelopmentTreeItem,
  section: WorkspaceSecondarySection,
  options: {
    preferredCheckoutId?: string | null;
    knowledgeSelection?: ProjectKnowledgeDocument | null;
  } = {},
): Promise<void> {
  const requestId = ++secondaryNavigationRequestId;
  const checkout = projectCheckout(
    item.meta.projectId,
    options.preferredCheckoutId ?? item.meta.checkoutId,
  );
  if (!checkout) return;
  if (section === "knowledge" && checkout.knowledgeEnabled === false) return;
  // Record the intent before opening a cold checkout: a second click must be
  // able to cancel it, and a later node selection must supersede this request.
  secondaryNavigation.value = { section, projectId: item.meta.projectId, checkoutId: checkout.checkoutId };
  // Browsing a list must not focus a different pane or replace its session context.
  try {
    if (!checkout.runtime) await workspaceContextBaseStore.openCheckout(checkout.root);
  } catch (error) {
    if (requestId === secondaryNavigationRequestId) closeSecondaryNavigation();
    throw error;
  }
  if (requestId !== secondaryNavigationRequestId) return;
  secondaryKnowledgeSelection.value = options.knowledgeSelection
    ? { requestId, document: { ...options.knowledgeSelection } }
    : null;
  // Retire clean list tabs from earlier layouts; dirty editors retain their close guard.
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of [...group.tabs]) {
      if (!editor.dirty && editor.resource.projectId === item.meta.projectId
        && workspaceSecondaryResourceSection(editor.resource) === section) {
        await closeWorkbenchEditor(group.paneId, editor.editorId);
      }
    }
  }
}

async function toggleSecondaryNavigation(item: DevelopmentTreeItem, section: WorkspaceSecondarySection): Promise<void> {
  if (secondarySectionIsOpen(item.meta.projectId, section)) {
    closeSecondaryNavigation();
    return;
  }
  await showSecondaryNavigation(item, section);
}

async function openSecondaryKnowledgeDocument(document: KnowledgeDocumentSummary): Promise<void> {
  const target = secondaryNavigation.value;
  const checkout = secondaryCheckout.value;
  if (!target || !checkout) return;
  secondaryDocuments.value[`${target.projectId}:${document.id}`] = {
    ...document,
    sourceCheckoutId: checkout.checkoutId,
    sourceWorkspaceGeneration: checkout.runtime?.workspaceGeneration,
    sourceRoot: checkout.root,
    availableCheckoutIds: [checkout.checkoutId],
  };
  await openSecondaryResource({
    resource: { kind: "knowledge", projectId: target.projectId, documentId: document.id },
    title: knowledgeDocumentName(document),
    checkoutId: checkout.checkoutId,
  });
}

async function openSecondaryFile(file: { path: string; name: string }): Promise<void> {
  const target = secondaryNavigation.value;
  if (!target) return;
  await openSecondaryResource({
    resource: { kind: "asset", projectId: target.projectId, path: file.path },
    title: file.name,
    checkoutId: target.checkoutId,
  });
}

async function openSecondaryKnowledgePage(page: KnowledgeWorkbenchPage): Promise<void> {
  const target = secondaryNavigation.value;
  if (!target) return;
  await openSecondaryResource({
    resource: { kind: "section", projectId: target.projectId, section: "knowledge", knowledgePage: page },
    title: knowledgeWorkbenchPageTitle(page),
    checkoutId: target.checkoutId,
  });
}

function knowledgeWorkbenchPageTitle(page: KnowledgeWorkbenchPage): string {
  return page.kind === "retrieval" ? t("knowledge.retrieval.entry")
    : page.kind === "injection" ? t("knowledge.injectionPreview.entry")
    : `${shortPath(page.path)} · ${t("knowledge.explorer.folderConfig")}`;
}

const archivedSessions = ref<SessionSummary[]>([]);
const archivedRefreshKey = ref(0);
const unarchivingIds = ref(new Set<string>());
const archivedTreeItems = computed<DevelopmentTreeItem[]>(() => {
  const activeEditor = workbenchStore.activeEditor(WORKBENCH_WINDOW_ID);
  const target = secondaryNavigation.value?.section === "archived" ? secondaryNavigation.value
    : activeEditor?.resource.kind === "section" && activeEditor.resource.section === "archived"
      ? { projectId: activeEditor.resource.projectId, checkoutId: activeEditor.checkoutBinding?.checkoutId } : null;
  if (!target) return [];
  const snapshot = explorerStore.snapshots[target.projectId];
  const nodesBySession = new Map(snapshot?.nodes.filter((node) => node.resourceKind === "session").map((node) => [node.resourceId, node]));
  const items: DevelopmentTreeItem[] = archivedSessions.value.map((session) => {
    const key = 'archived-session:' + target.projectId + ':' + session.id;
    const node = nodesBySession.get(session.id) ?? {
      nodeId: `archived-session:${session.id}`, projectId: target.projectId,
      nodeKind: "resource" as const, resourceKind: "session", resourceId: session.id,
      parentNodeId: null, position: 0, hidden: false,
    };
    const state = node ? workspaceTreeItemState(snapshot, node.nodeId) : null;
    const status = sessionTreeStatusForSession(session, chatStore.streamingSessionIds);
    const selected = activeResource.value?.kind === "section" && activeResource.value.section === "archived"
      && activeResource.value.sessionId === session.id;
    const name = sessionTreeDisplayTitle(session.title, session.sessionType, Boolean(session.parentSessionId)) || t("chat.session.newSession");
    return {
      key,
      treeRow: {
        key, name, kind: "file", depth: 0, title: session.title, ariaLabel: name,
        selected: selected || isWorkspaceSessionMultiSelected(session.id) || isWorkspaceSessionContextSelected(session.id),
        editing: sessionInlineRename.value?.sessionId === session.id,
        dragEnabled: true,
        pinned: state?.pinned,
        starred: state?.highlighted,
        classes: { ...runtimeStatusClasses(status, "session"), "is-open": selected },
        session: { ...sessionRowPresentation(session, status, true), actionDisabled: unarchivingIds.value.has(session.id) },
      },
      meta: { kind: "session", projectId: target.projectId, checkoutId: target.checkoutId, explorerNode: node, session, archived: true },
    };
  });
  const pinned = items.filter((item) => item.treeRow?.pinned);
  const unpinned = items.filter((item) => !item.treeRow?.pinned);
  if (pinned.length && unpinned.length) pinned[pinned.length - 1]!.treeRow!.pinnedSectionEnd = true;
  return [...pinned, ...unpinned];
});

function completeSessionUnarchive(sessionId: string, projectId: string): void {
  archivedSessions.value = archivedSessions.value.filter((session) => session.id !== sessionId);
  archivedRefreshKey.value += 1;
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (editor.resource.kind !== "section" || editor.resource.section !== "archived" || editor.resource.sessionId !== sessionId) continue;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        resource: { kind: "session", projectId, sessionId },
      });
    }
  }
  const active = workbenchStore.activeEditor(WORKBENCH_WINDOW_ID);
  if (active) activeResource.value = active.resource;

}

function handleSessionUnarchived(sessionId: string, projectId: string): void {
  completeSessionUnarchive(sessionId, projectId);
  void Promise.all([chatStore.refreshSessions(), explorerStore.refreshProjectSessions(projectId)])
    .catch((error) => notificationStore.addNotice("error", normalizeAppError(error).message));
}

async function restoreArchivedSession(sessionId: string, projectId: string): Promise<void> {
  if (unarchivingIds.value.has(sessionId)) return;
  unarchivingIds.value = new Set(unarchivingIds.value).add(sessionId);
  try {
    await unarchiveSession(sessionId);
    await Promise.all([chatStore.refreshSessions(), explorerStore.refreshProjectSessions(projectId)]);
    completeSessionUnarchive(sessionId, projectId);
  } catch (error) {
    notificationStore.addNotice("error", t("development.archived.unarchiveFailed", normalizeAppError(error).message));
  } finally {
    const next = new Set(unarchivingIds.value); next.delete(sessionId); unarchivingIds.value = next;
  }
}

async function openSecondaryResource(descriptor: TreeEditorDescriptor): Promise<void> {
  try {
    await openWorkbenchResourceFromWorkspaceTree(descriptor, { preview: false, pinned: true });
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

const collapsedSessionParents = ref<Set<string>>(new Set());
const activeResource = ref<DevelopmentResourceRef | null>(
  workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.resource ?? null,
);
const selectedSessionIds = ref<Set<string>>(new Set());
const lastSessionSelectionAnchorId = ref<string | null>(null);
// File-mode rows are selected by tree key; Ctrl/Shift selection stays within one checkout.
const selectedFileKeys = ref<Set<string>>(new Set());
const lastFileSelectionAnchorKey = ref<string | null>(null);
const contextMenu = ref<DevelopmentContextMenuState | null>(null);
const contextCanEditLayout = computed(() => {
  const item = contextMenu.value?.item;
  if (!item) return false;
  if (item.meta.kind === "folder" || item.meta.kind === "session") return true;
  return (item.meta.kind === "project" || item.meta.kind === "newSession")
    && workspaceNavigation.getMode(navigationCheckout(item.meta.projectId)?.checkoutId) === "sessions";
});
/** Multi-workspace trees manage their root workspaces from project rows and the empty area. */
const contextCanManageWorkspaces = computed(() => !props.fixedWorkspaceRef
  && workspaceDisplayMode.value === "multi"
  && contextMenu.value?.item.meta.kind === "project");
const resourceActions = ref<InstanceType<typeof ExplorerResourceActions> | null>(null);
const viewResourceActions = ref<InstanceType<typeof ViewResourceActions> | null>(null);
const contextViewTarget = computed(() => contextMenu.value?.item.meta.kind === "view" ? contextMenu.value.item.meta.view : null);

async function runContextViewAction(action: ViewResourceAction): Promise<void> {
  const target = contextViewTarget.value;
  contextMenu.value = null;
  if (target) await viewResourceActions.value?.run(action, target);
}

async function viewResourceChanged(target: ViewResourceTarget): Promise<void> {
  const checkout = workspaceContextBaseStore.checkoutsById[target.workspaceRef.checkoutId];
  if (!checkout) return;
  try { await explorerStore.loadProject(checkout.projectId, true); }
  catch (error) { notificationStore.addNotice("warning", normalizeAppError(error).message); }
}
const { showsFullPath, movePathPreference } = useExplorerPathDisplay();
const contextFileTarget = computed(() => contextMenu.value ? treeFileTarget(contextMenu.value.item) : null);
const contextCreateTarget = computed(() => contextMenu.value ? treeFileCreateTarget(contextMenu.value.item) : null);
const managedWorktreeSource = ref<string | null>(null);
const worktreeSelectionBusy = ref(false);
const displayMenu = ref<{ x: number; y: number } | null>(null);
/** Submenu of the display menu: node visibility, or which nodes the top bar shows. */
const specialNodesMenu = ref<{ x: number; y: number; kind: "visibility" | "topBar" } | null>(null);
const workspaceMenu = ref<{ x: number; y: number } | null>(null);
const recentWorkspaceContextMenu = ref<{ x: number; y: number; path: string } | null>(null);
const folderDialog = ref<FolderDialogState | null>(null);
const folderInput = ref<HTMLInputElement | null>(null);
const inlineCreate = ref<WorkspaceInlineCreateState | null>(null);
const inlineCreateInput = ref<HTMLInputElement | null>(null);
const inlineCreateRow = ref<HTMLElement | null>(null);
const presetDialog = ref<PresetDialogState | null>(null);
const presetInput = ref<HTMLInputElement | null>(null);
const sessionDeleteDialog = ref<SessionDeleteDialogState | null>(null);
const sessionInlineRename = ref<SessionInlineRenameState | null>(null);
const sessionEditorRefs = new Map<string, InstanceType<typeof WorkbenchSessionEditor>>();
const replacedWorkspaceSessionDrafts = new Map<string, UserMessageDraft>();
const workspaceFileEditorRefs = new Map<string, InstanceType<typeof WorkspaceFilePreview>>();
const pendingWorkStateEditors = new Map(Object.entries(savedWindowState(WORKBENCH_WINDOW_ID)?.editors ?? {}));
const restoringWorkStateEditors = new Set<string>();
/** Busy session editors (running or waiting for input) stay mounted while hidden. */
const retainedSessionEditorIds = reactive(new Set<string>());
const sessionEditorWorkStateRestores = new Map<string, Promise<void>>();
const sessionEditorMountLimit = computed(() => displaySettings.mountedSessionEditorsPerGroup);
let releaseWorkStateProvider: (() => void) | null = null;

async function restoreEditorWorkState(editorId: string): Promise<void> {
  const snapshot = pendingWorkStateEditors.get(editorId);
  if (!snapshot || restoringWorkStateEditors.has(editorId)) return;
  restoringWorkStateEditors.add(editorId);
  try {
    await nextTick();
    const restored = snapshot.kind === "workspaceFile" || snapshot.kind === "knowledge"
      ? await workspaceFileEditorRefs.get(editorId)?.restoreWorkState(snapshot)
      : snapshot.kind === "session" ? await sessionEditorRefs.get(editorId)?.restoreWorkState(snapshot)
      : snapshot.kind === "agent" ? await agentEditorRefs.get(editorId)?.restoreWorkState(snapshot)
      : await applyWorkbenchEditorTransferSnapshot(editorId, snapshot);
    if (restored) pendingWorkStateEditors.delete(editorId);
  } catch (error) { console.warn("[work-state] editor restore failed", error); }
  finally { restoringWorkStateEditors.delete(editorId); }
}

async function captureWorkbenchWorkState(): Promise<void> {
  if (props.prewarm || !savedWindowState(WORKBENCH_WINDOW_ID)) return;
  workbenchStore.persist(WORKBENCH_WINDOW_ID);
  const state = savedWindowState(WORKBENCH_WINDOW_ID)!;
  const snapshots = { ...state.editors };
  const allEditorIds = new Set(Object.values(state.layouts).flatMap(layout =>
    Object.values(layout.groups).flatMap(group => group.tabs.map(editor => editor.editorId))));
  for (const id of Object.keys(snapshots)) if (!allEditorIds.has(id)) delete snapshots[id];
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (pendingWorkStateEditors.has(editor.editorId)) continue;
      if (!sessionEditorRefs.has(editor.editorId) && !workspaceFileEditorRefs.has(editor.editorId)
        && !workbenchViewEditorRefs.has(editor.editorId) && !agentEditorRefs.has(editor.editorId)) continue;
      // Saved work state omits buffers that match their files on disk.
      const snapshot = await exportWorkbenchEditorTransferSnapshot(editor, { workState: true });
      if (snapshot.kind !== "resource") snapshots[editor.editorId] = snapshot;
    }
  }
  updateWindowWorkState(WORKBENCH_WINDOW_ID, { editors: snapshots, navigation: {
    ...state.navigation,
    [workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) ?? ""]: {
      expanded: [...expanded.value], secondary: secondaryNavigation.value ? { ...secondaryNavigation.value } : null,
    },
  } });
}
const agentEditorRefs = new Map<string, InstanceType<typeof AgentView>>();
function setAgentEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "saveFile" in value) {
    // Vue calls function refs on every patch of the editor stack slot.
    if (agentEditorRefs.get(editorId) === value) return;
    agentEditorRefs.set(editorId, value as InstanceType<typeof AgentView>);
    void restoreEditorWorkState(editorId);
  } else {
    const snapshot = agentEditorRefs.get(editorId)?.exportTransferSnapshot();
    if (snapshot) rememberEditorWorkState(editorId, snapshot);
    agentEditorRefs.delete(editorId);
  }
}
const workbenchAssetEditorRefs = new Map<string, InstanceType<typeof WorkbenchAssetEditor>>();
const workbenchViewEditorRefs = new Map<string, InstanceType<typeof WorkbenchViewEditor>>();
const workbenchViewIcons = reactive<Record<string, string | null>>({});
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
let unlistenExplorerFileAction: UnlistenFn | null = null;
let unlistenWorkbenchInspectorOpen: UnlistenFn | null = null;
let unlistenWorkbenchFileOpen: UnlistenFn | null = null;
let transferHostReady = false;
let appliedInitialTransferToken = "";
let unregisterSharedTransferTarget: (() => void) | null = null;
const internalDrag = useInternalDragController();
let appWindow: Pick<TauriWindowHandle, "label" | "listen" | "setFocus"> | null = props.nativeWindow ?? null;
let workbenchDisposed = false;
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
// Dragging a workspace item that opens a tab reveals the covered editor groups as drop targets.
const workspaceOverlayShown = computed(() => workspaceOverlayOpen.value
  && !(dragging.value && treeEditorDescriptor(dragging.value)));
const dropTargetKey = ref<string | null>(null);
const layoutDropIntent = ref<LayoutDropIntent | null>(null);
const FOLDER_DRAG_EXPAND_DELAY_MS = 800;
watch(
  () => layoutDropIntent.value?.targetKey === dropTargetKey.value
    ? dropTargetKey.value
    : null,
  (key, _previousKey, onCleanup) => {
    if (!key || expanded.value.has(key)) return;
    const timer = ownerWindow.setTimeout(() => {
      expanded.value = new Set([...expanded.value, key]);
    }, FOLDER_DRAG_EXPAND_DELAY_MS);
    // Stop pending expansion when the drop target changes or the panel unmounts.
    onCleanup(() => ownerWindow.clearTimeout(timer));
  },
  { flush: "sync" },
);
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
let settlingLayoutDropId = 0;
const sessionTreeInteractionActive = computed(() => treePointerPressed.value || !!internalDrag.dragging.value
  || !!settlingLayoutDrop.value || !!contextMenu.value || !!sessionInlineRename.value || !!resourceActions.value?.draft
  || !!inlineCreate.value || !!displayMenu.value || !!workspaceMenu.value || selectedSessionIds.value.size > 0);
const protectedTreeSnapshots = ref<Record<string, ProjectExplorerSnapshot>>({});
watch(sessionTreeInteractionActive, (active) => {
  protectedTreeSnapshots.value = active ? { ...explorerStore.snapshots } : {};
}, { flush: "sync" });

function renderedExplorerSnapshot(projectId: string): ProjectExplorerSnapshot | undefined {
  const current = protectedTreeSnapshots.value[projectId] ?? explorerStore.snapshots[projectId];
  const pending = settlingLayoutDrop.value?.snapshot;
  return pending?.projectId === projectId && pending.presetId === current?.presetId
    ? pending
    : current;
}

const SESSION_REVEAL_TIMEOUT_MS = 10_000;
const SESSION_ROW_FLASH_MS = 1_200;
// Sessions created in this window are revealed once their row is placed in the tree.
const pendingSessionReveals = ref<Record<string, string>>({});
const sessionRevealTimers = new Map<string, number>();
const flashingSessionIds = ref<ReadonlySet<string>>(new Set());
const sessionRowFlashTimers = new Map<string, number>();

function isSessionPlacedInTree(projectId: string, sessionId: string): boolean {
  return renderedExplorerSnapshot(projectId)?.nodes.some((node) => (
    node.resourceKind === "session" && node.resourceId === sessionId
  )) ?? false;
}

function revealSessionWhenPlaced(projectId: string, sessionId: string): void {
  pendingSessionReveals.value = { ...pendingSessionReveals.value, [sessionId]: projectId };
  ownerWindow.clearTimeout(sessionRevealTimers.get(sessionId));
  sessionRevealTimers.set(sessionId, ownerWindow.setTimeout(
    () => cancelSessionReveal(sessionId),
    SESSION_REVEAL_TIMEOUT_MS,
  ));
}

function cancelSessionReveal(sessionId: string): void {
  ownerWindow.clearTimeout(sessionRevealTimers.get(sessionId));
  sessionRevealTimers.delete(sessionId);
  if (!(sessionId in pendingSessionReveals.value)) return;
  const next = { ...pendingSessionReveals.value };
  delete next[sessionId];
  pendingSessionReveals.value = next;
}

function flashSessionRow(sessionId: string): void {
  ownerWindow.clearTimeout(sessionRowFlashTimers.get(sessionId));
  flashingSessionIds.value = new Set([...flashingSessionIds.value, sessionId]);
  sessionRowFlashTimers.set(sessionId, ownerWindow.setTimeout(() => {
    sessionRowFlashTimers.delete(sessionId);
    flashingSessionIds.value = new Set([...flashingSessionIds.value].filter((id) => id !== sessionId));
  }, SESSION_ROW_FLASH_MS));
}

// Non-session rows located from a tab flash by tree key.
const flashingTreeRowKeys = ref<ReadonlySet<string>>(new Set());
const treeRowFlashTimers = new Map<string, number>();

function flashTreeRow(key: string): void {
  ownerWindow.clearTimeout(treeRowFlashTimers.get(key));
  flashingTreeRowKeys.value = new Set([...flashingTreeRowKeys.value, key]);
  treeRowFlashTimers.set(key, ownerWindow.setTimeout(() => {
    treeRowFlashTimers.delete(key);
    flashingTreeRowKeys.value = new Set([...flashingTreeRowKeys.value].filter((candidate) => candidate !== key));
  }, SESSION_ROW_FLASH_MS));
}

function withFlashingTreeRows(items: DevelopmentTreeItem[]): DevelopmentTreeItem[] {
  const keys = flashingTreeRowKeys.value;
  if (keys.size === 0) return items;
  return items.map((item) => (item.treeRow && keys.has(item.key)
    ? { ...item, treeRow: { ...item.treeRow, classes: { ...item.treeRow.classes, "is-reveal-flash": true } } }
    : item));
}

/** Highlight a newly placed session row and scroll to it when it is out of view. */
async function revealPlacedSession(sessionId: string): Promise<void> {
  flashSessionRow(sessionId);
  if (!props.showExplorer || props.prewarm) return;
  await nextTick();
  const sessionsList = secondaryNavigation.value?.section === "sessions";
  const index = (sessionsList ? secondarySessionItems.value : treeItems.value)
    .findIndex((item) => item.meta.kind === "session" && item.meta.session?.id === sessionId);
  if (index >= 0) (sessionsList ? sessionsTreeRef.value : workspaceTreeRef.value)?.scrollToIndex(index);
}

watch(
  () => Object.entries(pendingSessionReveals.value)
    .filter(([sessionId, projectId]) => isSessionPlacedInTree(projectId, sessionId))
    .map(([sessionId]) => sessionId),
  (placedSessionIds) => {
    for (const sessionId of placedSessionIds) {
      cancelSessionReveal(sessionId);
      void revealPlacedSession(sessionId);
    }
  },
);

onUnmounted(() => {
  for (const timer of [...sessionRevealTimers.values(), ...sessionRowFlashTimers.values(), ...treeRowFlashTimers.values()]) {
    ownerWindow.clearTimeout(timer);
  }
  sessionRevealTimers.clear();
  sessionRowFlashTimers.clear();
  treeRowFlashTimers.clear();
});
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
  || internalDrag.isDraggingType(VIEW_TREE_INTERNAL_DRAG_TYPE)
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
const pinDropProjectId = ref<string | null>(null);
const pinDropInsertion = ref<PinnedDropInsertion | null>(null);
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
let releaseNativeUnityAssetDrop: (() => void) | null = null;
let releaseUnitySendToLocus: (() => void) | null = null;
let unregisterWorkbenchInternalDropTarget: (() => void) | null = null;
let unregisterTopBarInternalDropTarget: (() => void) | null = null;

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
const AGENTS_SYSTEM_RESOURCE_ID = "agents";
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
  { resourceId: AGENTS_SYSTEM_RESOURCE_ID, labelKey: "app.tab.agent", icon: Bot },
  { resourceId: ARCHIVED_SYSTEM_RESOURCE_ID, labelKey: "app.tab.archived", icon: Archive },
];

// Single-workspace system nodes can leave the tree for the app's top tab bar.
// Only the main workbench of a window that shows that bar takes part.
const topBarNodesEnabled = computed(() => !!props.topBarTarget && !props.auxiliary && !props.fixedWorkspaceRef
  && props.showExplorer && !props.prewarm && workspaceDisplayMode.value === "single");
const topBarPlacedNodeIds = computed<ReadonlySet<string>>(() => new Set(
  topBarNodesEnabled.value ? topBarNodeIds(settingValue(WORKSPACE_TOP_BAR_NODES_SETTING)) : [],
));
/** Set while the top bar builds its entries, which the tree itself leaves out. */
let includeTopBarNodes = false;

const visibleProjects = computed<ProjectContextDescriptor[]>(() => {
  if (props.fixedWorkspaceRef) {
    const checkout = workspaceContextStore.checkoutsById[props.fixedWorkspaceRef.checkoutId];
    const project = checkout ? workspaceContextStore.projectsById[checkout.projectId] : null;
    return project ? [project] : [];
  }
  if (workspaceDisplayMode.value === "multi") return workspaceContextStore.projects;
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  const scopedCheckout = scopedCheckoutId
    ? workspaceContextStore.checkoutsById[scopedCheckoutId]
    : null;
  const focused = scopedCheckout
    ? workspaceContextStore.projectsById[scopedCheckout.projectId] ?? null
    : workspaceContextStore.focusedProject;
  // Failed startup recovery intentionally leaves no focused workspace. Do not
  // resurrect the first historical checkout (or its old project name).
  return focused ? [focused] : [];
});

const explorerHeaderCheckout = computed<WorkspaceCheckoutDescriptor | null>(() => {
  const scopedCheckoutId = singleWorkspaceScopeId.value
    ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
  return (scopedCheckoutId ? workspaceContextStore.checkoutsById[scopedCheckoutId] : null)
    ?? workspaceContextStore.focusedCheckout
    ?? visibleProjects.value[0]?.checkouts[0]
    ?? null;
});

/** Worktrees are entered through their source workspace, so the header names it. */
const explorerHeaderEntryCheckout = computed(() => {
  const checkout = explorerHeaderCheckout.value;
  return checkout
    ? projectEntryCheckout(workspaceContextStore.projectsById[checkout.projectId], checkout)
    : null;
});

const explorerHeaderLabel = computed(() => {
  if (!props.fixedWorkspaceRef && workspaceDisplayMode.value === "multi") {
    return t("development.explorer");
  }
  const root = explorerHeaderEntryCheckout.value?.root ?? "";
  return root ? shortPath(root) : t("development.explorer");
});

const globalSearchTargets = computed<GlobalSearchTarget[]>(() => visibleProjects.value.flatMap((project) => {
  const preferred = props.fixedWorkspaceRef?.checkoutId
    ?? (workspaceContextStore.focusedCheckout?.projectId === project.projectId ? workspaceContextStore.focusedCheckout.checkoutId : null);
  const checkout = project.checkouts.find((entry) => entry.checkoutId === preferred && entry.available !== false && entry.runtime)
    ?? project.checkouts.find((entry) => entry.available !== false && entry.runtime)
    ?? project.checkouts.find((entry) => entry.available !== false);
  if (!checkout) return [];
  return [{ projectId: project.projectId, projectName: projectLabel(project),
    workspaceRef: checkout.runtime ? checkoutWorkspaceRef(checkout) : { checkoutId: checkout.checkoutId } }];
}));

async function openGlobalSearchResult(hit: GlobalSearchResult): Promise<void> {
  const { projectId, workspaceRef } = hit.target;
  const checkout = workspaceContextStore.checkoutsById[workspaceRef.checkoutId];
  if (!checkout?.runtime || checkout.available === false
    || checkout.runtime.workspaceGeneration !== workspaceRef.expectedGeneration
    || !workspaceMaterializationMatches(workspaceRef.expectedMaterializationEpoch, checkout.runtime.materializationEpoch)) {
    throw new Error(t("workbench.unavailable.checkout"));
  }
  if (hit.kind === "file" && hit.path) {
    await openFileInWorkbench({ workspaceRef, filePath: hit.path });
  } else if (hit.kind === "view") {
    await openViewInWorkbench({
      workspaceRef: { ...workspaceRef, expectedGeneration: checkout.runtime.workspaceGeneration },
      viewId: hit.id, title: hit.title, targetLabel: WORKBENCH_WINDOW_ID,
    });
  } else if (hit.kind === "session") {
    const sessionCheckout = workspaceContextStore.checkoutsById[hit.checkoutId ?? checkout.checkoutId];
    if (!sessionCheckout || sessionCheckout.projectId !== projectId || sessionCheckout.available === false) {
      throw new Error(t("workbench.unavailable.checkout"));
    }
    if (!await workspaceContextStore.focusCheckout(sessionCheckout.checkoutId)) throw new Error(t("workbench.unavailable.checkout"));
    await openWorkspaceSessionDescriptor({
      resource: hit.archived
        ? { kind: "section", section: "archived", projectId, sessionId: hit.id }
        : { kind: "session", projectId, sessionId: hit.id },
      title: hit.title, checkoutId: sessionCheckout.checkoutId,
    }, "newTab");
  } else if (hit.docType && hit.path) {
    const result = await knowledgeRead({ kind: "document", type: hit.docType, path: hit.path, part: "summary" }, workspaceRef);
    if (!result.document) throw new Error(t("workbench.unavailable.knowledge"));
    secondaryDocuments.value[`${projectId}:${hit.id}`] = {
      ...result.document, sourceCheckoutId: checkout.checkoutId,
      sourceWorkspaceGeneration: checkout.runtime.workspaceGeneration,
      sourceRoot: checkout.root, availableCheckoutIds: [checkout.checkoutId],
    };
    await openWorkbenchResourceFromWorkspaceTree({
      resource: { kind: "knowledge", projectId, documentId: hit.id },
      title: hit.title, checkoutId: checkout.checkoutId,
    }, { preview: false, pinned: true });
  }
}

const explorerHeaderTitle = computed(() => {
  if (!props.fixedWorkspaceRef && workspaceDisplayMode.value === "multi") return undefined;
  const checkout = explorerHeaderCheckout.value;
  const entry = explorerHeaderEntryCheckout.value;
  if (!checkout) return undefined;
  return entry && entry.checkoutId !== checkout.checkoutId
    ? `${entry.root}\n${checkout.root}`
    : checkout.root;
});

const presetProjectId = computed(() => activeResource.value?.projectId
  ?? workspaceContextStore.focusedProject?.projectId
  ?? visibleProjects.value[0]?.projectId
  ?? "");

function navigationCheckout(projectId: string): WorkspaceCheckoutDescriptor | null {
  const checkoutId = props.fixedWorkspaceRef?.checkoutId ?? singleWorkspaceScopeId.value
    ?? workspaceContextStore.focusedCheckout?.checkoutId;
  return projectCheckout(projectId, checkoutId);
}

const navigationModeCheckout = computed(() => navigationCheckout(presetProjectId.value));
const navigationMode = computed(() => workspaceNavigation.getMode(navigationModeCheckout.value?.checkoutId));
function fileOrderItemKey(item: DevelopmentTreeItem): string | null {
  if (item.meta.workspaceFile && !item.meta.workspaceFile.loadMore) return workspaceFileOrderKey(item.meta.workspaceFile.node.path);
  const node = item.meta.explorerNode;
  return node?.resourceKind === SYSTEM_RESOURCE_KIND ? workspaceNodeOrderKey(node.nodeId) : null;
}
function fileOrderDragItem(item: DevelopmentTreeItem): WorkspaceOrderDragItem | null {
  const checkout = navigationCheckout(item.meta.projectId), key = fileOrderItemKey(item);
  return key && checkout?.runtime && workspaceNavigation.getMode(checkout.checkoutId) === "files"
    ? { item: key, projectId: item.meta.projectId, workspaceRef: checkoutWorkspaceRef(checkout) } : null;
}
useWorkspaceFileOrderDrop({
  root: () => explorerRootRef.value,
  source: source => {
    if (source.payload.type === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
      const data = source.payload.data as WorkspaceLayoutInternalDragData;
      // The separate session sidebar continues to use its own saved layout.
      return data.items && data.items.length > 1 ? null : fileOrderDragItem(data.item);
    }
    if (source.payload.type !== WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) return null;
    const data = source.payload.data as WorkbenchReferenceDragData;
    const checkout = navigationCheckout(data.origin.projectId), entry = data.entries[0];
    if (!data.fileSystemMove || data.entries.length !== 1 || entry?.kind !== "file" || !checkout
      || workspaceNavigation.getMode(checkout.checkoutId) !== "files") return null;
    const path = workspaceRelativeEntryPath(data.origin.workspaceRoot, entry.path);
    return path ? { item: workspaceFileOrderKey(path), projectId: data.origin.projectId, workspaceRef: data.origin.workspaceRef } : null;
  },
  row: hit => {
    const element = hit.closest<HTMLElement>(".workspace-tree-row-shell");
    const item = element && treeItems.value.find(candidate => candidate.key === element.dataset.treeKey);
    const target = item && fileOrderDragItem(item);
    return target && element ? { ...target, element,
      folder: !!item?.meta.workspaceFile && item.treeRow?.kind === "folder",
      insideDrop: item?.meta.kind === "newSession",
    } : null;
  },
  move: async (source, anchor, side) => {
    const state = workspaceFileStates[source.workspaceRef.checkoutId];
    const checkout = navigationCheckout(source.projectId);
    if (!state || !checkout || JSON.stringify(checkoutWorkspaceRef(checkout)) !== JSON.stringify(source.workspaceRef)) {
      throw new Error(t("workbench.unavailable.checkout"));
    }
    // A node dragged from the top bar returns to the tree first, so the rows it
    // is ordered among below include it.
    const topBarNode = explorerStore.snapshots[source.projectId]?.nodes.find((node) => (
      node.resourceKind === SYSTEM_RESOURCE_KIND && workspaceNodeOrderKey(node.nodeId) === source.item
    ));
    if (isTopBarNodeId(topBarNode?.resourceId) && topBarPlacedNodeIds.value.has(topBarNode.resourceId)) {
      setTopBarNodePlacement(topBarNode.resourceId, false);
    }
    const directory = workspaceOrderDirectory(source.item);
    const siblings = await state.siblings(directory);
    const nodes = directory === "." ? treeItems.value.filter(item => item.meta.projectId === source.projectId
      && item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND).map(fileOrderItemKey).filter((key): key is string => !!key) : [];
    // Special nodes use the session layout's existing natural order, while the
    // file-mode overrides remain independent of that layout.
    const snapshot = explorerStore.snapshots[source.projectId];
    nodes.sort((left, right) => (snapshot?.nodes.find(node => workspaceNodeOrderKey(node.nodeId) === left)?.position ?? 0)
      - (snapshot?.nodes.find(node => workspaceNodeOrderKey(node.nodeId) === right)?.position ?? 0));
    const newSessionKey = snapshot?.nodes.find(isNewSessionNode)?.nodeId;
    if (newSessionKey) {
      const index = nodes.indexOf(workspaceNodeOrderKey(newSessionKey));
      if (index >= 0) nodes.unshift(...nodes.splice(index, 1));
    }
    await state.move(directory, source.item, anchor, side, [...nodes, ...siblings.map(entry => workspaceFileOrderKey(entry.node.path))]);
  },
});
useWorkspaceFileDrop({
  root: () => explorerRootRef.value,
  resolve: hit => {
    const row = hit.closest<HTMLElement>("[data-tree-key]");
    const item = row ? treeItems.value.find(item => item.key === row.dataset.treeKey) : undefined;
    if (item && item.meta.kind !== "workspaceFile" && item.meta.kind !== "project") return null;
    if (item?.meta.workspaceFile?.loadMore) return null;
    const checkout = item ? navigationCheckout(item.meta.projectId) : navigationModeCheckout.value;
    if (!checkout?.runtime || workspaceNavigation.getMode(checkout.checkoutId) !== "files") return null;
    const element = row ?? hit.closest<HTMLElement>(".file-tree-list, .development-explorer-header");
    if (!element) return null;
    return { projectId: checkout.projectId, root: checkout.root, workspaceRef: checkoutWorkspaceRef(checkout),
      directory: row?.dataset.fileDropDirectory ?? ".", element };
  },
  expand: target => {
    const state = workspaceFileStates[target.workspaceRef.checkoutId];
    const entry = state?.entries.find(entry => entry.node.path === target.directory && !entry.loadMore);
    if (entry?.node.kind === "folder" && !entry.treeRow?.expanded) state?.toggle(entry.node.path);
  },
});
// Files dragged in from the OS copy into the Files folder they are released on.
const nativeWorkspaceFileDrop = createNativeWorkspaceFileDrop();
const fileModeCheckouts = computed(() => props.showExplorer && !props.prewarm
  ? visibleProjects.value.flatMap(project => {
    const checkout = navigationCheckout(project.projectId);
    return checkout && checkout.available !== false && workspaceNavigation.getMode(checkout.checkoutId) === "files"
      ? [checkout] : [];
  }) : []);

watch(fileModeCheckouts, checkouts => {
  const ids = new Set(checkouts.map(checkout => checkout.checkoutId));
  for (const id of Object.keys(workspaceFileStates)) if (!ids.has(id)) delete workspaceFileStates[id];
  for (const checkout of checkouts) {
    if (!checkout.runtime) void workspaceContextBaseStore.openCheckout(checkout.root)
      .catch(error => notificationStore.addNotice("error", normalizeAppError(error).message));
  }
}, { immediate: true });

function setNavigationMode(mode: WorkspaceNavigationMode, checkout = navigationModeCheckout.value): void {
  if (!checkout) return;
  workspaceNavigation.setMode(checkout.checkoutId, mode);
  displayMenu.value = null;
  contextMenu.value = null;
  resetSessionMultiSelection();
  resetFileMultiSelection();
  if (secondaryNavigation.value?.checkoutId === checkout.checkoutId
    && (secondaryNavigation.value.section === "sessions" || secondaryNavigation.value.section === "assets"
      || secondaryNavigation.value.section === "archivedFiles")) {
    closeSecondaryNavigation();
  }
}

const activePresetId = computed(() => (
  presetProjectId.value
    ? explorerStore.snapshots[presetProjectId.value]?.presetId ?? ""
    : ""
));

const specialNodeVisibilityItems = computed(() => {
  const nodes = explorerStore.snapshots[presetProjectId.value]?.nodes ?? [];
  return WORKSPACE_SPECIAL_NODE_DEFINITIONS.flatMap((definition) => {
    if (definition.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID && !workspaceKnowledgeEnabled(presetProjectId.value)) return [];
    const node = nodes.find((candidate) => (
      candidate.resourceKind === SYSTEM_RESOURCE_KIND
      && candidate.resourceId === definition.resourceId
    ));
    const presentation = definition.resourceId === ASSETS_SYSTEM_RESOURCE_ID && navigationMode.value === "files"
      ? { labelKey: "development.sessions", icon: MessageSquare } : {};
    return node ? [{ ...definition, ...presentation, node }] : [];
  });
});

const specialNodeVisibilityBusy = ref<Set<string>>(new Set());

watch(displayMenu, (menu) => {
  if (!menu) specialNodesMenu.value = null;
});

watch(workspaceMenu, (menu) => {
  if (!menu) recentWorkspaceContextMenu.value = null;
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
  const focused = workspaceContextStore.focusedCheckout;
  const current = focused
    ? projectEntryCheckout(workspaceContextStore.projectsById[focused.projectId], focused)?.root ?? ""
    : "";
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
    if (workspaceDisplayMode.value === "single") {
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
  const focused = project.checkouts.find(
    (checkout) => checkout.checkoutId === workspaceContextStore.focusedCheckout?.checkoutId,
  ) ?? project.checkouts[0];
  const preferred = projectEntryCheckout(project, focused);
  return preferred ? shortPath(preferred.root) : project.projectId;
}

const sessionBranchLabels = computed(() => {
  const labels = new Map<string, string>();
  if (!displaySettings.worktreeEnabled) return labels;
  const localCheckoutId = workbenchWorkspaceScopeId.value ?? workspaceContextStore.focusedCheckout?.checkoutId;
  for (const project of visibleProjects.value) {
    const checkout = projectCheckout(project.projectId, localCheckoutId);
    if (!checkout) continue;
    const currentHead = workspaceGitHead({
      checkoutId: checkout.checkoutId,
      expectedGeneration: checkout.runtime?.workspaceGeneration,
      expectedMaterializationEpoch: checkout.runtime?.materializationEpoch,
    });
    for (const session of [...(explorerStore.resources[project.projectId]?.sessions ?? []), ...archivedSessions.value.filter((session) => session.projectId === project.projectId)]) {
      const label = differingSessionBranchLabel(session.executionTarget, currentHead);
      if (label) labels.set(session.id, label);
    }
  }
  return labels;
});

function sessionRowPresentation(session: SessionSummary, status: SessionTreeStatus | null = null, archived = false): NonNullable<WorkspaceTreeRow['session']> {
  return {
    branch: sessionBranchLabel(session),
    branchTitle: session.executionTarget?.branchRef || session.executionTarget?.headOid || undefined,
    updatedTime: treeModifiedTime(session.updatedAt),
    unread: displaySettings.showSessionUnreadIndicators && isSessionUnread(session.id),
    animated: isAnimatedSessionStatus(status),
    pending: chatStore.pendingSelectionSessionId === session.id,
    status,
    statusLabel: sessionStatusLabel(status),
    archived,
    renameValue: sessionInlineRename.value?.sessionId === session.id ? sessionInlineRename.value.value : undefined,
  };
}

function sessionBranchLabel(session?: SessionSummary): string {
  return session ? sessionBranchLabels.value.get(session.id) ?? "" : "";
}

function treeModifiedTime(modifiedAt?: number): string {
  return formatWorkspaceUpdatedTime(modifiedAt, treeTimeNow.value);
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

// Evaluated once per session-list or focus change rather than per caller: the
// editor stack asks for every session tab on every render, and each tree build.
const focusedProjectChatSessions = computed(() => {
  const projectId = workspaceContextStore.focusedProject?.projectId ?? null;
  const owned = !!projectId && !chatStore.sessions.some(
    (session) => session.projectId && session.projectId !== projectId,
  );
  return { projectId, owned };
});

function sessionsForProject(
  projectId: string,
  fallback: SessionSummary[],
): SessionSummary[] {
  const focused = focusedProjectChatSessions.value;
  if (focused.projectId !== projectId) return fallback;
  return focused.owned ? chatStore.sessions : fallback;
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

function itemSessionIsUnread(item: DevelopmentTreeItem): boolean {
  return displaySettings.showSessionUnreadIndicators && !!item.meta.session
    && isSessionUnread(item.meta.session.id);
}

const unreadTreeCounts = computed(() => {
  const counts = new Map<string, number>();
  if (!displaySettings.showSessionUnreadIndicators) return counts;
  for (const project of visibleProjects.value) {
    const snapshot = explorerStore.snapshots[project.projectId];
    const sessionIds = new Set(sessionsForProject(project.projectId, explorerStore.resources[project.projectId]?.sessions ?? []).map((session) => session.id));
    const nodes = new Map(snapshot?.nodes.map((node) => [node.nodeId, node]));
    const pinnedNodes = new Set(snapshot?.itemStates?.filter((item) => item.pinned && !item.relativePath).map((item) => item.nodeId));
    const counted = new Set<string>();
    for (const node of nodes.values()) {
      if (node.resourceKind !== "session" || !node.resourceId || !sessionIds.has(node.resourceId)
        || !isSessionUnread(node.resourceId) || counted.has(node.resourceId)) continue;
      let current: ProjectExplorerNode | undefined = node;
      const ancestors: string[] = [];
      const visited = new Set<string>();
      while (current && !visited.has(current.nodeId)) {
        visited.add(current.nodeId);
        if (current.hidden) break;
        ancestors.push(`${project.projectId}:${current.nodeId}`);
        if (pinnedNodes.has(current.nodeId)) { current = undefined; break; }
        current = current.parentNodeId ? nodes.get(current.parentNodeId) : undefined;
      }
      if (current) continue;
      counted.add(node.resourceId);
      for (const key of ancestors) counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    counts.set(project.projectId, counted.size);
  }
  return counts;
});

const unreadSessionCount = computed(() => visibleProjects.value.reduce((total, project) => total + (unreadTreeCounts.value.get(project.projectId) ?? 0), 0));

function itemUnreadCount(item: DevelopmentTreeItem): number {
  if (item.treeRow?.expanded) return 0;
  if (item.meta.kind === "project" || item.meta.kind === "sessionsRoot") return unreadTreeCounts.value.get(item.meta.projectId) ?? 0;
  const node = item.meta.explorerNode;
  const count = node ? unreadTreeCounts.value.get(`${item.meta.projectId}:${node.nodeId}`) ?? 0 : 0;
  return item.meta.kind === "session" ? count - Number(itemSessionIsUnread(item)) : count;
}

async function revealNextUnreadSession(): Promise<void> {
  const candidates = visibleProjects.value.flatMap((project) => (explorerStore.snapshots[project.projectId]?.nodes ?? [])
    .filter((node) => node.resourceKind === "session" && node.resourceId && isSessionUnread(node.resourceId)
      && unreadTreeCounts.value.has(`${project.projectId}:${node.nodeId}`)));
  const current = candidates.findIndex((node) => node.resourceId === (lastRevealedUnreadSessionId.value ?? activeWorkspaceSessionId()));
  const node = candidates[(current + 1) % candidates.length];
  if (!node) return;
  lastRevealedUnreadSessionId.value = node.resourceId ?? null;
  await revealSessionNode(node);
}

/** Expand a layout node's project, folders and parent sessions so its row is rendered. */
function expandExplorerNodeAncestors(node: ProjectExplorerNode): void {
  const nextExpanded = new Set(expanded.value);
  const nextCollapsed = new Set(collapsedSessionParents.value);
  nextExpanded.add(`project:${node.projectId}`);
  let parent = node.parentNodeId;
  const visited = new Set<string>();
  while (parent && !visited.has(parent)) {
    visited.add(parent);
    const ancestor = explorerStore.snapshots[node.projectId]?.nodes.find((item) => item.nodeId === parent);
    if (!ancestor) break;
    if (ancestor.resourceKind === "session") nextCollapsed.delete(`session:${node.projectId}:${ancestor.resourceId}`);
    else nextExpanded.add(`folder:${node.projectId}:${parent}`);
    parent = ancestor.parentNodeId;
  }
  expanded.value = nextExpanded;
  collapsedSessionParents.value = nextCollapsed;
}

/** Scroll to a session row: the layout tree in session mode, the session list in file mode. */
async function revealSessionNode(node: ProjectExplorerNode): Promise<boolean> {
  expandExplorerNodeAncestors(node);
  const checkout = navigationCheckout(node.projectId);
  const filesMode = workspaceNavigation.getMode(checkout?.checkoutId) === "files";
  if (filesMode && checkout) await showSecondaryNavigation({ key: `sessions:${node.projectId}`,
    meta: { kind: "sessionsRoot", projectId: node.projectId, checkoutId: checkout.checkoutId } }, "sessions");
  await nextTick();
  const index = (filesMode ? secondarySessionItems.value : treeItems.value).findIndex((item) => item.meta.session?.id === node.resourceId);
  if (index >= 0) (filesMode ? sessionsTreeRef.value : workspaceTreeRef.value)?.scrollToIndex(index);
  return index >= 0;
}

/** Wait for a main-tree row; the file tree source re-emits its rows from a pre-flush watcher. */
async function renderedTreeItemIndex(key: string): Promise<number> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await nextTick();
    const index = treeItems.value.findIndex((item) => item.key === key);
    if (index >= 0) return index;
  }
  return -1;
}

async function scrollToRenderedTreeRow(key: string): Promise<boolean> {
  const index = await renderedTreeItemIndex(key);
  if (index < 0) return false;
  workspaceTreeRef.value?.scrollToIndex(index);
  flashTreeRow(key);
  return true;
}

/** Workspace files: the file tree in file mode, the Assets list in session mode. */
async function revealWorkspaceFileInWorkspace(
  projectId: string,
  checkoutId: string | null,
  path: string,
): Promise<boolean> {
  const checkout = navigationCheckout(projectId);
  if (checkout && workspaceNavigation.getMode(checkout.checkoutId) === "files") {
    // The file tree lists only the navigation checkout; another worktree's path is a different file.
    if (checkoutId && checkoutId !== checkout.checkoutId) return false;
    const listedPath = await workspaceFileStates[checkout.checkoutId]?.reveal(path);
    if (!listedPath) return false;
    expanded.value = new Set([...expanded.value, `project:${projectId}`]);
    return scrollToRenderedTreeRow(`workspace-file:${checkout.checkoutId}:${listedPath}`);
  }
  const target = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] ?? null : projectCheckout(projectId);
  if (!target || target.projectId !== projectId) return false;
  await showSecondaryNavigation({ key: `assets:${projectId}`,
    meta: { kind: "assetsRoot", projectId, checkoutId: target.checkoutId } }, "assets", { preferredCheckoutId: target.checkoutId });
  for (let attempt = 0; attempt < 3 && !secondaryAssetViewRef.value; attempt += 1) await nextTick();
  if (secondaryNavigation.value?.section !== "assets" || secondaryNavigation.value.checkoutId !== target.checkoutId) return false;
  return await secondaryAssetViewRef.value?.revealPath(path) ?? false;
}

/** Mounted files live in the session layout; file mode can only show mounts inside the checkout. */
async function revealLocalFileInWorkspace(
  editor: WorkbenchEditorInput,
  resource: Extract<DevelopmentResourceRef, { kind: "localFile" }>,
): Promise<boolean> {
  const { projectId, nodeId, relativePath } = resource;
  const checkout = navigationCheckout(projectId);
  if (checkout && workspaceNavigation.getMode(checkout.checkoutId) === "files") {
    const path = editor.sourcePath ? workspaceRelativeEntryPath(checkout.root, editor.sourcePath) : null;
    return !!path && revealWorkspaceFileInWorkspace(projectId, checkout.checkoutId, path);
  }
  const node = explorerStore.snapshots[projectId]?.nodes.find((candidate) => candidate.nodeId === nodeId);
  if (!node) return false;
  expandExplorerNodeAncestors(node);
  if (!relativePath) return scrollToRenderedTreeRow(`local-file:${projectId}:${nodeId}`);
  await explorerStore.loadMount(projectId, nodeId);
  const segments = relativePath.split("/").filter(Boolean);
  const nextExpanded = new Set(expanded.value);
  nextExpanded.add(`folder:${projectId}:${nodeId}`);
  for (let index = 1; index < segments.length; index += 1) {
    nextExpanded.add(mountedEntryKey(projectId, nodeId, segments.slice(0, index).join("/")));
  }
  expanded.value = nextExpanded;
  return scrollToRenderedTreeRow(mountedEntryKey(projectId, nodeId, relativePath));
}

async function revealEditorResourceInWorkspace(editor: WorkbenchEditorInput): Promise<boolean> {
  const resource = editor.resource;
  switch (resource.kind) {
    case "session": {
      const node = explorerStore.snapshots[resource.projectId]?.nodes.find((candidate) => (
        candidate.resourceKind === "session" && candidate.resourceId === resource.sessionId
      ));
      if (!node || !await revealSessionNode(node)) return false;
      flashSessionRow(resource.sessionId);
      return true;
    }
    case "knowledge": {
      const document = editorKnowledgeDocument(editor);
      const checkout = document
        ? projectCheckout(resource.projectId, document.sourceCheckoutId ?? editor.checkoutBinding?.checkoutId)
        : null;
      if (!document || !checkout || checkout.knowledgeEnabled === false) return false;
      await showSecondaryNavigation({ key: `knowledge:${resource.projectId}`,
        meta: { kind: "knowledgeRoot", projectId: resource.projectId, checkoutId: checkout.checkoutId } },
      "knowledge", { preferredCheckoutId: checkout.checkoutId, knowledgeSelection: document });
      return secondaryNavigation.value?.section === "knowledge";
    }
    case "localFile":
      return revealLocalFileInWorkspace(editor, resource);
    case "asset":
    case "workspaceFile":
      return revealWorkspaceFileInWorkspace(resource.projectId, editor.checkoutBinding?.checkoutId ?? null, resource.path);
    default:
      return false;
  }
}

/** Tab menu: activate the tab, then select its resource in the workspace tree or a side list. */
async function revealWorkbenchEditorInWorkspace(paneId: string, editorId: string): Promise<void> {
  const editor = workbenchGroup(paneId)?.tabs.find((candidate) => candidate.editorId === editorId);
  if (!editor || !props.showExplorer || props.prewarm) return;
  try {
    await focusWorkbenchEditor(paneId, editorId);
    if (!await revealEditorResourceInWorkspace(editor)) {
      notificationStore.addNotice("info", t("workbench.tabs.revealInWorkspaceUnavailable", editor.title));
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

/** Tab menu: select the tab's file in the system file manager. */
async function openWorkbenchEditorInFileExplorer(paneId: string, editorId: string): Promise<void> {
  const editor = workbenchGroup(paneId)?.tabs.find((candidate) => candidate.editorId === editorId);
  if (!editor) return;
  const resource = editor.resource;
  const document = editorKnowledgeDocument(editor);
  const path = resource.kind === "localFile" ? editor.sourcePath
    : resource.kind === "asset" || resource.kind === "workspaceFile" ? resource.path : null;
  try {
    if (resource.kind === "knowledge" ? !document : !path) throw new Error(t("workbench.unavailable.default"));
    // Relative paths resolve against the tab's own checkout; reveal with its current runtime.
    const checkoutId = document?.sourceCheckoutId ?? editorExecutionBinding(editor)?.checkoutId;
    const checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : projectCheckout(resource.projectId);
    if (!checkout) throw new Error(t("workbench.unavailable.checkout"));
    const runtime = checkout.runtime ?? await workspaceContextBaseStore.openCheckout(checkout.root);
    const workspaceRef: WorkspaceRef = {
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
      expectedMaterializationEpoch: runtime.materializationEpoch,
    };
    if (document) {
      await knowledgeRevealTarget({ kind: "document", docType: document.type, path: document.path }, workspaceRef);
    } else if (path) {
      await showInFolder(workspaceRef, path);
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
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

function clearFileMultiSelection(): void {
  if (selectedFileKeys.value.size > 0) selectedFileKeys.value = new Set();
}

function resetFileMultiSelection(): void {
  clearFileMultiSelection();
  lastFileSelectionAnchorKey.value = null;
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
      if (resource.section === "agents") {
        return matchingWorkbenchEditors(resource)[0]?.editor.title
          || [...agentStore.agents, ...agentStore.subagents].find(agent => agent.id === resource.agentId)?.name
          || resource.agentId || t("app.tab.agent");
      }
      if (resource.section === "knowledge" && resource.knowledgePage) {
        return knowledgeWorkbenchPageTitle(resource.knowledgePage);
      }
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
    case "fileHistory": return `${shortPath(resource.path)} · ${t("fileHistory.title")}`;
    case "asset": return shortPath(resource.path);
    case "sceneObject": return shortPath(resource.objectPath);
    case "view": return workspaceTreeViewReference(resource.projectId, resource.viewId)?.view.name || resource.viewId;
    case "localDirectory": return sourcePath ? shortPath(sourcePath) : resource.relativePath
      ? shortPath(resource.relativePath)
      : resource.nodeId;
    case "localFile": return sourcePath ? shortPath(sourcePath) : resource.relativePath
      ? shortPath(resource.relativePath)
      : resource.nodeId;
  }
}

/**
 * Layout checkout for a session that runs in `target`, or null when its tab
 * belongs to `target` itself. A worktree is chosen per session: a
 * single-workspace window keeps the tab in its current workspace of the same
 * project, and a Locus-managed worktree is otherwise placed under its source
 * checkout. Selecting or reopening a worktree session therefore never moves
 * the workbench scope, pane focus or header to the worktree.
 */
function sessionLayoutCheckoutId(projectId: string, target: WorkspaceCheckoutDescriptor): string | null {
  if (usesCheckoutScopedWorkbench()) {
    const scopeId = workbenchWorkspaceScopeId.value;
    const scope = scopeId ? workspaceContextStore.checkoutsById[scopeId] : null;
    if (scope?.projectId === projectId && (props.fixedWorkspaceRef || !isManagedWorktreeCheckout(scope))) {
      return scope.checkoutId === target.checkoutId ? null : scope.checkoutId;
    }
  }
  if (!isManagedWorktreeCheckout(target)) return null;
  const entry = projectEntryCheckout(workspaceContextStore.projectsById[projectId], target);
  return entry && entry.checkoutId !== target.checkoutId ? entry.checkoutId : null;
}

/** The workspace a descriptor's tab belongs to; a session held per checkout stays in the current layout. */
function descriptorLayoutCheckoutId(descriptor: TreeEditorDescriptor): string | null {
  const checkoutId = descriptor.checkoutId?.trim();
  if (!checkoutId) return null;
  const checkout = workspaceContextStore.checkoutsById[checkoutId];
  const resource = descriptor.resource;
  if (!checkout || (resource.kind !== "session" && resource.kind !== "newSession")) return checkoutId;
  return sessionLayoutCheckoutId(resource.projectId, checkout) ?? checkoutId;
}

/** New sessions start in the worktree last chosen for the project while it still holds that assignment. */
function defaultNewSessionWorktree(projectId: string): {
  checkout: WorkspaceCheckoutDescriptor;
  materializationEpoch: number | null;
} | null {
  if (!displaySettings.worktreeEnabled || props.fixedWorkspaceRef) return null;
  const preference = lastSessionWorktree(projectId);
  const checkout = preference ? workspaceContextStore.checkoutsById[preference.checkoutId] : null;
  if (!preference || !checkout || checkout.projectId !== projectId || checkout.available === false
    || !isManagedWorktreeCheckout(checkout)) return null;
  // A recycled pool slot holds another branch now.
  if (checkout.runtime && !workspaceMaterializationMatches(preference.materializationEpoch, checkout.runtime.materializationEpoch)) {
    return null;
  }
  return { checkout, materializationEpoch: preference.materializationEpoch };
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
  let checkoutId = preferredCheckoutIdForResource(resource, paneId, options.checkoutId);
  let checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : null;
  const sessionBinding=resource.kind === "session"
    ? explorerStore.resources[resource.projectId]?.sessions.find((session)=>session.id===resource.sessionId)
      ?? chatStore.sessions.find((session)=>session.id===resource.sessionId)
    : undefined;
  let materializationEpoch=sessionBinding?.defaultCheckoutId===checkoutId
    ? sessionBinding.defaultMaterializationEpoch : checkout?.runtime?.materializationEpoch ?? null;
  if (resource.kind === "newSession" && (!checkout || !isManagedWorktreeCheckout(checkout))) {
    const remembered = defaultNewSessionWorktree(resource.projectId);
    if (remembered) {
      checkout = remembered.checkout;
      checkoutId = checkout.checkoutId;
      materializationEpoch = remembered.materializationEpoch ?? checkout.runtime?.materializationEpoch ?? null;
    }
  }
  // A session in another checkout of the project keeps its tab in the
  // workspace layout; only the conversation runs in that checkout.
  let layoutCheckoutId = checkoutId;
  let sessionCheckout: EditorCheckoutBinding | null = null;
  if (checkoutId && checkout && (resource.kind === "session" || resource.kind === "newSession")) {
    const sessionLayoutId = sessionLayoutCheckoutId(resource.projectId, checkout);
    if (sessionLayoutId) {
      sessionCheckout = {
        checkoutId,
        expectedGeneration: checkout.runtime?.workspaceGeneration ?? null,
        expectedMaterializationEpoch: materializationEpoch,
      };
      layoutCheckoutId = sessionLayoutId;
    }
  }
  const layoutCheckout = layoutCheckoutId ? workspaceContextStore.checkoutsById[layoutCheckoutId] : null;
  return createWorkbenchEditorInput(resource, options.title ?? titleForResource(resource, options.sourcePath), {
    preview: options.preview ?? true,
    pinned: options.pinned ?? false,
    checkoutBinding: layoutCheckoutId
      ? {
          checkoutId: layoutCheckoutId,
          expectedGeneration: layoutCheckout?.runtime?.workspaceGeneration ?? null,
          expectedMaterializationEpoch: sessionCheckout
            ? layoutCheckout?.runtime?.materializationEpoch ?? null
            : materializationEpoch,
        }
      : null,
    sessionCheckout,
    sourcePath: options.sourcePath ?? null,
    capabilities: resource.kind === "fileHistory"
      ? { split: false, detach: false, duplicate: false }
      : resource.kind === "section" && resource.section === "agents"
      ? { split: true, detach: false, duplicate: false }
      : resource.kind === "view"
      ? { split: false, detach: true, duplicate: false }
      : undefined,
  });
}

function treeEditorDescriptor(item: DevelopmentTreeItem): TreeEditorDescriptor | null {
  if (workspaceSecondarySection(item.meta.kind)) return null;
  switch (item.meta.kind) {
    // A click covers the tabs with these pages; dragging them opens a full-page tab.
    case "collaboration":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "collab" },
        title: t("app.tab.collab"),
        checkoutId: item.meta.checkoutId,
      };
    case "agentsRoot":
      return {
        resource: { kind: "section", projectId: item.meta.projectId, section: "agents" },
        title: t("app.tab.agent"),
        checkoutId: item.meta.checkoutId,
      };
    case "view":
      return item.meta.explorerNode?.resourceId ? {
        resource: { kind: "view", projectId: item.meta.projectId, viewId: item.meta.explorerNode.resourceId },
        title: item.treeRow?.name ?? item.meta.explorerNode.resourceId,
        checkoutId: item.meta.checkoutId,
      } : null;
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
      if (item.meta.archived && item.meta.session) return {
        resource: { kind: "section", section: "archived", projectId: item.meta.projectId, sessionId: item.meta.session.id },
        title: item.treeRow?.name ?? item.meta.session.title,
        checkoutId: item.meta.checkoutId,
      };
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
  // Revealing an editor exits the collaboration or Agent overlay.
  closeWorkspaceOverlay();
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
  if (editor.availability === "unavailable" || !binding?.checkoutId) {
    if (focusPane) workspaceContextStore.activatePane(WORKBENCH_WINDOW_ID, paneId);
    return;
  }
  const existingContext = workspaceContextStore.paneContextAt(WORKBENCH_WINDOW_ID, paneId);
  const runtime=workspaceContextStore.checkoutsById[binding.checkoutId]?.runtime;
  const reference=workspaceRefForEditorBinding(binding,runtime)!;
  if (runtime && !editorBindingMatchesRuntime(binding,runtime)) {
    workbenchStore.updateEditor(WORKBENCH_WINDOW_ID,paneId,editorId,{availability:"unavailable",unavailableReason:t("workbench.unavailable.checkout")});
    return;
  }
  const expectedGeneration = reference.expectedGeneration ?? null;
  const context = existingContext?.focusedCheckoutId === binding.checkoutId
    && (expectedGeneration === null || existingContext.workspaceGeneration === expectedGeneration)
    && workspaceMaterializationMatches(binding.expectedMaterializationEpoch,existingContext.materializationEpoch)
    ? existingContext
    : await workspaceContextStore.focusWorkspaceRefInPane(
         reference,
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
        expectedMaterializationEpoch: binding.expectedMaterializationEpoch,
      },
    });
  }
  const activeSessionId = editor.resource.kind === "session" ? editor.resource.sessionId : null;
  if ((context.activeSessionId ?? null) !== activeSessionId) {
    await workspaceContextStore.setActiveSessionInPane(
      activeSessionId,
      WORKBENCH_WINDOW_ID,
      paneId,
      { activate: focusPane, expectedIntentEpoch: context.intentEpoch },
    );
  }
  if (
    options.refreshServices !== false
    && workspaceRefScopeKey(workspaceContextStore.focusedWorkspaceRef)
      !== lastRefreshedCheckoutServicesScopeKey
    && workbenchWindow.value.focusedPaneId === paneId
    && workbenchWindow.value.groups[paneId]?.activeEditorId === editorId
  ) await refreshFocusedCheckoutServices();
  if ((focusPane || paneWasFocused)
    && workbenchWindow.value.focusedPaneId === paneId
    && workbenchWindow.value.groups[paneId]?.activeEditorId === editorId) {
    showFileHistorySecondarySidebar(editor);
  }
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
    || (draft.knowledgeQuotes?.length ?? 0) > 0
    || (draft.sessionRefs?.length ?? 0) > 0
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
  const targetOpen = item.meta.kind === "newSession"
    ? group?.tabs.some((editor) => (
      editor.resource.kind === "newSession"
      && editor.resource.projectId === item.meta.projectId
    )) === true
    : item.meta.kind === "session"
    && !!item.meta.session
    && matchingWorkbenchEditors(treeEditorDescriptor(item)!.resource).length > 0;
  return workbenchSessionNavigationMode({
    targetOpen,
    splitLayout: workbenchWindow.value.layout.kind === "split",
    focusedGroupTabCount: group?.tabs.length ?? 0,
    currentEditorProtected: currentWorkbenchEditorIsProtected(),
  });
}

async function replaceFocusedWorkbenchResource(
  descriptor: TreeEditorDescriptor,
  target?: { paneId: string; editorId: string },
): Promise<WorkbenchEditorInput> {
  const paneId = target?.paneId ?? workbenchWindow.value.focusedPaneId;
  const current = target
    ? workbenchWindow.value.groups[paneId]?.tabs.find(editor => editor.editorId === target.editorId)
    : editorForPane(paneId);
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
  const paneId = workbenchWindow.value.focusedPaneId;
  if (descriptor.resource.kind === "session" || (descriptor.resource.kind === "section" && descriptor.resource.section === "archived" && descriptor.resource.sessionId)) {
    const matches = matchingWorkbenchEditors(descriptor.resource);
    if (matches.length > 0) {
      return openWorkbenchResourceFromWorkspaceTree(descriptor, {
        preview: false,
        pinned: true,
      });
    }
  } else if (descriptor.resource.kind === "newSession") {
    const group = workbenchGroup(paneId);
    const matches = group?.tabs.filter((editor) => (
      editor.resource.kind === "newSession"
      && editor.resource.projectId === descriptor.resource.projectId
    )) ?? [];
    const existing = matches.find((editor) => editor.editorId === group?.activeEditorId)
      ?? matches[0];
    if (existing) {
      await focusWorkbenchEditor(paneId, existing.editorId);
      await nextTick();
      flashWorkspaceTreeEditorTabs([existing]);
      return existing;
    }
  }

  let resolvedMode = mode;
  if (resolvedMode === "reuse" && currentWorkbenchEditorIsProtected()) {
    resolvedMode = "newTab";
  }
  let editor: WorkbenchEditorInput;
  if (resolvedMode === "reuse") {
    editor = await replaceFocusedWorkbenchResource(descriptor);
  } else {
    editor = await openWorkbenchResource(descriptor, {
      paneId,
      preview: false,
      pinned: true,
      replacePreview: false,
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
    // A session continues in its own checkout (such as its worktree) when
    // that checkout is held per session rather than as the workspace.
    const target = workspaceContextStore.checkoutsById[item.meta.session.executionTarget?.checkoutId ?? ""];
    const runsInOwnCheckout = !!target && target.projectId === project.projectId
      && target.available !== false && !!sessionLayoutCheckoutId(project.projectId, target);
    return await openWorkspaceSessionDescriptor({
      ...descriptor,
      checkoutId: runsInOwnCheckout ? target.checkoutId : checkout.checkoutId,
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
  const saved = await (agentEditorRefs.get(dialog.editorId) ?? workspaceFileEditorRefs.get(dialog.editorId))?.saveFile();
  if (!saved) return;
  dirtyEditorCloseDialog.value = null;
  await closeWorkbenchEditor(dialog.paneId, dialog.editorId, true);
  await continueQueuedWorkbenchEditorCloses();
}

async function discardAndCloseDirtyEditor(): Promise<void> {
  const dialog = dirtyEditorCloseDialog.value;
  if (!dialog) return;
  workspaceFileEditorRefs.get(dialog.editorId)?.discardChanges();
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
    checkoutId: editorExecutionBinding(editor)?.checkoutId,
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

/** Where the editor operates: a session's own checkout, otherwise its layout checkout. */
function editorExecutionBinding(editor: WorkbenchEditorInput): EditorCheckoutBinding | null {
  return editor.sessionCheckout ?? editor.checkoutBinding ?? null;
}

function editorWorkspaceRef(editor: WorkbenchEditorInput): WorkspaceRef | null {
  const binding = editorExecutionBinding(editor);
  const checkoutId = binding?.checkoutId;
  if (!checkoutId) return null;
  const runtime = workspaceContextStore.checkoutsById[checkoutId]?.runtime;
  const reference=workspaceRefForEditorBinding(binding,runtime)!;
  const expectedGeneration = reference.expectedGeneration;
  const expectedMaterializationEpoch=reference.expectedMaterializationEpoch;
  const cached = editorWorkspaceRefs.get(checkoutId);
  if (cached && cached.expectedGeneration === expectedGeneration && cached.expectedMaterializationEpoch === expectedMaterializationEpoch) return cached;
  const workspaceRef: WorkspaceRef = {
    checkoutId,
    expectedGeneration,
    expectedMaterializationEpoch,
  };
  editorWorkspaceRefs.set(checkoutId, workspaceRef);
  return workspaceRef;
}

function editorWorkingDir(editor: WorkbenchEditorInput): string {
  const checkoutId = (editor.sessionCheckout ?? editor.checkoutBinding)?.checkoutId;
  return checkoutId ? workspaceContextStore.checkoutsById[checkoutId]?.root ?? "" : "";
}

function editorKnowledgeDocument(editor: WorkbenchEditorInput): ProjectKnowledgeDocument | null {
  const resource = editor.resource;
  if (resource.kind !== "knowledge") return null;
  return explorerStore.resources[resource.projectId]?.knowledge.find(
    (document) => document.id === resource.documentId,
  ) ?? secondaryDocuments.value[`${resource.projectId}:${resource.documentId}`] ?? null;
}

function editorProject(editor: WorkbenchEditorInput): ProjectContextDescriptor | null {
  return workspaceContextStore.projectsById[editor.resource.projectId] ?? null;
}

function setSessionEditorRef(
  editorId: string,
  value: unknown,
): void {
  if (value && typeof value === "object" && "applyDraftPrefill" in value) {
    // Vue calls function refs on every patch of the editor stack slot, i.e. on
    // every tab switch for every mounted editor; only a new instance matters.
    if (sessionEditorRefs.get(editorId) === value) return;
    sessionEditorRefs.set(editorId, value as InstanceType<typeof WorkbenchSessionEditor>);
    const restore = restoreEditorWorkState(editorId);
    sessionEditorWorkStateRestores.set(editorId, restore);
    void restore.finally(() => {
      if (sessionEditorWorkStateRestores.get(editorId) === restore) sessionEditorWorkStateRestores.delete(editorId);
    });
  }
  else {
    const snapshot = sessionEditorRefs.get(editorId)?.exportTransferSnapshot();
    if (snapshot) rememberEditorWorkState(editorId, snapshot);
    sessionEditorRefs.delete(editorId);
    retainedSessionEditorIds.delete(editorId);
  }
}

function setSessionEditorKeepMounted(editorId: string, keepMounted: boolean): void {
  if (keepMounted) retainedSessionEditorIds.add(editorId);
  else retainedSessionEditorIds.delete(editorId);
}

/**
 * Captures a session tab's work state before its group's mount limit unmounts it.
 * The snapshot is persisted for window restore and applied when the tab mounts again.
 */
function handleWorkbenchEditorEvicted(editor: WorkbenchEditorInput): void {
  const sessionEditor = sessionEditorRefs.get(editor.editorId);
  if (!sessionEditor) return;
  const snapshot = sessionEditor.exportTransferSnapshot();
  const draft = snapshot.composerDraft as UserMessageDraft | null;
  // Applying a draft focuses the composer, so carry it only when it has content.
  const evicted = draft && userMessageDraftHasContent(draft) ? snapshot : { ...snapshot, composerDraft: null };
  pendingWorkStateEditors.delete(editor.editorId);
  rememberEditorWorkState(editor.editorId, evicted);
  pendingWorkStateEditors.set(editor.editorId, evicted);
}

/**
 * An open session tab can take composer input whether or not it is mounted:
 * tabs mount when first shown and may be unloaded by the group limit, and
 * focusing one mounts it (see `restoredSessionEditor`).
 */
function sessionEditorAvailable(editor: WorkbenchEditorInput): boolean {
  return editor.availability !== "unavailable";
}

/**
 * The session editor after a remount has re-applied its saved draft. Restoring
 * replaces the composer content, so appending before it finishes loses the append.
 */
async function restoredSessionEditor(
  editorId: string,
): Promise<InstanceType<typeof WorkbenchSessionEditor> | null> {
  await sessionEditorWorkStateRestores.get(editorId);
  return sessionEditorRefs.get(editorId) ?? null;
}

function setWorkspaceFileEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "saveFile" in value) {
    if (workspaceFileEditorRefs.get(editorId) === value) return;
    workspaceFileEditorRefs.set(editorId, value as InstanceType<typeof WorkspaceFilePreview>);
    void restoreEditorWorkState(editorId);
  } else {
    const snapshot = workspaceFileEditorRefs.get(editorId)?.exportTransferSnapshot({ workState: true });
    if (snapshot) rememberEditorWorkState(editorId, snapshot);
    workspaceFileEditorRefs.delete(editorId);
  }
}

function setWorkbenchAssetEditorRef(editorId: string, value: unknown): void {
  if (value && typeof value === "object" && "refreshIfChanged" in value) {
    if (workbenchAssetEditorRefs.get(editorId) === value) return;
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
    if (workbenchViewEditorRefs.get(editorId) === value) return;
    workbenchViewEditorRefs.set(editorId, value as InstanceType<typeof WorkbenchViewEditor>);
    void restoreEditorWorkState(editorId);
  } else {
    workbenchViewEditorRefs.delete(editorId);
    delete workbenchViewIcons[editorId];
  }
}

function workbenchViewIcon(editor: WorkbenchEditorInput): string | null | undefined {
  return workbenchViewIconValue(editor, workbenchViewIcons, explorerStore.viewResources);
}

function editorWorkspaceLabel(editor: WorkbenchEditorInput): string | null {
  const project = workspaceContextBaseStore.projectsById[editor.resource.projectId];
  if (!project) return null;
  const bound = editor.checkoutBinding
    ? workspaceContextBaseStore.checkoutsById[editor.checkoutBinding.checkoutId]
    : null;
  const entry = bound ? projectEntryCheckout(project, bound) : null;
  return entry ? shortPath(entry.root) : projectLabel(project);
}

const workbenchTabWorkspaceLabels = computed(() => sessionTabWorkspaceLabels(
  Object.values(workbenchWindow.value.groups),
  editorWorkspaceLabel,
));

function workbenchTabWorkspaceLabel(editor: WorkbenchEditorInput): string | null {
  return workbenchTabWorkspaceLabels.value.get(editor.editorId) ?? null;
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
  if (!workspaceMaterializationMatches(payload.workspaceRef.expectedMaterializationEpoch,checkout.runtime.materializationEpoch)) return;
  if (!await activateCheckoutScopedWorkbench(checkout.checkoutId)) return;
  if (WORKBENCH_WINDOW_ID === "main") uiStore.setPage("development");
  const existing = Object.values(workbenchWindow.value.groups).flatMap((group) => (
    group.tabs
      .filter((editor) => editor.resource.kind === "view"
        && editor.resource.projectId === checkout.projectId
        && editor.resource.viewId === payload.viewId
        && editor.checkoutBinding?.checkoutId === checkout.checkoutId
        && editorBindingMatchesRuntime(editor.checkoutBinding, checkout.runtime))
      .map((editor) => ({ paneId: group.paneId, editor }))
  ))[0];
  if (existing) {
    workbenchStore.pinEditor(WORKBENCH_WINDOW_ID, existing.paneId, existing.editor.editorId);
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

provide(WORKBENCH_FILE_OPEN_KEY, openFileInWorkbench);
provide(KNOWLEDGE_QUOTE_SELECTION_KEY, quoteKnowledgeSelectionInConversation);
provide(DOCUMENT_QUOTE_IN_NEW_SESSION_KEY, quoteDocumentSelectionInNewSession);
// Document editors of this window report their selection; session composers read it.
provide(DOCUMENT_SELECTION_HUB_KEY, createDocumentSelectionHub());

async function quoteKnowledgeSelectionInConversation(
  workspaceRef: WorkspaceRef,
  quote: KnowledgeQuoteSelection,
): Promise<void> {
  const checkout = sendToLocusCheckout(workspaceRef);
  if (!checkout) throw new Error(t("workbench.unavailable.checkout"));
  let target = lastFocusedSendToLocusSessionEditor(checkout.checkoutId);
  if (!target) {
    for (const group of Object.values(workbenchWindow.value.groups)) {
      const editor = group.tabs.find((candidate) =>
        candidate.checkoutBinding?.checkoutId === checkout.checkoutId
        && (candidate.resource.kind === "session" || candidate.resource.kind === "newSession")
        && sessionEditorAvailable(candidate));
      if (editor) { target = { paneId: group.paneId, editor }; break; }
    }
  }
  const draft = attachmentDraft({});
  draft.knowledgeQuotes = [quote];
  if (!target) {
    await createNewSessionWithAttachmentsForCheckout(checkout, draft);
    return;
  }
  await focusWorkbenchEditor(target.paneId, target.editor.editorId);
  await nextTick();
  const session = await restoredSessionEditor(target.editor.editorId);
  if (!session) throw new Error(t("editor.quoteFailed"));
  await session.appendComposerDraft(draft);
}

/** Opens a new conversation to the right of the document's pane with the quote attached. */
async function quoteDocumentSelectionInNewSession(
  workspaceRef: WorkspaceRef | null,
  quote: UserMessageDraftKnowledgeQuote,
): Promise<void> {
  // Opening the editor context menu focused the pane that shows the document.
  const sourcePaneId = workbenchWindow.value.focusedPaneId;
  const sourceEditor = editorForPane(sourcePaneId);
  const reference = workspaceRef ?? (sourceEditor ? editorWorkspaceRef(sourceEditor) : null);
  const checkout = reference ? sendToLocusCheckout(reference) : workspaceContextStore.focusedCheckout;
  if (!checkout) throw new Error(t("workbench.unavailable.checkout"));
  const draft = attachmentDraft({});
  draft.knowledgeQuotes = [{ ...quote }];
  await createNewSessionWithAttachmentsForCheckout(checkout, draft, { besidePaneId: sourcePaneId });
}

/**
 * Opens the editor in the pane right of `paneId`, splitting one off when none
 * exists or when `reuseRightPane` rejects the existing one.
 */
async function openWorkbenchResourceBeside(
  descriptor: TreeEditorDescriptor,
  paneId: string,
  reuseRightPane: (group: WorkbenchEditorGroup) => boolean = () => true,
): Promise<WorkbenchEditorInput> {
  const checkoutId = descriptor.checkoutId;
  if (
    checkoutId
    && usesCheckoutScopedWorkbench()
    && workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== checkoutId
    && !await activateCheckoutScopedWorkbench(checkoutId)
  ) throw new Error(`Workbench checkout switch was superseded: ${checkoutId}`);
  // A checkout switch replaces the layout, and with it the source pane.
  const sourcePaneId = workbenchWindow.value.groups[paneId] ? paneId : workbenchWindow.value.focusedPaneId;
  const options = { preview: false, pinned: true, replacePreview: false, allowDuplicate: true };
  const rightPaneId = workbenchPaneRightOf(workbenchWindow.value.layout, sourcePaneId);
  const rightGroup = rightPaneId ? workbenchGroup(rightPaneId) : null;
  if (rightPaneId && rightGroup && reuseRightPane(rightGroup)) {
    return openWorkbenchResource(descriptor, { ...options, paneId: rightPaneId });
  }
  const input = createEditorForResource(descriptor.resource, {
    paneId: sourcePaneId,
    title: descriptor.title,
    checkoutId,
    sourcePath: descriptor.sourcePath,
    preview: false,
    pinned: true,
  });
  const newPaneId = workbenchStore.splitPane(WORKBENCH_WINDOW_ID, sourcePaneId, "right", input);
  if (!newPaneId) return openWorkbenchResource(descriptor, { ...options, paneId: sourcePaneId });
  await focusWorkbenchEditor(newPaneId, input.editorId);
  return input;
}

/** A new pane mounts its editors only after the layout registered its Teleport target. */
async function mountedSessionEditor(
  editorId: string,
): Promise<InstanceType<typeof WorkbenchSessionEditor> | null> {
  for (let tick = 0; tick < 5 && !sessionEditorRefs.has(editorId); tick += 1) await nextTick();
  return sessionEditorRefs.get(editorId) ?? null;
}

function groupHostsSessions(group: WorkbenchEditorGroup): boolean {
  return group.tabs.some((editor) => (
    editor.resource.kind === "session" || editor.resource.kind === "newSession"
  ));
}

/** Knowledge documents and text files (code, Markdown, CSV); not media or asset previews. */
function isTextDocumentEditor(editor: WorkbenchEditorInput): boolean {
  const resource = editor.resource;
  if (resource.kind === "knowledge") return true;
  if (
    resource.kind !== "workspaceFile"
    && resource.kind !== "localFile"
    && !(resource.kind === "asset" && isWorkbenchDocumentPath(resource.path))
  ) return false;
  return workspaceFileEditorRefs.get(editor.editorId)?.isTextContent() === true;
}

/**
 * The new-session shortcut on a text document: starts the session in the
 * session pane to its right, or splits one off when that pane holds no session.
 */
async function openNewSessionBesideDocumentEditor(
  paneId: string,
  editor: WorkbenchEditorInput,
): Promise<void> {
  const rightPaneId = workbenchPaneRightOf(workbenchWindow.value.layout, paneId);
  const rightEditor = rightPaneId ? editorForPane(rightPaneId) : null;
  let target: WorkbenchEditorInput;
  if (rightPaneId && rightEditor?.resource.kind === "newSession") {
    // Like the shortcut inside a session pane: an untouched new session is reused.
    await focusWorkbenchEditor(rightPaneId, rightEditor.editorId);
    target = rightEditor;
  } else {
    target = await openWorkbenchResourceBeside({
      resource: { kind: "newSession", projectId: editor.resource.projectId },
      title: t("chat.session.newSession"),
      checkoutId: editor.checkoutBinding?.checkoutId,
    }, paneId, groupHostsSessions);
  }
  await (await mountedSessionEditor(target.editorId))?.focusComposerInput();
}

const { state: shortcutState } = useKeyboardShortcuts();

function handleDocumentNewSessionShortcut(event: KeyboardEvent): void {
  if (event.defaultPrevented || event.repeat) return;
  if (!matchesShortcut(event, shortcutState.newChat)) return;
  const root = workbenchRootRef.value;
  if (!root || workspaceOverlayShown.value || root.getClientRects().length === 0) return;
  const paneId = workbenchWindow.value.focusedPaneId;
  const editor = editorForPane(paneId);
  if (!editor || editor.availability === "unavailable" || !isTextDocumentEditor(editor)) return;
  event.preventDefault();
  void openNewSessionBesideDocumentEditor(paneId, editor).catch((error) => {
    console.warn("[DevelopmentWorkbench] failed to open a new session beside the document", error);
  });
}

onMounted(() => ownerWindow.addEventListener("keydown", handleDocumentNewSessionShortcut));
onUnmounted(() => ownerWindow.removeEventListener("keydown", handleDocumentNewSessionShortcut));

async function openFileInWorkbench(payload: WorkbenchFileOpenRequest): Promise<void> {
  if (payload.targetLabel && payload.targetLabel !== WORKBENCH_WINDOW_ID) return;
  const checkout = workspaceContextStore.checkoutsById[payload.workspaceRef.checkoutId];
  if (!checkout) throw new Error(t("workbench.unavailable.checkout"));
  if (
    (payload.workspaceRef.expectedGeneration != null
      && checkout.runtime?.workspaceGeneration !== payload.workspaceRef.expectedGeneration)
    || !workspaceMaterializationMatches(
      payload.workspaceRef.expectedMaterializationEpoch,
      checkout.runtime?.materializationEpoch,
    )
  ) throw new Error(t("workbench.unavailable.checkout"));
  if (!payload.filePath.trim()) return;
  if (payload.mode === "history") {
    if (WORKBENCH_WINDOW_ID === "main") uiStore.setPage("development");
    await openFileHistory({ projectId: checkout.projectId, path: payload.filePath, workspaceRef: payload.workspaceRef });
    await appWindow?.setFocus().catch(() => undefined);
    return;
  }
  const resource = await resolveWorkbenchFileTarget(payload.filePath, checkout, {
    documents: () => explorerStore.resources[checkout.projectId]?.knowledge ?? [],
    refresh: () => explorerStore.loadProject(checkout.projectId, true),
  });
  if (!await activateCheckoutScopedWorkbench(checkout.checkoutId)) return;
  if (WORKBENCH_WINDOW_ID === "main") uiStore.setPage("development");
  const existing = Object.values(workbenchWindow.value.groups).flatMap((group) => (
    group.tabs
      .filter((editor) => workbenchResourceKey(editor.resource) === workbenchResourceKey(resource)
        && editor.checkoutBinding?.checkoutId === checkout.checkoutId
        && editorBindingMatchesRuntime(editor.checkoutBinding, checkout.runtime))
      .map((editor) => ({ paneId: group.paneId, editor }))
  ))[0];
  const editor = await openWorkbenchResource({
    resource,
    title: titleForResource(resource),
    checkoutId: checkout.checkoutId,
  }, {
    paneId: existing?.paneId ?? workbenchWindow.value.focusedPaneId,
    preview: false,
    pinned: true,
  });
  await nextTick();
  await workspaceFileEditorRefs.get(editor.editorId)?.revealToolFileHighlight(payload.highlight);
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
  if (!workspaceMaterializationMatches(payload.workspaceRef.expectedMaterializationEpoch, checkout.runtime?.materializationEpoch)) {
    throw new Error(t("workbench.unavailable.checkout"));
  }
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
        && editor.checkoutBinding?.checkoutId === checkout.checkoutId
        && editorBindingMatchesRuntime(editor.checkoutBinding, checkout.runtime))
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
  options: { workState?: boolean } = {},
): Promise<WorkbenchEditorTransferSnapshot> {
  if (agentEditorRefs.has(editor.editorId)) return agentEditorRefs.get(editor.editorId)!.exportTransferSnapshot();
  if (editor.resource.kind === "view") {
    return { kind: "view", state: workbenchViewEditorRefs.get(editor.editorId)?.exportTransferSnapshot() ?? {} };
  }
  if (editor.resource.kind === "session" || editor.resource.kind === "newSession") {
    const mounted = sessionEditorRefs.get(editor.editorId);
    if (mounted) return mounted.exportTransferSnapshot();
    // An unmounted tab (not shown yet since restore, or unloaded by the group
    // limit) carries the snapshot it will apply when it mounts.
    const pending = pendingWorkStateEditors.get(editor.editorId);
    return pending?.kind === "session" ? pending : { kind: "session" };
  }
  if (
    editor.resource.kind === "workspaceFile"
    || editor.resource.kind === "localFile"
    || (editor.resource.kind === "asset" && isWorkbenchDocumentPath(editor.resource.path))
    || editor.resource.kind === "knowledge"
  ) {
    return workspaceFileEditorRefs.get(editor.editorId)?.exportTransferSnapshot(options)
      ?? { kind: "resource" };
  }
  return { kind: "resource" };
}

function rememberEditorWorkState(editorId: string, snapshot: WorkbenchEditorTransferSnapshot): void {
  const state = savedWindowState(WORKBENCH_WINDOW_ID);
  if (!state || pendingWorkStateEditors.has(editorId) || snapshot.kind === "resource") return;
  updateWindowWorkState(WORKBENCH_WINDOW_ID, { editors: { ...state.editors, [editorId]: snapshot } });
}

async function applyWorkbenchEditorTransferSnapshot(
  editorId: string,
  snapshot: WorkbenchEditorTransferSnapshot | null | undefined,
): Promise<boolean> {
  if (!snapshot || snapshot.kind === "resource") return true;
  await nextTick();
  if (snapshot.kind === "view") {
    return workbenchViewEditorRefs.get(editorId)?.applyTransferSnapshot(snapshot.state) ?? false;
  }
  if (snapshot.kind === "agent") return await agentEditorRefs.get(editorId)?.restoreWorkState(snapshot) ?? false;
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
  if (hit?.nodeType !== 1) return null;
  // The titlebar remains a merge target while the lone tab lives in it.
  if (props.titlebarTarget?.contains(hit) && !hit.closest(".workbench-window-controls")) {
    const paneId = workbenchWindow.value.focusedPaneId;
    const group = workbenchWindow.value.groups[paneId];
    return group ? {
      windowId: WORKBENCH_WINDOW_ID,
      paneId,
      direction: "center",
      index: group.tabs.length,
    } : null;
  }
  if (!workbenchRootRef.value?.contains(hit)) return null;
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

function setFileEditorPath(paneId: string, editor: WorkbenchEditorInput, path: string): void {
  if (!("projectId" in editor.resource)) return;
  const resource = editor.resource;
  const root = workspaceContextStore.checkoutsById[editor.checkoutBinding?.checkoutId ?? ""]?.root;
  const filePath = root ? workspaceRelativeEntryPath(root, path) ?? path : path;
  const nextResource = resource.kind === "localFile"
    ? { ...resource, relativePath: resource.relativePath?.replace(/[^/\\]+$/, () => shortPath(path)) }
    : { kind: "workspaceFile" as const, projectId: resource.projectId, path: filePath };
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editor.editorId, {
    resource: nextResource,
    sourcePath: root ? explorerFilePath(root, path) : path,
    title: path.split(/[\\/]/).pop() || path,
  });
  if (workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.editorId === editor.editorId) activeResource.value = nextResource;
}

function setWorkspaceFileEditorDirty(
  paneId: string,
  editorId: string,
  dirty: boolean,
): void {
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editorId, { dirty });
}

async function handleWorkbenchSessionSelected(
  paneId: string,
  payload: { editorId: string; sessionId: string },
): Promise<void> {
  const source = workbenchWindow.value.groups[paneId]?.tabs.find(editor => editor.editorId === payload.editorId);
  if (!source || (source.resource.kind === "session" && source.resource.sessionId === payload.sessionId)) return;
  const projectId = source.resource.projectId;
  const session = sessionsForProject(projectId, explorerStore.resources[projectId]?.sessions ?? [])
    .find(candidate => candidate.id === payload.sessionId);
  if (!session) return;
  const resource: DevelopmentResourceRef = { kind: "session", projectId, sessionId: session.id };
  try {
    // This picker replaces its own tab even when the target is open in another pane.
    const editor = await replaceFocusedWorkbenchResource({
      resource,
      title: session.title || t("chat.session.newSession"),
      checkoutId: props.fixedWorkspaceRef?.checkoutId
        ?? session.executionTarget?.checkoutId
        ?? session.defaultCheckoutId
        ?? source.checkoutBinding?.checkoutId,
    }, { paneId, editorId: payload.editorId });
    await restoreReplacedWorkspaceSessionDraft(resource, editor.editorId);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function handleWorkbenchSessionCreated(
  paneId: string,
  payload: { editorId: string; sessionId: string },
): Promise<void> {
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find(
    (candidate) => candidate.editorId === payload.editorId,
  );
  if (!editor || editor.resource.kind !== "newSession") return;
  revealSessionWhenPlaced(editor.resource.projectId, payload.sessionId);
  workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, payload.editorId, {
    resource: {
      kind: "session",
      projectId: editor.resource.projectId,
      sessionId: payload.sessionId,
    },
    preview: false,
    pinned: true,
  });
  try {
    await workspaceContextStore.setActiveSessionInPane(
      payload.sessionId,
      WORKBENCH_WINDOW_ID,
      paneId,
    );
    await explorerStore.refreshProjectSessions(editor.resource.projectId);
  } catch (error) {
    cancelSessionReveal(payload.sessionId);
    throw error;
  }
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
        ?? editorExecutionBinding(sourceEditor)?.checkoutId
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
    if (isSessionPlacedInTree(projectId, payload.forkedSessionId)) {
      void revealPlacedSession(payload.forkedSessionId);
    }
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
    messageId?: string;
  },
): Promise<void> {
  const sourceSessionId = source.sessionId;
  const sourceTitle = source.title || sourceSessionId.slice(0, 8);

  try {
    const result = await exportSessionContext(sourceSessionId, null, source.messageId);
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
  payload: { editorId: string; request: { sessionId: string; messageId?: string } },
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
    checkoutId: editorExecutionBinding(sourceEditor)?.checkoutId,
    sessionId: sourceSessionId,
    title: sourceTitle,
    messageId: payload.request.messageId,
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
    || sourceWorkspaceRef.expectedMaterializationEpoch !== payload.request.workspaceRef.expectedMaterializationEpoch
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

async function selectSessionWorktree(paneId: string, editorId: string, item: ManagedWorktree): Promise<void> {
  const headObservation = beginWorkspaceGitHeadObservation();
  const editor = workbenchWindow.value.groups[paneId]?.tabs.find((candidate) => candidate.editorId === editorId);
  if (!editor || editor.resource.kind !== "newSession") throw new Error(t("worktrees.selector.locked"));
  if (editor.resource.projectId !== item.projectId) throw new Error(t("worktrees.selector.unavailable"));
  worktreeSelectionBusy.value = true;
  try {
    // The worktree belongs to this session only: open its runtime without
    // focusing the pane or changing the workbench workspace.
    const runtime = await workspaceContextStore.openCheckout(item.root);
    rememberWorkspaceGitHead({
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
      expectedMaterializationEpoch: item.materializationEpoch,
    }, {
      kind: item.branch ? "attached" : "detached",
      refName: item.branch,
      hash: item.headOid || null,
    }, headObservation);
    const current = workbenchWindow.value.groups[paneId]?.tabs.find((candidate) => candidate.editorId === editorId);
    if (!current || current.resource.kind !== "newSession") throw new Error(t("worktrees.selector.locked"));
    const checkout = workspaceContextStore.checkoutsById[runtime.checkoutId];
    rememberSessionWorktree(item.projectId, checkout && isManagedWorktreeCheckout(checkout)
      ? { checkoutId: runtime.checkoutId, materializationEpoch: item.materializationEpoch }
      : null);
    workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editorId, {
      sessionCheckout: runtime.checkoutId === current.checkoutBinding?.checkoutId
        ? null
        : {
            checkoutId: runtime.checkoutId,
            expectedGeneration: runtime.workspaceGeneration,
            expectedMaterializationEpoch: item.materializationEpoch,
          },
    });
  } finally {
    worktreeSelectionBusy.value = false;
  }
}

function sessionCheckoutEditors(): Array<{ paneId: string; editor: WorkbenchEditorInput; binding: EditorCheckoutBinding }> {
  return Object.values(workbenchWindow.value.groups).flatMap((group) => group.tabs.flatMap((editor) => (
    editor.sessionCheckout ? [{ paneId: group.paneId, editor, binding: editor.sessionCheckout }] : []
  )));
}

// No pane focuses a session's checkout, so its runtime is opened here after
// restore, tab creation or a window transfer.
const pendingSessionCheckoutRuntimes = computed(() => [...new Set(sessionCheckoutEditors()
  .map(({ binding }) => workspaceContextStore.checkoutsById[binding.checkoutId])
  .filter((checkout) => !!checkout && !checkout.runtime && checkout.available !== false)
  .map((checkout) => checkout!.checkoutId))].sort().join("\n"));
const sessionCheckoutRuntimeRequests = new Set<string>();
watch(pendingSessionCheckoutRuntimes, (ids) => {
  for (const checkoutId of ids ? ids.split("\n") : []) {
    const root = workspaceContextStore.checkoutsById[checkoutId]?.root;
    if (!root || sessionCheckoutRuntimeRequests.has(checkoutId)) continue;
    sessionCheckoutRuntimeRequests.add(checkoutId);
    void workspaceContextStore.openCheckout(root)
      .catch((error) => console.warn(`[DevelopmentWorkbench] session checkout ${checkoutId} is unavailable`, error))
      .finally(() => sessionCheckoutRuntimeRequests.delete(checkoutId));
  }
}, { immediate: true });

// A removed worktree or a recycled pool slot no longer holds the session's
// branch. A new session falls back to its workspace; an existing one is
// shown as unavailable rather than continuing on another branch.
const staleSessionCheckouts = computed(() => sessionCheckoutEditors().filter(({ editor, binding }) => {
  const checkout = workspaceContextStore.checkoutsById[binding.checkoutId];
  if (!checkout) return false;
  const stale = checkout.available === false
    || (!!checkout.runtime && !editorBindingMatchesRuntime(binding, checkout.runtime));
  return stale && (editor.resource.kind === "newSession" || editor.availability !== "unavailable");
}));
watch(staleSessionCheckouts, (entries) => {
  for (const { paneId, editor } of entries) {
    workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editor.editorId, editor.resource.kind === "newSession"
      ? { sessionCheckout: null }
      : { availability: "unavailable", unavailableReason: t("workbench.unavailable.checkout") });
  }
});

// Worktrees used to be entered as the workspace. A single-workspace window
// still scoped to a Locus-managed worktree returns to its project's source
// workspace; the worktree's sessions reopen there per session.
const worktreeScopeMigrationTarget = computed(() => {
  if (props.fixedWorkspaceRef || props.auxiliary || workspaceDisplayMode.value !== "single") return null;
  const scopeId = workbenchWorkspaceScopeId.value;
  const scope = scopeId ? workspaceContextStore.checkoutsById[scopeId] : null;
  if (!scope || !isManagedWorktreeCheckout(scope)) return null;
  const entry = projectEntryCheckout(workspaceContextStore.projectsById[scope.projectId], scope);
  return entry && entry.checkoutId !== scope.checkoutId && entry.available !== false
    ? { scopeId: scope.checkoutId, root: entry.root }
    : null;
});
const migratedWorktreeScopes = new Set<string>();
let worktreeScopeMigrationReady = false;
function migrateWorktreeScope(target: { scopeId: string; root: string } | null): void {
  if (!worktreeScopeMigrationReady || !target || migratedWorktreeScopes.has(target.scopeId)) return;
  migratedWorktreeScopes.add(target.scopeId);
  void (async () => {
    try {
      await workspaceContextStore.openAndFocus(target.root);
      await refreshFocusedCheckoutServices();
    } catch (error) {
      console.warn("[DevelopmentWorkbench] could not leave the worktree workspace", error);
    }
  })();
}
watch(worktreeScopeMigrationTarget, migrateWorktreeScope);
onMounted(() => {
  worktreeScopeMigrationReady = true;
  migrateWorktreeScope(worktreeScopeMigrationTarget.value);
});

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
      let session: SessionSummary | null = null;

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
            session = await resolveWorkbenchSession(
              resource.projectId,
              resource.sessionId,
              explorerStore.resources[resource.projectId]?.sessions,
            );
            available = !!session;
            reason = available ? null : t("workbench.unavailable.session");
            break;
          case "knowledge":
            available = explorerStore.resources[resource.projectId]?.knowledge.some(
              (document) => document.id === resource.documentId,
            ) === true;
            reason = available ? null : t("workbench.unavailable.knowledge");
            break;
          case "workspaceFile":
          case "fileHistory":
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

      const checkoutId = editor.checkoutBinding?.checkoutId ?? preferredCheckoutIdForResource(
        editor.resource,
        group.paneId,
        editor.checkoutBinding?.checkoutId,
      );
      const checkout = checkoutId ? workspaceContextStore.checkoutsById[checkoutId] : null;
      if (checkout?.knowledgeEnabled === false && (resource.kind === "knowledge" || resource.kind === "knowledgeRoot"
        || (resource.kind === "section" && resource.section === "knowledge"))) {
        available = false;
        reason = t("workspace.setup.knowledgeDisabled");
      }
      // A session tab's binding follows the session's current assignment; with
      // a separate session checkout the tab binding is the layout workspace.
      const sessionCheckout = sessionEditorBinding(editor.sessionCheckout, session);
      const checkoutBinding = editor.sessionCheckout
        ? editor.checkoutBinding ?? null
        : sessionEditorBinding(editor.checkoutBinding, session);
      if (checkoutBinding && (!checkout || checkout.available === false || (checkout.runtime && !editorBindingMatchesRuntime(checkoutBinding,checkout.runtime)))) {
        available=false; reason=t("workbench.unavailable.checkout");
      }
      const sessionTarget = sessionCheckout
        ? workspaceContextStore.checkoutsById[sessionCheckout.checkoutId]
        : null;
      if (resource.kind === "session" && sessionCheckout && sessionTarget && (sessionTarget.available === false
        || (sessionTarget.runtime && !editorBindingMatchesRuntime(sessionCheckout, sessionTarget.runtime)))) {
        available = false; reason = t("workbench.unavailable.checkout");
      }
      if (
        epoch !== workbenchReconcileEpoch
        || workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== expectedWorkspaceScopeId
      ) return;
      if (editor.resource !== resource || !group.tabs.includes(editor)) continue;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        title: resource.kind === "section" && resource.section === "archived" && resource.sessionId
          ? editor.title
          : editor.resource.kind === "view" ? editor.title : titleForResource(editor.resource, sourcePath),
        sourcePath,
        availability: available ? "available" : "unavailable",
        unavailableReason: reason,
        checkoutBinding: checkoutId
          ? {
              checkoutId,
              expectedGeneration: checkout?.runtime?.workspaceGeneration ?? null,
              expectedMaterializationEpoch: checkoutBinding?.expectedMaterializationEpoch,
            }
          : null,
        ...(sessionCheckout !== (editor.sessionCheckout ?? null) ? { sessionCheckout } : {}),
      });
    }
  }
}

let workbenchPaneRestoreEpoch = 0;

async function restoreWorkbenchPaneContexts(
  expectedWorkspaceScopeId = workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID),
): Promise<void> {
  const epoch = ++workbenchPaneRestoreEpoch;
  const restoringWindow = workbenchWindow.value;
  const isCurrent = () => epoch === workbenchPaneRestoreEpoch
    && workbenchWindow.value === restoringWindow
    && workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) === expectedWorkspaceScopeId;
  const focusedPaneId = restoringWindow.focusedPaneId;
  for (const group of Object.values(restoringWindow.groups)) {
    if (!isCurrent()) return;
    const editor = group.tabs.find((candidate) => candidate.editorId === group.activeEditorId);
    const isCurrentEditor = () => isCurrent()
      && workbenchWindow.value.groups[group.paneId] === group
      && group.activeEditorId === editor?.editorId;
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
    const reference = workspaceRefForEditorBinding(
      editor.checkoutBinding,
      workspaceContextStore.checkoutsById[checkoutId]?.runtime,
    );
    if (!reference) continue;
    let context;
    try {
      context = await workspaceContextStore.focusWorkspaceRefInPane(
        reference,
        WORKBENCH_WINDOW_ID,
        group.paneId,
        { activate: false },
      );
    } catch {
      if (!isCurrentEditor()) continue;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        availability: "unavailable",
        unavailableReason: t("workbench.unavailable.checkout"),
      });
      await workspaceContextStore.disposePane(WORKBENCH_WINDOW_ID, group.paneId).catch(() => false);
      continue;
    }
    if (!isCurrent()) return;
    if (!context || !isCurrentEditor()) continue;
    await workspaceContextStore.setActiveSessionInPane(
      editor.resource.kind === "session" ? editor.resource.sessionId : null,
      WORKBENCH_WINDOW_ID,
      group.paneId,
      { activate: false, expectedIntentEpoch: context.intentEpoch },
    );
  }
  if (!isCurrent()) return;
  // Hidden pool windows share Pinia with the main window. Restoring their
  // empty layout must not replace the visible window's workspace focus.
  if (!props.prewarm && restoringWindow.focusedPaneId === focusedPaneId) {
    await focusWorkbenchPane(focusedPaneId);
  }
}

function isExpanded(key: string): boolean {
  return expanded.value.has(key);
}

function isSessionParentExpanded(key: string): boolean {
  return !collapsedSessionParents.value.has(key);
}

function knowledgeFolderKind(nodeId: string): KnowledgeFolderKind | null {
  if (!nodeId.startsWith("knowledge-type:")) return null;
  const match = nodeId.match(/:(plan|memory|design|skill|reference)$/);
  return (match?.[1] as KnowledgeFolderKind | undefined) ?? null;
}

function workspaceKnowledgeEnabled(projectId: string): boolean {
  const checkout = projectCheckout(projectId);
  return (checkout?.knowledgeEnabled ?? checkout?.runtime?.knowledgeEnabled) !== false;
}

function isExplorerNodeVisible(
  node: ProjectExplorerNode,
  knowledge: ProjectKnowledgeDocument[],
  projectId: string,
): boolean {
  if (!workspaceKnowledgeEnabled(projectId) && (
    (node.resourceKind === SYSTEM_RESOURCE_KIND && node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID)
    || node.resourceKind === "knowledge" || node.sourceKind === "knowledge"
  )) return false;
  if (node.resourceKind === SYSTEM_RESOURCE_KIND) {
    if (!includeTopBarNodes && topBarPlacedNodeIds.value.has(node.resourceId ?? "")) return false;
    if (node.resourceId === AGENTS_SYSTEM_RESOURCE_ID && !displaySettings.showAgentTab) return false;
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

function knowledgeDocumentName(document: Pick<KnowledgeDocumentSummary, "path" | "title">): string {
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
  const statusKey = Object.keys(options.classes ?? {}).find((key) => key.startsWith("session-status-") && options.classes?.[key]);
  const status = statusKey?.slice("session-status-".length) as SessionTreeStatus | undefined;
  return {
    activity: status ? { status, statusLabel: sessionStatusLabel(status), animated: isAnimatedSessionStatus(status) } : undefined,
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
    || draft.fileTarget
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
  pinnedRootPath?: string,
): void {
  const listing = explorerStore.mountListing(projectId, mountNode.nodeId);
  if (!listing) return;
  const directoryPaths = new Set(
    listing.entries.filter((entry) => entry.isDir).map((entry) => entry.relativePath),
  );
  const pinnedPaths = new Set((renderedExplorerSnapshot(projectId)?.itemStates ?? [])
    .filter((state) => state.nodeId === mountNode.nodeId && state.pinned && state.relativePath)
    .map((state) => state.relativePath!));
  for (const entry of listing.entries) {
    if (!mountedEntryInPinnedSubtree(entry.relativePath, pinnedPaths, pinnedRootPath)) continue;
    const segments = entry.relativePath.split("/").filter(Boolean);
    const ancestorPaths = segments.slice(0, -1).map((_, index) => (
      segments.slice(0, index + 1).join("/")
    ));
    if (ancestorPaths.some((path) => (
      (!pinnedRootPath || path === pinnedRootPath || path.startsWith(`${pinnedRootPath}/`))
      && directoryPaths.has(path)
      && !isExpanded(mountedEntryKey(projectId, mountNode.nodeId, path))
    ))) continue;
    const key = mountedEntryKey(projectId, mountNode.nodeId, entry.relativePath);
    const selected = entry.isDir
      ? isActiveLocalDirectory(projectId, mountNode.nodeId, entry.relativePath)
      : isActiveLocalFile(projectId, mountNode.nodeId, entry.relativePath);
    const multiSelected = !entry.isDir && selectedFileKeys.value.has(key);
    items.push({
      key,
      treeRow: makeRow(key, entry.name, depth + entry.depth - (pinnedRootPath ? pinnedRootPath.split("/").length - 1 : 0), entry.isDir ? "folder" : "file", {
        expandable: entry.isDir,
        expanded: entry.isDir ? isExpanded(key) : undefined,
        selected: selected || multiSelected,
        dragEnabled: true,
        title: entry.absolutePath,
        classes: {
          "is-open": selected,
          "is-multi-selected": multiSelected,
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

function workspaceTreeViewReference(projectId: string, viewId: string): WorkspaceViewReference | undefined {
  const candidates = (explorerStore.viewResources[projectId] ?? []).filter((item) => item.view.id === viewId);
  return candidates.find((item) => item.workspaceRef.checkoutId === workspaceContextStore.focusedCheckout?.checkoutId)
    ?? candidates[0];
}

function isNewSessionNode(node: ProjectExplorerNode): boolean {
  return node.resourceKind === SYSTEM_RESOURCE_KIND && node.resourceId === NEW_SESSION_SYSTEM_RESOURCE_ID;
}

function appendNewSessionRow(
  items: DevelopmentTreeItem[],
  project: ProjectContextDescriptor,
  depth: number,
): void {
  const node = renderedExplorerSnapshot(project.projectId)?.nodes.find(isNewSessionNode);
  if (!node || !isExplorerNodeVisible(node, explorerStore.resources[project.projectId]?.knowledge ?? [], project.projectId)) return;
  const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
    ? workspaceContextStore.focusedCheckout
    : project.checkouts[0];
  if (!preferredCheckout) return;
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
      t("chat.session.newSession"),
      depth,
      "file",
      {
        dragEnabled: true,
        selected: activeResource.value?.kind === "newSession"
          && activeResource.value.projectId === project.projectId
          && chatStore.activeSessionId === null,
        classes: {
          "is-new-session-row": true,
          "is-new-session-drop-zone": dropAvailable,
          "is-hidden-node": node.hidden,
        },
      },
    ),
  });
}

const knowledgeDocumentIndexes = new WeakMap<readonly ProjectKnowledgeDocument[], Map<string, ProjectKnowledgeDocument>>();
/** Per knowledge list (replaced, not mutated, on change); tree levels used to rebuild it each. */
function knowledgeDocumentsById(documents: readonly ProjectKnowledgeDocument[]): Map<string, ProjectKnowledgeDocument> {
  let index = knowledgeDocumentIndexes.get(documents);
  if (!index) {
    index = new Map(documents.map((document) => [document.id, document]));
    knowledgeDocumentIndexes.set(documents, index);
  }
  return index;
}

function appendLayoutChildren(
  items: DevelopmentTreeItem[],
  project: ProjectContextDescriptor,
  parentNodeId: string | null,
  depth: number,
  sessionById: Map<string, SessionSummary>,
  runtimeStatusByNodeId: Map<string, SessionTreeStatus | null>,
  pinnedRootNodeId?: string,
  inPinnedSection = !!pinnedRootNodeId,
): void {
  const snapshot = renderedExplorerSnapshot(project.projectId);
  const projectResources = explorerStore.resources[project.projectId];
  if (!snapshot || !projectResources) return;
  const knowledgeById = knowledgeDocumentsById(projectResources.knowledge);
  const renderedIntent = inPinnedSection ? null : layoutDropIntent.value;
  const renderedSource = dragging.value;
  const internalSourceNodeId = internalDrag.previewMode.value !== "floating"
    && renderedSource?.meta.projectId === project.projectId
    && renderedIntent
    ? renderedSource.meta.explorerNode?.nodeId ?? null
    : null;
  // Indexed by parent: scanning every node per level and per row made the
  // tree quadratic, and it is rebuilt on every selection change (tab switch).
  const levelNodes = pinnedRootNodeId
    ? snapshot.nodes.filter((node) => node.nodeId === pinnedRootNodeId)
    : explorerChildNodes(snapshot.nodes, parentNodeId);
  const nodes = levelNodes
    .filter((node) => (
      (!!pinnedRootNodeId || !workspaceTreeItemState(snapshot, node.nodeId)?.pinned)
      && isExplorerNodeVisible(node, projectResources.knowledge, project.projectId)
      && !isNewSessionNode(node)
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
      const layoutChildren = explorerChildNodes(snapshot.nodes, node.nodeId).some((candidate) => {
        if (isNewSessionNode(candidate)) return false;
        if (workspaceTreeItemState(snapshot, candidate.nodeId)?.pinned) return false;
        if (!isExplorerNodeVisible(candidate, projectResources.knowledge, project.projectId)) return false;
        if (candidate.nodeKind === "folder") return true;
        if (candidate.resourceKind === "session") {
          return sessionById.has(candidate.resourceId ?? "");
        }
        if (candidate.resourceKind === "knowledge") {
          return knowledgeById.has(candidate.resourceId ?? "");
        }
        if (candidate.resourceKind === SYSTEM_RESOURCE_KIND) return true;
        if (candidate.resourceKind === "view") return !!candidate.resourceId;
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
              undefined,
              inPinnedSection,
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
      && node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID
    ) {
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const key = `knowledge-root:${project.projectId}`;
      const selected = secondarySectionIsOpen(project.projectId, "knowledge");
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
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const key = `collaboration:${project.projectId}`;
      const collaborationOverlayOpen = workspaceOverlayIsOpen(project.projectId, "collab");
      items.push({
        key,
        treeRow: makeRow(key, t("app.tab.collab"), depth, "folder", {
          selected: collaborationOverlayOpen || activeResource.value?.projectId === project.projectId
            && (
              activeResource.value.kind === "checkout"
              || activeResource.value.kind === "collaboration"
              || (activeResource.value.kind === "section" && activeResource.value.section === "collab")
            ),
          dragEnabled: true,
          classes: { "is-open": collaborationOverlayOpen, "is-hidden-node": node.hidden },
        }),
        meta: {
          kind: "collaboration",
          projectId: project.projectId,
          checkoutId: preferredCheckout.checkoutId,
          explorerNode: node,
        },
      });
      continue;
    }
    if (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && (node.resourceId === ASSETS_SYSTEM_RESOURCE_ID || node.resourceId === VIEWS_SYSTEM_RESOURCE_ID || node.resourceId === AGENTS_SYSTEM_RESOURCE_ID)
    ) {
      const section = node.resourceId === ASSETS_SYSTEM_RESOURCE_ID ? "assets" as const : node.resourceId === AGENTS_SYSTEM_RESOURCE_ID ? "agents" as const : "views" as const;
      const preferredCheckout = workspaceContextStore.focusedCheckout?.projectId === project.projectId
        ? workspaceContextStore.focusedCheckout
        : project.checkouts[0];
      if (!preferredCheckout) continue;
      const kind = section === "assets" ? "assetsRoot" as const : section === "agents" ? "agentsRoot" as const : "viewsRoot" as const;
      const key = `${section}:${project.projectId}`;
      const open = section === "agents"
        ? workspaceOverlayIsOpen(project.projectId, "agents")
        : secondarySectionIsOpen(project.projectId, section);
      const selected = open || (section === "agents" && activeResource.value?.kind === "section"
        && activeResource.value.section === "agents" && activeResource.value.projectId === project.projectId);
      items.push({
        key,
        treeRow: makeRow(
          key,
          section === "assets" ? t("app.tab.asset") : section === "agents" ? t("app.tab.agent") : t("app.tab.views"),
          depth,
          section === "assets" ? "folder" : "package",
          {
            selected,
            dragEnabled: true,
            classes: {
              "is-open": open,
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
      const selected = secondarySectionIsOpen(project.projectId, "archived");
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
      const layoutChildren = explorerChildNodes(snapshot.nodes, node.nodeId).some((candidate) => (
        !workspaceTreeItemState(snapshot, candidate.nodeId)?.pinned
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
          session: sessionRowPresentation(session, runtimeStatus),
          dragEnabled: true,
          title: runtimeStatus
            ? `${session.title || displayTitle} — ${sessionStatusLabel(runtimeStatus)}`
            : session.title,
          ariaLabel: [displayTitle, runtimeStatus ? sessionStatusLabel(runtimeStatus) : "",
            displaySettings.showSessionUnreadIndicators && isSessionUnread(session.id) ? t("chat.session.unread") : ""].filter(Boolean).join(" — "),
          classes: {
            ...runtimeStatusClasses(runtimeStatus, "session"),
            "is-open": selected,
            "is-multi-selected": multiSelected,
            "is-context-selected": contextSelected,
            "is-session-pending": chatStore.pendingSelectionSessionId === session.id,
            "is-session-flash": flashingSessionIds.value.has(session.id),
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
          undefined,
          inPinnedSection,
        );
      }
      continue;
    }
    if (node.resourceKind === "knowledge" && node.resourceId) {
      const knowledge = knowledgeById.get(node.resourceId);
      if (!knowledge) continue;
      const key = `knowledge:${project.projectId}:${knowledge.id}`;
      const open = activeResource.value?.kind === "knowledge" && activeResource.value.documentId === knowledge.id;
      const multiSelected = selectedFileKeys.value.has(key);
      items.push({
        key,
        treeRow: makeRow(key, knowledgeDocumentName(knowledge), depth, "file", {
          selected: open || multiSelected,
          dragEnabled: true,
          title: `${knowledge.type}/${knowledge.path}`,
          classes: {
            "kx-leaf": true,
            "is-knowledge-row": true,
            "is-hidden-node": node.hidden,
            "is-open": open,
            "is-multi-selected": multiSelected,
          },
        }),
        meta: { kind: "knowledge", projectId: project.projectId, explorerNode: node, knowledge },
      });
      continue;
    }
    if (node.resourceKind === "view" && node.resourceId) {
      const view = workspaceTreeViewReference(project.projectId, node.resourceId);
      const key = `view:${project.projectId}:${node.resourceId}`;
      const selected = activeResource.value?.kind === "view"
        && activeResource.value.projectId === project.projectId
        && activeResource.value.viewId === node.resourceId;
      items.push({
        key,
        treeRow: makeRow(key, view?.view.name || node.resourceId, depth, "package", {
          selected,
          dragEnabled: true,
          title: view?.view.packageRoot || node.resourceId,
          classes: { "is-view-row": true, "is-open": selected, "is-hidden-node": node.hidden },
        }),
        meta: {
          kind: "view", projectId: project.projectId, explorerNode: node, view,
          checkoutId: view?.workspaceRef.checkoutId ?? projectCheckout(project.projectId)?.checkoutId,
        },
      });
      continue;
    }
    if (node.resourceKind === "local_file" && node.sourcePath) {
      const key = `local-file:${project.projectId}:${node.nodeId}`;
      const selected = isActiveLocalFile(project.projectId, node.nodeId);
      const multiSelected = selectedFileKeys.value.has(key);
      items.push({
        key,
        treeRow: makeRow(key, node.folderName || shortPath(node.sourcePath), depth, "file", {
          selected: selected || multiSelected,
          dragEnabled: true,
          title: node.sourcePath,
          classes: {
            "is-open": selected,
            "is-multi-selected": multiSelected,
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

/** In Files mode system nodes act on the navigation checkout, and Assets lists the sessions. */
function applyFilesModeNavigation(item: DevelopmentTreeItem, checkout: WorkspaceCheckoutDescriptor, depth: number): void {
  if (item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND) item.meta.checkoutId = checkout.checkoutId;
  if (item.meta.kind !== "assetsRoot") return;
  const projectId = item.meta.projectId;
  item.meta = { ...item.meta, kind: "sessionsRoot", checkoutId: checkout.checkoutId };
  item.key = `sessions:${projectId}`;
  item.treeRow = makeRow(item.key, t("development.sessions"), depth, "folder", {
    selected: secondarySectionIsOpen(projectId, "sessions"),
    classes: { "is-open": secondarySectionIsOpen(projectId, "sessions") },
  });
}

function buildWorkspaceTreeItems(projects: ProjectContextDescriptor[], showProjectNodes: boolean, projectFiles = true): DevelopmentTreeItem[] {
  const items: DevelopmentTreeItem[] = [];
  const pinnedItems: DevelopmentTreeItem[] = [];
  for (const project of projects) {
    const checkout = navigationCheckout(project.projectId);
    const filesMode = projectFiles && workspaceNavigation.getMode(checkout?.checkoutId) === "files";
    const projectStart = items.length;
    const projectKey = `project:${project.projectId}`;
    const projectOpen = !showProjectNodes || isExpanded(projectKey);
    const resourceDepth = showProjectNodes ? 1 : 0;
    const projectResources = explorerStore.resources[project.projectId];
    const projectSessions = sessionsForProject(
      project.projectId,
      projectResources?.sessions ?? [],
    );
    const sessionById = new Map(projectSessions.map((session) => [session.id, session]));
    const snapshot = renderedExplorerSnapshot(project.projectId);
    const runtimeStatusByNodeId = buildLayoutRuntimeStatuses(
      snapshot?.nodes ?? [],
      sessionById,
    );
    for (const state of snapshot?.itemStates ?? []) {
      if (filesMode) continue;
      if (!state.pinned) continue;
      const node = snapshot?.nodes.find((candidate) => candidate.nodeId === state.nodeId);
      if (!node || !isExplorerNodeVisible(node, projectResources?.knowledge ?? [], project.projectId)) continue;
      const groupStart = pinnedItems.length;
      if (state.relativePath) {
        appendMountedEntries(pinnedItems, project.projectId, node, 0, state.relativePath);
      } else {
        appendLayoutChildren(pinnedItems, project, null, 0, sessionById, runtimeStatusByNodeId, node.nodeId);
      }
      for (const item of pinnedItems.slice(groupStart)) {
        item.meta.pinnedRoot = { nodeId: state.nodeId, relativePath: state.relativePath };
      }
    }
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

    appendNewSessionRow(items, project, resourceDepth);
    if (filesMode) {
      // Special nodes stay reachable even when the session layout has nested or
      // pinned them. Switching modes never rewrites that saved layout.
      for (const node of (snapshot?.nodes ?? []).filter(node => node.resourceKind === SYSTEM_RESOURCE_KIND)
        .sort((left, right) => left.position - right.position)) {
        appendLayoutChildren(items, project, null, resourceDepth, sessionById, runtimeStatusByNodeId, node.nodeId);
      }
    } else {
      appendLayoutChildren(items, project, null, resourceDepth, sessionById, runtimeStatusByNodeId);
    }
    if (filesMode && checkout) {
      const navigationItems = items.splice(projectStart).filter(item => item.meta.kind === "project"
        || item.meta.kind === "checkout" || item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND);
      for (const item of navigationItems) applyFilesModeNavigation(item, checkout, resourceDepth);
      const fileItems: DevelopmentTreeItem[] = navigationItems.filter(item => item.meta.kind !== "project");
      items.push(...navigationItems.filter(item => item.meta.kind === "project"));
      for (const entry of workspaceFileStates[checkout.checkoutId]?.entries ?? []) {
        const key = `workspace-file:${checkout.checkoutId}:${entry.key}`;
        const open = !entry.loadMore && (activeResource.value?.kind === "asset" || activeResource.value?.kind === "workspaceFile")
          && activeResource.value.projectId === project.projectId
          && explorerFileKey(explorerFilePath(checkout.root, activeResource.value.path)) === explorerFileKey(explorerFilePath(checkout.root, entry.node.path))
          && workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.checkoutBinding?.checkoutId === checkout.checkoutId;
        const multiSelected = !entry.loadMore && selectedFileKeys.value.has(key);
        const contextSelected = contextMenu.value?.fileTargets?.some(target => target.key === key) === true;
        fileItems.push({ key, treeRow: { ...entry.treeRow!, key,
          depth: resourceDepth + entry.treeRow!.depth,
          selected: open || multiSelected || contextSelected, title: explorerFilePath(checkout.root, entry.node.path),
          dragEnabled: !entry.loadMore,
          classes: { "is-open": open, "is-multi-selected": multiSelected, "is-context-selected": contextSelected },
        }, meta: { kind: "workspaceFile", projectId: project.projectId, checkoutId: checkout.checkoutId, workspaceFile: entry } });
      }
      const ordered = orderWorkspaceFileRows(fileItems, workspaceFileStates[checkout.checkoutId]?.order.placements ?? [], fileOrderItemKey);
      const archiveKey = `archived-files:${checkout.checkoutId}`;
      const archiveSelected = secondarySectionIsOpen(project.projectId, "archivedFiles");
      // Keep this file-mode entry beside the existing special nodes, outside file ordering overrides.
      const specialEnd = ordered.reduce((end, item, index) => item.meta.kind !== "workspaceFile" ? index + 1 : end, 0);
      ordered.splice(specialEnd, 0, {
        key: archiveKey,
        treeRow: makeRow(archiveKey, t("development.archivedFiles"), resourceDepth, "folder", {
          selected: archiveSelected, classes: { "is-open": archiveSelected },
        }),
        meta: { kind: "archivedFilesRoot", projectId: project.projectId, checkoutId: checkout.checkoutId },
      });
      appendWorkspaceFileCreate(ordered, checkout.checkoutId, resourceDepth);
      items.push(...ordered);
    }
  }
  for (const item of pinnedItems) {
    if (item.treeRow) {
      item.treeRow.pinned = true;
      const dropActive = pinDropProjectId.value === item.meta.projectId;
      const insertion = dropActive ? pinDropInsertion.value : null;
      item.treeRow.classes = {
        ...item.treeRow.classes,
        "is-pin-drop-target": dropActive && !insertion,
        "is-pin-drop-before": insertion?.lineKey === item.key && insertion.side === "before",
        "is-pin-drop-after": insertion?.lineKey === item.key && insertion.side === "after",
      };
    }
  }
  const lastPinnedRow = [...pinnedItems].reverse().find((item) => item.treeRow)?.treeRow;
  if (lastPinnedRow) lastPinnedRow.pinnedSectionEnd = true;
  return [...pinnedItems, ...items].map((item) => {
    const node = item.meta.explorerNode;
    if (item.treeRow && node) {
      item.treeRow.starred = workspaceTreeItemState(
        renderedExplorerSnapshot(item.meta.projectId), node.nodeId, item.meta.mountEntry?.relativePath,
      )?.highlighted;
    }
    return item;
  });
}

const treeItems = computed(() => withFlashingTreeRows(
  buildWorkspaceTreeItems(visibleProjects.value, workspaceDisplayMode.value === "multi"),
));

/** The tree rows of the nodes shown in the top bar, built as the tree would build them. */
const topBarTreeItems = computed(() => {
  const result = new Map<TopBarNodeId, DevelopmentTreeItem>();
  const project = topBarPlacedNodeIds.value.size > 0 ? visibleProjects.value[0] : undefined;
  const snapshot = project ? renderedExplorerSnapshot(project.projectId) : undefined;
  if (!project || !snapshot) return result;
  const checkout = navigationCheckout(project.projectId);
  const filesMode = workspaceNavigation.getMode(checkout?.checkoutId) === "files";
  includeTopBarNodes = true;
  try {
    for (const id of topBarPlacedNodeIds.value) {
      const node = snapshot.nodes.find((candidate) => (
        candidate.resourceKind === SYSTEM_RESOURCE_KIND && candidate.resourceId === id
      ));
      if (!node || !isTopBarNodeId(id)) continue;
      const items: DevelopmentTreeItem[] = [];
      appendLayoutChildren(items, project, null, 0, new Map(), new Map(), node.nodeId);
      const item = items.find((candidate) => candidate.meta.explorerNode?.nodeId === node.nodeId);
      if (!item) continue;
      if (filesMode && checkout) applyFilesModeNavigation(item, checkout, 0);
      result.set(id, item);
    }
  } finally {
    includeTopBarNodes = false;
  }
  return result;
});

const topBarNodeEntries = computed(() => [...topBarTreeItems.value].map(([id, item]) => ({
  id,
  label: item.treeRow?.name ?? id,
  icon: itemIcon(item),
  active: !!item.treeRow?.classes?.["is-open"],
})));

function topBarSystemNode(id: TopBarNodeId): ProjectExplorerNode | undefined {
  return explorerStore.snapshots[presetProjectId.value]?.nodes.find((node) => (
    node.resourceKind === SYSTEM_RESOURCE_KIND && node.resourceId === id
  ));
}

/** Moves a node between the tree and the top bar; a node placed in the bar is shown again. */
function setTopBarNodePlacement(id: TopBarNodeId, inTopBar: boolean, before?: TopBarEntryId | null): void {
  const current = settingValue(WORKSPACE_TOP_BAR_NODES_SETTING);
  const next = inTopBar ? placeTopBarNode(current, id, before) : removeTopBarNode(current, id);
  if (!sameTopBarLayout(current, next)) saveSetting(WORKSPACE_TOP_BAR_NODES_SETTING, next);
  const node = inTopBar ? topBarSystemNode(id) : undefined;
  if (node?.hidden && presetProjectId.value) {
    void explorerStore.applyOperations(presetProjectId.value, [{ kind: "setNodeHidden", nodeId: node.nodeId, hidden: false }])
      .catch((error) => notificationStore.addNotice("error", normalizeAppError(error).message));
  }
}

function returnTopBarNodesToTree(items: readonly DevelopmentTreeItem[]): void {
  for (const item of items) {
    const id = item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND ? item.meta.explorerNode.resourceId : null;
    if (isTopBarNodeId(id) && topBarPlacedNodeIds.value.has(id)) setTopBarNodePlacement(id, false);
  }
}

/** A top bar click opens the node's page or list; `toggle` closes it when it is already open. */
async function activateTopBarNode(id: string, toggle = true): Promise<void> {
  const item = isTopBarNodeId(id) ? topBarTreeItems.value.get(id) : undefined;
  const project = item ? workspaceContextStore.projectsById[item.meta.projectId] : undefined;
  if (!item || !project) return;
  resetSessionMultiSelection();
  try {
    const secondary = workspaceSecondarySection(item.meta.kind);
    if (secondary) {
      if (toggle || !secondarySectionIsOpen(project.projectId, secondary)) await toggleSecondaryNavigation(item, secondary);
      return;
    }
    const overlay = workspaceOverlaySection(item.meta.kind);
    if (overlay && (toggle || !workspaceOverlayIsOpen(project.projectId, overlay))) {
      await activateWorkspaceOverlayItem(item, overlay, project);
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

/** Closes the page or list a top bar node opened, returning to the editor groups. */
function closeTopBarNodePages(): void {
  for (const [id, item] of topBarTreeItems.value) {
    if (!topBarNodeEntries.value.find((entry) => entry.id === id)?.active) continue;
    if (workspaceSecondarySection(item.meta.kind)) closeSecondaryNavigation();
    else if (workspaceOverlaySection(item.meta.kind)) closeWorkspaceOverlay();
  }
}

/** Dragging a top bar node can reorder it, return it to the tree or open its page in a tab group. */
function startTopBarNodeDrag(id: string, event: PointerEvent): void {
  const item = isTopBarNodeId(id) ? topBarTreeItems.value.get(id) : undefined;
  if (item) onDragPointerDown(item, event);
}

function topBarAcceptsTreeItem(item: DevelopmentTreeItem): boolean {
  const node = item.meta.explorerNode;
  return topBarNodesEnabled.value && node?.resourceKind === SYSTEM_RESOURCE_KIND && isTopBarNodeId(node.resourceId);
}

function topBarDragNodeId(source: InternalDragSource): TopBarNodeId | null {
  if (!topBarNodesEnabled.value || source.payload.type !== WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) return null;
  const data = source.payload.data as WorkspaceLayoutInternalDragData;
  if ((data.items?.length ?? 1) !== 1) return null;
  const node = data.item.meta.explorerNode;
  if (node?.resourceKind !== SYSTEM_RESOURCE_KIND || !isTopBarNodeId(node.resourceId)) return null;
  return data.item.meta.projectId === visibleProjects.value[0]?.projectId ? node.resourceId : null;
}

/** Insertion point shown in the top bar: `null` after the last tab, `undefined` when not dropping there. */
const topBarDropBefore = ref<TopBarEntryId | null | undefined>(undefined);

const topBarInternalDropTarget: InternalDropTargetRegistration<
  WorkspaceLayoutInternalDragData,
  { node: TopBarNodeId; before: TopBarEntryId | null }
> = {
  id: `workspace-top-bar:${WORKBENCH_WINDOW_ID}`,
  root: () => (topBarNodesEnabled.value ? props.topBarTarget ?? null : null),
  accepts: (source) => topBarDragNodeId(source) !== null,
  resolve: ({ source, point }) => {
    const node = topBarDragNodeId(source);
    const root = props.topBarTarget;
    if (!node || !root) return null;
    const tabs = [...root.querySelectorAll<HTMLElement>("[data-top-bar-entry]")].flatMap((element) => {
      const id = element.dataset.topBarEntry;
      if (!isTopBarPageId(id) && !isTopBarNodeId(id)) return [];
      const bounds = element.getBoundingClientRect();
      return [{ id, left: bounds.left, right: bounds.right }];
    });
    const before = topBarInsertionBefore(point.x, tabs);
    return { key: `top-bar:${node}:${before ?? "end"}`, operation: "move", intent: { node, before } };
  },
  onTargetChange: (decision) => {
    topBarDropBefore.value = decision ? decision.intent.before : undefined;
  },
  drop: ({ decision }) => setTopBarNodePlacement(decision.intent.node, true, decision.intent.before),
};

defineExpose({
  topBarNodeEntries,
  topBarDropBefore,
  activateTopBarNode,
  closeTopBarNodePages,
  startTopBarNodeDrag,
  moveTopBarNodeToTree: (id: string) => {
    if (isTopBarNodeId(id)) setTopBarNodePlacement(id, false);
  },
});
const secondarySessionItems = computed(() => {
  const target = secondaryNavigation.value;
  const project = target?.section === "sessions" ? workspaceContextStore.projectsById[target.projectId] : null;
  if (!project) return [];
  const nodes = explorerStore.snapshots[project.projectId]?.nodes ?? [];
  const sessionFolders = new Set<string>();
  for (const node of nodes) {
    if (node.resourceKind !== "session") continue;
    let parent = node.parentNodeId;
    while (parent && !sessionFolders.has(parent)) {
      sessionFolders.add(parent);
      parent = nodes.find(candidate => candidate.nodeId === parent)?.parentNodeId ?? null;
    }
  }
  const items = buildWorkspaceTreeItems([project], false, false).filter(item => item.meta.kind === "session"
    || item.meta.kind === "inlineCreate" || item.meta.kind === "dropPreview"
    || (item.meta.kind === "folder" && ((!item.meta.explorerNode?.sourcePath
      && !knowledgeFolderKind(item.meta.explorerNode?.nodeId ?? ""))
      || sessionFolders.has(item.meta.explorerNode?.nodeId ?? ""))));
  for (const item of items) if (item.treeRow) item.treeRow.pinnedSectionEnd = false;
  const lastPin = [...items].reverse().find(item => item.treeRow?.pinned)?.treeRow;
  if (lastPin) lastPin.pinnedSectionEnd = true;
  return items;
});

watch(() => treeItems.value.slice(treeVisibleRange.value.start, treeVisibleRange.value.end + 1), items => {
  for (const item of items) {
    const entry = item.meta.workspaceFile;
    if (entry?.loadMore && entry.node.kind === "folder" && !entry.node.loading && entry.node.hasMore) {
      workspaceFileStates[item.meta.checkoutId!]?.loadMore(entry.node.path);
    }
  }
}, { flush: "post" });

function treeFileTimeTarget(item: DevelopmentTreeItem): WorkspaceFileTimeTarget | null {
  const path = item.meta.kind === "localFile" ? item.meta.explorerNode?.sourcePath
    : item.meta.kind === "mountedFile" ? item.meta.mountEntry?.absolutePath : null;
  return path ? { projectId: item.meta.projectId, path } : null;
}

const treeFileTimes = useWorkspaceFileModifiedTimes(() => props.showExplorer && !props.prewarm
  ? treeItems.value.slice(treeVisibleRange.value.start, treeVisibleRange.value.end + 1)
    .flatMap((item) => { const target = treeFileTimeTarget(item); return target ? [target] : []; })
  : [], () => treeTimeNow.value, ownerWindow,
// The tree shows the materialized checkouts of every visible project.
() => props.showExplorer && !props.prewarm
  ? visibleProjects.value.flatMap((project) => project.checkouts
    .filter((checkout) => checkout.runtime).map(checkoutWorkspaceRef))
  : []);

const sessionPromotionRequests = new Set<string>();
watch([
  () => sessionAttention.value,
  () => visibleProjects.value.map((project) => `${project.projectId}:${explorerStore.snapshots[project.projectId]?.revision}:${explorerStore.snapshots[project.projectId]?.presetId}`).join("|"),
  sessionTreeInteractionActive,
  () => displaySettings.autoPromoteCompletedSessions,
  () => props.showExplorer && !props.prewarm,
], () => {
  if (sessionTreeInteractionActive.value || !props.showExplorer || props.prewarm) return;
  for (const project of visibleProjects.value) {
    if (sessionPromotionRequests.has(project.projectId)) continue;
    sessionPromotionRequests.add(project.projectId);
    void explorerStore.promoteCompletedSessions(project.projectId, () => !sessionTreeInteractionActive.value)
      .catch((error) => console.warn("[DevelopmentWorkbench] session promotion failed", error))
      .finally(() => sessionPromotionRequests.delete(project.projectId));
  }
}, { immediate: true });

function releaseTreePointer(): void { treePointerPressed.value = false; }
onMounted(() => {
  releaseWorkStateProvider = registerWorkStateProvider(WORKBENCH_WINDOW_ID, captureWorkbenchWorkState);
  scheduleWorkStateCapture();
  ownerWindow.addEventListener("pointerup", releaseTreePointer, true);
  ownerWindow.addEventListener("pointercancel", releaseTreePointer, true);
  ownerWindow.addEventListener("blur", releaseTreePointer);
});
onUnmounted(() => {
  ownerWindow.removeEventListener("pointerup", releaseTreePointer, true);
  ownerWindow.removeEventListener("pointercancel", releaseTreePointer, true);
  ownerWindow.removeEventListener("blur", releaseTreePointer);
});

function documentModifiedTime(item: DevelopmentTreeItem): string {
  if (item.meta.kind === "knowledge") {
    // Knowledge timestamps are milliseconds; session and file row times use seconds.
    const modifiedAt = item.meta.knowledge?.modifiedAt;
    return treeModifiedTime(modifiedAt == null ? undefined : modifiedAt / 1000);
  }
  const target = treeFileTimeTarget(item);
  return target ? treeModifiedTime(treeFileTimes.modifiedAt(target)) : "";
}

watch(() => visibleProjects.value.flatMap((project) => {
  const snapshot = explorerStore.snapshots[project.projectId];
  return [...new Set((snapshot?.itemStates ?? [])
    .filter((state) => state.pinned && state.relativePath)
    .map((state) => state.nodeId))].map((nodeId) => ({ projectId: project.projectId, nodeId, presetId: snapshot?.presetId }));
}), (mounts) => {
  for (const { projectId, nodeId } of mounts) {
    void explorerStore.loadMount(projectId, nodeId).catch((error) => {
      notificationStore.addNotice("error", normalizeAppError(error).message);
    });
  }
}, { immediate: true });

function visibleWorkspaceSessionTargets(archived = false): DevelopmentSessionTarget[] {
  return (archived ? archivedTreeItems.value : secondaryNavigation.value?.section === "sessions" ? secondarySessionItems.value : treeItems.value).flatMap((item) => (
    item.meta.kind === "session" && item.meta.session
      ? [{ item, projectId: item.meta.projectId, session: item.meta.session }]
      : []
  ));
}

function activeWorkspaceSessionId(): string | null {
  const resource = activeResource.value;
  if (resource?.kind === "session" || (resource?.kind === "section" && resource.section === "archived")) return resource.sessionId ?? null;
  return chatStore.activeSessionId;
}

function resolveSessionClickSelection(item: DevelopmentTreeItem, event?: MouseEvent): boolean {
  const sessionId = item.meta.session?.id;
  if (!sessionId) return false;
  const visibleSessionIds = visibleWorkspaceSessionTargets(!!item.meta.archived).map((target) => target.session.id);
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

function visibleWorkspaceFileItems(checkoutId: string): DevelopmentTreeItem[] {
  return treeItems.value.filter(item => item.meta.kind === "workspaceFile" && item.meta.checkoutId === checkoutId
    && !!item.meta.workspaceFile && !item.meta.workspaceFile.loadMore);
}

function selectedWorkspaceFileItems(checkoutId: string): DevelopmentTreeItem[] {
  return visibleWorkspaceFileItems(checkoutId).filter(item => selectedFileKeys.value.has(item.key));
}

/** Document and file rows placed in the session layout, including entries of mounted folders. */
function isLayoutFileItem(item: DevelopmentTreeItem): boolean {
  return (item.meta.kind === "knowledge" && !!item.meta.knowledge)
    || (item.meta.kind === "localFile" && !!item.meta.explorerNode?.sourcePath)
    || (item.meta.kind === "mountedFile" && !!item.meta.mountEntry);
}

function isFileSelectionItem(item: DevelopmentTreeItem): boolean {
  return isLayoutFileItem(item) || (item.meta.kind === "workspaceFile" && !!item.meta.checkoutId
    && !!item.meta.workspaceFile && !item.meta.workspaceFile.loadMore && !item.meta.archivedFile);
}

/**
 * Rows sharing one Ctrl/Shift selection with `item`: Files-mode rows of its
 * checkout, or the document and file rows of its project in the layout tree.
 */
function fileSelectionGroupItems(item: DevelopmentTreeItem): DevelopmentTreeItem[] {
  if (item.meta.kind === "workspaceFile") return item.meta.checkoutId ? visibleWorkspaceFileItems(item.meta.checkoutId) : [];
  if (!isLayoutFileItem(item)) return [];
  return treeItems.value.filter(candidate => isLayoutFileItem(candidate) && candidate.meta.projectId === item.meta.projectId);
}

function resolveFileClickSelection(item: DevelopmentTreeItem, event?: MouseEvent): boolean {
  if (!isFileSelectionItem(item)) return true;
  const visible = fileSelectionGroupItems(item);
  const visibleKeys = visible.map(candidate => candidate.key);
  // The session resolver is id-agnostic: Ctrl toggles, Shift selects a visible
  // range, and a plain click clears the selection and opens the row.
  const result = resolveWorkspaceSessionSelection({
    visibleSessionIds: visibleKeys,
    selectedSessionIds: new Set(visibleKeys.filter(key => selectedFileKeys.value.has(key))),
    anchorSessionId: lastFileSelectionAnchorKey.value,
    activeSessionId: visible.find(candidate => candidate.treeRow?.classes?.["is-open"])?.key ?? null,
    clickedSessionId: item.key,
    shiftKey: event?.shiftKey ?? false,
    ctrlKey: event?.ctrlKey ?? false,
    metaKey: event?.metaKey ?? false,
  });
  selectedFileKeys.value = result.nextSelectedSessionIds;
  lastFileSelectionAnchorKey.value = result.nextAnchorSessionId;
  return result.shouldActivateSession;
}

function itemIcon(item: DevelopmentTreeItem) {
  switch (item.meta.kind) {
    case "project": {
      const project = workspaceContextStore.projectsById[item.meta.projectId];
      const projectIcon = projectIconForServices(project?.detectedServices ?? []);
      if (projectIcon) return projectIcon;
      return item.treeRow?.expanded ? ChevronDown : ChevronRight;
    }
    case "collaboration": return GitMerge;
    case "assetsRoot": return Folder;
    case "sessionsRoot": return MessageSquare;
    case "workspaceFile": return item.meta.workspaceFile?.node.kind === "folder"
      ? item.treeRow?.expanded ? ChevronDown : ChevronRight
      : unityAssetIconNodeForPath(item.meta.workspaceFile?.node.path ?? "file", { isFolder: false });
    case "agentsRoot": return Bot;
    case "viewsRoot": return Eye;
    case "view": return resolveLocusViewIcon(item.meta.view?.view.icon);
    case "archivedRoot": return Archive;
    case "archivedFilesRoot": return Archive;
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
    case "mountedFolder": return item.treeRow?.expanded ? ChevronDown : ChevronRight;
    default:
      if (item.meta.explorerNode && knowledgeFolderKind(item.meta.explorerNode.nodeId)) {
        return BookOpen;
      }
      if (item.treeRow?.kind === "package") return Package;
      return item.treeRow?.expanded ? ChevronDown : ChevronRight;
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
        : item.meta.kind === "workspaceFile" && item.meta.workspaceFile?.node.kind === "file"
          ? item.meta.workspaceFile.node.path
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
    && source.payload.type !== VIEW_TREE_INTERNAL_DRAG_TYPE
  )) return null;
  return workspaceDragPreviewForInternalSource(source);
});

const layoutDragPreview = computed<WorkspaceDragPreview | null>(() => (
  internalLayoutDragPreview.value
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
    ? `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}:${workspaceRef.expectedMaterializationEpoch ?? "empty"}`
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
    agentStore.loadWorkspaceAgents(workspaceRef).then(() => {
      if (workspaceRefScopeKey(workspaceContextStore.focusedWorkspaceRef) === scopeKey) {
        return chatStore.refreshSessions();
      }
    }),
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
  if (workspaceDisplayMode.value !== "single") return;
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

const workspaceWindowTitleBar = computed(() => settingValue(WORKSPACE_WINDOW_TITLE_BAR_SETTING) === true);

/** The toolbar moves between the title bar and the workspace, so the menu's anchor goes away. */
function toggleWorkspaceWindowTitleBar(): void {
  displayMenu.value = null;
  saveSetting(WORKSPACE_WINDOW_TITLE_BAR_SETTING, !workspaceWindowTitleBar.value);
}

/** Windows without the top tab bar reach the app-level pages from the menu. */
async function openAppPageWindow(page: "plugins" | "settings"): Promise<void> {
  displayMenu.value = null;
  try {
    await openWorkspacePageWindow({
      scope: "app",
      page,
      title: t(page === "plugins" ? "app.tab.plugins" : "app.tab.settings"),
    });
  } catch (cause) {
    const error = normalizeAppError(cause);
    notificationStore.addNotice("error", error.message, {
      code: error.code,
      operation: "openWorkspacePageWindow",
      skipConsoleLog: true,
    });
  }
}

function openSpecialNodesMenu(event: MouseEvent | FocusEvent, kind: "visibility" | "topBar" = "visibility"): void {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  specialNodesMenu.value = { x: rect.right + 2, y: rect.top, kind };
}

const topBarMenuItems = computed(() => specialNodeVisibilityItems.value.flatMap((item) => (
  isTopBarNodeId(item.resourceId) ? [{ ...item, id: item.resourceId, inTopBar: topBarPlacedNodeIds.value.has(item.resourceId) }] : []
)));

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
      ...(workspaceDisplayMode.value === "multi" ? [`project:${projectId}`] : []),
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

function openRecentWorkspaceContextMenu(path: string, event: MouseEvent): void {
  recentWorkspaceContextMenu.value = { x: event.clientX, y: event.clientY, path };
}

async function configureRecentWorkspace(): Promise<void> {
  const path = recentWorkspaceContextMenu.value?.path;
  recentWorkspaceContextMenu.value = null;
  workspaceMenu.value = null;
  if (path) await configureWorkspace(path);
}

async function openRecentWorkspaceInNewWindow(): Promise<void> {
  const path = recentWorkspaceContextMenu.value?.path;
  if (!path) return;
  recentWorkspaceContextMenu.value = null;
  workspaceMenu.value = null;
  try {
    await openWorkspaceWindow(path);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function removeRecentWorkspace(): Promise<void> {
  const target = recentWorkspaceContextMenu.value;
  recentWorkspaceContextMenu.value = null;
  if (!target) return;
  try {
    await projectStore.removeRecentDir(target.path);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function removeContextWorkspace(): Promise<void> {
  const item = contextMenu.value?.item;
  const allowed = contextCanManageWorkspaces.value;
  contextMenu.value = null;
  // Empty-area menus target the active project implicitly; only a project row names one.
  if (!allowed || item?.meta.kind !== "project" || !item.treeRow) return;
  const project = workspaceContextStore.projectsById[item.meta.projectId];
  if (!project) return;
  const approved = await confirm(t("development.removeWorkspaceConfirm", projectLabel(project)), {
    title: t("development.removeWorkspace"),
    kind: "warning",
    okLabel: t("development.removeWorkspaceAction"),
    cancelLabel: t("common.cancel"),
  });
  if (!approved) return;
  // Open tabs stay usable. Only a pane focused on the project without showing
  // one of its editors moves to another workspace.
  const refocus = workspaceContextStore.focusedCheckout?.projectId === project.projectId
    && workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.resource.projectId !== project.projectId;
  try {
    if (!await workspaceContextBaseStore.removeProject(project.projectId)) return;
    await projectStore.loadRecentDirs();
    const fallback = refocus
      ? workspaceContextStore.projects.flatMap((candidate) => candidate.checkouts)
        .find((checkout) => checkout.available !== false)
      : null;
    if (fallback && await workspaceContextStore.focusCheckout(fallback.checkoutId)) {
      await refreshFocusedCheckoutServices();
    }
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
  if (workspaceDisplayMode.value === "multi") {
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
  if (item.meta.kind !== "workspaceFile" && !isLayoutFileItem(item)) resetFileMultiSelection();
  try {
    if (item.meta.kind === "project") {
      toggleItem(item);
      return;
    }
    const pinEditor = (event?.detail ?? 1) >= 2;
    explorerStore.selectedNodeKey = item.key;
    if (item.meta.kind === "workspaceFile" && item.meta.workspaceFile) {
      resetSessionMultiSelection();
      if (!resolveFileClickSelection(item, event)) return;
      const { node, loadMore } = item.meta.workspaceFile;
      const state = workspaceFileStates[item.meta.checkoutId!];
      if (loadMore) state?.loadMore(node.path);
      else if (node.kind === "folder") state?.toggle(node.path);
      else await openWorkbenchResourceFromWorkspaceTree({
        resource: { kind: "asset", projectId: project.projectId, path: node.path },
        title: node.name, checkoutId: item.meta.checkoutId,
      }, { preview: !pinEditor, pinned: pinEditor });
      return;
    }
    if (item.meta.kind === "view" && item.meta.explorerNode?.resourceId) {
      resetSessionMultiSelection();
      const checkout = projectCheckout(project.projectId, item.meta.checkoutId);
      if (!checkout) return;
      const runtime = checkout.runtime ?? await workspaceContextBaseStore.openCheckout(checkout.root);
      const workspaceRef = item.meta.view
        ? workspaceRefForEditorBinding(item.meta.view.workspaceRef, runtime)!
        : {
        checkoutId: checkout.checkoutId,
        expectedGeneration: runtime.workspaceGeneration,
        expectedMaterializationEpoch: runtime.materializationEpoch,
        };
      await openViewPackage(workspaceRef, item.meta.explorerNode.resourceId, WORKBENCH_WINDOW_ID);
      return;
    }
    if (item.meta.kind === "folder") {
      resetSessionMultiSelection();
      if (item.treeRow?.expandable) toggleItem(item);
      if (item.meta.explorerNode?.sourcePath) {
        await explorerStore.loadMount(project.projectId, item.meta.explorerNode.nodeId);
      }
      return;
    }
    if (item.meta.kind === "mountedFolder") {
      toggleItem(item);
      resetSessionMultiSelection();
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
    const secondarySection = workspaceSecondarySection(item.meta.kind);
    if (secondarySection) {
      resetSessionMultiSelection();
      await toggleSecondaryNavigation(item, secondarySection);
      return;
    }
    const overlaySection = workspaceOverlaySection(item.meta.kind);
    if (overlaySection) {
      resetSessionMultiSelection();
      // The second click of a double-click must not close the page the first one opened.
      if ((event?.detail ?? 1) > 1) return;
      await activateWorkspaceOverlayItem(item, overlaySection, project);
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
      if (!resolveFileClickSelection(item, event)) return;
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
      if (!resolveFileClickSelection(item, event)) return;
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

async function activateWorkspaceOverlayItem(
  item: DevelopmentTreeItem,
  section: WorkspaceOverlaySection,
  project: ProjectContextDescriptor,
): Promise<void> {
  if (workspaceOverlayIsOpen(project.projectId, section)) {
    closeWorkspaceOverlay();
    return;
  }
  const focused = workspaceContextStore.focusedCheckout;
  const checkout = projectCheckout(
    project.projectId,
    focused?.projectId === project.projectId ? focused.checkoutId : item.meta.checkoutId,
  );
  if (checkout) await showWorkspaceOverlay(section, project.projectId, checkout);
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
  archivedRefreshKey.value += 1;
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (editor.resource.kind === "section" && editor.resource.section === "archived" && editor.resource.sessionId === draft.sessionId) {
        workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, { title });
      }
    }
  }
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
      if (!(editor.resource.kind === "session" || (editor.resource.kind === "section" && editor.resource.section === "archived")) || editor.resource.sessionId !== sessionId) continue;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        availability: "unavailable",
        unavailableReason: t("workbench.unavailable.session"),
      });
    }
  }
}

async function archiveSessionEntry(target: DevelopmentSessionTarget): Promise<void> {
  if (target.item.meta.archived) {
    await restoreArchivedSession(target.session.id, target.projectId);
    return;
  }
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
  archivedRefreshKey.value += 1;
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

function canSetTreeItemState(item: DevelopmentTreeItem): boolean {
  return !!item.meta.explorerNode && item.meta.explorerNode.resourceKind !== SYSTEM_RESOURCE_KIND;
}

function contextTreeStateItems(): DevelopmentTreeItem[] {
  const menu = contextMenu.value;
  if (!menu) return [];
  return (menu.sessionTargets?.length ? menu.sessionTargets.map((target) => target.item) : menu.fileTargets ?? [menu.item])
    .filter(canSetTreeItemState);
}

function contextTreeStateEnabled(field: "pinned" | "starred"): boolean {
  const items = contextTreeStateItems();
  const storedField = field === "starred" ? "highlighted" : field;
  return items.length > 0 && items.every((item) => workspaceTreeItemState(
    explorerStore.snapshots[item.meta.projectId], item.meta.explorerNode!.nodeId, item.meta.mountEntry?.relativePath,
  )?.[storedField]);
}

async function setTreeItemsState(items: DevelopmentTreeItem[], field: "pinned" | "starred", value: boolean): Promise<void> {
  await ensureArchivedDropPlacements(items);
  // Keep the existing preset flag so previously highlighted items remain marked.
  const storedField = field === "starred" ? "highlighted" : field;
  const byProject = new Map<string, ProjectExplorerOperation[]>();
  for (const item of items.filter(canSetTreeItemState)) {
    const operations = byProject.get(item.meta.projectId) ?? [];
    operations.push({ kind: "setItemState", nodeId: item.meta.explorerNode!.nodeId,
      relativePath: item.meta.mountEntry?.relativePath, [storedField]: value });
    byProject.set(item.meta.projectId, operations);
  }
  for (const [projectId, operations] of byProject) {
    await explorerStore.applyOperations(projectId, operations);
  }
}

async function toggleContextTreeState(field: "pinned" | "starred"): Promise<void> {
  const items = contextTreeStateItems();
  const value = !contextTreeStateEnabled(field);
  contextMenu.value = null;
  try {
    await setTreeItemsState(items, field, value);
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function openContextMenu(raw: WorkspaceTreeItem, event: MouseEvent): void {
  const item = raw as DevelopmentTreeItem;
  if (!(item.meta.kind === "project"
    || item.meta.kind === "newSession"
    || item.meta.kind === "folder"
    || item.meta.kind === "checkout"
    || item.meta.kind === "collaboration"
    || item.meta.kind === "session"
    || item.meta.kind === "knowledge"
    || item.meta.kind === "localFile"
    || item.meta.kind === "mountedFile"
    || item.meta.kind === "mountedFolder"
    || (item.meta.kind === "workspaceFile" && item.meta.workspaceFile && !item.meta.workspaceFile.loadMore)
    || item.meta.kind === "view")) return;
  event.preventDefault();
  if (isFileSelectionItem(item)) {
    resetSessionMultiSelection();
    const visible = fileSelectionGroupItems(item);
    const targetKeys = resolveWorkspaceSessionContextIds({
      visibleSessionIds: visible.map(candidate => candidate.key),
      selectedSessionIds: selectedFileKeys.value,
      targetSessionId: item.key,
    });
    if (targetKeys.length === 1 && !selectedFileKeys.value.has(item.key)) clearFileMultiSelection();
    const targetKeySet = new Set(targetKeys);
    contextMenu.value = { x: event.clientX, y: event.clientY, item,
      fileTargets: targetKeys.length > 1 ? visible.filter(candidate => targetKeySet.has(candidate.key)) : undefined };
    return;
  }
  resetFileMultiSelection();
  if (item.meta.kind === "session" && item.meta.session) {
    const visibleTargets = visibleWorkspaceSessionTargets(!!item.meta.archived);
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

function openExplorerBackgroundContextMenu(event: MouseEvent): void {
  const target = event.target as HTMLElement | null;
  if (!target?.closest(".development-tree") || target.closest("[data-tree-key], .development-inline-create-row")) return;
  const projectId = activeResource.value?.projectId
    ?? workspaceContextStore.focusedProject?.projectId
    ?? visibleProjects.value[0]?.projectId;
  if (!projectId) return;
  event.preventDefault();
  resetSessionMultiSelection();
  resetFileMultiSelection();
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

function openArchivedFileContextMenu(entry: WorkspaceFileEntry, event: MouseEvent): void {
  const target = secondaryNavigation.value;
  if (!target || target.section !== "archivedFiles") return;
  openContextMenu({ ...entry, meta: { kind: "workspaceFile", projectId: target.projectId,
    checkoutId: target.checkoutId, workspaceFile: entry, archivedFile: true } } as DevelopmentTreeItem, event);
}

function setContextFileHidden(hidden: boolean): void {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (!item?.meta.checkoutId || !item.meta.workspaceFile || item.meta.workspaceFile.loadMore) return;
  try { workspaceFileVisibility.setHidden(item.meta.checkoutId, item.meta.workspaceFile.node.path, hidden); }
  catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
}

async function runContextFileBatchAction(action: "copy" | "delete"): Promise<void> {
  const items = contextMenu.value?.fileTargets ?? [];
  contextMenu.value = null;
  if (action === "delete") resetFileMultiSelection();
  try {
    const targets: ExplorerResourceTarget[] = [];
    for (const item of items) {
      const target = treeFileTarget(item);
      // Copying needs only paths; deleting a document goes through its checkout.
      const resolved = target && action === "delete" ? await withResourceWorkspace(item, target) : target;
      if (resolved) targets.push(resolved);
    }
    await resourceActions.value?.runMany(action, targets);
  } catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
}

/** Rows the batch Hide applies to; each one matches the row's own hide button. */
const contextHideFileTargets = computed(() => (contextMenu.value?.fileTargets ?? []).filter(item => (
  item.meta.kind === "workspaceFile" || item.meta.kind === "knowledge" || canHideWorkspaceFileItem(item)
)));

async function hideContextFiles(): Promise<void> {
  const items = contextHideFileTargets.value;
  contextMenu.value = null;
  resetFileMultiSelection();
  const paths = items.flatMap(item => item.meta.checkoutId && item.meta.workspaceFile
    ? [{ checkoutId: item.meta.checkoutId, path: item.meta.workspaceFile.node.path }] : []);
  // A descendant of another hidden folder is already hidden with it.
  const outermost = paths.filter(entry => !paths.some(parent => parent.checkoutId === entry.checkoutId
    && entry.path.startsWith(`${parent.path}/`)));
  try { for (const entry of outermost) workspaceFileVisibility.setHidden(entry.checkoutId, entry.path, true); }
  catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
  // Layout rows leave the workspace exactly as their own hide buttons do; each reports its own failure.
  for (const item of items) {
    if (item.meta.kind === "knowledge") await removeKnowledgeItemFromWorkspace(item);
    else if (item.meta.kind !== "workspaceFile") await hideWorkspaceFileItem(item);
  }
}

const contextCanRestoreFile = computed(() => {
  const item = contextMenu.value?.item;
  return !!item?.meta.archivedFile && !!item.meta.checkoutId && !!item.meta.workspaceFile
    && item.meta.workspaceFile.treeRow?.depth === 0
    && workspaceFileVisibility.paths(item.meta.checkoutId).includes(item.meta.workspaceFile.node.path);
});

function appendWorkspaceFileCreate(items: DevelopmentTreeItem[], checkoutId: string, rootDepth: number): void {
  const draft = inlineCreate.value;
  if (draft?.fileTarget?.workspaceRef.checkoutId !== checkoutId) return;
  const parentIndex = items.findIndex(item => item.meta.workspaceFile?.node.path === draft.fileTarget!.directory
    && item.meta.workspaceFile.node.kind === "folder" && !item.meta.workspaceFile.loadMore);
  const parent = items[parentIndex];
  if (draft.fileTarget.directory !== "." && !parent?.treeRow?.expanded) return;
  items.splice(parent ? parentIndex + 1 : items.length, 0, {
    key: `inline-create:${checkoutId}:${draft.fileTarget.directory}`,
    treeRow: null,
    meta: { kind: "inlineCreate", projectId: draft.projectId, inlineCreate: draft,
      inlineCreateDepth: parent ? parent.treeRow!.depth + 1 : rootDepth },
  });
}

function treeFileCreateTarget(item: DevelopmentTreeItem): WorkspaceFileDropScope | null {
  if (item.meta.archivedFile) return null;
  if (item.meta.kind !== "project" && item.meta.kind !== "workspaceFile") return null;
  if (item.meta.workspaceFile?.loadMore) return null;
  const checkout = item.meta.checkoutId
    ? projectCheckout(item.meta.projectId, item.meta.checkoutId) : navigationCheckout(item.meta.projectId);
  if (!checkout?.runtime || workspaceNavigation.getMode(checkout.checkoutId) !== "files") return null;
  const node = item.meta.workspaceFile?.node;
  return { projectId: item.meta.projectId, root: checkout.root, workspaceRef: checkoutWorkspaceRef(checkout),
    directory: node ? node.kind === "folder" ? node.path : workspaceFileParent(node.path) : "." };
}

function contextCheckout(): WorkspaceCheckoutDescriptor | null {
  const item = contextMenu.value?.item;
  // The collaboration row targets the checkout its page opens.
  if ((item?.meta.kind !== "checkout" && item?.meta.kind !== "collaboration") || !item.meta.checkoutId) return null;
  return workspaceContextStore.checkoutsById[item.meta.checkoutId] ?? null;
}

function checkoutWorkspaceRef(checkout: WorkspaceCheckoutDescriptor): WorkspaceRef {
  return {
    checkoutId: checkout.checkoutId,
    expectedGeneration: checkout.runtime?.workspaceGeneration,
    expectedMaterializationEpoch: checkout.runtime?.materializationEpoch ?? 0,
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

async function selectContextKnowledgeInList(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  const document = item?.meta.kind === "knowledge" ? item.meta.knowledge : null;
  if (!item || !document) return;
  try {
    await showSecondaryNavigation(item, "knowledge", {
      preferredCheckoutId: document.sourceCheckoutId,
      knowledgeSelection: document,
    });
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

function treeFileTarget(item: DevelopmentTreeItem): ExplorerResourceTarget | null {
  if (item.meta.kind === "workspaceFile" && item.meta.workspaceFile && !item.meta.workspaceFile.loadMore) {
    const checkout = projectCheckout(item.meta.projectId, item.meta.checkoutId);
    return checkout ? { projectId: item.meta.projectId, root: checkout.root,
      path: explorerFilePath(checkout.root, item.meta.workspaceFile.node.path),
      isDirectory: item.meta.workspaceFile.node.kind === "folder",
      workspaceRef: checkout.runtime ? checkoutWorkspaceRef(checkout) : null } : null;
  }
  const document = item.meta.kind === "knowledge" ? item.meta.knowledge : undefined;
  const path = document?.path ?? (item.meta.kind === "localFile" ? item.meta.explorerNode?.sourcePath
    : item.meta.kind === "mountedFile" ? item.meta.mountEntry?.absolutePath : null);
  if (!path) return null;
  const checkout = projectCheckout(item.meta.projectId, document?.sourceCheckoutId ?? item.meta.checkoutId);
  const root = document?.sourceRoot ?? checkout?.root ?? "";
  return {
    projectId: item.meta.projectId, root, path, document,
    mounted: !document && !explorerFileKey(path).startsWith(`${explorerFileKey(root).replace(/\/$/, "")}/`),
    workspaceRef: checkout?.runtime ? checkoutWorkspaceRef(checkout) : null,
  };
}

function treeItemDisplayName(item: DevelopmentTreeItem): string {
  const target = treeFileTarget(item);
  return target && showsFullPath(resourcePathKey(target))
    ? target.document ? knowledgeResourcePath(target.document) : target.path
    : item.treeRow?.name ?? "";
}

async function runContextResourceAction(action: ExplorerResourceAction): Promise<void> {
  const item = contextMenu.value?.item;
  const target = contextFileTarget.value;
  if (!item || !target) return;
  if (action === "reveal" && item.meta.kind !== "workspaceFile") { await revealContextFileInFileSystem(); return; }
  contextMenu.value = null;
  try {
    const resolved = await withResourceWorkspace(item, target);
    if (resolved) await resourceActions.value?.run(action, resolved, item.key);
  } catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
}

/** Checkout-scoped actions need an open runtime; a tree row may outlive it. Mounted paths act directly. */
async function withResourceWorkspace(item: DevelopmentTreeItem, target: ExplorerResourceTarget): Promise<ExplorerResourceTarget | null> {
  if (target.mounted || target.workspaceRef) return target;
  const checkout = projectCheckout(item.meta.projectId, item.meta.knowledge?.sourceCheckoutId ?? item.meta.checkoutId);
  if (!checkout) return null;
  const runtime = await workspaceContextBaseStore.openCheckout(checkout.root);
  return { ...target, workspaceRef: { checkoutId: runtime.checkoutId, expectedGeneration: runtime.workspaceGeneration,
    expectedMaterializationEpoch: runtime.materializationEpoch } };
}

function resourceFileChanged(target: ExplorerResourceTarget, newPath: string | null): void {
  const files = target.workspaceRef && workspaceFileStates[target.workspaceRef.checkoutId];
  if (files) void (async () => {
    await files.refresh(resourceRelativePath(target), "delete");
    if (newPath) await files.refresh(resourceRelativePath({ ...target, path: newPath }), "upsert");
  })().catch(error => notificationStore.addNotice("error", normalizeAppError(error).message));
  if (!target.document && newPath) movePathPreference(target.path, newPath);
  for (const group of Object.values(workbenchWindow.value.groups)) {
    for (const editor of group.tabs) {
      if (editor.resource.projectId !== target.projectId) continue;
      const resource = editor.resource;
      const editorFilePath = resource.kind === "localFile" || resource.kind === "localDirectory" ? editor.sourcePath
        : resource.kind === "workspaceFile" || resource.kind === "asset"
          ? explorerFilePath(workspaceContextStore.checkoutsById[editor.checkoutBinding?.checkoutId ?? ""]?.root ?? target.root, resource.path) : null;
      const movedPath = editorFilePath && newPath ? workspaceMovedPath(editorFilePath, target.path, newPath, !!target.isDirectory) : null;
      const insideDeletedFolder = !newPath && !!target.isDirectory && !!editorFilePath
        && workspaceMovedPath(editorFilePath, target.path, target.path, true) !== null;
      const matches = target.document
        ? resource.kind === "knowledge" && resource.documentId === target.document.id
        : !!movedPath || insideDeletedFolder || (resource.kind === "localFile" && !!editor.sourcePath && explorerFileKey(editor.sourcePath) === explorerFileKey(target.path))
          || ((resource.kind === "workspaceFile" || resource.kind === "asset") && explorerFileKey(explorerFilePath(
            workspaceContextStore.checkoutsById[editor.checkoutBinding?.checkoutId ?? ""]?.root ?? target.root, resource.path,
          )) === explorerFileKey(target.path));
      if (!matches) continue;
      if (!newPath) {
        if (!editor.dirty) workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, { availability: "unavailable" });
        continue;
      }
      const fileEditor = workspaceFileEditorRefs.get(editor.editorId);
      const draftSnapshot = editor.dirty && !fileEditor?.isRenaming() ? fileEditor?.exportTransferSnapshot() : null;
      const nextPath = movedPath ?? newPath;
      const nextResource = resource.kind === "asset" || resource.kind === "workspaceFile"
        ? { ...resource, path: resourceRelativePath({ ...target, path: nextPath }) }
        : resource.kind === "localFile" && resource.relativePath
          ? { ...resource, relativePath: resource.relativePath.replace(/[^/\\]+$/, shortPath(nextPath)) } : resource;
      workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, group.paneId, editor.editorId, {
        resource: nextResource, title: shortPath(nextPath),
        ...(target.document ? {} : { sourcePath: nextPath }),
      });
      if (workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.editorId === editor.editorId) activeResource.value = nextResource;
      if (draftSnapshot) void applyWorkbenchEditorTransferSnapshot(editor.editorId, draftSnapshot).then((applied) => {
        if (!applied) notificationStore.addNotice("error", t("development.editor.transferConflict"));
      }).catch((error) => notificationStore.addNotice("error", normalizeAppError(error).message));
    }
  }
  if (target.document) {
    const key = `${target.projectId}:${target.document.id}`;
    if (secondaryDocuments.value[key] && newPath) secondaryDocuments.value[key] = { ...secondaryDocuments.value[key], path: newPath };
    if (!newPath) delete secondaryDocuments.value[key];
  }
}

async function revealContextFileInFileSystem(): Promise<void> {
  const item = contextMenu.value?.item;
  contextMenu.value = null;
  if (!item) return;
  const document = item.meta.kind === "knowledge" ? item.meta.knowledge : null;
  const filePath = item.meta.kind === "localFile"
    ? item.meta.explorerNode?.sourcePath
    : item.meta.kind === "mountedFile"
      ? item.meta.mountEntry?.absolutePath
      : null;
  if (!document && !filePath) return;

  const checkout = projectCheckout(
    item.meta.projectId,
    document?.sourceCheckoutId ?? item.meta.checkoutId,
  );
  if (!checkout) return;
  try {
    const runtime = checkout.runtime ?? await workspaceContextBaseStore.openCheckout(checkout.root);
    const workspaceRef: WorkspaceRef = {
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
      expectedMaterializationEpoch: runtime.materializationEpoch,
    };
    if (document) {
      await knowledgeRevealTarget({
        kind: "document",
        docType: document.type,
        path: document.path,
      }, workspaceRef);
    } else if (filePath) {
      await showInFolder(workspaceRef, filePath);
    }
  } catch (error) {
    notificationStore.addNotice("error", normalizeAppError(error).message);
  }
}

async function configureCheckoutWorkspace(): Promise<void> {
  const checkout = contextCheckout();
  contextMenu.value = null;
  if (checkout) await configureWorkspace(checkout.root);
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
  if (workspaceDisplayMode.value === "multi") {
    nextExpanded.add(`project:${item.meta.projectId}`);
  }
  if (item.meta.kind === "folder") nextExpanded.add(item.key);
  expanded.value = nextExpanded;
  void nextTick(() => {
    inlineCreateInput.value?.focus();
    inlineCreateInput.value?.select();
  });
}

async function beginCreateWorkspaceDocument(kind: WorkspaceInlineCreateState["kind"]): Promise<void> {
  const target = contextCreateTarget.value;
  contextMenu.value = null;
  if (!target) return;
  inlineCreate.value = { kind, projectId: target.projectId, parentNodeId: null, name: "", fileTarget: target };
  expanded.value = new Set([...expanded.value, `project:${target.projectId}`]);
  const state = workspaceFileStates[target.workspaceRef.checkoutId];
  const parent = state?.entries.find(entry => entry.node.path === target.directory && !entry.loadMore);
  if (parent && !parent.treeRow?.expanded) state?.toggle(target.directory);
  await nextTick();
  const index = treeItems.value.findIndex(item => item.meta.inlineCreate === inlineCreate.value);
  if (index >= 0) workspaceTreeRef.value?.scrollToIndex(index);
  await nextTick();
  inlineCreateInput.value?.focus();
}

function inlineCreateLabel(): string {
  return t(inlineCreate.value?.kind === "document" ? "knowledge.explorer.createDoc"
    : inlineCreate.value?.kind === "csv" ? "csv.new" : "development.newFolder");
}

function cancelInlineCreate(): void {
  inlineCreate.value = null;
}

async function submitInlineCreate(): Promise<void> {
  const draft = inlineCreate.value;
  const name = draft?.name.trim() ?? "";
  if (!draft || !name) return;
  if (draft.fileTarget && !validResourceName(name)) {
    notificationStore.addNotice("error", t("development.invalidFileName"));
    return;
  }
  inlineCreate.value = null;
  try {
    if (draft.fileTarget) {
      const target = draft.fileTarget;
      if (draft.kind === "folder") {
        const path = await workspaceCreateFolder(target.workspaceRef, target.directory, name);
        // Creation is committed: a refresh failure must not restore a draft that would create it twice.
        try { await workspaceFileStates[target.workspaceRef.checkoutId]?.refresh(path, "upsert"); }
        catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
        return;
      }
      const extension = draft.kind === "csv" ? ".csv" : ".md";
      const fileName = (draft.kind === "csv" ? /\.csv$/i : /\.(md|markdown)$/i).test(name) ? name : `${name}${extension}`;
      const path = await workspaceCreateDocument(target.workspaceRef, target.directory, fileName);
      // Creation is committed: a refresh/open failure must not restore a draft that would create it twice.
      try {
        await workspaceFileStates[target.workspaceRef.checkoutId]?.refresh(path, "upsert");
        await openWorkbenchResourceFromWorkspaceTree({
          resource: { kind: "asset", projectId: target.projectId, path }, title: fileName,
          checkoutId: target.workspaceRef.checkoutId,
        }, { preview: false, pinned: true });
      } catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
      return;
    }
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
  if (!item) return;
  await setWorkspaceNodeHidden(item, hidden);
}

async function setWorkspaceNodeHidden(item: DevelopmentTreeItem, hidden: boolean): Promise<void> {
  const node = item.meta.explorerNode;
  if (!node || node.resourceKind !== SYSTEM_RESOURCE_KIND) return;
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

function canHideWorkspaceFileItem(item: DevelopmentTreeItem): boolean {
  const node = item.meta.explorerNode;
  if (!node) return false;
  if (item.meta.kind === "localFile") return !!node.sourcePath;
  return item.meta.kind === "mountedFile" && !!workspaceTreeItemState(
    explorerStore.snapshots[item.meta.projectId], node.nodeId, item.meta.mountEntry?.relativePath,
  )?.pinned;
}

async function hideWorkspaceFileItem(item: DevelopmentTreeItem): Promise<void> {
  if (!canHideWorkspaceFileItem(item)) return;
  if (item.meta.kind === "mountedFile") {
    try {
      await setTreeItemsState([item], "pinned", false);
    } catch (error) {
      notificationStore.addNotice("error", normalizeAppError(error).message);
    }
    return;
  }
  await removeMountedNodeFromWorkspace(item);
}

async function removeMountedNodeFromWorkspace(item: DevelopmentTreeItem): Promise<void> {
  const node = item?.meta.explorerNode;
  if (!node || (!node.sourcePath && item.meta.kind !== "view")) return;
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
  return key ? [...treeItems.value, ...secondarySessionItems.value].find((candidate) => candidate.key === key) ?? null : null;
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
  const divider = pinnedDividerTarget(hit, y);
  if (divider) return divider.pin
    ? { key: `pin:${divider.item.meta.projectId}`, operation: "copy", intent: { kind: "pin", projectId: divider.item.meta.projectId } }
    : { key: divider.layout.targetKey, operation: "copy", intent: { kind: "layout", layout: divider.layout, target: null } };
  const rowHit = developmentTreeItemFromHit(hit);
  const target = rowHit?.item ?? nearestExternalDropTarget(hit);
  externalDropTarget.value = target;
  if (rowHit?.item.treeRow?.pinned) {
    return { key: `pin:${rowHit.item.meta.projectId}`, operation: "copy",
      intent: { kind: "pin", projectId: rowHit.item.meta.projectId } };
  }
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
    key: `layout:${intent.targetKey}:${intent.parentNodeId ?? "root"}:${intent.position}`,
    operation: "copy",
    intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
  } : null;
}

/** Element under a native drag position, when it belongs to this workbench. */
function nativeWorkbenchHit(x: number, y: number): Element | null {
  const hit = ownerDocument.elementFromPoint(x, y);
  return hit?.nodeType === 1 && workbenchRootRef.value?.contains(hit) ? hit : null;
}

function updateNativeWorkbenchDropTarget(x: number, y: number): void {
  // Local files over a Files folder are copied into it instead of referenced.
  if (locusFileWorkspaceDragActive.value && !unityAssetWorkspaceDragActive.value
    && nativeWorkspaceFileDrop.update(nativeWorkbenchHit(x, y))) {
    handleWorkbenchInternalTargetChange(null);
    externalDropTarget.value = null;
    return;
  }
  nativeWorkspaceFileDrop.clear();
  const decision = nativeWorkbenchDropDecisionAt(x, y);
  handleWorkbenchInternalTargetChange(decision);
  if (!decision) externalDropTarget.value = null;
}

function clearNativeWorkbenchDropTarget(): void {
  nativeWorkspaceFileDrop.clear();
  pinDropProjectId.value = null;
  externalDropTarget.value = null;
  layoutDropIntent.value = null;
  dropTargetKey.value = null;
  editorDropIntent.value = null;
  composerDropTarget.value = null;
}

function currentNativeWorkbenchDropIntent(): WorkbenchInternalDropIntent | null {
  if (pinDropProjectId.value) return { kind: "pin", projectId: pinDropProjectId.value };
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
    ?? (intent.kind === "pin" ? intent.projectId : null)
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
    workspaceRef: editor ? editorWorkspaceRef(editor)! : {
      checkoutId: checkout.checkoutId,
      expectedGeneration: checkout.runtime?.workspaceGeneration,
      expectedMaterializationEpoch: checkout.runtime?.materializationEpoch ?? 0,
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
  // Resolve a Files folder at the release point: a Unity asset drop delivered
  // first for the same gesture clears the hover state.
  const hit = Number.isFinite(payload.x) && Number.isFinite(payload.y)
    ? nativeWorkbenchHit(payload.x!, payload.y!) : null;
  const intent = currentNativeWorkbenchDropIntent();
  const data = intent ? nativeFileReferenceData(payload, intent) : null;
  clearNativeWorkbenchDropTarget();
  locusFileWorkspaceDragActive.value = false;
  locusFileWorkspaceDragCount.value = 0;
  locusFileWorkspaceTabEligible.value = false;
  clearWorkspaceDragPointer();
  if (await nativeWorkspaceFileDrop.drop(hit, payload.files.map((file) => file.path))) return;
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
  // Scene objects have no file payload, so a native dragover (and its cached
  // target) is not guaranteed. Resolve the target from the actual release.
  if (Number.isFinite(payload.x) && Number.isFinite(payload.y)) {
    handleUnityAssetWorkspaceDragState({ hasRefs: true, refs: payload.refs });
    updateNativeWorkbenchDropTarget(payload.x!, payload.y!);
  }
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

function handleNativeUnityAssetDrop(payload: UnityEmbedAssetDropPayload & { x: number; y: number }): void {
  void handleWorkspaceUnityAssetDrop({
    ...payload,
    x: payload.x / ownerWindow.devicePixelRatio,
    y: payload.y / ownerWindow.devicePixelRatio,
  });
}

function onDragPointerDown(raw: WorkspaceTreeItem, event: PointerEvent): void {
  const item = raw as DevelopmentTreeItem;
  if (item.meta.kind === "workspaceFile" && item.meta.workspaceFile && !item.meta.workspaceFile.loadMore) {
    const checkout = projectCheckout(item.meta.projectId, item.meta.checkoutId);
    if (!checkout?.runtime) return;
    // Dragging a row inside the multi-selection carries every selected row.
    const selected = selectedFileKeys.value.has(item.key) && item.meta.checkoutId
      ? selectedWorkspaceFileItems(item.meta.checkoutId) : [];
    const nodes = selected.length > 1 ? selected.map(candidate => candidate.meta.workspaceFile!.node) : item.meta.workspaceFile.node;
    const data = assetWorkspaceReferenceDragData({ projectId: item.meta.projectId,
      workspaceRef: checkoutWorkspaceRef(checkout), workspaceRoot: checkout.root }, nodes);
    if (data) internalDrag.start(event, workbenchReferenceInternalDragSource(data,
      event.currentTarget instanceof HTMLElement ? event.currentTarget : undefined));
    return;
  }
  if (settlingLayoutDrop.value || !item.meta.explorerNode || isKnowledgeTypeFolder(item)) return;
  const items = item.meta.kind === "session" && item.meta.session && selectedSessionIds.value.has(item.meta.session.id)
    ? (item.meta.archived ? archivedTreeItems.value : secondaryNavigation.value?.section === "sessions" ? secondarySessionItems.value : treeItems.value).filter((candidate) => (
        candidate.meta.kind === "session"
        && candidate.meta.session
        && selectedSessionIds.value.has(candidate.meta.session.id)
      ))
    : isLayoutFileItem(item) && selectedFileKeys.value.has(item.key)
      ? fileSelectionGroupItems(item).filter((candidate) => selectedFileKeys.value.has(candidate.key))
      : [item];
  const dragData = { item, items } satisfies WorkspaceLayoutInternalDragData;
  // The top bar lies on the window's top edge, where reaching the edge would
  // move the drag into a new window instead of dropping it there.
  const canExternalize = !topBarAcceptsTreeItem(item)
    && items.some((candidate) => treeEditorDescriptor(candidate) !== null);
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
    allowedOperations: (item.meta.kind === "newSession" && !fileOrderDragItem(item))
      || items.some((candidate) => (candidate.meta.kind === "mountedFile" || candidate.meta.kind === "mountedFolder")
        && !workspaceTreeItemState(explorerStore.snapshots[candidate.meta.projectId], candidate.meta.explorerNode!.nodeId,
          candidate.meta.mountEntry?.relativePath)?.pinned)
      ? ["copy"]
      : ["move", "copy"],
    cancelOnWindowBlur: canExternalize ? false : undefined,
    externalize: canExternalize ? () => handleWorkspaceTreeExternalize(dragData) : undefined,
    onActivated: () => {
      contextMenu.value = null;
      displayMenu.value = null;
      workspaceMenu.value = null;
    },
    onFinished: () => {
      pinDropProjectId.value = null;
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
  if (target.meta.kind === "workspaceFile") return null;
  if (rowElement?.closest(".development-explorer")
    && workspaceNavigation.getMode(navigationCheckout(target.meta.projectId)?.checkoutId) === "files") return null;
  if (target.meta.kind === "newSession" || target.meta.kind === "mountedFolder" || target.meta.kind === "mountedFile") return null;
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

function resolveExplorerRootDropIntent(projectId = presetProjectId.value, sessionList = false): LayoutDropIntent | null {
  if (!sessionList && workspaceNavigation.getMode(navigationCheckout(projectId)?.checkoutId) === "files") return null;
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
  if (!sourceNode || !snapshot || source.meta.kind === "newSession" || source.meta.projectId !== intent.projectId) return false;
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
    knowledgeQuotes: [],
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
  if (!workspaceMaterializationMatches(workspaceRef.expectedMaterializationEpoch, checkout.runtime?.materializationEpoch)) return null;
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
    || !sessionEditorAvailable(editor)
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
  await (await restoredSessionEditor(target.editor.editorId))?.appendComposerDraft(draft);
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
      isDir: entry.isDir,
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
  pin = false,
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
  const snapshot = await (pin ? explorerStore.pinResources : explorerStore.applyOperations)(intent.projectId, operations);
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

    const attachment = workbenchComposerTreeFileAttachment({
      kind: item.meta.kind,
      explorerNode: item.meta.explorerNode,
      mountEntry: item.meta.mountEntry,
      name: item.treeRow?.name,
    }, workspaceRoot);
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
  if (sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE) {
    const data = sourceData as WorkbenchEditorTabInternalDragData;
    if (data.windowId !== WORKBENCH_WINDOW_ID) return null;
    const sourceEditor = workbenchWindow.value.groups[data.paneId]?.tabs.find(
      (editor) => editor.editorId === data.editorId,
    );
    const targetEditor = editorId
      ? workbenchGroup(paneId)?.tabs.find((editor) => editor.editorId === editorId)
      : editorForPane(paneId);
    if (!sourceEditor || !targetEditor) return null;
    const attachment = workbenchComposerEditorAttachment(sourceEditor, {
      workspaceRoot: editorWorkingDir(sourceEditor),
      targetWorkspaceRoot: editorWorkingDir(targetEditor),
      knowledgeDocument: editorKnowledgeDocument(sourceEditor),
    });
    return attachment ? attachmentDraft({
      assetRefs: attachment.assetRef ? [attachment.assetRef] : [],
      localFiles: attachment.localFile ? [attachment.localFile] : [],
    }) : null;
  }
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
  options: { besidePaneId?: string } = {},
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
  if (workspaceDisplayMode.value === "single") {
    await adoptWorkbenchWorkspaceContext(checkout.checkoutId);
  }
  if (checkoutChanged) await refreshFocusedCheckoutServices();
  const focusedCheckout = workspaceContextStore.checkoutsById[checkout.checkoutId] ?? checkout;
  const descriptor: TreeEditorDescriptor = {
    resource: { kind: "newSession", projectId: project.projectId },
    title: t("chat.session.newSession"),
    checkoutId: focusedCheckout.checkoutId,
  };
  const editor = options.besidePaneId
    ? await openWorkbenchResourceBeside(descriptor, options.besidePaneId)
    : await openWorkbenchResource(descriptor, {
      preview: false,
      pinned: true,
      replacePreview: false,
      allowDuplicate: true,
    });
  await (await mountedSessionEditor(editor.editorId))?.applyDraftPrefill(draft);
}

async function placeKnowledgeWorkspaceDrag(
  intent: LayoutDropIntent,
  payload: KnowledgeWorkspaceDragPayload,
  pin = false,
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
  const snapshot = await (pin ? explorerStore.pinResources : explorerStore.applyOperations)(intent.projectId, operations);
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

function isSecondarySessionTreeHit(hit: Element): boolean {
  return sessionsTreeRef.value?.$el?.contains(hit) === true;
}

function developmentTreeItemFromHit(hit: Element): {
  item: DevelopmentTreeItem;
  rowElement: HTMLElement;
} | null {
  const rowElement = hit.closest<HTMLElement>(".workspace-tree-row-shell");
  if (!rowElement || (!explorerRootRef.value?.contains(rowElement) && !isSecondarySessionTreeHit(rowElement))) return null;
  const item = [...treeItems.value, ...secondarySessionItems.value].find((candidate) => candidate.key === rowElement.dataset.treeKey);
  return item ? { item, rowElement } : null;
}

function pinnedDividerTarget(hit: Element, clientY: number): {
  item: DevelopmentTreeItem;
  pin: boolean;
  layout: LayoutDropIntent;
} | null {
  const divider = hit.closest<HTMLElement>(".workspace-tree-divider");
  if (!divider || (!explorerRootRef.value?.contains(divider) && !isSecondarySessionTreeHit(divider))) return null;
  const item = [...treeItems.value, ...secondarySessionItems.value].find((candidate) => candidate.key === divider.dataset.pinnedTreeKey);
  if (!item) return null;
  const bounds = divider.getBoundingClientRect();
  return {
    item,
    pin: clientY < bounds.top + bounds.height / 2,
    layout: { projectId: item.meta.projectId, parentNodeId: null, position: 0, targetKey: `below-pins:${item.meta.projectId}` },
  };
}

function pinnedRowInsertion(
  target: DevelopmentTreeItem,
  sources: DevelopmentTreeItem[],
  after: boolean,
): PinnedDropInsertion | undefined {
  const reference = target.meta.pinnedRoot;
  if (!reference) return undefined;
  const group = [...treeItems.value, ...secondarySessionItems.value].filter((item) => item.treeRow && item.meta.projectId === target.meta.projectId
    && item.meta.pinnedRoot && sameWorkspaceTreeItem(item.meta.pinnedRoot, reference));
  const boundary = after ? group[group.length - 1] : group[0];
  if (!boundary) return undefined;
  const moving = sources.map((item) => ({ nodeId: item.meta.explorerNode!.nodeId, relativePath: item.meta.mountEntry?.relativePath }));
  return {
    before: pinnedInsertionBefore(explorerStore.snapshots[target.meta.projectId]?.itemStates ?? [], moving, reference, after),
    lineKey: boundary.key,
    side: after ? "after" : "before",
  };
}

function resolveWorkbenchInternalDrop(
  context: InternalDropResolveContext<WorkbenchInternalDragData>,
): InternalDropDecision<WorkbenchInternalDropIntent> | null {
  const sourceType = context.source.payload.type;
  const sourceData = context.source.payload.data as WorkbenchInternalDragData;
  const sourceCanOpenEditor = sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE
    || (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE && !!(sourceData as ViewWorkspaceDragPayload).workspaceView)
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
    // A tab released over the composer must never fall through to a layout move.
    if (sourceType === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE) return null;
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

  // A rejected filesystem drop in a Files list must not fall through to opening
  // an editor or adding a workspace reference.
  if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    && (sourceData as WorkbenchReferenceDragData).fileSystemMove
    && context.hit.closest(".asset-view") && context.hit.closest(".file-tree-list, .asr-list")) return null;
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
  const sessionListHit = isSecondarySessionTreeHit(context.hit);
  if (!explorerRootRef.value?.contains(context.hit) && !sessionListHit) return null;
  if (sessionListHit && sourceType !== WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) return null;
  const rowHit = developmentTreeItemFromHit(context.hit);
  const divider = pinnedDividerTarget(context.hit, context.point.y);
  const pinTarget = rowHit?.item.treeRow?.pinned ? rowHit.item : divider?.pin ? divider.item : null;
  if (pinTarget) {
    const projectId = pinTarget.meta.projectId;
    const sourceItems = sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      ? (sourceData as WorkspaceLayoutInternalDragData).items ?? [(sourceData as WorkspaceLayoutInternalDragData).item]
      : [];
    const accepts = sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      ? sourceItems.length > 0 && sourceItems.every((item) => item.meta.projectId === projectId && canSetTreeItemState(item))
      : sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
        ? (sourceData as WorkbenchReferenceDragData).origin.projectId === projectId
        : sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE
          || (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE && (sourceData as ViewWorkspaceDragPayload).workspaceView?.projectId === projectId);
    if (!accepts) return null;
    const bounds = rowHit?.rowElement.getBoundingClientRect();
    const insertion = sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      ? pinnedRowInsertion(pinTarget, sourceItems, !bounds || context.point.y >= bounds.top + bounds.height / 2)
      : undefined;
    const reordering = sourceItems.length > 0 && sourceItems.every((item) => workspaceTreeItemState(
      explorerStore.snapshots[projectId], item.meta.explorerNode!.nodeId, item.meta.mountEntry?.relativePath,
    )?.pinned);
    return {
      key: `pin:${projectId}:${insertion?.lineKey ?? "end"}:${insertion?.side ?? "after"}`,
      operation: reordering ? "move" : "copy",
      intent: { kind: "pin", projectId, insertion },
      previewMode: "floating",
    };
  }
  if (rowHit?.item.meta.kind === "dropPreview") {
    const intent = layoutDropIntent.value;
    return intent ? {
      key: `layout:${intent.targetKey}:${intent.parentNodeId ?? "root"}:${intent.position}`,
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
    const data = context.source.payload.data as WorkspaceLayoutInternalDragData;
    const sourceItems = data.items?.length ? data.items : [data.item];
    // Mounted entries share their mount root's node; moving them would move the whole mount.
    if (sourceItems.some((item) => item.meta.kind === "mountedFile" || item.meta.kind === "mountedFolder")) return null;
    const intent = divider?.layout ?? (rowHit
      ? resolveLayoutDropIntentAt(rowHit.item, context.point.y, rowHit.rowElement)
      : resolveExplorerRootDropIntent(sessionListHit ? secondaryNavigation.value?.projectId : undefined, sessionListHit));
    if (!intent || sourceItems.some((item) => item.meta.projectId !== intent.projectId || !canMoveExplorerNodeToIntent(item, intent))) {
      return null;
    }
    return {
      key: `layout:${intent.targetKey}:${intent.parentNodeId ?? "root"}:${intent.position}`,
      operation: "move",
      intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
    };
  }

  if (
    sourceType !== KNOWLEDGE_INTERNAL_DRAG_TYPE
    && sourceType !== WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    && sourceType !== VIEW_TREE_INTERNAL_DRAG_TYPE
  ) return null;
  const intent = divider?.layout ?? (rowHit
    ? resolveLayoutDropIntentAt(rowHit.item, context.point.y, rowHit.rowElement)
    : resolveExplorerRootDropIntent());
  if (!intent) return null;
  if (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE) {
    const reference = (sourceData as ViewWorkspaceDragPayload).workspaceView;
    if (!reference || reference.projectId !== intent.projectId) return null;
  }
  if (
    sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
    && (sourceData as WorkbenchReferenceDragData).origin.projectId !== intent.projectId
  ) return null;
  return {
    key: `layout:${intent.targetKey}:${intent.parentNodeId ?? "root"}:${intent.position}`,
    operation: "copy",
    intent: { kind: "layout", layout: intent, target: rowHit?.item ?? null },
  };
}

function handleWorkbenchInternalTargetChange(
  decision: InternalDropDecision<WorkbenchInternalDropIntent> | null,
): void {
  pinDropProjectId.value = decision?.intent.kind === "pin" ? decision.intent.projectId : null;
  pinDropInsertion.value = decision?.intent.kind === "pin" ? decision.intent.insertion ?? null : null;
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
  if (decision.intent.kind === "pin") {
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

function workspaceLayoutPinReferences(data: WorkspaceLayoutInternalDragData): ProjectExplorerItemRef[] {
  const items = data.items?.length ? data.items : [data.item];
  return items.filter(canSetTreeItemState).map((item) => ({
    nodeId: item.meta.explorerNode!.nodeId,
    relativePath: item.meta.mountEntry?.relativePath,
  }));
}

async function commitWorkbenchInternalDrop(
  sourceType: string,
  sourceData: WorkbenchInternalDragData,
  intent: WorkbenchInternalDropIntent,
): Promise<void> {
  if (intent.kind === "pin") {
    const layout: LayoutDropIntent = { projectId: intent.projectId, parentNodeId: null,
      position: explorerStore.snapshots[intent.projectId]?.nodes.filter((node) => !node.parentNodeId).length ?? 0,
      targetKey: `pin:${intent.projectId}` };
    if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE) {
      const data = sourceData as WorkspaceLayoutInternalDragData;
      const items = data.items?.length ? data.items : [data.item];
      await ensureArchivedDropPlacements(items);
      await explorerStore.applyOperations(intent.projectId, [
        ...workspaceTreePinOperations(workspaceLayoutPinReferences(data), intent.insertion?.before),
        ...items.filter((item) => item.meta.archived && item.meta.explorerNode?.hidden)
          .map((item) => ({ kind: "setNodeHidden" as const, nodeId: item.meta.explorerNode!.nodeId, hidden: false })),
      ]);
      await restoreArchivedDropItems(items);
    } else if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
      await placeWorkbenchReferenceDrag(layout, sourceData as WorkbenchReferenceDragData, true);
    } else if (sourceType === KNOWLEDGE_INTERNAL_DRAG_TYPE) {
      await placeKnowledgeWorkspaceDrag(layout, (sourceData as KnowledgeInternalDragData).payload, true);
    } else if (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE) {
      const reference = (sourceData as ViewWorkspaceDragPayload).workspaceView;
      if (!reference || reference.projectId !== intent.projectId) return;
      explorerStore.rememberView(reference);
      await explorerStore.pinResources(intent.projectId, [{ kind: "placeResource", resourceKind: "view",
        resourceId: reference.view.id, parentNodeId: null, position: layout.position }]);
    }
    return;
  }
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
    } else if (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE) {
      const reference = (sourceData as ViewWorkspaceDragPayload).workspaceView;
      if (reference) descriptors = [{
        resource: { kind: "view", projectId: reference.projectId, viewId: reference.view.id },
        title: reference.view.name,
        checkoutId: reference.workspaceRef.checkoutId,
      }];
    } else if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
      descriptors = await referenceEditorDescriptors(sourceData as WorkbenchReferenceDragData);
    }
    if (descriptors.length === 0) return;

    let destinationPaneId = intent.paneId;
    if (usesCheckoutScopedWorkbench()) {
      const checkoutIds = new Set(
        descriptors
          .map((descriptor) => descriptorLayoutCheckoutId(descriptor))
          .filter((checkoutId): checkoutId is string => !!checkoutId),
      );
      if (checkoutIds.size > 1) {
        throw new Error("A single-workspace drop cannot mix checkout bindings.");
      }
      const checkoutId = checkoutIds.values().next().value
        ?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID);
      if (!checkoutId) throw new Error(t("workbench.unavailable.checkout"));
      const switchingCheckout = workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID) !== checkoutId;
      if (!await activateCheckoutScopedWorkbench(checkoutId)) return;
      if (switchingCheckout || !workbenchWindow.value.groups[destinationPaneId]) {
        destinationPaneId = workbenchWindow.value.focusedPaneId;
      }
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
      const data = sourceData as WorkspaceLayoutInternalDragData;
      // A node dragged from the top bar into the tree returns to the tree there.
      returnTopBarNodesToTree(data.items?.length ? data.items : [data.item]);
      await moveExplorerNodeToIntent(data.item, intent.layout, data.items);
    }
    return;
  }
  if (sourceType === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE) {
    const data = sourceData as WorkbenchReferenceDragData;
    if (intent.kind === "layout") await placeWorkbenchReferenceDrag(intent.layout, data);
    return;
  }
  if (sourceType === VIEW_TREE_INTERNAL_DRAG_TYPE) {
    const reference = (sourceData as ViewWorkspaceDragPayload).workspaceView;
    if (!reference || intent.kind !== "layout" || reference.projectId !== intent.layout.projectId) return;
    const detail = await viewRead(reference.workspaceRef, reference.view.id);
    explorerStore.rememberView({ ...reference, view: detail.summary });
    await explorerStore.applyOperations(reference.projectId, [{
      kind: "placeResource", resourceKind: "view", resourceId: reference.view.id,
      parentNodeId: intent.layout.parentNodeId, position: intent.layout.position,
    }]);
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
    || source.payload.type === VIEW_TREE_INTERNAL_DRAG_TYPE
    || source.payload.type === WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
  resolve: resolveWorkbenchInternalDrop,
  onTargetChange: handleWorkbenchInternalTargetChange,
  drop: async ({ source, decision }) => {
    const settlingId = ++settlingLayoutDropId;
    if (
      source.payload.type === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE
      && (decision.intent.kind === "layout" || decision.intent.kind === "pin")
    ) {
      const data = source.payload.data as WorkspaceLayoutInternalDragData;
      const item = data.item;
      const snapshot = explorerStore.snapshots[item.meta.projectId];
      if (snapshot && item.meta.explorerNode) {
        // The floating preview is gone on pointerup. Show the real row now and
        // keep one layout until the async snapshot and drop cleanup both finish.
        settlingLayoutDrop.value = {
          id: settlingId,
          snapshot: decision.intent.kind === "pin"
            ? previewWorkspaceTreePin(snapshot, workspaceLayoutPinReferences(data), decision.intent.insertion?.before)
            : previewWorkspaceTreeMove(
              snapshot,
              workspaceTreeMoveOperation(item.meta.explorerNode, decision.intent.layout),
            ),
        };
      }
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
      pinDropProjectId.value = null;
    }
  },
  previewMode: ({ hit }) => {
    if (hit.closest(".workspace-tree-row-shell.is-pinned")) return "floating";
    if (hit.closest(".workspace-tree-row-shell.is-new-session-row")) return "floating";
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
  if (target.treeRow?.pinned) {
    externalDropTarget.value = target;
    handleWorkbenchInternalTargetChange({ key: `pin:${target.meta.projectId}`, operation: "copy",
      intent: { kind: "pin", projectId: target.meta.projectId } });
    return;
  }
  pinDropProjectId.value = null;
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

async function ensureArchivedDropPlacements(items: DevelopmentTreeItem[]): Promise<void> {
  const byProject = new Map<string, ProjectExplorerOperation[]>();
  for (const item of items) {
    const node = item.meta.explorerNode;
    if (!item.meta.archived || !item.meta.session || !node) continue;
    const snapshot = explorerStore.snapshots[item.meta.projectId];
    if (snapshot?.nodes.some((candidate) => candidate.nodeId === node.nodeId)) continue;
    const operations = byProject.get(item.meta.projectId) ?? [];
    operations.push({ kind: "placeResource", nodeId: node.nodeId, resourceKind: "session",
      resourceId: item.meta.session.id, parentNodeId: null,
      position: (snapshot?.nodes.filter((candidate) => !candidate.parentNodeId).length ?? 0) + operations.length });
    byProject.set(item.meta.projectId, operations);
  }
  for (const [projectId, operations] of byProject) await explorerStore.applyOperations(projectId, operations);
}

async function restoreArchivedDropItems(items: DevelopmentTreeItem[]): Promise<void> {
  for (const item of items) {
    if (item.meta.archived && item.meta.session) {
      await restoreArchivedSession(item.meta.session.id, item.meta.projectId);
    }
  }
  if (items.some((item) => item.meta.archived)) resetSessionMultiSelection();
}

async function moveExplorerNodeToIntent(
  source: DevelopmentTreeItem,
  intent: LayoutDropIntent,
  selectedItems?: DevelopmentTreeItem[],
): Promise<void> {
  const items = selectedItems?.length ? selectedItems : [source];
  if (!items.every((item) => canMoveExplorerNodeToIntent(item, intent))) return;
  try {
    await ensureArchivedDropPlacements(items);
    const operations: ProjectExplorerOperation[] = workspaceTreeMoveOperations(
      explorerStore.snapshots[intent.projectId]!, items.map((item) => item.meta.explorerNode!), intent,
    );
    for (const item of items) {
      const node = item.meta.explorerNode!;
      if (workspaceTreeItemState(explorerStore.snapshots[item.meta.projectId], node.nodeId)?.pinned) {
        operations.push({ kind: "setItemState", nodeId: node.nodeId, pinned: false });
      }
      if (item.meta.archived && node.hidden) {
        operations.push({ kind: "setNodeHidden", nodeId: node.nodeId, hidden: false });
      }
    }
    await explorerStore.applyOperations(intent.projectId, operations);
    await restoreArchivedDropItems(items);
    if (intent.parentNodeId) {
      expanded.value = new Set([...expanded.value, `folder:${intent.projectId}:${intent.parentNodeId}`]);
    }
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
  if (target?.nodeType === 1 && (target as Element).closest(".workspace-tree-divider")) {
    updateNativeWorkbenchDropTarget(event.clientX, event.clientY);
    return;
  }
  pinDropProjectId.value = null;
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
  pinDropProjectId.value = null;
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
  try {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string" || !selected.trim()) return;
    if (!await requestWorkspaceSetup(selected)) return;
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

async function configureWorkspace(path: string): Promise<void> {
  try {
    if (!await requestWorkspaceSetup(path, "edit")) return;
    const runtime = await workspaceContextStore.openCheckout(path);
    await projectStore.handleExtraWorkdirsUpdated(runtime.root);
    await broadcastExtraWorkdirsUpdated(runtime.root, {
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
      expectedMaterializationEpoch: runtime.materializationEpoch,
    });
    await explorerStore.loadProject(runtime.projectId, true);
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
    || (!props.auxiliary && workspaceDisplayMode.value === "single");
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
  const runtime = workspaceContextBaseStore.checkoutsById[checkoutId]?.runtime;
  if (paneContext?.focusedCheckoutId !== checkoutId
    || paneContext.workspaceGeneration !== runtime?.workspaceGeneration
    || !workspaceMaterializationMatches(paneContext.materializationEpoch, runtime?.materializationEpoch)) {
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

useWorkbenchPaneLifecycle(WORKBENCH_WINDOW_ID, () => Object.keys(workbenchWindow.value.groups));

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
      const checkout = workspaceContextStore.focusedCheckout;
      if (checkout) {
        void openWorkbenchResource({
          resource: { kind: "newSession", projectId: checkout.projectId },
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
    (explorerStore.viewResources[projectId] ?? []).map(({ workspaceRef, view }) => `${workspaceRef.checkoutId}:${view.id}:${view.name}`).join(","),
  ].join("|")).join("\n"),
  () => {
    for (const group of Object.values(workbenchWindow.value.groups)) {
      for (const editor of group.tabs) {
        const resource = editor.resource;
        const title = resource.kind === "view"
          ? (explorerStore.viewResources[resource.projectId] ?? []).find(item => item.view.id === resource.viewId
            && item.workspaceRef.checkoutId === editor.checkoutBinding?.checkoutId)?.view.name ?? editor.title
          : titleForResource(resource, editor.sourcePath);
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

watch(() => Object.values(workspaceContextStore.checkoutsById).map(checkout => `${checkout.checkoutId}:${checkout.knowledgeEnabled}`).join("|"), () => {
  const navigation = secondaryNavigation.value;
  if (navigation?.section === "knowledge" && workspaceContextStore.checkoutsById[navigation.checkoutId]?.knowledgeEnabled === false) {
    closeSecondaryNavigation();
  }
  void reconcileRestoredWorkbenchEditors().catch(error => console.warn("[workspace setup]", error));
});

onMounted(() => {
  const editor = workbenchStore.activeEditor(WORKBENCH_WINDOW_ID);
  if (editor) showFileHistorySecondarySidebar(editor);
  treeTimeRefreshTimer = ownerWindow.setInterval(() => {
    treeTimeNow.value = Date.now() / 1000;
  }, 60_000);
  unregisterWorkbenchInternalDropTarget = internalDrag.registerTarget(workbenchInternalDropTarget);
  unregisterTopBarInternalDropTarget = internalDrag.registerTarget(topBarInternalDropTarget);
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
      unlistenExplorerFileAction = await appWindow.listen<ExplorerResourceTarget & { newPath: string | null }>(
        "explorer-file-action", ({ payload }) => {
          resourceFileChanged(payload, payload.newPath);
          const mounts = explorerStore.snapshots[payload.projectId]?.nodes.filter((node) => node.nodeKind === "folder" && node.sourcePath) ?? [];
          for (const node of mounts) void explorerStore.loadMount(payload.projectId, node.nodeId, true).catch(() => {});
        },
      );
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
      unlistenWorkbenchFileOpen = await appWindow.listen<WorkbenchFileOpenRequest>(
        WORKBENCH_FILE_OPEN_EVENT,
        (event) => {
          void openFileInWorkbench(event.payload).catch((error) => {
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
  void subscribeLocusFileDragState(handleLocusFileDragState, props.nativeWindow).then((release) => {
    if (workbenchDisposed) { release(); return; }
    releaseLocusFileDragState = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] file drag subscription failed", error);
  });
  void subscribeLocusFileDrop((payload) => {
    void handleLocusFileDrop(payload);
  }, props.nativeWindow).then((release) => {
    if (workbenchDisposed) { release(); return; }
    releaseLocusFileDrop = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] file drop subscription failed", error);
  });
  void subscribeUnityEmbedAssetDrop((payload) => {
    void handleWorkspaceUnityAssetDrop(payload);
  }, props.nativeWindow).then((release) => {
    if (workbenchDisposed) { release(); return; }
    releaseUnityAssetDrop = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] Unity asset drop subscription failed", error);
  });
  void subscribeUnitySendToLocus((payload) => {
    void handleUnitySendToLocus(payload);
  }).then((release) => {
    if (workbenchDisposed) { release(); return; }
    releaseUnitySendToLocus = release;
  }).catch((error) => {
    console.warn("[DevelopmentWorkbench] Send to Locus subscription failed", error);
  });
  if (props.nativeWindow) {
    void props.nativeWindow.listen<UnityEmbedAssetDropPayload & { x: number; y: number }>("unity-native-embed:asset-drop", event => {
      handleNativeUnityAssetDrop(event.payload);
    }).then(release => { if (workbenchDisposed) release(); else releaseNativeUnityAssetDrop = release; });
  }
  void subscribeUnityEmbedAssetDragState(handleUnityAssetWorkspaceDragState, props.nativeWindow)
    .then((release) => {
      if (workbenchDisposed) { release(); return; }
      releaseUnityAssetDragState = release;
    })
    .catch((error) => {
      console.warn("[DevelopmentWorkbench] Unity asset drag subscription failed", error);
    });
});

const releaseFrontendWorkbench = registerFrontendWorkbench({
  ownerWindow,
  async ready(workspaceRef) {
    const started = Date.now();
    while (!transferHostReady || (workspaceRef && !workspaceContextStore.checkoutsById[workspaceRef.checkoutId]?.runtime)) {
      if (Date.now() - started > 10_000) throw new Error("Workbench initialization did not complete.");
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
  },
  tabs: () => Object.entries(workbenchWindow.value.groups).flatMap(([paneId, group]) => group.tabs.map((editor) => ({
    editorId: editor.editorId, paneId, title: editor.title, kind: editor.resource.kind,
    active: group.activeEditorId === editor.editorId, checkoutId: editorWorkspaceRef(editor)?.checkoutId,
  }))),
  async activate(editorId) {
    const pane = Object.entries(workbenchWindow.value.groups).find(([, group]) => group.tabs.some((editor) => editor.editorId === editorId));
    if (!pane) throw new Error("Workbench editor is not open: " + editorId);
    await focusWorkbenchEditor(pane[0], editorId);
  },
  async close(editorId) {
    const pane = Object.entries(workbenchWindow.value.groups).find(([, group]) => group.tabs.some((editor) => editor.editorId === editorId));
    if (!pane) throw new Error("Workbench editor is not open: " + editorId);
    await closeWorkbenchEditor(pane[0], editorId);
  },
}, WORKBENCH_WINDOW_ID);
onUnmounted(() => {
  workbenchPaneRestoreEpoch += 1;
  workbenchDisposed = true;
  ownerWindow.clearInterval(treeTimeRefreshTimer);
  cancelSessionRowPreload();
  releaseFrontendWorkbench();
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
  unlistenExplorerFileAction?.();
  unlistenExplorerFileAction = null;
  unlistenWorkbenchInspectorOpen?.();
  unlistenWorkbenchInspectorOpen = null;
  unlistenWorkbenchFileOpen?.();
  unlistenWorkbenchFileOpen = null;
  for (const pending of outgoingWorkbenchTransfers.values()) {
    window.clearTimeout(pending.timer);
    pending.reject(new Error(t("workbench.window.targetUnavailable")));
  }
  outgoingWorkbenchTransfers.clear();
  unregisterWorkbenchInternalDropTarget?.();
  unregisterTopBarInternalDropTarget?.();
  releaseWorkStateProvider?.();
  unregisterWorkbenchInternalDropTarget = null;
  unregisterTopBarInternalDropTarget = null;
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
  releaseNativeUnityAssetDrop?.();
  releaseNativeUnityAssetDrop = null;
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
    <GlobalSearch
      :active="!props.prewarm && (props.auxiliary || uiStore.activePage === 'development')"
      :targets="globalSearchTargets"
      :owner-window="ownerWindow"
      :open-result="openGlobalSearchResult"
    />
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
      data-search-boundary
      :class="{ 'is-workspace-drop-target': workspaceDropAffordanceActive }"
      :style="{ width: `${explorerWidth}px` }"
      @contextmenu="openExplorerBackgroundContextMenu"
      @dragenter.capture="onExplorerDragOver"
      @dragover.capture="onExplorerDragOver"
      @dragleave="onExplorerDragLeave"
      @drop="onExplorerDrop"
    >
      <Teleport :to="props.toolbarTarget ?? ownerDocument.body" :disabled="!props.toolbarTarget">
      <div
        class="development-explorer-toolbar"
        :class="{ 'is-window-titlebar': props.toolbarTarget }"
        :style="props.toolbarTarget ? { width: `${explorerWidth - 1}px` } : undefined"
      >
        <button
          v-if="!props.fixedWorkspaceRef && workspaceDisplayMode === 'single'"
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
            v-if="unreadSessionCount"
            type="button"
            :title="t('development.nextUnreadSession')"
            :aria-label="t('development.unreadSessions', unreadSessionCount)"
            @click="revealNextUnreadSession"
          >{{ unreadSessionCount }}</button>
          <button
            v-if="!props.fixedWorkspaceRef"
            type="button"
            :title="t('development.openWorkspace')"
            :aria-label="t('development.openWorkspace')"
            @click="browseWorkspace"
          >
            <LucideIcon :icon="FolderOpen" :size="15" />
          </button>
          <button
            type="button"
            :title="t('common.more')"
            @click="toggleDisplayMenu"
          >
            <LucideIcon :icon="MoreHorizontal" :size="15" />
          </button>
        </div>
      </div>
      </Teleport>

      <template v-for="checkout in fileModeCheckouts" :key="checkout.checkoutId">
        <WorkspaceFileTreeSource
          v-if="checkout.runtime"
          :working-dir="checkout.root"
          :workspace-ref="checkoutWorkspaceRef(checkout)"
          @change="workspaceFileStates[checkout.checkoutId] = $event"
          @error="notificationStore.addNotice('error', $event)"
        />
      </template>
      <WorkspaceTree
        ref="workspaceTreeRef"
        :resource-rename="resourceActions?.draft"
        navigation
        class="development-tree"
        @pointerdown.capture="handleTreePointerDown"
        @pointerover="handleSessionRowPointerOver"
        @pointerleave="cancelSessionRowPreload"
        :items="treeItems"
        :row-height="fileModeCheckouts.length ? WORKSPACE_FILE_ROW_HEIGHT : 30"
        :base-indent="12"
        :indent-size="14"
        :row-tab-index="0"
        @session-action="archiveSessionItem($event as DevelopmentTreeItem)"
        @rename-change="sessionInlineRename && (sessionInlineRename.value = $event)"
        @rename-submit="submitSessionRename"
        @rename-cancel="cancelSessionRename"
        @activate="activateItem"
        @contextmenu="openContextMenu"
        @drag-pointer-down="onDragPointerDown"
        @dragover="onExternalRowDragOver"
        @visible-range-change="treeVisibleRange = $event"
      >
        <template #icon="{ item, size }">
          <span
            v-if="(item as DevelopmentTreeItem).meta.kind === 'empty'"
            class="development-empty-folder-icon"
            aria-hidden="true"
          />
          <LucideIcon
            v-else
            :class="itemIconClass(item as DevelopmentTreeItem)"
            :icon="itemIcon(item as DevelopmentTreeItem)"
            :size="size"
            :stroke-width="2"
          />
        </template>
        <template #name="{ item }"><span>{{ treeItemDisplayName(item as DevelopmentTreeItem) }}</span></template>
        <template #trailing="{ item }">
          <span v-if="itemUnreadCount(item as DevelopmentTreeItem) > 0" class="development-unread-count"
            :title="t('development.unreadSessions', itemUnreadCount(item as DevelopmentTreeItem))">
            {{ itemUnreadCount(item as DevelopmentTreeItem) }}
          </span>
          <span
            v-if="(item as DevelopmentTreeItem).meta.kind === 'checkout' && checkoutBranchLabel((item as DevelopmentTreeItem).meta.projectId, (item as DevelopmentTreeItem).meta.checkoutId)"
            class="development-branch-label"
          >
            {{ checkoutBranchLabel((item as DevelopmentTreeItem).meta.projectId, (item as DevelopmentTreeItem).meta.checkoutId) }}
          </span>
          <span
            v-else-if="documentModifiedTime(item as DevelopmentTreeItem)"
            class="development-knowledge-modified-time"
          >
            {{ documentModifiedTime(item as DevelopmentTreeItem) }}
          </span>
          <button
            v-if="(item as DevelopmentTreeItem).meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND"
            type="button"
            class="development-knowledge-remove-button"
            :title="t('development.hideNode')"
            :aria-label="t('development.hideNode')"
            @pointerdown.stop
            @click.stop="setWorkspaceNodeHidden(item as DevelopmentTreeItem, true)"
          >
            <LucideIcon :icon="EyeOff" :size="12" :stroke-width="2" />
          </button>
          <button
            v-else-if="(item as DevelopmentTreeItem).meta.kind === 'knowledge'"
            type="button"
            class="development-knowledge-remove-button"
            :title="t('development.hideNode')"
            :aria-label="t('development.hideNode')"
            @pointerdown.stop
            @click.stop="removeKnowledgeItemFromWorkspace(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="EyeOff" :size="12" :stroke-width="2" />
          </button>
          <button
            v-else-if="canHideWorkspaceFileItem(item as DevelopmentTreeItem)"
            type="button"
            class="development-knowledge-remove-button"
            :title="t('development.hideNode')"
            :aria-label="t('development.hideNode')"
            @pointerdown.stop
            @click.stop="hideWorkspaceFileItem(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="EyeOff" :size="12" :stroke-width="2" />
          </button>
          <button
            v-else-if="(item as DevelopmentTreeItem).meta.kind === 'view'"
            type="button"
            class="development-view-hide-button"
            :title="t('development.hideNode')"
            :aria-label="t('development.hideNode')"
            @pointerdown.stop
            @click.stop="removeMountedNodeFromWorkspace(item as DevelopmentTreeItem)"
          >
            <LucideIcon :icon="EyeOff" :size="12" :stroke-width="2" />
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
                :aria-label="inlineCreateLabel()"
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

    <div class="development-stage" :class="{ 'has-workspace-overlay': workspaceOverlayShown }">
    <Transition
      :css="false"
      @enter="enterWorkbenchSidebar"
      @leave="leaveWorkbenchSidebar"
      @enter-cancelled="cancelWorkbenchSidebarMotion"
      @leave-cancelled="cancelWorkbenchSidebarMotion"
    >
    <WorkbenchSecondarySidebar
      v-if="secondaryNavigation && secondaryCheckout && secondaryWorkspaceRef && (props.showExplorer || secondaryNavigation.section === 'fileHistory')"
      :title="secondaryTitle"
      :content-key="`${secondaryNavigation.section}:${secondaryNavigation.checkoutId}:${secondaryNavigation.editorId ?? ''}`"
      :owner-window="ownerWindow"
      @close="closeSecondaryNavigation"
    >
      <template #default="{ toolbarTarget }">
      <div
        v-if="secondaryNavigation.section === 'fileHistory'"
        :ref="(value) => { fileHistorySidebarTarget = value as HTMLElement | null; fileHistorySidebarToolbarTarget = toolbarTarget; }"
        class="file-history-secondary-content"
      />
      <KnowledgeView
        v-else-if="secondaryNavigation.section === 'knowledge'"
        :key="`knowledge:${secondaryNavigation.checkoutId}`"
        list-only
        :selected-document-id="secondaryKnowledgeSelection?.document.id ?? null"
        :selected-document-target="secondaryKnowledgeSelection?.document ?? null"
        :selection-request-id="secondaryKnowledgeSelection?.requestId ?? 0"
        :working-dir="secondaryCheckout.root"
        :workspace-ref="secondaryWorkspaceRef"
        :selected-model-id="modelStore.selectedModelId"
        :model-defaults="modelStore.modelDefaults"
        @open-document="openSecondaryKnowledgeDocument"
        @open-page="openSecondaryKnowledgePage"
      />
      <WorkspaceTree
        v-else-if="secondaryNavigation.section === 'sessions'"
        ref="sessionsTreeRef"
        class="development-tree"
        navigation
        @pointerdown.capture="handleTreePointerDown"
        @pointerover="handleSessionRowPointerOver"
        @pointerleave="cancelSessionRowPreload"
        :items="secondarySessionItems"
        :base-indent="12"
        :row-tab-index="0"
        @activate="activateItem"
        @contextmenu="openContextMenu"
        @drag-pointer-down="onDragPointerDown"
        @session-action="archiveSessionItem($event as DevelopmentTreeItem)"
        @rename-change="sessionInlineRename && (sessionInlineRename.value = $event)"
        @rename-submit="submitSessionRename"
        @rename-cancel="cancelSessionRename"
      >
        <template #icon="{ item, size }">
          <LucideIcon :icon="itemIcon(item as DevelopmentTreeItem)"
            :class="itemIconClass(item as DevelopmentTreeItem)" :size="size" />
        </template>
        <template #custom="{ item }">
          <div v-if="(item as DevelopmentTreeItem).meta.kind === 'inlineCreate'" ref="inlineCreateRow"
            class="development-inline-create-row"
            :style="{ paddingLeft: `${12 + ((item as DevelopmentTreeItem).meta.inlineCreateDepth ?? 0) * 14}px` }">
            <span class="development-inline-create-bullet" aria-hidden="true" />
            <div class="development-inline-create-body">
              <input ref="inlineCreateInput" v-model="inlineCreate!.name" class="development-inline-create-input"
                :placeholder="t('knowledge.explorer.namePlaceholder')" :aria-label="t('development.newFolder')"
                @keydown.enter.prevent="submitInlineCreate" @keydown.esc.prevent.stop="cancelInlineCreate" />
              <div class="development-inline-create-actions">
                <BaseButton class="development-inline-create-action" :title="t('common.confirm')"
                  :disabled="!inlineCreate?.name.trim()" @click="submitInlineCreate">
                  <LucideIcon :icon="Check" :size="12" :stroke-width="2.4" />
                </BaseButton>
                <BaseButton class="development-inline-create-action" :title="t('common.cancel')" @click="cancelInlineCreate">
                  <LucideIcon :icon="X" :size="12" :stroke-width="2.4" />
                </BaseButton>
              </div>
            </div>
          </div>
        </template>
        <template #empty><div class="development-tree-empty">{{ t('development.sessions.empty') }}</div></template>
      </WorkspaceTree>
      <AssetView
        v-else-if="secondaryNavigation.section === 'assets'"
        ref="secondaryAssetViewRef"
        @file-history="openFileHistory"
        :key="`assets:${secondaryNavigation.checkoutId}`"
        list-only
        :project-id="secondaryNavigation.projectId"
        :working-dir="secondaryCheckout.root"
        :workspace-ref="secondaryWorkspaceRef"
        @open-file="openSecondaryFile"
        @resource-changed="resourceFileChanged"
      />
      <WorkspaceArchivedFiles
        v-else-if="secondaryNavigation.section === 'archivedFiles'"
        :resource-rename="resourceActions?.draft"
        :key="`archived-files:${secondaryNavigation.checkoutId}`"
        :working-dir="secondaryCheckout.root"
        :workspace-ref="secondaryWorkspaceRef"
        :selected-path="activeResource?.projectId === secondaryNavigation.projectId
          && workbenchStore.activeEditor(WORKBENCH_WINDOW_ID)?.checkoutBinding?.checkoutId === secondaryNavigation.checkoutId
          && (activeResource.kind === 'asset' || activeResource.kind === 'workspaceFile') ? activeResource.path : null"
        @open-file="openSecondaryFile"
        @contextmenu="openArchivedFileContextMenu"
        @error="notificationStore.addNotice('error', $event)"
      />
      <ViewPackageView
        v-else-if="secondaryNavigation.section === 'views'"
        :key="`views:${secondaryNavigation.checkoutId}`"
        list-only
        :project-id="secondaryNavigation.projectId"
        :toolbar-target="toolbarTarget"
        :working-dir="secondaryCheckout.root"
        :workspace-ref="secondaryWorkspaceRef"
      />
      <WorkbenchArchivedSessionsEditor
        v-else-if="secondaryNavigation.section === 'archived'"
        :key="`archived:${secondaryNavigation.checkoutId}`"
        :toolbar-target="toolbarTarget"
        :project-id="secondaryNavigation.projectId"
        :workspace-ref="secondaryWorkspaceRef"
        :items="archivedTreeItems"
        :refresh-key="archivedRefreshKey"
        @sessions-change="archivedSessions = $event"
        @activate="activateItem"
        @contextmenu="openContextMenu"
        @drag-pointer-down="onDragPointerDown"
        @session-action="archiveSessionItem($event as DevelopmentTreeItem)"
        @rename-change="sessionInlineRename && (sessionInlineRename.value = $event)"
        @rename-submit="submitSessionRename"
        @rename-cancel="cancelSessionRename"
      />
      </template>
    </WorkbenchSecondarySidebar>
    </Transition>

    <main class="development-editor">
      <WorkbenchSplitHost
        :node="workbenchWindow.layout"
        :groups="workbenchWindow.groups"
        :focused-pane-id="workbenchWindow.focusedPaneId"
        :active-drop-key="activeEditorDropKey"
        :show-single-tabs="props.auxiliary || workbenchWindow.layout.kind === 'split'"
        :tabs-in-titlebar="!!props.singleTabTitleTarget"
        @focus-pane="focusWorkbenchPane"
        @resize="resizeWorkbenchSplit"
      >
        <template #group="{ group, paneId, focused }">
          <template v-if="group">
            <Teleport :to="props.singleTabTitleTarget ?? 'body'" :disabled="!props.singleTabTitleTarget">
              <WorkbenchEditorTabs
                :window-id="WORKBENCH_WINDOW_ID"
                :group="group"
                :view-icon="workbenchViewIcon"
                :workspace-label="workbenchTabWorkspaceLabel"
                :show-single-tab="props.auxiliary || workbenchWindow.layout.kind === 'split'"
                :window-title="!!props.singleTabTitleTarget"
                :workspace-reveal="props.showExplorer && !props.prewarm"
                :drop-active="renderedEditorDropIntent?.paneId === paneId && renderedEditorDropIntent.direction === 'center'"
                :drop-index="renderedEditorDropIntent?.paneId === paneId && renderedEditorDropIntent.direction === 'center' ? renderedEditorDropIntent.index : undefined"
                @activate="focusWorkbenchEditor(paneId, $event)"
                @close="closeWorkbenchEditor(paneId, $event)"
                @close-many="closeWorkbenchEditors(paneId, $event)"
                @pin="pinWorkbenchEditor(paneId, $event)"
                @reveal-in-workspace="revealWorkbenchEditorInWorkspace(paneId, $event)"
                @open-in-file-explorer="openWorkbenchEditorInFileExplorer(paneId, $event)"
                @drag-externalize="handleWorkbenchTabExternalize"
                @title-keydown="handleWorkbenchKeydown"
              />
            </Teleport>
            <WorkbenchEditorStack
              :group="group"
              :mount-limit="sessionEditorMountLimit"
              :is-evictable="isWorkbenchSessionEditorInput"
              :retained-editor-ids="retainedSessionEditorIds"
              @pin="pinWorkbenchEditor(paneId, $event)"
              @evict="handleWorkbenchEditorEvicted"
            >
              <template #default="{ editor, ready, interactive, contentActive, presented }">
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
                  v-else-if="editor.resource.kind === 'session' || editor.resource.kind === 'newSession' || (editor.resource.kind === 'section' && editor.resource.section === 'archived' && !!editor.resource.sessionId)"
                  @session-unarchived="handleSessionUnarchived($event, editor.resource.projectId)"
                  :ref="editorRefCallback(editor, 'session')"
                  :editor="editor"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :sessions="sessionEditorSessions(editor, contentActive)"
                  :select-worktree="fixedWorkspaceRef ? undefined : sessionWorktreeSelector(paneId, editor)"
                  :workspace-changing="worktreeSelectionBusy"
                  :reference-drop-available="composerAcceptsCurrentDrag(paneId, editor)"
                  :reference-drop-active="
                    composerDropTarget?.paneId === paneId
                      && composerDropTarget.editorId === editor.editorId
                  "
                  :shortcut-active="focused && interactive"
                  :active="focused && interactive"
                  :presented="presented"
                  :surface-hidden="!contentActive"
                  :new-chat-shortcut-action="newSessionShortcutAction(group, editor)"
                  @ready="() => { ready(); void restoreEditorWorkState(editor.editorId); }"
                   @session-created="handleWorkbenchSessionCreated(paneId, $event)"
                   @session-selected="handleWorkbenchSessionSelected(paneId, $event)"
                   @session-forked="handleWorkbenchSessionForked(paneId, $event)"
                   @export-session-context="handleWorkbenchSessionExport(paneId, $event)"
                   @review-session-context="handleWorkbenchSessionReview(paneId, $event)"
                   @open-knowledge-document="handleWorkbenchKnowledgeDocument(paneId, $event)"
                   @new-session-requested="handleWorkbenchNewSessionRequested(paneId, $event)"
                  @composer-draft-change="handleWorkbenchComposerDraftChange(paneId, $event)"
                  @composer-focus="handleWorkbenchComposerFocus(paneId, $event)"
                  @keep-mounted-change="setSessionEditorKeepMounted(editor.editorId, $event)"
                />
                <AgentView
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'agents'"
                  :ref="editorRefCallback(editor, 'agent')"
                  :embedded="!!editor.resource.agentId"
                  :agent-id="editor.resource.agentId"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                />
                <KnowledgeView
                  v-else-if="editor.resource.kind === 'knowledge' || editor.resource.kind === 'knowledgeRoot' || (editor.resource.kind === 'section' && editor.resource.section === 'knowledge')"
                  :ref="editorRefCallback(editor, 'knowledge')"
                  :embedded="editor.resource.kind === 'knowledge' || (editor.resource.kind === 'section' && !!editor.resource.knowledgePage)"
                  :knowledge-page="editor.resource.kind === 'section' ? editor.resource.knowledgePage : null"
                  :active="contentActive"
                  :selected-document-id="editorKnowledgeDocument(editor)?.id ?? null"
                  :selected-document-target="editorKnowledgeDocument(editor)"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :selected-model-id="modelStore.selectedModelId"
                  :model-defaults="modelStore.modelDefaults"
                  @ready="() => { ready(); void restoreEditorWorkState(editor.editorId); }"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                  @close-page="closeWorkbenchEditor(paneId, editor.editorId)"
                />
                <WorkbenchCollabPage
                  v-else-if="editor.resource.kind === 'collaboration' || editor.resource.kind === 'checkout' || (editor.resource.kind === 'section' && editor.resource.section === 'collab')"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="focused && group.activeEditorId === editor.editorId"
                  :visible="group.activeEditorId === editor.editorId"
                  :selected-model-id="modelStore.selectedModelId"
                  :selected-agent-id="agentStore.selectedAgentId"
                  :models="modelStore.availableModels"
                  :activate-editor="editorActivator(paneId, editor)"
                  @select-model="(id: string) => modelStore.selectModel(id)"
                />
                <AssetView
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'assets'"
                  @file-history="openFileHistory"
                  :project-id="editor.resource.projectId"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <FileHistoryPanel
                  v-else-if="editor.resource.kind === 'fileHistory'"
                  :key="`${editor.editorId}:${JSON.stringify(editor.checkoutBinding)}`"
                  :target="fileHistoryEditorTarget(editor)"
                  :sidebar-target="fileHistorySidebarOwner?.editor.editorId === editor.editorId ? fileHistorySidebarTarget : null"
                  :sidebar-toolbar-target="fileHistorySidebarOwner?.editor.editorId === editor.editorId ? fileHistorySidebarToolbarTarget : null"
                  :activate-editor="editorActivator(paneId, editor)"
                />
                <WorkspaceFilePreview
                  v-else-if="editor.resource.kind === 'workspaceFile' || (editor.resource.kind === 'asset' && isWorkbenchDocumentPath(editor.resource.path))"
                  @path-change="setFileEditorPath(paneId, editor, $event)"
                  :ref="editorRefCallback(editor, 'workspaceFile')"
                  :project-id="editor.resource.projectId"
                  :path="editor.resource.path"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="contentActive"
                  @ready="() => { ready(); void restoreEditorWorkState(editor.editorId); }"
                  @dirty-change="setWorkspaceFileEditorDirty(paneId, editor.editorId, $event)"
                />
                <WorkbenchAssetEditor
                  v-else-if="editor.resource.kind === 'asset' || editor.resource.kind === 'sceneObject'"
                  :ref="editorRefCallback(editor, 'asset')"
                  :editor="editor"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <WorkbenchViewEditor
                  v-else-if="editor.resource.kind === 'view'"
                  :ref="editorRefCallback(editor, 'view')"
                  :view-id="editor.resource.viewId"
                  :instance-id="editor.editorId"
                  @icon-change="workbenchViewIcons[editor.editorId] = $event"
                  @title-change="workbenchStore.updateEditor(WORKBENCH_WINDOW_ID, paneId, editor.editorId, { title: $event })"
                  :window-label="WORKBENCH_WINDOW_ID"
                  :owner-window="ownerWindow"
                  @activate="focusWorkbenchEditor(paneId, editor.editorId)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :active="group.activeEditorId === editor.editorId"
                />
                <ViewPackageView
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'views'"
                  :project-id="editor.resource.projectId"
                  :working-dir="editorWorkingDir(editor)"
                  :workspace-ref="editorWorkspaceRef(editor)"
                />
                <WorkbenchSecondarySidebar
                  v-else-if="editor.resource.kind === 'section' && editor.resource.section === 'archived'"
                  :title="t('app.tab.archived')" @close="closeWorkbenchEditor(paneId, editor.editorId)"
                >
                  <template #default="{ toolbarTarget }">
                <WorkbenchArchivedSessionsEditor :toolbar-target="toolbarTarget"
                  :project-id="editor.resource.projectId"
                  :workspace-ref="editorWorkspaceRef(editor)"
                  :items="archivedTreeItems"
                  :refresh-key="archivedRefreshKey"
                  @sessions-change="archivedSessions = $event"
                  @activate="activateItem"
                  @contextmenu="openContextMenu"
                  @session-action="archiveSessionItem($event as DevelopmentTreeItem)"
                  @drag-pointer-down="onDragPointerDown"
                  @rename-change="sessionInlineRename && (sessionInlineRename.value = $event)"
                  @rename-submit="submitSessionRename"
                  @rename-cancel="cancelSessionRename"
                  :active="group.activeEditorId === editor.editorId"
                />
                  </template>
                </WorkbenchSecondarySidebar>
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
                  @path-change="setFileEditorPath(paneId, editor, $event)"
                  :ref="editorRefCallback(editor, 'workspaceFile')"
                  :project-id="editor.resource.projectId"
                  :path="editor.sourcePath"
                  :active="contentActive"
                  @ready="() => { ready(); void restoreEditorWorkState(editor.editorId); }"
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
              </template>
              <template #empty>
                <div class="workbench-editor-empty">
                  <BaseButton
                    v-if="!props.fixedWorkspaceRef && !workspaceContextStore.focusedCheckout"
                    @click="browseWorkspace"
                  >{{ t('development.openWorkspace') }}</BaseButton>
                  <template v-else>{{ t('workbench.empty') }}</template>
                </div>
              </template>
            </WorkbenchEditorStack>
          </template>
        </template>
      </WorkbenchSplitHost>
    </main>

    <section
      v-if="collabOverlayPage || agentOverlayPage"
      v-show="workspaceOverlayShown"
      class="development-workspace-overlay"
      data-native-surface-occluder
      data-search-boundary
      :aria-label="workspaceOverlayTitle"
    >
      <header class="development-workspace-overlay-header">
        <LucideIcon
          class="development-workspace-overlay-icon"
          :icon="workspaceOverlay?.section === 'agents' ? Bot : GitMerge"
          :size="14"
        />
        <span class="development-workspace-overlay-title">{{ workspaceOverlayTitle }}</span>
        <span
          v-if="workspaceOverlaySubtitle"
          class="development-workspace-overlay-subtitle"
          :title="workspaceOverlaySubtitle.title"
        >{{ workspaceOverlaySubtitle.label }}</span>
        <div ref="collabOverlayToolbarTarget" class="development-workspace-overlay-actions" />
        <BaseButton
          class="development-workspace-overlay-close"
          :title="t('common.close')"
          :aria-label="t('common.close')"
          @click="closeWorkspaceOverlay"
        >
          <LucideIcon :icon="X" :size="14" />
        </BaseButton>
      </header>
      <div class="development-workspace-overlay-body">
        <WorkbenchCollabPage
          v-if="collabOverlayPage"
          v-show="workspaceOverlay?.section === 'collab'"
          :key="`collab:${collabOverlayPage.target.checkoutId}`"
          :working-dir="collabOverlayPage.root"
          :workspace-ref="collabOverlayPage.workspaceRef"
          :active="collabOverlayActive"
          :visible="collabOverlayActive"
          :toolbar-target="collabOverlayToolbarTarget"
          :selected-model-id="modelStore.selectedModelId"
          :selected-agent-id="agentStore.selectedAgentId"
          :models="modelStore.availableModels"
          @select-model="(id: string) => modelStore.selectModel(id)"
        />
        <AgentView
          v-if="agentOverlayPage"
          v-show="workspaceOverlay?.section === 'agents'"
          :key="`agents:${agentOverlayPage.target.checkoutId}`"
          :working-dir="agentOverlayPage.root"
          :workspace-ref="agentOverlayPage.workspaceRef"
          :active="agentOverlayActive"
        />
      </div>
    </section>
    </div>

    <WorktreeManager v-if="managedWorktreeSource" :source-root="managedWorktreeSource" @close="managedWorktreeSource = null" />
    <BaseContextMenu
      v-if="contextMenu"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :min-width="164"
      @close="contextMenu = null"
    >
      <template v-if="canSetTreeItemState(contextMenu.item)">
        <button type="button" @click="toggleContextTreeState('starred')">
          <LucideIcon :icon="Star" :size="13" />
          {{ t(contextTreeStateEnabled('starred') ? 'development.unstarItem' : 'development.starItem') }}
        </button>
        <button type="button" @click="toggleContextTreeState('pinned')">
          <LucideIcon :icon="contextTreeStateEnabled('pinned') ? PinOff : Pin" :size="13" />
          {{ t(contextTreeStateEnabled('pinned') ? 'development.unpinItem' : 'development.pinItem') }}
        </button>
        <div class="base-context-menu-separator" />
      </template>
      <template v-if="contextMenu.item.meta.kind === 'checkout' || contextMenu.item.meta.kind === 'collaboration'">
        <button type="button" @click="managedWorktreeSource = contextCheckout()?.root ?? null; contextMenu = null">
          <LucideIcon :icon="GitBranch" :size="13" />
          {{ t("worktrees.title") }}
        </button>
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
        <button type="button" @click="configureCheckoutWorkspace">
          <LucideIcon :icon="FolderCog" :size="13" />
          {{ t("workspace.setup.title") }}
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
          <template v-if="contextMenu.item.meta.archived">{{ t("chat.session.unarchive") }}</template>
          <template v-else-if="(contextMenu.sessionTargets?.length ?? 0) <= 1">
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
      <template v-else-if="(contextMenu.fileTargets?.length ?? 0) > 1">
        <button type="button" @click="runContextFileBatchAction('copy')">
          <LucideIcon :icon="Copy" :size="13" />{{ t("knowledge.explorer.copyRelativePath") }}
        </button>
        <div class="base-context-menu-separator" />
        <button v-if="contextHideFileTargets.length" type="button" @click="hideContextFiles">
          <LucideIcon :icon="EyeOff" :size="13" />{{ t("development.hideMany", contextHideFileTargets.length) }}
        </button>
        <button type="button" class="danger" @click="runContextFileBatchAction('delete')">
          <LucideIcon :icon="Trash2" :size="13" />{{ t("knowledge.explorer.deleteMany", contextMenu.fileTargets?.length ?? 0) }}
        </button>
      </template>
      <template v-else>
        <template v-if="contextCreateTarget">
          <button type="button" @click="beginCreateWorkspaceDocument('document')">
            <LucideIcon :icon="FilePlus" :size="13" />{{ t("knowledge.explorer.createDoc") }}
          </button>
          <button type="button" @click="beginCreateWorkspaceDocument('csv')">
            <LucideIcon :icon="Table2" :size="13" />{{ t("csv.new") }}
          </button>
          <button type="button" @click="beginCreateWorkspaceDocument('folder')">
            <LucideIcon :icon="FolderPlus" :size="13" />{{ t("development.newFolder") }}
          </button>
          <div v-if="contextFileTarget" class="base-context-menu-separator" />
        </template>
        <template v-if="contextMenu.item.meta.kind === 'knowledge'">
          <button type="button" @click="selectContextKnowledgeInList">
            <LucideIcon :icon="BookOpen" :size="13" />
            {{ t("development.selectInKnowledgeList") }}
          </button>
          <div class="base-context-menu-separator" />
        </template>
        <template v-if="contextFileTarget">
          <ResourceFileMenuItems :target="contextFileTarget" @action="runContextResourceAction" @close="contextMenu = null" />
          <div v-if="contextMenu.item.meta.kind !== 'workspaceFile'" class="base-context-menu-separator" />
        </template>
        <template v-if="contextViewTarget">
          <ViewResourceMenuItems :view="contextViewTarget.view" :busy="viewResourceActions?.busy" @action="runContextViewAction" />
          <div class="base-context-menu-separator" />
        </template>
        <template v-if="contextMenu.item.meta.kind === 'workspaceFile' && (!contextMenu.item.meta.archivedFile || contextCanRestoreFile)">
          <div class="base-context-menu-separator" />
          <button v-if="contextCanRestoreFile" type="button" @click="setContextFileHidden(false)">
            <LucideIcon :icon="Eye" :size="13" />{{ t('development.archivedFiles.restore') }}
          </button>
          <button v-else type="button" @click="setContextFileHidden(true)">
            <LucideIcon :icon="EyeOff" :size="13" />{{ t('development.hideNode') }}
          </button>
        </template>
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
          v-if="contextCanEditLayout"
          type="button"
          @click="beginCreateFolder"
        >
          <LucideIcon :icon="FolderPlus" :size="13" />
          {{ t("development.newFolder") }}
        </button>
        <button
          v-if="contextCanEditLayout && secondaryNavigation?.section !== 'sessions'"
          type="button"
          @click="addLocalFiles"
        >
          <LucideIcon :icon="File" :size="13" />
          {{ t("development.addFiles") }}
        </button>
        <button
          v-if="contextCanEditLayout && secondaryNavigation?.section !== 'sessions'"
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
          v-if="(contextMenu.item.meta.explorerNode?.sourcePath && !contextMenu.item.meta.mountEntry) || contextMenu.item.meta.kind === 'view'"
          type="button"
          @click="removeContextMountedNode"
        >
          <LucideIcon :icon="Trash2" :size="13" />
          {{ t(contextMenu.item.meta.kind === 'view' ? "development.removeFromWorkspace" : "development.removeMount") }}
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
        <template v-if="contextCanManageWorkspaces">
          <div v-if="contextCreateTarget || contextCanEditLayout" class="base-context-menu-separator" />
          <button type="button" @click="browseWorkspace">
            <LucideIcon :icon="FolderOpen" :size="13" />
            {{ t("development.addWorkspace") }}
          </button>
          <button v-if="contextMenu.item.treeRow" type="button" @click="removeContextWorkspace">
            <LucideIcon :icon="X" :size="13" />
            {{ t("development.removeWorkspace") }}
          </button>
        </template>
      </template>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="displayMenu"
      :x="displayMenu.x"
      :y="displayMenu.y"
      :min-width="176"
      @close="displayMenu = null"
    >
      <template v-if="navigationModeCheckout">
        <button type="button" role="menuitemradio" :aria-checked="navigationMode === 'sessions'" @click="setNavigationMode('sessions')">
          <LucideIcon :icon="navigationMode === 'sessions' ? Check : MessageSquare" :size="13" />
          {{ t('development.navigationMode.sessions') }}
        </button>
        <button type="button" role="menuitemradio" :aria-checked="navigationMode === 'files'" @click="setNavigationMode('files')">
          <LucideIcon :icon="navigationMode === 'files' ? Check : Folder" :size="13" />
          {{ t('development.navigationMode.files') }}
        </button>
        <div class="base-context-menu-separator" />
      </template>
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
      <template v-if="!props.fixedWorkspaceRef && !props.singleWorkspaceOnly">
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
        :aria-expanded="specialNodesMenu?.kind === 'visibility'"
        :disabled="specialNodeVisibilityItems.length === 0"
        @click="openSpecialNodesMenu"
        @focus="openSpecialNodesMenu"
        @mouseenter="openSpecialNodesMenu"
      >
        <LucideIcon :icon="Eye" :size="13" />
        {{ t("development.specialNodeVisibility") }}
        <LucideIcon class="development-submenu-chevron" :icon="ChevronRight" :size="13" />
      </button>
      <button
        v-if="topBarNodesEnabled"
        type="button"
        class="development-submenu-trigger"
        aria-haspopup="menu"
        :aria-expanded="specialNodesMenu?.kind === 'topBar'"
        :disabled="topBarMenuItems.length === 0"
        @click="openSpecialNodesMenu($event, 'topBar')"
        @focus="openSpecialNodesMenu($event, 'topBar')"
        @mouseenter="openSpecialNodesMenu($event, 'topBar')"
      >
        <LucideIcon :icon="PanelTop" :size="13" />
        {{ t("development.topBarNodes") }}
        <LucideIcon class="development-submenu-chevron" :icon="ChevronRight" :size="13" />
      </button>
      <button
        v-if="workspaceKnowledgeEnabled(presetProjectId)"
        type="button"
        role="menuitemcheckbox"
        :aria-checked="displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments"
        :title="t('settings.display.autoPlaceNewPlanDesignKnowledgeDocuments')"
        @mouseenter="specialNodesMenu = null"
        @click="setDisplaySetting('autoPlaceNewPlanDesignKnowledgeDocuments', !displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments)"
      >
        <LucideIcon :icon="displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments ? Check : BookOpen" :size="13" />
        {{ t("development.autoShowNewDocuments") }}
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
      <div v-if="props.titleBarOptional || props.toolbarTarget" class="base-context-menu-separator" />
      <button
        v-if="props.titleBarOptional"
        type="button"
        role="menuitemcheckbox"
        :aria-checked="workspaceWindowTitleBar"
        @mouseenter="specialNodesMenu = null"
        @click="toggleWorkspaceWindowTitleBar"
      >
        <LucideIcon :icon="workspaceWindowTitleBar ? Check : PanelTop" :size="13" />
        {{ t("development.showTitleBar") }}
      </button>
      <template v-if="props.toolbarTarget">
        <button
          v-if="displaySettings.showPluginsTab"
          type="button"
          @mouseenter="specialNodesMenu = null"
          @click="openAppPageWindow('plugins')"
        >
          <LucideIcon :icon="Puzzle" :size="13" />
          {{ t("app.tab.plugins") }}
        </button>
        <button type="button" @mouseenter="specialNodesMenu = null" @click="openAppPageWindow('settings')">
          <LucideIcon :icon="Settings" :size="13" />
          {{ t("app.tab.settings") }}
        </button>
      </template>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="displayMenu && specialNodesMenu"
      :x="specialNodesMenu.x"
      :y="specialNodesMenu.y"
      :min-width="168"
      :z-index="10001"
      :show-backdrop="false"
      :aria-label="t(specialNodesMenu.kind === 'topBar' ? 'development.topBarNodes' : 'development.specialNodeVisibility')"
      @close="specialNodesMenu = null"
    >
      <template v-if="specialNodesMenu.kind === 'topBar'">
        <button
          v-for="item in topBarMenuItems"
          :key="item.resourceId"
          type="button"
          role="menuitemcheckbox"
          :aria-checked="item.inTopBar"
          @click="setTopBarNodePlacement(item.id, !item.inTopBar)"
        >
          <LucideIcon :icon="item.icon" :size="13" />
          <span class="development-special-node-label">{{ t(item.labelKey) }}</span>
          <LucideIcon
            class="development-menu-check"
            :class="{ visible: item.inTopBar }"
            :icon="Check"
            :size="13"
          />
        </button>
      </template>
      <button
        v-for="item in specialNodesMenu.kind === 'visibility' ? specialNodeVisibilityItems : []"
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
      @click.capture="recentWorkspaceContextMenu = null"
      @close="workspaceMenu = null"
    >
      <template v-for="path in projectStore.recentDirs" :key="path">
        <button
          type="button"
          class="development-recent-workspace"
          :class="{ active: isCurrentWorkspacePath(path) }"
          :title="path"
          @click="selectRecentWorkspace(path)"
          @contextmenu.prevent.stop="openRecentWorkspaceContextMenu(path, $event)"
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
      <button type="button" @click="browseWorkspace">
        <LucideIcon :icon="FolderOpen" :size="13" />
        {{ t("development.openWorkspace") }}
      </button>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="workspaceMenu && recentWorkspaceContextMenu"
      :x="recentWorkspaceContextMenu.x"
      :y="recentWorkspaceContextMenu.y"
      :z-index="10001"
      :show-backdrop="false"
      @close="recentWorkspaceContextMenu = null"
    >
      <button type="button" role="menuitem" @click="configureRecentWorkspace">
        <LucideIcon :icon="FolderCog" :size="13" />
        {{ t("workspace.setup.title") }}
      </button>
      <button type="button" role="menuitem" @click="openRecentWorkspaceInNewWindow">
        <LucideIcon :icon="AppWindow" :size="13" />
        {{ t("app.dir.openInNewWindow") }}
      </button>
      <button type="button" role="menuitem" @click="removeRecentWorkspace">
        <LucideIcon :icon="X" :size="13" />
        {{ t("app.dir.removeRecent") }}
      </button>
    </BaseContextMenu>

    <ExplorerResourceActions ref="resourceActions" @changed="resourceFileChanged" @history="openFileHistory" />
    <ViewResourceActions ref="viewResourceActions" @changed="viewResourceChanged" />
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
.file-history-secondary-content {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.development-workbench {
  position: relative;
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

.development-explorer-toolbar.is-window-titlebar {
  -webkit-app-region: no-drag;
  flex: 0 0 auto;
  min-width: 0;
  max-width: 100%;
  height: 100%;
  border-bottom: 0;
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
  padding-top: 12px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.development-tree :deep(.workspace-tree-row) {
  gap: 6px;
  cursor: default;
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

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone .workspace-tree-row) {
  outline: 1px dashed var(--border-strong);
  outline-offset: -3px;
}

.development-tree :deep(.workspace-tree-row-shell.is-new-session-drop-zone.drop-target .workspace-tree-row) {
  outline-color: color-mix(in srgb, var(--accent-color) 72%, var(--border-strong));
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

.development-tree :deep(.workspace-tree-row-shell.is-session-flash::after),
.development-tree :deep(.workspace-tree-row-shell.is-reveal-flash::after) {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 1;
  background: color-mix(in srgb, var(--accent-color) 16%, transparent);
  opacity: 0;
  pointer-events: none;
  animation: development-session-row-flash 1200ms ease-out;
}

@keyframes development-session-row-flash {
  0%, 35% { opacity: 1; }
  100% { opacity: 0; }
}

@media (prefers-reduced-motion: reduce) {
  .development-tree :deep(.workspace-tree-row-shell.is-session-flash::after),
  .development-tree :deep(.workspace-tree-row-shell.is-reveal-flash::after) {
    animation: none;
  }
}

.development-tree :deep(.workspace-tree-row-shell.is-drag-source .workspace-tree-row) {
  cursor: grabbing;
}

.development-tree :deep(.workspace-tree-row-shell.is-hidden-node) {
  opacity: 0.52;
}





.development-tree :deep(.workspace-tree-row-shell.editing .workspace-tree-trailing) {
  display: none;
}

:where(.development-knowledge-remove-button, .development-view-hide-button) {
  position: absolute;
  top: 50%;
  right: 20px;
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

.development-tree :deep(.workspace-tree-row-shell:hover .development-knowledge-remove-button),
.development-knowledge-remove-button:focus-visible,
.development-tree :deep(.workspace-tree-row-shell.is-view-row:hover .development-view-hide-button),
.development-view-hide-button:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.development-tree :deep(.workspace-tree-trailing:has(.development-knowledge-remove-button)) {
  min-width: 40px;
}

.development-tree :deep(.workspace-tree-row-shell:has(.development-knowledge-remove-button):hover .development-knowledge-modified-time),
.development-tree :deep(.workspace-tree-row-shell:has(.development-knowledge-remove-button:focus-visible) .development-knowledge-modified-time) {
  opacity: 0;
}

:where(.development-knowledge-remove-button, .development-view-hide-button):hover,
:where(.development-knowledge-remove-button, .development-view-hide-button):focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.development-inline-create-row {
  display: flex;
  align-items: center;
  gap: 8px;
  box-sizing: border-box;
  height: var(--workspace-tree-row-height, 30px);
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
  height: calc(var(--workspace-tree-row-height, 30px) - 4px);
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

.development-unread-count {
  margin-right: 4px;
  color: var(--text-secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.development-branch-label,
.development-knowledge-modified-time {
  align-self: center;
  max-width: 88px;
  margin-right: 8px;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
}

.development-branch-label {
  font-family: var(--font-mono-identifier);
}
.development-knowledge-modified-time {
  font-family: var(--font-ui);
  font-variant-numeric: tabular-nums;
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

.development-stage {
  position: relative;
  flex: 1;
  display: flex;
  min-width: 0;
  min-height: 0;
}

/* Covered content keeps its layout, so closing the overlay restores it unchanged. */
.development-stage.has-workspace-overlay > .workbench-secondary-sidebar,
.development-stage.has-workspace-overlay > .development-editor {
  visibility: hidden;
}

.development-workspace-overlay {
  position: absolute;
  inset: 0;
  /* Below the explorer resize handle; covered content is already hidden. */
  z-index: 10;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--panel-bg);
}

.development-workspace-overlay-header {
  display: flex;
  align-items: center;
  flex: 0 0 34px;
  gap: 8px;
  min-width: 0;
  padding: 0 7px 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.development-workspace-overlay-icon {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.development-workspace-overlay-title {
  flex: 0 0 auto;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
}

.development-workspace-overlay-subtitle {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-workspace-overlay-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.development-workspace-overlay-close {
  width: 26px;
  min-width: 26px;
  height: 26px;
  padding: 0;
  border-color: transparent;
}

.development-workspace-overlay-body {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
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
