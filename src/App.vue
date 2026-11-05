
<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, shallowRef, onMounted, onUnmounted, watch } from "vue";
import type { Component, ShallowRef } from "vue";
import { AppWindow, PanelLeft } from "lucide";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { normalizeAppError } from "./services/errors";
import { startUnityNativeEmbedHosts, unityNativeEmbedHosts } from "./services/unityNativeEmbed";
import { useUiStore, type AppPage } from "./stores/ui";
import { useAuthStore } from "./stores/auth";
import { useAgentStore } from "./stores/agent";
import { useModelStore } from "./stores/model";
import { claudePrewarmSettings, prewarmClaudeSession } from "./services/claudeConnection";
import { protocolModelId } from "@locus/server-client";
import { claudeCodeCliEnabled } from "./services/claudeCodeCli";
import { useProjectStore } from "./stores/project";
import { useWorkspaceContextStore } from "./stores/workspaceContext";
import { useChatStore } from "./stores/chat";
import { useNotificationStore } from "./stores/notification";
import { useAppUpdateStore } from "./stores/appUpdate";
import { useAppBootstrap } from "./composables/useAppBootstrap";
import { watchSessionMaintenance } from "./composables/useSessionMaintenanceNotice";
import { bootstrapWorkspaceWindow } from "./composables/bootstrapWorkspaceWindow";
import { isWorkspaceWindowLabel, WORKSPACE_WINDOW_TITLE_BAR_SETTING, workspaceWindowQuery } from "./services/workspaceWindow";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAgentWorkspaceScope } from "./composables/useAgentWorkspaceScope";
import { useUnityAssetDropTarget } from "./composables/useUnityAssetDropTarget";
import { KNOWLEDGE_EMBEDDING_STATUS_EVENT, knowledgeGetEmbeddingStatus } from "./services/knowledge";
import { listenWorkspaceEvent } from "./services/workspaceEventHub";
import type { RoutedWorkspaceEvent } from "./services/project";
import { APP_CLOSE_REQUESTED_EVENT, getRunningTaskCount, requestAppExit } from "./services/system";
import { watchPrimaryWindow } from "./services/primaryWindow";

import TopBannerHost from "./components/TopBannerHost.vue";
import RemotePairingPrompt from "./components/RemotePairingPrompt.vue";
import BaseButton from "./components/ui/BaseButton.vue";
import BaseContextMenu from "./components/ui/BaseContextMenu.vue";
import LucideIcon from "./components/icons/LucideIcon.vue";
import AppUpdateModal from "./components/AppUpdateModal.vue";
import DevelopmentWorkbench from "./components/workbench/DevelopmentWorkbench.vue";
import {
  WORKSPACE_TOP_BAR_NODES_SETTING,
  isTopBarNodeId,
  topBarLayout,
  type TopBarEntryId,
} from "./components/workbench/workspaceTopBarNodes";
import { settingValue } from "./composables/useSettingsStore";
import InternalDragOverlay from "./components/ui/InternalDragOverlay.vue";
import WorkspaceSetupDialog from "./components/WorkspaceSetupDialog.vue";
import { requestWorkspaceSetup } from "./composables/useWorkspaceSetup";

import { provideDiffOverlay } from "./composables/useDiffOverlay";
import { provideInternalDragController } from "./composables/useInternalDrag";
import { initTheme } from "./composables/useTheme";
import { initFonts, useDisplaySettings } from "./composables/useDisplaySettings";
import { isKnowledgeDownloadWindowLocation } from "./services/knowledgeDownloadWindow";
import { isKnowledgeLexicalProgressWindowLocation } from "./services/knowledgeLexicalProgressWindow";
import { isFeishuReferenceImportWindowLocation } from "./services/feishuReferenceImportWindow";
import { isUnityReferenceImportWindowLocation } from "./services/unityReferenceImportWindow";
import { isReferenceExternalImportWindowLocation } from "./services/referenceExternalImportWindow";
import { isCollabSearchWindowLocation } from "./services/collabSearchWindow";
import { isChatDiffReviewWindowLocation } from "./services/chatDiffReviewWindow";
import { isPlanViewWindowLocation } from "./services/planViewWindow";
import { isUnityValueEditorWindowLocation } from "./services/unityValueEditorWindow";
import {
  isExtraWorkdirsWindowLocation,
  listenExtraWorkdirsUpdated,
} from "./services/extraWorkdirsWindow";
import { prepareSubWindowPool } from "./services/subWindow";
import {
  prepareSharedWorkbenchWindowPool,
  restoreSharedWorkbenchWindows,
  sharedWorkbenchWindowHosts,
} from "./services/sharedWorkbenchWindow";
import {
  isAppWorkspacePageId,
  isCheckoutWorkspacePageId,
  isWorkspacePageWindowLocation,
  openWorkspacePageWindow,
  WORKSPACE_PAGE_RESET_ONBOARDING_EVENT,
} from "./services/workspacePageWindow";
import {
  canStartWindowDragFromTarget,
  getCurrentTauriWindowLabel,
  showCurrentTauriWindow,
  startCurrentWindowDragging,
} from "./services/tauriRuntime";
import { markStartupPhase } from "./services/startupPerf";
import { initializeWorkState, revealWorkStateWindow, finishWorkStateRestore } from "./services/workState";
import { currentUnityEmbedWorkspaceRef } from "./services/unity";
const isUnityEmbedTestWindow = window.location.pathname === "/unity-embed-test";
const isUnityEmbedWindow = !isUnityEmbedTestWindow && window.location.pathname === "/unity-embed";
const unityEmbedParams = new URLSearchParams(window.location.search);
const unityEmbedTarget = unityEmbedParams.get("target") || "session";
const unityEmbedTargetId = unityEmbedParams.get("id") || "";
const unityEmbedWindowId = getCurrentTauriWindowLabel() || "unity-embed";
const isUnityEmbedViewWindow = isUnityEmbedWindow && unityEmbedTarget === "view";
const isKnowledgeDownloadWindow = isKnowledgeDownloadWindowLocation();
const isKnowledgeLexicalProgressWindow = isKnowledgeLexicalProgressWindowLocation();
const isFeishuReferenceImportWindow = isFeishuReferenceImportWindowLocation();
const isUnityReferenceImportWindow = isUnityReferenceImportWindowLocation();
const isReferenceExternalImportWindow = isReferenceExternalImportWindowLocation();
const isCollabSearchWindow = isCollabSearchWindowLocation();
const isChatDiffReviewWindow = isChatDiffReviewWindowLocation();
const isWorkspacePageWindow = isWorkspacePageWindowLocation();
const isPlanViewWindow = isPlanViewWindowLocation();
const isUnityValueEditorWindow = isUnityValueEditorWindowLocation();
const isExtraWorkdirsWindow = isExtraWorkdirsWindowLocation();
const applicationWindowId = getCurrentTauriWindowLabel() || "main";
const isWorkspaceWindow = isWorkspaceWindowLabel(applicationWindowId);
// True in exactly one application window, the oldest still open: it hosts the
// process-wide renderer duties. See services/primaryWindow.ts.
const isPrimaryWindow = ref(false);
const isStandaloneWindow = isUnityEmbedWindow
  || isUnityEmbedTestWindow
  || isKnowledgeDownloadWindow
  || isKnowledgeLexicalProgressWindow
  || isFeishuReferenceImportWindow
  || isUnityReferenceImportWindow
  || isReferenceExternalImportWindow
  || isCollabSearchWindow
  || isChatDiffReviewWindow
  || isWorkspacePageWindow
  || isPlanViewWindow
  || isUnityValueEditorWindow
  || isExtraWorkdirsWindow;

const KnowledgeDownloadProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeDownloadProgressWindow.vue"));
const KnowledgeLexicalProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeLexicalProgressWindow.vue"));
const FeishuReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/FeishuReferenceImportProgressWindow.vue"));
const UnityReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/UnityReferenceImportProgressWindow.vue"));
const ReferenceExternalImportWindow = defineAsyncComponent(() => import("./components/ReferenceExternalImportWindow.vue"));
const CollabSearchWindow = defineAsyncComponent(() => import("./components/CollabSearchWindow.vue"));
const ChatDiffReviewWindow = defineAsyncComponent(() => import("./components/ChatDiffReviewWindow.vue"));
const WorkspacePageWindow = defineAsyncComponent(() => import("./components/WorkspacePageWindow.vue"));
const PlanViewWindow = defineAsyncComponent(() => import("./components/PlanViewWindow.vue"));
const UnityValueEditorWindow = defineAsyncComponent(() => import("./components/UnityValueEditorWindow.vue"));
const ExtraWorkdirsConfigWindow = defineAsyncComponent(() => import("./components/ExtraWorkdirsConfigWindow.vue"));
const ViewHostWindow = defineAsyncComponent(() => import("./components/ViewHostWindow.vue"));
const UnityEmbeddedSessionView = defineAsyncComponent(() => import("./components/UnityEmbeddedSessionView.vue"));
const UnityNativeEmbedHost = defineAsyncComponent(() => import("./components/UnityNativeEmbedHost.vue"));
const UnityEmbedTestView = defineAsyncComponent(() => import("./components/UnityEmbedTestView.vue"));
const OnboardingView = defineAsyncComponent(() => import("./components/OnboardingView.vue"));
const FileDiffOverlay = defineAsyncComponent(() => import("./components/diff/FileDiffOverlay.vue"));
const WorkbenchWindow = defineAsyncComponent(() => import("./components/WorkbenchWindow.vue"));
const showPluginEntry = true;

initTheme(isUnityEmbedWindow ? "unityEmbed" : "main");
initFonts();

// -- Stores --
const uiStore = useUiStore();
const authStore = useAuthStore();
const agentStore = useAgentStore();
const modelStore = useModelStore();
const projectStore = useProjectStore();
const workspaceContextStore = useWorkspaceContextStore();
const chatStore = useChatStore();
const notificationStore = useNotificationStore();
const appUpdateStore = useAppUpdateStore();
const mainWorkspaceRestored = ref(false);
let startupClaudeConnectionRequested = false;
watch(() => [mainWorkspaceRestored.value, claudeCodeCliEnabled.value && claudePrewarmSettings.value.enabled,
  workspaceContextStore.focusedWorkspaceRef, modelStore.selectedModelId, modelStore.allModels] as const,
  ([restored, enabled, workspaceRef, selected, models]) => {
    if (isStandaloneWindow || startupClaudeConnectionRequested || !restored || !enabled || !workspaceRef) return;
    const model = protocolModelId(selected).startsWith("claude_code/") ? selected
      : (models.find(item => item.provider === "claude_code" && item.isDefault)
        ?? models.find(item => item.provider === "claude_code"))?.id ?? "claude_code/";
    startupClaudeConnectionRequested = true;
    // Start the configured server pool as soon as startup restores a workspace,
    // even when the current page is not Chat. An empty model suffix lets the
    // native CLI choose its default until the catalog is available.
    void prewarmClaudeSession(null, model, workspaceRef, true).catch(error => {
      startupClaudeConnectionRequested = false;
      console.warn("Claude startup connection failed", normalizeAppError(error).message);
    });
  }, { immediate: true });
const mainBootstrapError = ref("");
const workspaceWindowToolbarTarget = ref<HTMLElement | null>(null);
const topTabBar = ref<HTMLElement | null>(null);
const developmentWorkbench = ref<InstanceType<typeof DevelopmentWorkbench> | null>(null);
let mainBootstrapCompleted = false;
// Start recovery during setup, before the first mounted hook or auth request.
// Utility/embedded windows bind their own explicit workspace in their owner.
const mainWorkspaceReady = isStandaloneWindow ? undefined : (async () => {
  markStartupPhase("main_workspace_restore_start");
  await initializeWorkState(applicationWindowId);
  await bootstrapWorkspaceWindow(workspaceContextStore, applicationWindowId, window.location.search);
  mainWorkspaceRestored.value = true;
  if (!workspaceContextStore.focusedCheckout
    && Object.values(workspaceContextStore.checkoutsById).some((checkout) => checkout.available === false)) {
    notificationStore.addNotice("warning", t("workspace.unavailable"), {
      operation: "startup-workspace-unavailable",
      replaceOperation: true,
    });
  }
  markStartupPhase("main_workspace_restore_done");
})();
void mainWorkspaceReady?.catch((error) => {
  mainBootstrapError.value = normalizeAppError(error).message;
});
watch(() => workspaceContextStore.focusedRuntime, (runtime) => {
  if (!isWorkspaceWindow || !runtime || workspaceContextStore.windowId !== applicationWindowId) return;
  const query = workspaceWindowQuery({
    checkoutId: runtime.checkoutId,
    expectedGeneration: runtime.workspaceGeneration,
    expectedMaterializationEpoch: runtime.materializationEpoch,
  });
  window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
  const name = runtime.root.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || runtime.root;
  void getCurrentWindow().setTitle(`Locus - ${name}`).catch(() => {});
});
function retryMainStartup() {
  window.location.reload();
}
const openingRecoveryWorkspace = ref(false);
async function openRecoveryWorkspace() {
  if (openingRecoveryWorkspace.value) return;
  openingRecoveryWorkspace.value = true;
  try {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string" || !selected.trim()) return;
    if (!await requestWorkspaceSetup(selected)) return;
    // Retry the catalog/CAS snapshot if its original startup request failed.
    await workspaceContextStore.ensureInitialized(getCurrentTauriWindowLabel() || "main", "main");
    const context = await workspaceContextStore.openAndFocusInPane(
      selected, getCurrentTauriWindowLabel() || "main", "main",
    );
    if (context) window.location.reload();
  } catch (error) {
    mainBootstrapError.value = normalizeAppError(error).message;
  } finally {
    openingRecoveryWorkspace.value = false;
  }
}
const { state: displaySettings } = useDisplaySettings();
const {
  runtime: agentWorkspaceRuntime,
  workingDir: agentWorkingDir,
  workspaceRef: agentWorkspaceRef,
} = useAgentWorkspaceScope();
const unityEmbedBootstrapped = ref(false);
const unityEmbedBootstrapError = ref<string | null>(null);
const KNOWLEDGE_RUNTIME_LOADING_OPERATION = "knowledgeEmbeddingRuntimeLoading";
let knowledgeRuntimeStatusRequestSeq = 0;
let knowledgeRuntimeStatusUnlisten: (() => void) | null = null;
let knowledgeRuntimeStatusListening = false;
let appCloseRequestUnlisten: UnlistenFn | null = null;
let unityNativeEmbedRelease: (() => void) | null = null;
let extraWorkdirsUpdatedUnlisten: UnlistenFn | null = null;
let workspacePageResetOnboardingUnlisten: UnlistenFn | null = null;
let sessionMaintenanceRelease: (() => void) | null = null;
let primaryWindowUnlisten: UnlistenFn | null = null;
let sessionMaintenanceStarted = false;
let unityNativeEmbedStarting: Promise<void> | null = null;

function startSessionMaintenance() {
  if (!isPrimaryWindow.value || sessionMaintenanceStarted) return;
  sessionMaintenanceStarted = true;
  void watchSessionMaintenance().then((release) => {
    sessionMaintenanceRelease = release;
  });
}

function startUnityNativeEmbed(): Promise<void> {
  if (!isPrimaryWindow.value) return Promise.resolve();
  unityNativeEmbedStarting ??= startUnityNativeEmbedHosts().then(
    (release) => {
      unityNativeEmbedRelease = release;
    },
    (error) => {
      unityNativeEmbedStarting = null;
      throw error;
    },
  );
  return unityNativeEmbedStarting;
}

// A window promoted after startup (the previous primary closed) picks up the
// duties it was not running. Before bootstrap completes, startup does it.
watch(isPrimaryWindow, (primary) => {
  if (!primary || !mainBootstrapCompleted) return;
  startSessionMaintenance();
  void startUnityNativeEmbed().catch((error) => {
    console.warn("[unity-embed] failed to take over embedded workbench hosting", error);
  });
});

// -- Diff overlay provider (must be called in App setup so all children can inject) --
const diffOverlay = provideDiffOverlay();
const internalDragController = provideInternalDragController();
onUnmounted(() => internalDragController.dispose());
const {
  bootstrapCritical,
  bootstrapDeferred,
  preloadTabsInBackground,
  registerListeners,
  cleanup,
  refreshAfterSettings,
  onOnboardingCompleted: handleOnboardingCompleted,
} = useAppBootstrap({
  workspaceReady: mainWorkspaceReady,
  // Follows the primary window, which moves when the first window closes.
  handleExternalScriptOpen: () => !isUnityEmbedWindow && !isStandaloneWindow && isPrimaryWindow.value,
});

async function bindUnityEmbedWorkspace() {
  const workspaceRef = currentUnityEmbedWorkspaceRef();
  if (!workspaceRef) {
    throw new Error("The Unity embed URL is missing its checkout workspace scope.");
  }

  await workspaceContextStore.initialize(unityEmbedWindowId, "main");
  const restoredRef = workspaceContextStore.focusedWorkspaceRef;
  if (
    !restoredRef
    || restoredRef.checkoutId !== workspaceRef.checkoutId
    || restoredRef.expectedGeneration !== workspaceRef.expectedGeneration
  ) {
    const context = await workspaceContextStore.focusWorkspaceRef(workspaceRef);
    if (!context) {
      throw new Error("The Unity embed workspace focus request was superseded.");
    }
  }

  const focusedRef = workspaceContextStore.focusedWorkspaceRef;
  if (
    !focusedRef
    || focusedRef.checkoutId !== workspaceRef.checkoutId
    || focusedRef.expectedGeneration !== workspaceRef.expectedGeneration
    || !workspaceContextStore.focusedRoot
  ) {
    throw new Error("The Unity embed window could not restore its workspace scope.");
  }
}
const {
  handleUnityAssetDrag: handleMainUnityAssetDrag,
  handleUnityAssetDrop: handleMainUnityAssetDrop,
} = useUnityAssetDropTarget({
  enabled: () => !isStandaloneWindow,
});

function createLazyViewState(
  loader: () => Promise<{ default: Component }>,
  operation: string,
) {
  const component: ShallowRef<Component | null> = shallowRef(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let pending: Promise<void> | null = null;

  function ensureLoaded() {
    if (component.value) {
      return Promise.resolve();
    }
    if (pending) {
      return pending;
    }

    loading.value = true;
    error.value = null;
    pending = loader()
      .then((module) => {
        component.value = module.default;
      })
      .catch((loadError: unknown) => {
        const err = normalizeAppError(loadError);
        error.value = err.message;
        notificationStore.addNotice("error", err.message, {
          code: err.code,
          operation,
        });
        pending = null;
        throw loadError;
      })
      .finally(() => {
        loading.value = false;
      });

    return pending;
  }

  return {
    component,
    loading,
    error,
    ensureLoaded,
  };
}

const pluginView = createLazyViewState(
  () => import("./components/PluginView.vue"),
  "loadPluginView",
);
const agentView = createLazyViewState(
  () => import("./components/AgentView.vue"),
  "loadAgentView",
);
const settingsView = createLazyViewState(
  () => import("./components/SettingsView.vue"),
  "loadSettingsView",
);

const pluginViewComponent = pluginView.component;
const pluginViewLoading = pluginView.loading;
const pluginViewError = pluginView.error;

const agentViewComponent = agentView.component;
const agentViewLoading = agentView.loading;
const agentViewError = agentView.error;

const settingsViewComponent = settingsView.component;
const settingsViewLoading = settingsView.loading;
const settingsViewError = settingsView.error;

type ProcessTab = AppPage;

interface TopTabItem {
  id: ProcessTab;
  labelKey: string;
  visible: boolean;
}

const topTabs = computed<TopTabItem[]>(() => [
  { id: "development", labelKey: "app.tab.development", visible: true },
  { id: "plugins", labelKey: "app.tab.plugins", visible: showPluginEntry && displaySettings.showPluginsTab },
  { id: "settings", labelKey: "app.tab.settings", visible: true },
]);

const visibleTopTabs = computed(() => topTabs.value.filter((tab) => tab.visible));
// The default window always carries the page tabs. Workspace windows carry
// them only while they are the primary (oldest open) window, so the tabs stay
// reachable after the default window closes without pinning a main window.
// Other workspace windows show them too when the user turns their title bar on.
const workspaceWindowTitleBarOptional = computed(() => isWorkspaceWindow && !isPrimaryWindow.value);
const showTopTabs = computed(() => !workspaceWindowTitleBarOptional.value
  || settingValue(WORKSPACE_WINDOW_TITLE_BAR_SETTING) === true);
const topTabContextMenu = ref<{ x: number; y: number; tab: TopTabItem } | null>(null);

// Workspace system nodes the workbench moved into the bar (single-workspace
// mode), shown in the stored order between the page tabs.
const topBarNodes = computed(() => developmentWorkbench.value?.topBarNodeEntries ?? []);
const activeTopBarNode = computed(() => (
  uiStore.activePage === "development" ? topBarNodes.value.find((node) => node.active) ?? null : null
));
interface TopBarRenderedEntry {
  id: TopBarEntryId;
  tab: TopTabItem | null;
  node: { id: string; label: string; active: boolean } | null;
}
const topBarEntries = computed<TopBarRenderedEntry[]>(() => {
  const nodes = new Map(topBarNodes.value.map((node) => [node.id, node]));
  return topBarLayout(settingValue(WORKSPACE_TOP_BAR_NODES_SETTING)).flatMap((id): TopBarRenderedEntry[] => {
    if (isTopBarNodeId(id)) {
      const node = nodes.get(id);
      return node ? [{ id, tab: null, node }] : [];
    }
    const tab = visibleTopTabs.value.find((candidate) => candidate.id === id);
    return tab ? [{ id, tab, node: null }] : [];
  });
});
/** Where a dragged node would land: before this entry, `null` after the last, `undefined` when not over the bar. */
const topBarDropBefore = computed<TopBarEntryId | null | undefined>(() => developmentWorkbench.value?.topBarDropBefore);
const topBarNodeContextMenu = ref<{ x: number; y: number; id: string } | null>(null);

function isTopTabActive(tab: TopTabItem) {
  return uiStore.activePage === tab.id && !(tab.id === "development" && activeTopBarNode.value);
}

function onTopBarNodeClick(id: string) {
  const onDevelopment = uiStore.activePage === "development";
  if (!onDevelopment) uiStore.setPage("development");
  // From another page, show the node's page without closing it if it was open.
  void developmentWorkbench.value?.activateTopBarNode(id, onDevelopment);
}

function onTopBarNodePointerDown(event: PointerEvent, id: string) {
  developmentWorkbench.value?.startTopBarNodeDrag(id, event);
}

function openTopBarNodeContextMenu(event: MouseEvent, id: string) {
  event.preventDefault();
  event.stopPropagation();
  topBarNodeContextMenu.value = { x: event.clientX, y: event.clientY, id };
}

function moveTopBarNodeToTree() {
  const menu = topBarNodeContextMenu.value;
  topBarNodeContextMenu.value = null;
  if (menu) developmentWorkbench.value?.moveTopBarNodeToTree(menu.id);
}

function topTabWorkspaceRuntime(tab: TopTabItem) {
  return tab.id === "agent" ? agentWorkspaceRuntime.value : workspaceContextStore.focusedRuntime;
}

function canOpenTopTabInWindow(tab: TopTabItem) {
  return tab.id !== "development" && (
    isAppWorkspacePageId(tab.id)
    || (topTabWorkspaceRuntime(tab) !== null && isCheckoutWorkspacePageId(tab.id))
  );
}

async function openTopTabInWindow(tab: TopTabItem) {
  topTabContextMenu.value = null;
  if (!canOpenTopTabInWindow(tab)) return;
  try {
    const runtime = topTabWorkspaceRuntime(tab);
    if (runtime && isCheckoutWorkspacePageId(tab.id)) {
      await openWorkspacePageWindow({
        scope: "checkout",
        page: tab.id,
        title: `${shortDir(runtime.root)} · ${t(tab.labelKey)}`,
        checkoutId: runtime.checkoutId,
        workspaceGeneration: runtime.workspaceGeneration,
        materializationEpoch: runtime.materializationEpoch,
      });
    } else if (isAppWorkspacePageId(tab.id)) {
      await openWorkspacePageWindow({
        scope: "app",
        page: tab.id,
        title: t(tab.labelKey),
      });
    }
  } catch (cause) {
    const error = normalizeAppError(cause);
    notificationStore.addNotice("error", error.message, {
      code: error.code,
      operation: "openWorkspacePageWindow",
      skipConsoleLog: true,
    });
  }
}

function onTopTabClick(event: MouseEvent, tab: TopTabItem) {
  if (event.ctrlKey && canOpenTopTabInWindow(tab)) {
    void openTopTabInWindow(tab);
    return;
  }
  // Development means the editor groups, not a page a top bar node opened over them.
  if (tab.id === "development") developmentWorkbench.value?.closeTopBarNodePages();
  uiStore.setPage(tab.id);
}

function openTopTabContextMenu(event: MouseEvent, tab: TopTabItem) {
  if (!canOpenTopTabInWindow(tab)) return;
  event.preventDefault();
  event.stopPropagation();
  topTabContextMenu.value = { x: event.clientX, y: event.clientY, tab };
}

// 离开设置页时做一次兜底刷新（顶栏切 Tab 不走 setTab 之外的逻辑，原 closeSettings 的副作用迁移到这里）。
watch(() => uiStore.activePage, (tab, prev) => {
  if (prev === "settings" && tab !== "settings") void refreshAfterSettings();
});

watch(() => [uiStore.pluginsMounted, displaySettings.showPluginsTab] as const, ([mounted]) => {
  if (!showPluginEntry || !displaySettings.showPluginsTab || !mounted) return;
  void pluginView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.agentMounted, (mounted) => {
  if (!mounted) return;
  void agentView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.settingsMounted, (mounted) => {
  if (!mounted) return;
  void settingsView.ensureLoaded();
}, { immediate: true });

// A workspace window that hands the primary role to another window drops its
// tabs; return it to its workspace instead of leaving a page with no tabs.
watch(showTopTabs, (shown) => {
  if (!shown && uiStore.activePage !== "development") uiStore.setPage("development");
});

watch([() => uiStore.activePage, visibleTopTabs], () => {
  if (!showTopTabs.value && uiStore.activePage !== "development") {
    const requestedTab = topTabs.value.find((tab) => tab.id === uiStore.activePage);
    uiStore.setPage("development");
    // Keep this window on its workspace. Explicit settings/plugin actions can
    // still use the existing detached page, without leaving a page with no tabs.
    if (mainBootstrapCompleted && requestedTab) void openTopTabInWindow(requestedTab);
    return;
  }
  if (visibleTopTabs.value.some((tab) => tab.id === uiStore.activePage)) return;
  uiStore.setPage("development");
}, { immediate: true });

const appCloseConfirmOpen = ref(false);
const appCloseBusy = ref(false);
const appCloseRunningTaskCount = ref(0);
const runningSessionCount = computed(() => chatStore.streamingSessionIds.size);
const showAppUpdateModal = computed(() =>
  Boolean(
    appUpdateStore.updateInfo
    && !appUpdateStore.dialogDismissed
    && authStore.authChecked
    && !uiStore.showOnboarding,
  ),
);
const pluginToastLabel = computed(() => {
  if (projectStore.pluginToast === "missing") return t("app.plugin.notInstalled");
  if (projectStore.pluginToast === "outdated") return t("app.plugin.needUpdate");
  return "";
});
const pluginToastAction = computed(() => {
  if (!projectStore.pluginToast) return "";
  if (projectStore.pluginInstalling) return t("app.plugin.installing");
  return projectStore.pluginToast === "missing"
    ? t("app.plugin.clickInstall")
    : t("app.plugin.clickUpdate");
});
const pluginToastTitle = computed(() =>
  pluginToastLabel.value && pluginToastAction.value
    ? `${pluginToastLabel.value} - ${pluginToastAction.value}`
    : pluginToastLabel.value,
);
const appLayoutStyle = computed(() => {
  if (!uiStore.isWindowResizing || !uiStore.nativeWindowWidth || !uiStore.nativeWindowHeight) {
    return undefined;
  }
  return {
    width: `${uiStore.nativeWindowWidth}px`,
    height: `${uiStore.nativeWindowHeight}px`,
  };
});

function onTabBarPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.detail > 1) return;
  if (!canStartWindowDragFromTarget(event.target)) return;
  event.preventDefault();
  startCurrentWindowDragging();
}

function shortDir(dir: string): string {
  if (!dir) return t("app.dir.notSet");
  const parts = dir.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : dir;
}

function closeAppCloseDialog() {
  if (appCloseBusy.value) return;
  appCloseConfirmOpen.value = false;
  appCloseRunningTaskCount.value = 0;
}

function reportWorkingDirSwitchError(error: unknown) {
  const err = normalizeAppError(error);
  notificationStore.addNotice("error", err.message, {
    code: err.code,
    operation: "switchWorkingDir",
    replaceOperation: true,
    skipConsoleLog: true,
  });
}

function reportAppCloseError(error: unknown) {
  const err = normalizeAppError(error);
  notificationStore.addNotice("error", t("app.closeFailed", err.message), {
    code: err.code,
    operation: "requestAppExit",
    replaceOperation: true,
    skipConsoleLog: true,
  });
}

async function confirmAppClose() {
  if (appCloseBusy.value) return;
  appCloseBusy.value = true;
  try {
    await requestAppExit();
  } catch (error) {
    appCloseBusy.value = false;
    reportAppCloseError(error);
  }
}

async function handleAppCloseRequest() {
  if (isStandaloneWindow || appCloseBusy.value || appCloseConfirmOpen.value) return;
  const runningTaskCount = await getRunningTaskCount().catch(() => runningSessionCount.value);
  if (runningTaskCount > 0) {
    appCloseRunningTaskCount.value = runningTaskCount;
    appCloseConfirmOpen.value = true;
    return;
  }
  await confirmAppClose();
}

function onResetOnboarding() {
  projectStore.resetWorkspaceState();
  chatStore.resetWorkspaceScope();
  uiStore.resetOnboarding();
}

async function handleSettingsAuthChanged() {
  await authStore.loadProviderStatus();
  await modelStore.loadModelDefaults();
  await modelStore.loadCodexAvailableModels();
  modelStore.resolveSelectedModel(!chatStore.activeSessionId);
}

function closeAppUpdateModal() {
  appUpdateStore.dismissDialog();
}

async function openAppUpdateRelease() {
  const updateInfo = appUpdateStore.updateInfo;
  if (!updateInfo) return;

  try {
    await openUrl(updateInfo.releaseUrl);
    appUpdateStore.dismissDialog();
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", t("app.update.openFailed", err.message), {
      code: err.code,
      operation: "openAppUpdateRelease",
      skipConsoleLog: true,
    });
  }
}

function applyKnowledgeRuntimeLoadingStatus(activating: boolean) {
  if (!activating) {
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    return;
  }
  notificationStore.addNotice("info", t("knowledge.retrieval.runtimeStarting"), {
    operation: KNOWLEDGE_RUNTIME_LOADING_OPERATION,
    replaceOperation: true,
    spinner: true,
    sticky: true,
    skipConsoleLog: true,
  });
}

function knowledgeRuntimeNoticeApplies(): boolean {
  return !isStandaloneWindow
    && !!workspaceContextStore.focusedWorkspaceRef
    && workspaceContextStore.focusedCheckout?.knowledgeEnabled !== false;
}

async function refreshKnowledgeRuntimeLoadingStatus() {
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  const requestSeq = ++knowledgeRuntimeStatusRequestSeq;
  if (!workspaceRef || !knowledgeRuntimeNoticeApplies()) {
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    return;
  }

  try {
    const status = await knowledgeGetEmbeddingStatus(workspaceRef);
    const currentRef = workspaceContextStore.focusedWorkspaceRef;
    if (
      requestSeq !== knowledgeRuntimeStatusRequestSeq
      || currentRef?.checkoutId !== workspaceRef.checkoutId
      || currentRef.expectedGeneration !== workspaceRef.expectedGeneration
    ) return;
    applyKnowledgeRuntimeLoadingStatus(status.activating);
  } catch {
    if (requestSeq !== knowledgeRuntimeStatusRequestSeq) return;
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
  }
}

/** One snapshot for the focused workspace, then the backend pushes each
 * embedding status change (`knowledge-embedding-status`); nothing polls. */
function startKnowledgeRuntimeStatusUpdates() {
  if (isStandaloneWindow) return;
  void refreshKnowledgeRuntimeLoadingStatus();
  if (knowledgeRuntimeStatusListening) return;
  knowledgeRuntimeStatusListening = true;
  void listenWorkspaceEvent<RoutedWorkspaceEvent<{ activating: boolean }>>(
    "App.knowledgeRuntimeStatus",
    (event) => {
      const routed = event.payload;
      if (routed.eventName !== KNOWLEDGE_EMBEDDING_STATUS_EVENT) return;
      const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
      if (
        !workspaceRef
        || routed.checkoutId !== workspaceRef.checkoutId
        || (workspaceRef.expectedGeneration != null
          && routed.workspaceGeneration !== workspaceRef.expectedGeneration)
        || !knowledgeRuntimeNoticeApplies()
      ) return;
      // A pushed state supersedes a snapshot still in flight.
      knowledgeRuntimeStatusRequestSeq += 1;
      applyKnowledgeRuntimeLoadingStatus(routed.payload.activating);
    },
  ).then((release) => {
    if (knowledgeRuntimeStatusListening) knowledgeRuntimeStatusUnlisten = release;
    else release();
  }).catch(() => {
    knowledgeRuntimeStatusListening = false;
  });
}

function stopKnowledgeRuntimeStatusUpdates() {
  knowledgeRuntimeStatusListening = false;
  knowledgeRuntimeStatusRequestSeq += 1;
  knowledgeRuntimeStatusUnlisten?.();
  knowledgeRuntimeStatusUnlisten = null;
}

async function registerAppCloseRequestListener() {
  if (isStandaloneWindow || appCloseRequestUnlisten) return;
  try {
    appCloseRequestUnlisten = await getCurrentWindow().listen<void>(APP_CLOSE_REQUESTED_EVENT, () => {
      void handleAppCloseRequest();
    });
  } catch (error) {
    console.warn("Failed to listen for app close requests:", error);
  }
}

async function registerExtraWorkdirsUpdatedListener() {
  if (isStandaloneWindow || extraWorkdirsUpdatedUnlisten) return;
  try {
    extraWorkdirsUpdatedUnlisten = await listenExtraWorkdirsUpdated(({ workspacePath }) => {
      void projectStore.handleExtraWorkdirsUpdated(workspacePath);
    });
  } catch (error) {
    console.warn("Failed to listen for extra workdirs updates:", error);
  }
}

function revealApplicationWindow() {
  if (isStandaloneWindow) return;
  const currentTauriWindowLabel = getCurrentTauriWindowLabel();
  if (currentTauriWindowLabel && currentTauriWindowLabel !== "main" && !isWorkspaceWindow) return;
  markStartupPhase("main_window_show_start");
  void Promise.resolve(mainWorkspaceReady).then(() => revealWorkStateWindow(applicationWindowId))
    .then(() => {
      markStartupPhase("main_window_show_done");
    })
    .catch((error) => {
      markStartupPhase("main_window_show_error");
      console.warn("[startup] failed to show main window", error);
      void showCurrentTauriWindow();
    });
}

// -- Lifecycle --
onMounted(async () => {
  markStartupPhase("app_mounted", {
    window: isUnityEmbedWindow ? "unity_embed" : isStandaloneWindow ? "standalone" : "main",
    path: window.location.pathname,
  });
  await nextTick();
  setTimeout(revealApplicationWindow, 0);
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => markStartupPhase("app_next_frame"));
  } else {
    setTimeout(() => markStartupPhase("app_next_frame"), 0);
  }

  if (isUnityEmbedWindow) {
    try {
      markStartupPhase("unity_embed_workspace_bind_start");
      await bindUnityEmbedWorkspace();
      markStartupPhase("unity_embed_workspace_bind_done");
      markStartupPhase("unity_embed_bootstrap_critical_start");
      await bootstrapCritical();
      markStartupPhase("unity_embed_bootstrap_critical_done");
      markStartupPhase("unity_embed_register_listeners_start");
      await registerListeners();
      markStartupPhase("unity_embed_register_listeners_done");
    } catch (error) {
      const err = normalizeAppError(error);
      markStartupPhase("unity_embed_bootstrap_error", { code: err.code });
      unityEmbedBootstrapError.value = err.message;
      notificationStore.addNotice("error", err.message, {
        code: err.code,
        operation: "unityEmbedBootstrap",
      });
    } finally {
      unityEmbedBootstrapped.value = true;
    }
    return;
  }
  if (isStandaloneWindow) {
    markStartupPhase("standalone_window_ready");
    return;
  }
  // Known before listeners and bootstrap start, so startup requests (an
  // external script open) reach the right window.
  primaryWindowUnlisten = await watchPrimaryWindow(applicationWindowId, (primary) => {
    isPrimaryWindow.value = primary;
  });
  await registerAppCloseRequestListener();
  await registerExtraWorkdirsUpdatedListener();
  workspacePageResetOnboardingUnlisten = await listen(
    WORKSPACE_PAGE_RESET_ONBOARDING_EVENT,
    onResetOnboarding,
  );
  startSessionMaintenance();
  markStartupPhase("main_dom_listeners_ready");
  markStartupPhase("main_bootstrap_critical_start");
  try {
    await bootstrapCritical();
    markStartupPhase("main_bootstrap_critical_done");
    mainBootstrapCompleted = true;
  } catch (error) {
    mainBootstrapError.value = normalizeAppError(error).message;
    reportWorkingDirSwitchError(error);
    return;
  }
  // Development is already interactive. Preload the remaining process-level pages.
  if (!isWorkspaceWindow) preloadTabsInBackground([
    settingsView.ensureLoaded,
    pluginView.ensureLoaded,
    agentView.ensureLoaded,
  ]);
  markStartupPhase("main_preload_tabs_scheduled");
  void bootstrapDeferred();
  markStartupPhase("main_bootstrap_deferred_scheduled");
  await startUnityNativeEmbed();
  // Each renderer recreates only the floating windows it originally owned.
  void restoreSharedWorkbenchWindows()
    .then(() => finishWorkStateRestore())
    .then(() => prepareSharedWorkbenchWindowPool())
    .catch((error) => {
      console.warn("[workbench-window] restore or pool warmup failed", error);
    });
  if (isPrimaryWindow.value) void appUpdateStore.checkForUpdates({ silent: true });
  markStartupPhase("main_update_check_scheduled");
  startKnowledgeRuntimeStatusUpdates();
  // Pre-warm the shared sub-window pool once startup work settled so the
  // first plan/diff/import window opens from an already-loaded shell.
  window.setTimeout(() => {
    void prepareSubWindowPool().catch(() => {
      /* pool is an optimization; opens fall back to direct creation */
    });
  }, 3000);
  markStartupPhase("main_startup_done");
});

onUnmounted(() => {
  unityNativeEmbedRelease?.();
  if (isUnityEmbedWindow) {
    cleanup();
    return;
  }
  if (isStandaloneWindow) return;
  primaryWindowUnlisten?.();
  primaryWindowUnlisten = null;
  appCloseRequestUnlisten?.();
  appCloseRequestUnlisten = null;
  extraWorkdirsUpdatedUnlisten?.();
  extraWorkdirsUpdatedUnlisten = null;
  workspacePageResetOnboardingUnlisten?.();
  workspacePageResetOnboardingUnlisten = null;
  sessionMaintenanceRelease?.();
  sessionMaintenanceRelease = null;
  notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
  stopKnowledgeRuntimeStatusUpdates();
  cleanup();
});

watch(() => workspaceContextStore.focusedWorkspaceRef, () => {
  if (workspaceContextStore.focusedWorkspaceRef) {
    notificationStore.clearByOperation("startup-workspace-unavailable");
  }
  startKnowledgeRuntimeStatusUpdates();
});
</script>

<template>
  <template v-if="isUnityEmbedViewWindow">
    <div v-if="unityEmbedBootstrapError" class="app-startup-state">{{ unityEmbedBootstrapError }}</div>
    <div v-else-if="!unityEmbedBootstrapped" class="app-startup-state">{{ t("common.loading") }}</div>
    <ViewHostWindow v-else embedded />
  </template>
  <UnityEmbeddedSessionView
    v-else-if="isUnityEmbedWindow"
    :bootstrapped="unityEmbedBootstrapped"
    :bootstrap-error="unityEmbedBootstrapError"
    :initial-session-id="unityEmbedTargetId"
    :window-id="unityEmbedWindowId"
  />
  <UnityEmbedTestView v-else-if="isUnityEmbedTestWindow" />
  <KnowledgeDownloadProgressWindow v-else-if="isKnowledgeDownloadWindow" />
  <KnowledgeLexicalProgressWindow v-else-if="isKnowledgeLexicalProgressWindow" />
  <FeishuReferenceImportProgressWindow v-else-if="isFeishuReferenceImportWindow" />
  <UnityReferenceImportProgressWindow v-else-if="isUnityReferenceImportWindow" />
  <ReferenceExternalImportWindow v-else-if="isReferenceExternalImportWindow" />
  <CollabSearchWindow v-else-if="isCollabSearchWindow" />
  <ChatDiffReviewWindow v-else-if="isChatDiffReviewWindow" />
  <WorkspacePageWindow v-else-if="isWorkspacePageWindow" />
  <PlanViewWindow v-else-if="isPlanViewWindow" />
  <UnityValueEditorWindow v-else-if="isUnityValueEditorWindow" />
  <ExtraWorkdirsConfigWindow v-else-if="isExtraWorkdirsWindow" />
  <div v-else-if="mainBootstrapError" class="app-startup-state">
    <span>{{ mainBootstrapError }}</span>
    <BaseButton size="sm" :disabled="openingRecoveryWorkspace" @click="openRecoveryWorkspace">{{ t("development.openWorkspace") }}</BaseButton>
    <BaseButton size="sm" @click="retryMainStartup">{{ t("common.refresh") }}</BaseButton>
  </div>
  <div v-else-if="!authStore.authChecked || !mainWorkspaceRestored" class="app-startup-state">
    <span>{{ t("common.loading") }}</span>
  </div>
  <OnboardingView v-else-if="authStore.authChecked && uiStore.showOnboarding" @completed="handleOnboardingCompleted" />
  <div
    class="app-layout"
    :class="{ 'is-window-resizing': uiStore.isWindowResizing }"
    :style="appLayoutStyle"
    v-else-if="authStore.authChecked"
    @contextmenu.prevent
    @dragenter.capture="handleMainUnityAssetDrag"
    @dragover.capture="handleMainUnityAssetDrag"
    @drop.capture="handleMainUnityAssetDrop"
  >
    <div class="main-area">
      <div
        ref="topTabBar"
        class="tab-bar"
        :class="{ 'is-workspace-window': !showTopTabs, 'is-node-drop-target': topBarDropBefore !== undefined }"
        @pointerdown="onTabBarPointerDown"
      >
        <div class="tab-drag-region" aria-hidden="true"></div>
        <!-- Workspace windows without the page tabs move the explorer toolbar
             into the title bar; with the tabs it stays in the workspace below. -->
        <div v-if="!showTopTabs" ref="workspaceWindowToolbarTarget" class="workspace-window-toolbar-host"></div>
        <template v-else>
        <span class="tab-brand">Locus</span>
        <template v-for="(entry, index) in topBarEntries" :key="entry.id">
          <button
            v-if="entry.tab"
            class="tab-item"
            :class="{
              active: isTopTabActive(entry.tab),
              'is-drop-before': topBarDropBefore === entry.id,
              'is-drop-after': topBarDropBefore === null && index === topBarEntries.length - 1,
            }"
            :data-top-bar-entry="entry.id"
            @click="onTopTabClick($event, entry.tab)"
            @contextmenu="openTopTabContextMenu($event, entry.tab)"
          >{{ t(entry.tab.labelKey) }}</button>
          <button
            v-else-if="entry.node"
            type="button"
            class="tab-item is-workspace-node"
            :class="{
              active: activeTopBarNode?.id === entry.id,
              'is-drop-before': topBarDropBefore === entry.id,
              'is-drop-after': topBarDropBefore === null && index === topBarEntries.length - 1,
            }"
            :data-top-bar-entry="entry.id"
            @pointerdown="onTopBarNodePointerDown($event, entry.id)"
            @click="onTopBarNodeClick(entry.id)"
            @contextmenu="openTopBarNodeContextMenu($event, entry.id)"
          >{{ entry.node.label }}</button>
        </template>
        <button
          v-if="projectStore.pluginToast"
          class="tab-plugin-warn"
          type="button"
          :title="pluginToastTitle"
          :aria-label="pluginToastTitle"
          :disabled="projectStore.pluginInstalling"
          @click="projectStore.installPlugin"
        >
          <span v-if="projectStore.pluginInstalling" class="tab-plugin-spinner" aria-hidden="true"></span>
          <svg
            v-else
            class="tab-plugin-icon"
            viewBox="0 0 16 16"
            width="14"
            height="14"
            fill="currentColor"
            aria-hidden="true"
          >
            <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm-.75 4a.75.75 0 0 1 1.5 0v3a.75.75 0 0 1-1.5 0V5zm.75 6.5a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5z"/>
          </svg>
          <span class="tab-plugin-label">{{ pluginToastLabel }}</span>
          <span class="tab-plugin-action">{{ pluginToastAction }}</span>
        </button>
        <div class="tab-spacer"></div>
        </template>
        <div class="window-controls">
          <button
            class="win-ctrl-btn"
            :class="{ 'win-pinned': uiStore.alwaysOnTop }"
            :title="uiStore.alwaysOnTop ? t('app.pin.unpin') : t('app.pin.pin')"
            @click="uiStore.toggleAlwaysOnTop"
          >
            <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor" :style="{ transform: uiStore.alwaysOnTop ? 'rotate(0deg)' : 'rotate(45deg)' }">
              <path d="M9.828 1.282a.75.75 0 0 1 .955.073l3.862 3.862a.75.75 0 0 1-.564 1.272h-.862L11.2 8.507a2.25 2.25 0 0 1-.039 2.994l-.56.56a.75.75 0 0 1-1.06 0L7.05 9.57l-3.72 3.72a.75.75 0 1 1-1.06-1.06l3.72-3.72L3.5 6.02a.75.75 0 0 1 0-1.06l.56-.56a2.25 2.25 0 0 1 2.994-.04L9.07 2.342V1.48a.75.75 0 0 1 .758-.198z"/>
            </svg>
          </button>
          <button class="win-ctrl-btn" @click="uiStore.winMinimize" :title="t('app.win.minimize')">
            <svg viewBox="0 0 12 12" width="12" height="12"><rect x="1" y="5.5" width="10" height="1" fill="currentColor"/></svg>
          </button>
          <button class="win-ctrl-btn" @click="uiStore.winToggleMaximize" :title="t('app.win.maximize')">
            <svg v-if="!uiStore.isMaximized" viewBox="0 0 12 12" width="12" height="12"><rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
            <svg v-else viewBox="0 0 12 12" width="12" height="12"><rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1"/><rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1"/></svg>
          </button>
          <button class="win-ctrl-btn win-close" @click="uiStore.winClose" :title="t('app.win.close')">
            <svg viewBox="0 0 12 12" width="12" height="12"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
          </button>
        </div>
      </div>
      <TopBannerHost />
      <RemotePairingPrompt />
      <div class="tab-content">
        <DevelopmentWorkbench
          ref="developmentWorkbench"
          v-show="uiStore.activePage === 'development'"
          :window-id="applicationWindowId"
          :toolbar-target="workspaceWindowToolbarTarget"
          :top-bar-target="showTopTabs ? topTabBar : null"
          :title-bar-optional="workspaceWindowTitleBarOptional"
          :single-workspace-only="isWorkspaceWindow"
        />

        <component
          :is="pluginViewComponent"
          v-if="showPluginEntry && uiStore.pluginsMounted && pluginViewComponent"
          v-show="uiStore.activePage === 'plugins'"
          working-dir=""
        />
        <div
          v-else-if="showPluginEntry && uiStore.pluginsMounted && uiStore.activePage === 'plugins'"
          class="tab-loading-state"
          :class="{ 'is-loading': pluginViewLoading, 'is-error': !!pluginViewError }"
        >
          {{ pluginViewError || t("common.loading") }}
        </div>

        <component
          :is="agentViewComponent"
          v-if="uiStore.agentMounted && agentViewComponent"
          v-show="uiStore.activePage === 'agent'"
          :active="uiStore.activePage === 'agent'"
          :working-dir="agentWorkingDir"
          :workspace-ref="agentWorkspaceRef"
          :agent-list="[...agentStore.agents, ...agentStore.subagents]"
        />
        <div
          v-else-if="uiStore.agentMounted && uiStore.activePage === 'agent'"
          class="tab-loading-state"
          :class="{ 'is-loading': agentViewLoading, 'is-error': !!agentViewError }"
        >
          {{ agentViewError || t("common.loading") }}
        </div>

        <component
          :is="settingsViewComponent"
          v-if="uiStore.settingsMounted && settingsViewComponent"
          v-show="uiStore.activePage === 'settings'"
          :active="uiStore.activePage === 'settings'"
          :all-models="modelStore.availableModels"
          :agents="agentStore.appAgents"
          :subagents="agentStore.appSubagents"
          @auth-changed="handleSettingsAuthChanged"
          @reset-onboarding="onResetOnboarding"
        />
        <div
          v-else-if="uiStore.settingsMounted && uiStore.activePage === 'settings'"
          class="tab-loading-state"
          :class="{ 'is-loading': settingsViewLoading, 'is-error': !!settingsViewError }"
        >
          {{ settingsViewError || t("common.loading") }}
        </div>
      </div>
    </div>
  </div>
  <AppUpdateModal
    :open="showAppUpdateModal"
    :info="appUpdateStore.updateInfo"
    @close="closeAppUpdateModal"
    @view="openAppUpdateRelease"
  />
  <BaseContextMenu
    v-if="topTabContextMenu"
    class="top-tab-ctx-menu"
    :x="topTabContextMenu.x"
    :y="topTabContextMenu.y"
    :min-width="170"
    :z-index="260"
    @close="topTabContextMenu = null"
  >
    <button
      type="button"
      class="top-tab-ctx-item"
      @click="openTopTabInWindow(topTabContextMenu.tab)"
    >
      <LucideIcon :icon="AppWindow" :size="13" />
      {{ t("app.tab.openInWindow") }}
    </button>
  </BaseContextMenu>
  <BaseContextMenu
    v-if="topBarNodeContextMenu"
    class="top-tab-ctx-menu"
    :x="topBarNodeContextMenu.x"
    :y="topBarNodeContextMenu.y"
    :min-width="170"
    :z-index="260"
    @close="topBarNodeContextMenu = null"
  >
    <button type="button" class="top-tab-ctx-item" @click="moveTopBarNodeToTree">
      <LucideIcon :icon="PanelLeft" :size="13" />
      {{ t("development.moveNodeToSidebar") }}
    </button>
  </BaseContextMenu>
  <Transition name="workspace-switch-modal">
    <div
      v-if="appCloseConfirmOpen"
      class="workspace-switch-overlay app-close-overlay"
      @click.self="closeAppCloseDialog"
    >
      <div
        class="workspace-switch-dialog app-close-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="app-close-title"
      >
        <div class="workspace-switch-header app-close-header">
          <span id="app-close-title" class="workspace-switch-title app-close-title">
            {{ t("app.close.runningConfirmTitle") }}
          </span>
          <button
            class="workspace-switch-close app-close-close"
            :disabled="appCloseBusy"
            @click="closeAppCloseDialog"
          >
            <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </button>
        </div>
        <div class="workspace-switch-body app-close-body">
          <p class="workspace-switch-message app-close-message">
            {{ t("app.close.runningConfirmMessage", String(appCloseRunningTaskCount)) }}
          </p>
          <p class="workspace-switch-warning app-close-warning">
            {{ t("app.close.runningConfirmWarning") }}
          </p>
        </div>
        <div class="workspace-switch-footer app-close-footer">
          <BaseButton :disabled="appCloseBusy" @click="closeAppCloseDialog">
            {{ t("common.cancel") }}
          </BaseButton>
          <BaseButton
            variant="danger"
            :disabled="appCloseBusy"
            @click="confirmAppClose"
          >
            {{ t("app.close.runningConfirmAction") }}
          </BaseButton>
        </div>
      </div>
    </div>
  </Transition>
  <FileDiffOverlay v-if="diffOverlay.visible.value" />
  <InternalDragOverlay />
  <WorkspaceSetupDialog />
  <Teleport
    v-for="host in sharedWorkbenchWindowHosts"
    :key="host.label"
    :to="host.container"
  >
    <WorkbenchWindow :shared-host="host" />
  </Teleport>
  <Teleport v-for="host in unityNativeEmbedHosts.values()" :key="host.surface.generation" :to="host.container">
    <UnityNativeEmbedHost :host="host" />
  </Teleport>
</template>
