
<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, shallowRef, onMounted, onUnmounted, watch } from "vue";
import type { Component, ShallowRef } from "vue";
import { AppWindow } from "lucide";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "./i18n";
import { normalizeAppError } from "./services/errors";
import { useUiStore, type AppPage } from "./stores/ui";
import { useAuthStore } from "./stores/auth";
import { useAgentStore } from "./stores/agent";
import { useModelStore } from "./stores/model";
import { useProjectStore } from "./stores/project";
import { useWorkspaceContextStore } from "./stores/workspaceContext";
import { useChatStore } from "./stores/chat";
import { useNotificationStore } from "./stores/notification";
import { useAppUpdateStore } from "./stores/appUpdate";
import { useAppBootstrap } from "./composables/useAppBootstrap";
import { useUnityAssetDropTarget } from "./composables/useUnityAssetDropTarget";
import { knowledgeGetEmbeddingStatus } from "./services/knowledge";
import { APP_CLOSE_REQUESTED_EVENT, getRunningTaskCount, requestAppExit } from "./services/system";

import TopBannerHost from "./components/TopBannerHost.vue";
import BaseButton from "./components/ui/BaseButton.vue";
import BaseContextMenu from "./components/ui/BaseContextMenu.vue";
import LucideIcon from "./components/icons/LucideIcon.vue";
import AppUpdateModal from "./components/AppUpdateModal.vue";
import DevelopmentWorkbench from "./components/workbench/DevelopmentWorkbench.vue";
import InternalDragOverlay from "./components/ui/InternalDragOverlay.vue";

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
import { isViewContentWindowLocation } from "./services/view";
import {
  canStartWindowDragFromTarget,
  getCurrentTauriWindowLabel,
  showCurrentTauriWindow,
  startCurrentWindowDragging,
} from "./services/tauriRuntime";
import { markStartupPhase } from "./services/startupPerf";
import { reloadPluginInspectorDrawers } from "./services/inspectorDrawerExtensions";
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
const isViewContentWindow = isViewContentWindowLocation();
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
  || isExtraWorkdirsWindow
  || isViewContentWindow;

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
const { state: displaySettings } = useDisplaySettings();
const unityEmbedBootstrapped = ref(false);
const unityEmbedBootstrapError = ref<string | null>(null);
const KNOWLEDGE_RUNTIME_LOADING_OPERATION = "knowledgeEmbeddingRuntimeLoading";
const KNOWLEDGE_RUNTIME_STARTUP_POLL_COUNT = 16;
let knowledgeRuntimeStatusTimer: ReturnType<typeof setTimeout> | null = null;
let knowledgeRuntimeStatusRequestSeq = 0;
let knowledgeRuntimeStartupPollsRemaining = 0;
let appCloseRequestUnlisten: UnlistenFn | null = null;
let extraWorkdirsUpdatedUnlisten: UnlistenFn | null = null;
let workspacePageResetOnboardingUnlisten: UnlistenFn | null = null;

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
  onOnboardingCompleted: completeOnboarding,
} = useAppBootstrap({
  handleExternalScriptOpen: !isUnityEmbedWindow && !isStandaloneWindow,
});

async function handleOnboardingCompleted() {
  await completeOnboarding();
  await workspaceContextStore.initialize(getCurrentTauriWindowLabel() || "main", "main");
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (workspaceRef) {
    await Promise.all([
      chatStore.refreshSessions(),
      agentStore.loadWorkspaceAgents(workspaceRef),
      projectStore.checkUnityConnection(),
      projectStore.checkUnityPlugin(),
      projectStore.loadAssetDbStatus(),
    ]);
    void reloadPluginInspectorDrawers();
  }
}

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
  { id: "agent", labelKey: "app.tab.agent", visible: displaySettings.showAgentTab },
  { id: "settings", labelKey: "app.tab.settings", visible: true },
]);

const visibleTopTabs = computed(() => topTabs.value.filter((tab) => tab.visible));
const topTabContextMenu = ref<{ x: number; y: number; tab: TopTabItem } | null>(null);

function isTopTabActive(tab: TopTabItem) {
  return uiStore.activePage === tab.id;
}

function canOpenTopTabInWindow(tab: TopTabItem) {
  return tab.id !== "development" && (
    isAppWorkspacePageId(tab.id)
    || (workspaceContextStore.focusedRuntime !== null && isCheckoutWorkspacePageId(tab.id))
  );
}

async function openTopTabInWindow(tab: TopTabItem) {
  topTabContextMenu.value = null;
  if (!canOpenTopTabInWindow(tab)) return;
  try {
    const runtime = workspaceContextStore.focusedRuntime;
    if (runtime && isCheckoutWorkspacePageId(tab.id)) {
      await openWorkspacePageWindow({
        scope: "checkout",
        page: tab.id,
        title: `${shortDir(runtime.root)} · ${t(tab.labelKey)}`,
        checkoutId: runtime.checkoutId,
        workspaceGeneration: runtime.workspaceGeneration,
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

watch([() => uiStore.activePage, visibleTopTabs], () => {
  if (visibleTopTabs.value.some((tab) => tab.id === uiStore.activePage)) return;
  uiStore.setPage("development");
}, { immediate: true });

const appCloseConfirmOpen = ref(false);
const appCloseBusy = ref(false);
const appCloseRunningTaskCount = ref(0);
const runningSessionCount = computed(() => chatStore.streamingSessionIds.size);
const focusedWorkspaceRoot = computed(
  () => workspaceContextStore.focusedRoot || projectStore.workingDir,
);
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

function clearKnowledgeRuntimeStatusTimer() {
  if (!knowledgeRuntimeStatusTimer) return;
  clearTimeout(knowledgeRuntimeStatusTimer);
  knowledgeRuntimeStatusTimer = null;
}

function scheduleKnowledgeRuntimeStatusPoll(delay = 700) {
  clearKnowledgeRuntimeStatusTimer();
  knowledgeRuntimeStatusTimer = setTimeout(() => {
    knowledgeRuntimeStatusTimer = null;
    void refreshKnowledgeRuntimeLoadingStatus();
  }, delay);
}

async function refreshKnowledgeRuntimeLoadingStatus() {
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  const requestSeq = ++knowledgeRuntimeStatusRequestSeq;
  if (isStandaloneWindow || !workspaceRef) {
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    clearKnowledgeRuntimeStatusTimer();
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
    if (status.activating) {
      notificationStore.addNotice("info", t("knowledge.retrieval.runtimeStarting"), {
        operation: KNOWLEDGE_RUNTIME_LOADING_OPERATION,
        replaceOperation: true,
        spinner: true,
        sticky: true,
        skipConsoleLog: true,
      });
      scheduleKnowledgeRuntimeStatusPoll();
      return;
    }

    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    if (knowledgeRuntimeStartupPollsRemaining > 0) {
      knowledgeRuntimeStartupPollsRemaining -= 1;
      scheduleKnowledgeRuntimeStatusPoll();
    }
  } catch {
    if (requestSeq !== knowledgeRuntimeStatusRequestSeq) return;
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    clearKnowledgeRuntimeStatusTimer();
  }
}

function startKnowledgeRuntimeStartupPolling() {
  if (isStandaloneWindow) return;
  knowledgeRuntimeStatusRequestSeq += 1;
  knowledgeRuntimeStartupPollsRemaining = KNOWLEDGE_RUNTIME_STARTUP_POLL_COUNT;
  scheduleKnowledgeRuntimeStatusPoll(120);
}

async function registerAppCloseRequestListener() {
  if (isStandaloneWindow || appCloseRequestUnlisten) return;
  try {
    appCloseRequestUnlisten = await listen<void>(APP_CLOSE_REQUESTED_EVENT, () => {
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

function revealMainWindow() {
  if (isStandaloneWindow) return;
  const currentTauriWindowLabel = getCurrentTauriWindowLabel();
  if (currentTauriWindowLabel && currentTauriWindowLabel !== "main") return;
  markStartupPhase("main_window_show_start");
  void showCurrentTauriWindow()
    .then(() => {
      markStartupPhase("main_window_show_done");
    })
    .catch((error) => {
      markStartupPhase("main_window_show_error");
      console.warn("[startup] failed to show main window", error);
    });
}

// -- Lifecycle --
onMounted(async () => {
  markStartupPhase("app_mounted", {
    window: isUnityEmbedWindow ? "unity_embed" : isStandaloneWindow ? "standalone" : "main",
    path: window.location.pathname,
  });
  await nextTick();
  setTimeout(revealMainWindow, 0);
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
  await registerAppCloseRequestListener();
  await registerExtraWorkdirsUpdatedListener();
  workspacePageResetOnboardingUnlisten = await listen(
    WORKSPACE_PAGE_RESET_ONBOARDING_EVENT,
    onResetOnboarding,
  );
  markStartupPhase("main_dom_listeners_ready");
  markStartupPhase("main_bootstrap_critical_start");
  await bootstrapCritical();
  markStartupPhase("main_bootstrap_critical_done");
  try {
    await workspaceContextStore.initialize(getCurrentTauriWindowLabel() || "main", "main");
    if (!workspaceContextStore.focusedWorkspaceRef && projectStore.workingDir) {
      await workspaceContextStore.openAndFocus(projectStore.workingDir);
    }
    if (workspaceContextStore.focusedWorkspaceRef) {
      await Promise.all([
        chatStore.refreshSessions(),
        agentStore.loadWorkspaceAgents(workspaceContextStore.focusedWorkspaceRef),
        projectStore.checkUnityConnection(),
        projectStore.checkUnityPlugin(),
        projectStore.loadAssetDbStatus(),
      ]);
      void reloadPluginInspectorDrawers();
    }
  } catch (error) {
    reportWorkingDirSwitchError(error);
  }
  markStartupPhase("main_register_listeners_start");
  await registerListeners();
  markStartupPhase("main_register_listeners_done");
  // Development is already interactive. Preload the remaining process-level pages.
  preloadTabsInBackground([
    settingsView.ensureLoaded,
    pluginView.ensureLoaded,
    agentView.ensureLoaded,
  ]);
  markStartupPhase("main_preload_tabs_scheduled");
  void bootstrapDeferred();
  markStartupPhase("main_bootstrap_deferred_scheduled");
  void restoreSharedWorkbenchWindows()
    .then(() => prepareSharedWorkbenchWindowPool())
    .catch((error) => {
      console.warn("[workbench-window] restore or pool warmup failed", error);
    });
  void appUpdateStore.checkForUpdates({ silent: true });
  markStartupPhase("main_update_check_scheduled");
  startKnowledgeRuntimeStartupPolling();
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
  if (isUnityEmbedWindow) {
    cleanup();
    return;
  }
  if (isStandaloneWindow) return;
  appCloseRequestUnlisten?.();
  appCloseRequestUnlisten = null;
  extraWorkdirsUpdatedUnlisten?.();
  extraWorkdirsUpdatedUnlisten = null;
  workspacePageResetOnboardingUnlisten?.();
  workspacePageResetOnboardingUnlisten = null;
  notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
  clearKnowledgeRuntimeStatusTimer();
  cleanup();
});

watch(() => workspaceContextStore.focusedWorkspaceRef, () => {
  startKnowledgeRuntimeStartupPolling();
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
  <ViewHostWindow v-else-if="isViewContentWindow" embedded />
  <div v-else-if="!authStore.authChecked" class="app-startup-state">
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
      <div class="tab-bar" @pointerdown="onTabBarPointerDown">
        <div class="tab-drag-region" aria-hidden="true"></div>
        <span class="tab-brand">Locus</span>
        <button
          v-for="tab in visibleTopTabs"
          :key="tab.id"
          class="tab-item"
          :class="{ active: isTopTabActive(tab) }"
          @click="onTopTabClick($event, tab)"
          @contextmenu="openTopTabContextMenu($event, tab)"
        >{{ t(tab.labelKey) }}</button>
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
      <div class="tab-content">
        <DevelopmentWorkbench v-show="uiStore.activePage === 'development'" />

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
          :working-dir="focusedWorkspaceRoot"
          :workspace-ref="workspaceContextStore.focusedWorkspaceRef"
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
          @model-defaults-changed="modelStore.applyModelDefaults"
          @codex-transport-changed="modelStore.applyCodexModelConfig"
          @custom-providers-changed="modelStore.applyCustomProviders"
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
  <Teleport
    v-for="host in sharedWorkbenchWindowHosts"
    :key="host.label"
    :to="host.container"
  >
    <WorkbenchWindow :shared-host="host" />
  </Teleport>
</template>
