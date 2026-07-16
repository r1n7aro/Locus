
<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, shallowRef, onMounted, onUnmounted, watch } from "vue";
import type { Component, ShallowRef } from "vue";
import { FolderCog, FolderOpen, ListX } from "lucide";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "./i18n";
import { normalizeAppError } from "./services/errors";
import { useUiStore } from "./stores/ui";
import { useAuthStore } from "./stores/auth";
import { useAgentStore } from "./stores/agent";
import { useModelStore } from "./stores/model";
import { useProjectStore } from "./stores/project";
import { useChatStore } from "./stores/chat";
import { useNotificationStore } from "./stores/notification";
import { useAppUpdateStore } from "./stores/appUpdate";
import { useAppBootstrap } from "./composables/useAppBootstrap";
import { useUnityAssetDropTarget } from "./composables/useUnityAssetDropTarget";
import { knowledgeGetEmbeddingStatus } from "./services/knowledge";
import { APP_CLOSE_REQUESTED_EVENT, requestAppExit } from "./services/system";

import TopBannerHost from "./components/TopBannerHost.vue";
import BaseButton from "./components/ui/BaseButton.vue";
import BaseContextMenu from "./components/ui/BaseContextMenu.vue";
import LucideIcon from "./components/icons/LucideIcon.vue";
import AppUpdateModal from "./components/AppUpdateModal.vue";

import { provideDiffOverlay } from "./composables/useDiffOverlay";
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
import {
  setLocusAssetInspectorPanelHostAvailable,
  useLocusAssetInspectorPanel,
} from "./composables/useLocusAssetInspectorPanel";
import { isUnityValueEditorWindowLocation } from "./services/unityValueEditorWindow";
import {
  isExtraWorkdirsWindowLocation,
  listenExtraWorkdirsUpdated,
  openExtraWorkdirsWindow,
} from "./services/extraWorkdirsWindow";
import { prepareSubWindowPool } from "./services/subWindow";
import type { ExtraWorkdirStatus } from "./services/extraWorkdirs";
import { isViewContentWindowLocation, isViewHostWindowLocation } from "./services/view";
import { isAgentGraphToolWindowLocation } from "./services/agentGraphTool";
import {
  canStartWindowDragFromTarget,
  getCurrentTauriWindowLabel,
  showCurrentTauriWindow,
  startCurrentWindowDragging,
} from "./services/tauriRuntime";
import { markStartupPhase } from "./services/startupPerf";
const isUnityEmbedTestWindow = window.location.pathname === "/unity-embed-test";
const isUnityEmbedWindow = !isUnityEmbedTestWindow && window.location.pathname === "/unity-embed";
const unityEmbedParams = new URLSearchParams(window.location.search);
const unityEmbedTarget = unityEmbedParams.get("target") || "session";
const unityEmbedTargetId = unityEmbedParams.get("id") || "";
const isUnityEmbedViewWindow = isUnityEmbedWindow && unityEmbedTarget === "view";
const isKnowledgeDownloadWindow = isKnowledgeDownloadWindowLocation();
const isKnowledgeLexicalProgressWindow = isKnowledgeLexicalProgressWindowLocation();
const isFeishuReferenceImportWindow = isFeishuReferenceImportWindowLocation();
const isUnityReferenceImportWindow = isUnityReferenceImportWindowLocation();
const isReferenceExternalImportWindow = isReferenceExternalImportWindowLocation();
const isCollabSearchWindow = isCollabSearchWindowLocation();
const isChatDiffReviewWindow = isChatDiffReviewWindowLocation();
const isPlanViewWindow = isPlanViewWindowLocation();
const isUnityValueEditorWindow = isUnityValueEditorWindowLocation();
const isExtraWorkdirsWindow = isExtraWorkdirsWindowLocation();
const isViewHostWindow = isViewHostWindowLocation();
const isViewContentWindow = isViewContentWindowLocation();
const isAgentGraphToolWindow = isAgentGraphToolWindowLocation();
const isStandaloneWindow = isUnityEmbedWindow
  || isUnityEmbedTestWindow
  || isKnowledgeDownloadWindow
  || isKnowledgeLexicalProgressWindow
  || isFeishuReferenceImportWindow
  || isUnityReferenceImportWindow
  || isReferenceExternalImportWindow
  || isCollabSearchWindow
  || isChatDiffReviewWindow
  || isPlanViewWindow
  || isUnityValueEditorWindow
  || isExtraWorkdirsWindow
  || isViewHostWindow
  || isViewContentWindow
  || isAgentGraphToolWindow;

const KnowledgeDownloadProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeDownloadProgressWindow.vue"));
const KnowledgeLexicalProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeLexicalProgressWindow.vue"));
const FeishuReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/FeishuReferenceImportProgressWindow.vue"));
const UnityReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/UnityReferenceImportProgressWindow.vue"));
const ReferenceExternalImportWindow = defineAsyncComponent(() => import("./components/ReferenceExternalImportWindow.vue"));
const CollabSearchWindow = defineAsyncComponent(() => import("./components/CollabSearchWindow.vue"));
const ChatDiffReviewWindow = defineAsyncComponent(() => import("./components/ChatDiffReviewWindow.vue"));
const PlanViewWindow = defineAsyncComponent(() => import("./components/PlanViewWindow.vue"));
const UnityValueEditorWindow = defineAsyncComponent(() => import("./components/UnityValueEditorWindow.vue"));
const ExtraWorkdirsConfigWindow = defineAsyncComponent(() => import("./components/ExtraWorkdirsConfigWindow.vue"));
const ViewHostWindow = defineAsyncComponent(() => import("./components/ViewHostWindow.vue"));
const AgentGraphToolWindow = defineAsyncComponent(() => import("./components/AgentGraphToolWindow.vue"));
const UnityEmbeddedSessionView = defineAsyncComponent(() => import("./components/UnityEmbeddedSessionView.vue"));
const UnityEmbedTestView = defineAsyncComponent(() => import("./components/UnityEmbedTestView.vue"));
const OnboardingView = defineAsyncComponent(() => import("./components/OnboardingView.vue"));
const FileDiffOverlay = defineAsyncComponent(() => import("./components/diff/FileDiffOverlay.vue"));
const LocusAssetInspectorPanel = defineAsyncComponent(() => import("./components/LocusAssetInspectorPanel.vue"));
const showPluginEntry = true;

initTheme(isUnityEmbedWindow ? "unityEmbed" : "main");
initFonts();

// -- Stores --
const uiStore = useUiStore();
const authStore = useAuthStore();
const agentStore = useAgentStore();
const modelStore = useModelStore();
const projectStore = useProjectStore();
const chatStore = useChatStore();
const notificationStore = useNotificationStore();
const appUpdateStore = useAppUpdateStore();
const { state: displaySettings } = useDisplaySettings();
const unityEmbedBootstrapped = ref(false);
const unityEmbedBootstrapError = ref<string | null>(null);
const KNOWLEDGE_RUNTIME_LOADING_OPERATION = "knowledgeEmbeddingRuntimeLoading";
const KNOWLEDGE_RUNTIME_STARTUP_POLL_COUNT = 16;
let knowledgeRuntimeStatusTimer: ReturnType<typeof setTimeout> | null = null;
let knowledgeRuntimeStartupPollsRemaining = 0;
let appCloseRequestUnlisten: UnlistenFn | null = null;
let extraWorkdirsUpdatedUnlisten: UnlistenFn | null = null;

// -- Diff overlay provider (must be called in App setup so all children can inject) --
const diffOverlay = provideDiffOverlay();
// The floating Locus inspector panel lives in the main window only; standalone
// windows fall back to the dedicated inspector window.
const locusAssetInspectorPanel = useLocusAssetInspectorPanel();
setLocusAssetInspectorPanelHostAvailable(!isStandaloneWindow);
const { bootstrapCritical, bootstrapDeferred, preloadTabsInBackground, registerListeners, cleanup, applyWorkingDir, refreshAfterSettings, onOnboardingCompleted } = useAppBootstrap();
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

const chatView = createLazyViewState(
  () => import("./components/ChatWorkspaceView.vue"),
  "loadChatWorkspaceView",
);
const collabView = createLazyViewState(
  () => import("./components/CollabView.vue"),
  "loadCollabView",
);
const knowledgeView = createLazyViewState(
  () => import("./components/KnowledgeView.vue"),
  "loadKnowledgeView",
);
const assetView = createLazyViewState(
  () => import("./components/AssetView.vue"),
  "loadAssetView",
);
const viewPackageView = createLazyViewState(
  () => import("./components/ViewPackageView.vue"),
  "loadViewPackageView",
);
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

const chatViewComponent = chatView.component;
const chatViewLoading = chatView.loading;
const chatViewError = chatView.error;

const collabViewComponent = collabView.component;
const collabViewLoading = collabView.loading;
const collabViewError = collabView.error;

const knowledgeViewComponent = knowledgeView.component;
const knowledgeViewLoading = knowledgeView.loading;
const knowledgeViewError = knowledgeView.error;

const assetViewComponent = assetView.component;
const assetViewLoading = assetView.loading;
const assetViewError = assetView.error;

const viewPackageViewComponent = viewPackageView.component;
const viewPackageViewLoading = viewPackageView.loading;
const viewPackageViewError = viewPackageView.error;

const pluginViewComponent = pluginView.component;
const pluginViewLoading = pluginView.loading;
const pluginViewError = pluginView.error;

const agentViewComponent = agentView.component;
const agentViewLoading = agentView.loading;
const agentViewError = agentView.error;

const settingsViewComponent = settingsView.component;
const settingsViewLoading = settingsView.loading;
const settingsViewError = settingsView.error;

type AppTab = typeof uiStore.activeTab;

interface TopTabItem {
  id: AppTab;
  labelKey: string;
  visible: boolean;
}

const topTabs = computed<TopTabItem[]>(() => [
  { id: "chat", labelKey: "app.tab.dev", visible: true },
  { id: "knowledge", labelKey: "app.tab.knowledge", visible: displaySettings.showKnowledgeTab },
  { id: "collab", labelKey: "app.tab.collab", visible: displaySettings.showCollabTab },
  { id: "asset", labelKey: "app.tab.asset", visible: displaySettings.showAssetTab },
  { id: "views", labelKey: "app.tab.views", visible: displaySettings.showViewsTab },
  { id: "plugins", labelKey: "app.tab.plugins", visible: showPluginEntry && displaySettings.showPluginsTab },
  { id: "agent", labelKey: "app.tab.agent", visible: displaySettings.showAgentTab },
  { id: "settings", labelKey: "app.tab.settings", visible: true },
]);

const visibleTopTabs = computed(() => topTabs.value.filter((tab) => tab.visible));

function isTopTabVisible(tab: AppTab) {
  return visibleTopTabs.value.some((item) => item.id === tab);
}

watch(() => uiStore.activeTab, (tab) => {
  if (tab !== "chat") return;
  void chatView.ensureLoaded();
}, { immediate: true });

// 离开设置页时做一次兜底刷新（顶栏切 Tab 不走 setTab 之外的逻辑，原 closeSettings 的副作用迁移到这里）。
watch(() => uiStore.activeTab, (tab, prev) => {
  if (prev === "settings" && tab !== "settings") void refreshAfterSettings();
});

watch(() => uiStore.collabMounted, (mounted) => {
  if (!mounted) return;
  void collabView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.knowledgeMounted, (mounted) => {
  if (!mounted) return;
  void knowledgeView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.assetMounted, (mounted) => {
  if (!mounted) return;
  void assetView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.viewMounted, (mounted) => {
  if (!mounted) return;
  void viewPackageView.ensureLoaded();
}, { immediate: true });

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

watch([() => uiStore.activeTab, visibleTopTabs], () => {
  if (isTopTabVisible(uiStore.activeTab)) return;
  uiStore.setTab("chat");
}, { immediate: true });

// -- Workspace dropdown (local UI) --
type RecentDirContextMenu = {
  x: number;
  y: number;
  dir: string;
};

const showDirDropdown = ref(false);
const dirDropdownRef = ref<HTMLElement | null>(null);
const recentDirContextMenu = ref<RecentDirContextMenu | null>(null);
const pendingWorkspaceSwitchPath = ref<string | null>(null);
const switchingWorkspacePath = ref<string | null>(null);
const workspaceSwitchBusy = ref(false);
const appCloseConfirmOpen = ref(false);
const appCloseBusy = ref(false);
const appCloseRunningTaskCount = ref(0);
const runningSessionCount = computed(() => chatStore.streamingSessionIds.size);
const workspaceSwitchTargetName = computed(() =>
  pendingWorkspaceSwitchPath.value ? shortDir(pendingWorkspaceSwitchPath.value) : "",
);
const workspaceButtonTitle = computed(() => {
  if (switchingWorkspacePath.value) {
    return t(
      "app.dir.switchingTitle",
      shortDir(switchingWorkspacePath.value),
      switchingWorkspacePath.value,
    );
  }
  return projectStore.workingDir || t("app.dir.notSetTitle");
});
const workspaceButtonLabel = computed(() =>
  switchingWorkspacePath.value ? t("app.dir.switching") : shortDir(projectStore.workingDir),
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

function parentPath(dir: string): string {
  const parts = dir.replace(/\\/g, "/").split("/").filter(Boolean);
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/");
}

function toggleDirDropdown() {
  if (workspaceSwitchBusy.value) return;
  showDirDropdown.value = !showDirDropdown.value;
  if (showDirDropdown.value) {
    void projectStore.loadExtraWorkdirs();
  } else {
    recentDirContextMenu.value = null;
  }
}

function extraWorkdirsFor(dir: string): ExtraWorkdirStatus[] {
  return projectStore.extraWorkdirs[dir] ?? [];
}

function extraWorkdirTooltip(extra: ExtraWorkdirStatus): string {
  return extra.comment ? `${extra.path} — ${extra.comment}` : extra.path;
}

function closeRecentDirContextMenu() {
  recentDirContextMenu.value = null;
}

function closeWorkspaceSwitchDialog() {
  if (workspaceSwitchBusy.value) return;
  pendingWorkspaceSwitchPath.value = null;
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

function notifyCancelledWorkspaceSessions(count: number) {
  if (count <= 0) return;
  notificationStore.addNotice("info", t("app.dir.runningCancelledNotice", String(count)), {
    operation: "workspaceSwitchCancelled",
    replaceOperation: true,
  });
}

async function performWorkingDirChange(dir: string, cancelledSessionCount = 0) {
  try {
    await applyWorkingDir(dir);
    notifyCancelledWorkspaceSessions(cancelledSessionCount);
    return true;
  } catch (error) {
    reportWorkingDirSwitchError(error);
    return false;
  }
}

async function requestWorkingDirChange(dir: string) {
  if (!dir || dir === projectStore.workingDir || workspaceSwitchBusy.value) return;
  if (runningSessionCount.value > 0) {
    pendingWorkspaceSwitchPath.value = dir;
    return;
  }
  workspaceSwitchBusy.value = true;
  switchingWorkspacePath.value = dir;
  try {
    await performWorkingDirChange(dir);
  } finally {
    switchingWorkspacePath.value = null;
    workspaceSwitchBusy.value = false;
  }
}

async function confirmWorkspaceSwitch() {
  const target = pendingWorkspaceSwitchPath.value;
  if (!target || workspaceSwitchBusy.value) return;
  workspaceSwitchBusy.value = true;
  switchingWorkspacePath.value = target;
  try {
    const sessionIds = Array.from(chatStore.streamingSessionIds);
    await chatStore.cancelSessions(sessionIds);
    const switched = await performWorkingDirChange(target, sessionIds.length);
    if (switched) {
      pendingWorkspaceSwitchPath.value = null;
    }
  } catch (error) {
    reportWorkingDirSwitchError(error);
  } finally {
    switchingWorkspacePath.value = null;
    workspaceSwitchBusy.value = false;
  }
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
  const runningTaskCount = runningSessionCount.value;
  if (runningTaskCount > 0) {
    appCloseRunningTaskCount.value = runningTaskCount;
    appCloseConfirmOpen.value = true;
    return;
  }
  await confirmAppClose();
}

async function selectRecentDir(dir: string) {
  if (workspaceSwitchBusy.value) return;
  closeRecentDirContextMenu();
  showDirDropdown.value = false;
  await requestWorkingDirChange(dir);
}

async function browseFromDropdown() {
  if (workspaceSwitchBusy.value) return;
  closeRecentDirContextMenu();
  showDirDropdown.value = false;
  try {
    const selected = await open({ directory: true, multiple: false, defaultPath: projectStore.workingDir || undefined });
    if (selected && typeof selected === "string") {
      await requestWorkingDirChange(selected);
    }
  } catch (e) {
    const err = normalizeAppError(e);
    console.error("browse_working_dir failed:", e);
    notificationStore.addNotice("error", err.message, {
      operation: "browseWorkingDir",
      skipConsoleLog: true,
    });
  }
}

function openRecentDirContextMenu(event: MouseEvent, dir: string) {
  if (workspaceSwitchBusy.value) return;
  event.preventDefault();
  event.stopPropagation();
  recentDirContextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    dir,
  };
}

async function openContextRecentDirInFileExplorer() {
  const dir = recentDirContextMenu.value?.dir;
  if (!dir) return;
  closeRecentDirContextMenu();
  try {
    await projectStore.openDirInFileExplorer(dir);
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openRecentDirInFileExplorer",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  }
}

async function removeContextRecentDir() {
  const dir = recentDirContextMenu.value?.dir;
  if (!dir) return;
  closeRecentDirContextMenu();
  try {
    await projectStore.removeRecentDir(dir);
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "removeRecentDir",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  }
}

async function configureContextRecentDirExtraWorkdirs() {
  const dir = recentDirContextMenu.value?.dir;
  if (!dir) return;
  closeRecentDirContextMenu();
  try {
    await openExtraWorkdirsWindow({ workspacePath: dir });
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openExtraWorkdirsWindow",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  }
}

function handleDirClickOutside(e: MouseEvent) {
  const target = e.target as Node;
  const targetElement = target instanceof Element ? target : target.parentElement;
  if (targetElement?.closest(".recent-dir-ctx-menu")) return;
  if (dirDropdownRef.value && !dirDropdownRef.value.contains(target)) {
    showDirDropdown.value = false;
    closeRecentDirContextMenu();
  }
}

function onResetOnboarding() {
  showDirDropdown.value = false;
  closeRecentDirContextMenu();
  projectStore.resetWorkspaceState();
  chatStore.resetWorkspaceScope();
  uiStore.resetOnboarding();
}

async function handleSettingsAuthChanged() {
  await authStore.loadProviderStatus();
  await modelStore.loadCodexAvailableModels();
  modelStore.resolveSelectedModel(true);
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
  if (isStandaloneWindow || !projectStore.workingDir.trim()) {
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    clearKnowledgeRuntimeStatusTimer();
    return;
  }

  try {
    const status = await knowledgeGetEmbeddingStatus();
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
    notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
    clearKnowledgeRuntimeStatusTimer();
  }
}

function startKnowledgeRuntimeStartupPolling() {
  if (isStandaloneWindow) return;
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
  document.addEventListener("click", handleDirClickOutside, true);
  await registerAppCloseRequestListener();
  await registerExtraWorkdirsUpdatedListener();
  markStartupPhase("main_dom_listeners_ready");
  markStartupPhase("main_bootstrap_critical_start");
  await bootstrapCritical();
  markStartupPhase("main_bootstrap_critical_done");
  markStartupPhase("main_register_listeners_start");
  await registerListeners();
  markStartupPhase("main_register_listeners_done");
  // Sessions page is now interactive — kick off background work. Passing the
  // lazy-view loaders lets the preloader fill each view's component ref, so
  // the first visit to these tabs mounts instantly (no loading-placeholder flash).
  preloadTabsInBackground([
    settingsView.ensureLoaded,
    collabView.ensureLoaded,
    knowledgeView.ensureLoaded,
    assetView.ensureLoaded,
    agentView.ensureLoaded,
  ]);
  markStartupPhase("main_preload_tabs_scheduled");
  void bootstrapDeferred();
  markStartupPhase("main_bootstrap_deferred_scheduled");
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
  document.removeEventListener("click", handleDirClickOutside, true);
  appCloseRequestUnlisten?.();
  appCloseRequestUnlisten = null;
  extraWorkdirsUpdatedUnlisten?.();
  extraWorkdirsUpdatedUnlisten = null;
  notificationStore.clearByOperation(KNOWLEDGE_RUNTIME_LOADING_OPERATION);
  clearKnowledgeRuntimeStatusTimer();
  cleanup();
});

watch(() => projectStore.workingDir, () => {
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
  />
  <UnityEmbedTestView v-else-if="isUnityEmbedTestWindow" />
  <KnowledgeDownloadProgressWindow v-else-if="isKnowledgeDownloadWindow" />
  <KnowledgeLexicalProgressWindow v-else-if="isKnowledgeLexicalProgressWindow" />
  <FeishuReferenceImportProgressWindow v-else-if="isFeishuReferenceImportWindow" />
  <UnityReferenceImportProgressWindow v-else-if="isUnityReferenceImportWindow" />
  <ReferenceExternalImportWindow v-else-if="isReferenceExternalImportWindow" />
  <CollabSearchWindow v-else-if="isCollabSearchWindow" />
  <ChatDiffReviewWindow v-else-if="isChatDiffReviewWindow" />
  <PlanViewWindow v-else-if="isPlanViewWindow" />
  <UnityValueEditorWindow v-else-if="isUnityValueEditorWindow" />
  <ExtraWorkdirsConfigWindow v-else-if="isExtraWorkdirsWindow" />
  <ViewHostWindow v-else-if="isViewContentWindow" embedded />
  <ViewHostWindow v-else-if="isViewHostWindow" />
  <AgentGraphToolWindow v-else-if="isAgentGraphToolWindow" />
  <div v-else-if="!authStore.authChecked" class="app-startup-state">
    <span>{{ t("common.loading") }}</span>
  </div>
  <OnboardingView v-else-if="authStore.authChecked && uiStore.showOnboarding" @completed="onOnboardingCompleted" />
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
          :class="{ active: uiStore.activeTab === tab.id }"
          @click="uiStore.setTab(tab.id)"
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
        <div class="workspace-selector" ref="dirDropdownRef">
          <button
            class="workspace-btn"
            :class="{ 'is-switching': workspaceSwitchBusy }"
            :title="workspaceButtonTitle"
            :disabled="workspaceSwitchBusy"
            :aria-busy="workspaceSwitchBusy"
            @click="toggleDirDropdown"
          >
            <svg class="ws-icon" viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
            </svg>
            <span class="ws-name">{{ workspaceButtonLabel }}</span>
            <span v-if="workspaceSwitchBusy" class="workspace-switch-spinner" aria-hidden="true"></span>
            <svg v-else class="ws-chevron" :class="{ open: showDirDropdown }" viewBox="0 0 16 16" fill="currentColor" width="10" height="10">
              <path d="M4.427 5.427a.75.75 0 0 1 1.06-.013L8 7.867l2.513-2.453a.75.75 0 1 1 1.047 1.073l-3 2.927a.75.75 0 0 1-1.047 0l-3-2.927a.75.75 0 0 1-.013-1.06z"/>
            </svg>
          </button>
          <Transition name="dropdown">
            <div v-if="showDirDropdown" class="dir-dropdown">
              <div class="dropdown-label">{{ t("app.dir.recentDirs") }}</div>
              <template v-for="dir in projectStore.recentDirs" :key="dir">
                <div
                  class="dir-item"
                  :class="{
                    active: dir === projectStore.workingDir,
                    'context-selected': recentDirContextMenu?.dir === dir,
                  }"
                  @click="selectRecentDir(dir)"
                  @contextmenu.prevent.stop="openRecentDirContextMenu($event, dir)"
                  :title="dir"
                >
                  <svg class="dir-item-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                    <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
                  </svg>
                  <div class="dir-item-text">
                    <span class="dir-item-name">{{ shortDir(dir) }}</span>
                    <span class="dir-item-path">{{ parentPath(dir) }}</span>
                  </div>
                  <span v-if="dir === projectStore.workingDir" class="dir-check">&#10003;</span>
                </div>
                <div
                  v-if="extraWorkdirsFor(dir).length > 0"
                  class="dir-item-extras"
                  @contextmenu.prevent.stop="openRecentDirContextMenu($event, dir)"
                >
                  <div
                    v-for="extra in extraWorkdirsFor(dir)"
                    :key="extra.path"
                    class="dir-extra-row"
                    :class="{ missing: !extra.exists }"
                    :title="extraWorkdirTooltip(extra)"
                    @click.stop
                  >
                    <svg class="dir-extra-icon" viewBox="0 0 16 16" fill="currentColor" width="10" height="10">
                      <path d="M7.775 3.275a.75.75 0 0 0 1.06 1.06l1.25-1.25a2 2 0 1 1 2.83 2.83l-2.5 2.5a2 2 0 0 1-2.83 0 .75.75 0 0 0-1.06 1.06 3.5 3.5 0 0 0 4.95 0l2.5-2.5a3.5 3.5 0 0 0-4.95-4.95l-1.25 1.25zm-4.69 9.64a2 2 0 0 1 0-2.83l2.5-2.5a2 2 0 0 1 2.83 0 .75.75 0 0 0 1.06-1.06 3.5 3.5 0 0 0-4.95 0l-2.5 2.5a3.5 3.5 0 0 0 4.95 4.95l1.25-1.25a.75.75 0 0 0-1.06-1.06l-1.25 1.25a2 2 0 0 1-2.83 0z"/>
                    </svg>
                    <span class="dir-extra-name">{{ shortDir(extra.path) }}</span>
                    <span v-if="extra.comment" class="dir-extra-comment">{{ extra.comment }}</span>
                    <span v-if="!extra.exists" class="dir-extra-missing">{{ t("extraWorkdirs.missingBadge") }}</span>
                  </div>
                </div>
              </template>
              <div v-if="projectStore.recentDirs.length === 0" class="dropdown-empty">{{ t("app.dir.noRecords") }}</div>
              <div class="dropdown-divider"></div>
              <div class="dir-item browse" @click="browseFromDropdown">
                <svg class="dir-item-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                  <path d="M8 2a.75.75 0 0 1 .75.75v4.5h4.5a.75.75 0 0 1 0 1.5h-4.5v4.5a.75.75 0 0 1-1.5 0v-4.5h-4.5a.75.75 0 0 1 0-1.5h4.5v-4.5A.75.75 0 0 1 8 2z"/>
                </svg>
                <span class="dir-item-name">{{ t("app.dir.browseOther") }}</span>
              </div>
            </div>
          </Transition>
        </div>
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
        <component
          :is="chatViewComponent"
          v-if="chatViewComponent"
          v-show="uiStore.activeTab === 'chat'"
          :active="uiStore.activeTab === 'chat'"
          layout-mode="auto"
        />
        <div
          v-else-if="uiStore.activeTab === 'chat'"
          class="tab-loading-state"
          :class="{ 'is-loading': chatViewLoading, 'is-error': !!chatViewError }"
        >
          {{ chatViewError || t("common.loading") }}
        </div>
        <component
          :is="collabViewComponent"
          v-if="uiStore.collabMounted && collabViewComponent"
          v-show="uiStore.activeTab === 'collab'"
          :working-dir="projectStore.workingDir"
          :is-active="uiStore.activeTab === 'collab'"
          :selected-model-id="modelStore.selectedModelId"
          :selected-agent-id="agentStore.selectedAgentId"
          :models="modelStore.availableModels"
          @select-model="(id: string) => modelStore.selectModel(id)"
        />
        <div
          v-else-if="uiStore.collabMounted && uiStore.activeTab === 'collab'"
          class="tab-loading-state"
          :class="{ 'is-loading': collabViewLoading, 'is-error': !!collabViewError }"
        >
          {{ collabViewError || t("common.loading") }}
        </div>

        <component
          :is="knowledgeViewComponent"
          v-if="uiStore.knowledgeMounted && knowledgeViewComponent"
          v-show="uiStore.activeTab === 'knowledge'"
          :working-dir="projectStore.workingDir"
          :selected-model-id="modelStore.selectedModelId"
          :model-defaults="modelStore.modelDefaults"
        />
        <div
          v-else-if="uiStore.knowledgeMounted && uiStore.activeTab === 'knowledge'"
          class="tab-loading-state"
          :class="{ 'is-loading': knowledgeViewLoading, 'is-error': !!knowledgeViewError }"
        >
          {{ knowledgeViewError || t("common.loading") }}
        </div>

        <component
          :is="assetViewComponent"
          v-if="uiStore.assetMounted && assetViewComponent"
          v-show="uiStore.activeTab === 'asset'"
          :working-dir="projectStore.workingDir"
        />
        <div
          v-else-if="uiStore.assetMounted && uiStore.activeTab === 'asset'"
          class="tab-loading-state"
          :class="{ 'is-loading': assetViewLoading, 'is-error': !!assetViewError }"
        >
          {{ assetViewError || t("common.loading") }}
        </div>

        <component
          :is="viewPackageViewComponent"
          v-if="uiStore.viewMounted && viewPackageViewComponent"
          v-show="uiStore.activeTab === 'views'"
          :working-dir="projectStore.workingDir"
        />
        <div
          v-else-if="uiStore.viewMounted && uiStore.activeTab === 'views'"
          class="tab-loading-state"
          :class="{ 'is-loading': viewPackageViewLoading, 'is-error': !!viewPackageViewError }"
        >
          {{ viewPackageViewError || t("common.loading") }}
        </div>

        <component
          :is="pluginViewComponent"
          v-if="showPluginEntry && uiStore.pluginsMounted && pluginViewComponent"
          v-show="uiStore.activeTab === 'plugins'"
          :working-dir="projectStore.workingDir"
        />
        <div
          v-else-if="showPluginEntry && uiStore.pluginsMounted && uiStore.activeTab === 'plugins'"
          class="tab-loading-state"
          :class="{ 'is-loading': pluginViewLoading, 'is-error': !!pluginViewError }"
        >
          {{ pluginViewError || t("common.loading") }}
        </div>

        <component
          :is="agentViewComponent"
          v-if="uiStore.agentMounted && agentViewComponent"
          v-show="uiStore.activeTab === 'agent'"
          :working-dir="projectStore.workingDir"
          :agent-list="[...agentStore.agents, ...agentStore.subagents]"
        />
        <div
          v-else-if="uiStore.agentMounted && uiStore.activeTab === 'agent'"
          class="tab-loading-state"
          :class="{ 'is-loading': agentViewLoading, 'is-error': !!agentViewError }"
        >
          {{ agentViewError || t("common.loading") }}
        </div>

        <component
          :is="settingsViewComponent"
          v-if="uiStore.settingsMounted && settingsViewComponent"
          v-show="uiStore.activeTab === 'settings'"
          :all-models="modelStore.availableModels"
          :agents="agentStore.agents"
          :subagents="agentStore.subagents"
          @auth-changed="handleSettingsAuthChanged"
          @model-defaults-changed="modelStore.applyModelDefaults"
          @codex-transport-changed="modelStore.applyCodexModelConfig"
          @custom-providers-changed="modelStore.applyCustomProviders"
          @reset-onboarding="onResetOnboarding"
        />
        <div
          v-else-if="uiStore.settingsMounted && uiStore.activeTab === 'settings'"
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
    v-if="recentDirContextMenu"
    class="recent-dir-ctx-menu"
    :x="recentDirContextMenu.x"
    :y="recentDirContextMenu.y"
    :min-width="180"
    :z-index="260"
    @close="closeRecentDirContextMenu"
  >
    <button
      type="button"
      class="recent-dir-ctx-item"
      @click="openContextRecentDirInFileExplorer"
    >
      <LucideIcon :icon="FolderOpen" :size="13" />
      {{ t("common.openInFileExplorer") }}
    </button>
    <button
      type="button"
      class="recent-dir-ctx-item"
      @click="configureContextRecentDirExtraWorkdirs"
    >
      <LucideIcon :icon="FolderCog" :size="13" />
      {{ t("app.dir.configureExtraWorkdirs") }}
    </button>
    <button
      type="button"
      class="recent-dir-ctx-item"
      @click="removeContextRecentDir"
    >
      <LucideIcon :icon="ListX" :size="13" />
      {{ t("app.dir.removeRecent") }}
    </button>
  </BaseContextMenu>
  <Transition name="workspace-switch-modal">
    <div
      v-if="pendingWorkspaceSwitchPath"
      class="workspace-switch-overlay"
      @click.self="closeWorkspaceSwitchDialog"
    >
      <div
        class="workspace-switch-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="workspace-switch-title"
      >
        <div class="workspace-switch-header">
          <span id="workspace-switch-title" class="workspace-switch-title">
            {{ t("app.dir.runningConfirmTitle") }}
          </span>
          <button
            class="workspace-switch-close"
            :disabled="workspaceSwitchBusy"
            @click="closeWorkspaceSwitchDialog"
          >
            <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </button>
        </div>
        <div class="workspace-switch-body">
          <p class="workspace-switch-message">
            {{ t("app.dir.runningConfirmMessage", String(runningSessionCount), workspaceSwitchTargetName) }}
          </p>
          <div class="workspace-switch-path">{{ pendingWorkspaceSwitchPath }}</div>
          <p class="workspace-switch-warning">
            {{ t("app.dir.runningConfirmWarning") }}
          </p>
        </div>
        <div class="workspace-switch-footer">
          <BaseButton :disabled="workspaceSwitchBusy" @click="closeWorkspaceSwitchDialog">
            {{ t("common.cancel") }}
          </BaseButton>
          <BaseButton
            variant="primary"
            :disabled="workspaceSwitchBusy"
            @click="confirmWorkspaceSwitch"
          >
            {{ t("app.dir.runningConfirmAction") }}
          </BaseButton>
        </div>
      </div>
    </div>
  </Transition>
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
  <LocusAssetInspectorPanel v-if="!isStandaloneWindow && locusAssetInspectorPanel.state.open" />
</template>

