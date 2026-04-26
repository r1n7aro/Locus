
<script setup lang="ts">
import { computed, defineAsyncComponent, ref, shallowRef, onMounted, onUnmounted, watch } from "vue";
import type { Component, ShallowRef } from "vue";
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

import TopBannerHost from "./components/TopBannerHost.vue";
import BaseButton from "./components/ui/BaseButton.vue";
import AppUpdateModal from "./components/AppUpdateModal.vue";

import { provideDiffOverlay } from "./composables/useDiffOverlay";
import { initTheme } from "./composables/useTheme";
import { initFonts } from "./composables/useDisplaySettings";
import { isKnowledgeDownloadWindowLocation } from "./services/knowledgeDownloadWindow";
import { isKnowledgeLexicalProgressWindowLocation } from "./services/knowledgeLexicalProgressWindow";
import { isFeishuReferenceImportWindowLocation } from "./services/feishuReferenceImportWindow";
import { isUnityReferenceImportWindowLocation } from "./services/unityReferenceImportWindow";
import { isReferenceExternalImportWindowLocation } from "./services/referenceExternalImportWindow";
import { isUnityHostLocation } from "./services/locusRuntime";
const isCanvasWindow = window.location.pathname === '/canvas'
                    || window.location.search.includes('specId=');
const isUnityEmbedTestWindow = window.location.pathname === "/unity-embed-test";
const isUnityEmbedWindow = !isUnityEmbedTestWindow
  && (window.location.pathname === "/unity-embed" || isUnityHostLocation());
const isKnowledgeDownloadWindow = isKnowledgeDownloadWindowLocation();
const isKnowledgeLexicalProgressWindow = isKnowledgeLexicalProgressWindowLocation();
const isFeishuReferenceImportWindow = isFeishuReferenceImportWindowLocation();
const isUnityReferenceImportWindow = isUnityReferenceImportWindowLocation();
const isReferenceExternalImportWindow = isReferenceExternalImportWindowLocation();
const isStandaloneWindow = isCanvasWindow
  || isUnityEmbedWindow
  || isUnityEmbedTestWindow
  || isKnowledgeDownloadWindow
  || isKnowledgeLexicalProgressWindow
  || isFeishuReferenceImportWindow
  || isUnityReferenceImportWindow
  || isReferenceExternalImportWindow;

const CanvasView = defineAsyncComponent(() => import("./components/CanvasView.vue"));
const KnowledgeDownloadProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeDownloadProgressWindow.vue"));
const KnowledgeLexicalProgressWindow = defineAsyncComponent(() => import("./components/KnowledgeLexicalProgressWindow.vue"));
const FeishuReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/FeishuReferenceImportProgressWindow.vue"));
const UnityReferenceImportProgressWindow = defineAsyncComponent(() => import("./components/UnityReferenceImportProgressWindow.vue"));
const ReferenceExternalImportWindow = defineAsyncComponent(() => import("./components/ReferenceExternalImportWindow.vue"));
const UnityEmbeddedSessionView = defineAsyncComponent(() => import("./components/UnityEmbeddedSessionView.vue"));
const UnityEmbedTestView = defineAsyncComponent(() => import("./components/UnityEmbedTestView.vue"));
const OnboardingView = defineAsyncComponent(() => import("./components/OnboardingView.vue"));
const FileDiffOverlay = defineAsyncComponent(() => import("./components/diff/FileDiffOverlay.vue"));

// Initialize theme & fonts for main window only; Canvas keeps its own styles.
if (!isCanvasWindow) {
  initTheme();
  initFonts();
}

// -- Stores --
const uiStore = useUiStore();
const authStore = useAuthStore();
const agentStore = useAgentStore();
const modelStore = useModelStore();
const projectStore = useProjectStore();
const chatStore = useChatStore();
const notificationStore = useNotificationStore();
const appUpdateStore = useAppUpdateStore();
const unityEmbedBootstrapped = ref(false);
const unityEmbedBootstrapError = ref<string | null>(null);

// -- Diff overlay provider (must be called in App setup so all children can inject) --
const diffOverlay = provideDiffOverlay();
const { bootstrapCritical, bootstrapDeferred, preloadTabsInBackground, registerListeners, cleanup, applyWorkingDir, closeSettings, onOnboardingCompleted } = useAppBootstrap();

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

const agentViewComponent = agentView.component;
const agentViewLoading = agentView.loading;
const agentViewError = agentView.error;

const settingsViewComponent = settingsView.component;
const settingsViewLoading = settingsView.loading;
const settingsViewError = settingsView.error;

watch(() => uiStore.activeTab, (tab) => {
  if (tab !== "chat") return;
  void chatView.ensureLoaded();
}, { immediate: true });

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

watch(() => uiStore.agentMounted, (mounted) => {
  if (!mounted) return;
  void agentView.ensureLoaded();
}, { immediate: true });

watch(() => uiStore.settingsMounted, (mounted) => {
  if (!mounted) return;
  void settingsView.ensureLoaded();
}, { immediate: true });

// -- Workspace dropdown (local UI) --
const showDirDropdown = ref(false);
const dirDropdownRef = ref<HTMLElement | null>(null);
const pendingWorkspaceSwitchPath = ref<string | null>(null);
const switchingWorkspacePath = ref<string | null>(null);
const workspaceSwitchBusy = ref(false);
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
}

function closeWorkspaceSwitchDialog() {
  if (workspaceSwitchBusy.value) return;
  pendingWorkspaceSwitchPath.value = null;
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

async function selectRecentDir(dir: string) {
  if (workspaceSwitchBusy.value) return;
  showDirDropdown.value = false;
  await requestWorkingDirChange(dir);
}

async function browseFromDropdown() {
  if (workspaceSwitchBusy.value) return;
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

function handleDirClickOutside(e: MouseEvent) {
  if (dirDropdownRef.value && !dirDropdownRef.value.contains(e.target as Node)) {
    showDirDropdown.value = false;
  }
}

function onResetOnboarding() {
  showDirDropdown.value = false;
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

async function openAppUpdateChangelog() {
  const updateInfo = appUpdateStore.updateInfo;
  if (!updateInfo) return;

  try {
    await openUrl(updateInfo.changelogUrl);
    appUpdateStore.dismissDialog();
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", t("app.update.openFailed", err.message), {
      code: err.code,
      operation: "openAppUpdateChangelog",
      skipConsoleLog: true,
    });
  }
}

// -- Lifecycle --
onMounted(async () => {
  if (isUnityEmbedWindow) {
    try {
      await bootstrapCritical();
      await registerListeners();
    } catch (error) {
      const err = normalizeAppError(error);
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
  if (isStandaloneWindow) return;
  document.addEventListener("click", handleDirClickOutside, true);
  await bootstrapCritical();
  await registerListeners();
  // Sessions page is now interactive — kick off background work
  preloadTabsInBackground();
  void bootstrapDeferred();
  void appUpdateStore.checkForUpdates({ silent: true });
});

onUnmounted(() => {
  if (isUnityEmbedWindow) {
    cleanup();
    return;
  }
  if (isStandaloneWindow) return;
  document.removeEventListener("click", handleDirClickOutside, true);
  cleanup();
});
</script>

<template>
  <CanvasView v-if="isCanvasWindow" />
  <UnityEmbeddedSessionView
    v-else-if="isUnityEmbedWindow"
    :bootstrapped="unityEmbedBootstrapped"
    :bootstrap-error="unityEmbedBootstrapError"
  />
  <UnityEmbedTestView v-else-if="isUnityEmbedTestWindow" />
  <KnowledgeDownloadProgressWindow v-else-if="isKnowledgeDownloadWindow" />
  <KnowledgeLexicalProgressWindow v-else-if="isKnowledgeLexicalProgressWindow" />
  <FeishuReferenceImportProgressWindow v-else-if="isFeishuReferenceImportWindow" />
  <UnityReferenceImportProgressWindow v-else-if="isUnityReferenceImportWindow" />
  <ReferenceExternalImportWindow v-else-if="isReferenceExternalImportWindow" />
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
  >
    <div class="main-area">
      <div class="tab-bar">
        <div class="tab-drag-region" aria-hidden="true"></div>
        <span class="tab-brand">Locus</span>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'chat' }"
          @click="uiStore.setTab('chat')"
        >{{ t("app.tab.dev") }}</button>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'knowledge' }"
          @click="uiStore.setTab('knowledge')"
        >{{ t("app.tab.knowledge") }}</button>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'collab' }"
          @click="uiStore.setTab('collab')"
        >{{ t("app.tab.collab") }}</button>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'asset' }"
          @click="uiStore.setTab('asset')"
        >{{ t("app.tab.asset") }}</button>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'agent' }"
          @click="uiStore.setTab('agent')"
        >{{ t("app.tab.agent") }}</button>
        <button
          class="tab-item"
          :class="{ active: uiStore.activeTab === 'settings' }"
          @click="uiStore.setTab('settings')"
        >{{ t("app.tab.settings") }}</button>
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
              <div
                v-for="dir in projectStore.recentDirs"
                :key="dir"
                class="dir-item"
                :class="{ active: dir === projectStore.workingDir }"
                @click="selectRecentDir(dir)"
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
          @close="closeSettings"
          @auth-changed="handleSettingsAuthChanged"
          @model-defaults-changed="modelStore.applyModelDefaults"
          @codex-transport-changed="modelStore.applyCodexModelConfig"
          @custom-endpoints-changed="modelStore.applyCustomEndpoints"
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
    @view="openAppUpdateChangelog"
  />
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
  <FileDiffOverlay v-if="diffOverlay.visible.value" />
</template>

<style>
:root {
  --bg-color: #f6f7f8;
  --sidebar-bg: #f3f4f6;
  --panel-bg: #ffffff;
  --surface-elevated: #ffffff;
  --text-color: #111318;
  --text-secondary: #5b6270;
  --text-tertiary: #7b8393;
  --radius-badge: 8px;
  --border-color: #e3e5e8;
  --border-strong: #d4d7dd;
  --hover-bg: #eef1f4;
  --active-bg: #e7ebf0;
  --input-bg: #f8f9fb;
  --msg-user-bg: #eff0f1;
  --msg-assistant-bg: #f9fafb;
  --msg-divider: color-mix(in srgb, var(--border-strong) 74%, var(--border-color) 26%);
  --msg-user-role: color-mix(in srgb, var(--text-color) 76%, var(--text-secondary) 24%);
  --accent-color: #4c5bd4;
  --accent-soft: rgba(76, 91, 212, 0.10);
  --accent-border: rgba(76, 91, 212, 0.22);

  --git-surface-nav: #f5f7fb;
  --git-surface-history: #fbfcfe;
  --git-surface-detail: #ffffff;
  --git-surface-header: #eef2f8;
  --git-surface-subheader: #f6f8fc;
  --git-surface-terminal: #f9fbfd;
  --git-row-hover: #eef4fb;
  --git-row-selected: #e4edfb;
  --git-divider: #d7dee8;
  --git-divider-strong: #b8c6d8;
  --git-text-primary: #18212b;
  --git-text-secondary: #526173;
  --git-text-tertiary: #748397;
  --git-focus: #2f6df6;
  --git-status-added: #2ea043;
  --git-status-modified: #d29b00;
  --git-status-deleted: #e15759;
  --git-status-renamed: #5d83b4;
  --git-status-stash: #8a63d2;
  --git-status-conflict: #e28a2e;
  --git-section-staged-bg: #f3fbf7;
  --git-section-staged-border: #69b587;
  --git-section-unstaged-bg: #fff8ef;
  --git-section-unstaged-border: #d4a658;
  --git-section-conflict-bg: #ffebe7;
  --git-section-conflict-border: #e7a18f;

  --status-good-fg: #18794e;
  --status-good-bg: rgba(24, 121, 78, 0.10);
  --status-good-border: rgba(24, 121, 78, 0.18);
  --status-warn-fg: #a16207;
  --status-warn-bg: rgba(202, 138, 4, 0.10);
  --status-warn-border: rgba(202, 138, 4, 0.18);
  --status-danger-fg: #b42318;
  --status-danger-bg: rgba(217, 45, 32, 0.10);
  --status-danger-border: rgba(217, 45, 32, 0.18);

  font-family: var(--font-ui);
  font-size: 14px;
  line-height: 1.5;
  color: var(--text-color);
  background: var(--bg-color);

  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

:root[data-theme="dark"] {
  --bg-color: #1d1d21;
  --sidebar-bg: #17181c;
  --panel-bg: #111216;
  --surface-elevated: #1a1b20;
  --text-color: #f3f3f5;
  --text-secondary: #a1a4ad;
  --text-tertiary: #787b84;
  --border-color: #26282e;
  --border-strong: #31333b;
  --hover-bg: #1d1f25;
  --active-bg: #23252c;
  --input-bg: #181a20;
  --msg-user-bg: #212125;
  --msg-assistant-bg: #17181d;
  --msg-divider: color-mix(in srgb, var(--border-strong) 76%, var(--border-color) 24%);
  --msg-user-role: color-mix(in srgb, var(--text-color) 74%, var(--text-secondary) 26%);
  --accent-color: #6f77f6;
  --accent-soft: rgba(111, 119, 246, 0.12);
  --accent-border: rgba(111, 119, 246, 0.24);

  --git-surface-nav: #17181c;
  --git-surface-history: #141519;
  --git-surface-detail: #17181d;
  --git-surface-header: #1b1d23;
  --git-surface-subheader: #16181d;
  --git-surface-terminal: #121318;
  --git-row-hover: #1f2128;
  --git-row-selected: #262932;
  --git-divider: #2c2f36;
  --git-divider-strong: #383c46;
  --git-text-primary: #f3f3f5;
  --git-text-secondary: #c0c4cc;
  --git-text-tertiary: #9498a1;
  --git-focus: #7c84ff;
  --git-status-added: #7fd39b;
  --git-status-modified: #f2c56b;
  --git-status-deleted: #ff9698;
  --git-status-renamed: #a9c2e6;
  --git-status-stash: #ccb8ff;
  --git-status-conflict: #f6b267;
  --git-section-staged-bg: #17261f;
  --git-section-staged-border: #4f8d68;
  --git-section-unstaged-bg: #342716;
  --git-section-unstaged-border: #b68035;
  --git-section-conflict-bg: #311919;
  --git-section-conflict-border: #a65e50;

  --status-good-fg: #6dcf9b;
  --status-good-bg: rgba(109, 207, 155, 0.14);
  --status-good-border: rgba(109, 207, 155, 0.28);
  --status-warn-fg: #f1c069;
  --status-warn-bg: rgba(241, 192, 105, 0.14);
  --status-warn-border: rgba(241, 192, 105, 0.28);
  --status-danger-fg: #ff8a8a;
  --status-danger-bg: rgba(255, 138, 138, 0.14);
  --status-danger-border: rgba(255, 138, 138, 0.30);
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--bg-color);
}

#app {
  display: flex;
}

/* Global scrollbar styling */
::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: rgba(0, 0, 0, 0.15);
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: rgba(0, 0, 0, 0.25);
}

:root[data-theme="dark"] ::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.15);
}

:root[data-theme="dark"] ::-webkit-scrollbar-thumb:hover {
  background: rgba(255, 255, 255, 0.25);
}

body {
  overflow: hidden;
  background: var(--bg-color);
  user-select: none;
  -webkit-user-select: none;
}

body.is-dragging-select-lock,
body.is-dragging-select-lock * {
  user-select: none !important;
}

.app-layout {
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  position: relative;
  overflow: hidden;
  background: var(--bg-color);
}

.app-layout.is-window-resizing .tab-content {
  pointer-events: none;
}

.app-layout.is-window-resizing :is(
  .chat-workspace-view,
  .chat-view-layout,
  .chat-view,
  .chat-main,
  .input-area,
  .chat-input-shell,
  .chat-input-shell-body,
  .chat-input-shell-stack,
  .chat-composer,
  .chat-transcript-scroll,
  .chat-transcript-content
) {
  transition: none !important;
}

.app-layout.is-window-resizing .chat-transcript-message.is-session {
  content-visibility: visible;
  contain-intrinsic-size: auto;
}

.app-layout.is-window-resizing :is(
  .workspace-btn,
  .tab-item,
  .win-ctrl-btn,
  .session-divider,
  .input-controls-toggle,
  .changes-toggle-btn,
  .sp-collapse-btn,
  .sp-session-item
) {
  transition-duration: 0s !important;
  transition-delay: 0s !important;
}

.app-layout.is-window-resizing :is(
  .tab-plugin-spinner,
  .workspace-switch-spinner,
  .chat-transcript-thinking-spinner,
  .sp-session-dot.is-running,
  .sp-expand-btn.is-running svg
) {
  animation-duration: 0s !important;
  animation-delay: 0s !important;
}

.ui-select-none {
  user-select: none;
  -webkit-user-select: none;
}

.ui-select-text,
.selectable-text,
:where(pre, code),
:is(input, textarea, [contenteditable="true"]) {
  user-select: text;
  -webkit-user-select: text;
}

.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  position: relative;
}

.app-startup-state {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100vw;
  height: 100vh;
  background: var(--bg-color);
  color: var(--text-secondary);
  font-size: 13px;
}

.tab-bar {
  --window-resize-hit-area: 6px;
  display: flex;
  align-items: center;
  gap: 0;
  padding: 0 0 0 16px;
  height: 38px;
  flex-shrink: 0;
  position: relative;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
  z-index: 20;
  overflow: visible;
  -webkit-app-region: no-drag;
}

.tab-drag-region {
  position: absolute;
  inset: var(--window-resize-hit-area) var(--window-resize-hit-area) 0 var(--window-resize-hit-area);
  z-index: 0;
  -webkit-app-region: drag;
}

.tab-bar > :not(.tab-drag-region) {
  position: relative;
  z-index: 1;
}

.tab-item {
  -webkit-app-region: no-drag;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  position: relative;
  padding: 0 14px;
  height: 100%;
  border: none;
  background: none;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: color 0.15s ease;
  line-height: 1;
  white-space: nowrap;
}

.tab-item:hover {
  color: var(--text-color);
}

.tab-item.active {
  color: var(--text-color);
}

.tab-item.active::after {
  content: "";
  position: absolute;
  bottom: 1px;
  left: 14px;
  right: 14px;
  height: 1px;
  background: var(--accent-color);
  border-radius: 999px;
  opacity: 0.72;
}

.tab-brand {
  -webkit-app-region: no-drag;
  flex: 0 0 auto;
  font-size: 14px;
  font-weight: 650;
  letter-spacing: -0.2px;
  margin-right: 10px;
  color: var(--text-color);
  white-space: nowrap;
}

.tab-spacer {
  -webkit-app-region: drag;
  flex: 1 1 auto;
  min-width: 8px;
  align-self: stretch;
}

.window-controls {
  -webkit-app-region: no-drag;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  margin-left: 10px;
  height: 100%;
}

.win-ctrl-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 42px;
  height: 100%;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s, color 0.1s;
}

.win-ctrl-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.win-ctrl-btn.win-pinned {
  color: var(--accent-color);
}

.win-ctrl-btn.win-close:hover {
  background: #e81123;
  color: #fff;
}

.workspace-selector {
  -webkit-app-region: no-drag;
  flex: 0 1 320px;
  width: 320px;
  min-width: 120px;
  max-width: 320px;
  position: relative;
  margin-right: 6px;
}

.workspace-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--sidebar-bg) 22%);
  color: var(--text-color);
  font-size: 12px;
  line-height: 21px;
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
  width: 100%;
  min-width: 0;
  max-width: none;
  height: 23px;
}

.workspace-btn:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

.workspace-btn:disabled {
  cursor: progress;
  opacity: 0.86;
}

.workspace-btn.is-switching {
  color: var(--text-secondary);
}

.ws-icon {
  opacity: 0.7;
  flex-shrink: 0;
}

.workspace-btn:hover .ws-icon {
  opacity: 1;
}

.ws-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
  text-align: center;
}

.ws-chevron {
  opacity: 0.4;
  flex-shrink: 0;
  margin-left: auto;
  transition: transform 0.2s;
}

.ws-chevron.open {
  transform: rotate(180deg);
}

.workspace-switch-spinner {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, currentColor 18%, transparent);
  border-top-color: currentColor;
  animation: workspace-switch-spin 0.8s linear infinite;
}

.dir-dropdown {
  position: absolute;
  right: 0;
  top: calc(100% + 6px);
  width: 280px;
  background: var(--surface-elevated);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  box-shadow: 0 10px 28px rgba(15, 17, 21, 0.12);
  z-index: 200;
  padding: 4px;
  max-height: 320px;
  overflow-y: auto;
}

:root[data-theme="dark"] .dir-dropdown {
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.34);
}

.dropdown-label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  padding: 6px 8px 4px;
}

.dir-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.12s;
  font-size: 12px;
  color: var(--text-color);
}

.dir-item:hover {
  background: var(--hover-bg);
}

.dir-item.active {
  background: var(--active-bg);
}

.dir-item-icon {
  opacity: 0.45;
  flex-shrink: 0;
}

.dir-item:hover .dir-item-icon {
  opacity: 0.7;
}

.dir-item-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.dir-item-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.dir-item-path {
  font-size: 10px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dir-check {
  font-size: 11px;
  color: var(--accent-color);
  flex-shrink: 0;
  opacity: 0.72;
}

.dropdown-empty {
  text-align: center;
  font-size: 11px;
  color: var(--text-secondary);
  padding: 8px;
}

.dropdown-divider {
  height: 1px;
  background: var(--border-color);
  margin: 4px 4px;
}

.dir-item.browse {
  color: var(--text-secondary);
}

.dir-item.browse:hover {
  color: var(--text-color);
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

.workspace-switch-overlay {
  position: fixed;
  inset: 0;
  z-index: 320;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: rgba(10, 12, 16, 0.32);
  backdrop-filter: blur(2px);
}

.workspace-switch-dialog {
  width: 420px;
  max-width: min(420px, calc(100vw - 32px));
  border-radius: 10px;
  border: 1px solid var(--border-color);
  background: var(--surface-elevated);
  box-shadow: 0 16px 34px rgba(15, 17, 21, 0.18);
  overflow: hidden;
}

:root[data-theme="dark"] .workspace-switch-dialog {
  box-shadow: 0 18px 36px rgba(0, 0, 0, 0.4);
}

.workspace-switch-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px 12px;
  border-bottom: 1px solid var(--border-color);
}

.workspace-switch-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.workspace-switch-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.workspace-switch-close:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.workspace-switch-close:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.workspace-switch-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 16px;
}

.workspace-switch-message,
.workspace-switch-warning {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
}

.workspace-switch-message {
  color: var(--text-secondary);
}

.workspace-switch-path {
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.5;
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}

.workspace-switch-warning {
  padding: 8px 10px;
  border: 1px solid var(--status-warn-border);
  border-radius: 6px;
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
}

.workspace-switch-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 16px 16px;
  border-top: 1px solid var(--border-color);
}

.workspace-switch-modal-enter-active,
.workspace-switch-modal-leave-active {
  transition: opacity 0.16s ease;
}

.workspace-switch-modal-enter-active .workspace-switch-dialog,
.workspace-switch-modal-leave-active .workspace-switch-dialog {
  transition: transform 0.16s ease, opacity 0.16s ease;
}

.workspace-switch-modal-enter-from,
.workspace-switch-modal-leave-to {
  opacity: 0;
}

.workspace-switch-modal-enter-from .workspace-switch-dialog,
.workspace-switch-modal-leave-to .workspace-switch-dialog {
  opacity: 0;
  transform: translateY(6px) scale(0.98);
}

.tab-content {
  flex: 1;
  display: flex;
  position: relative;
  z-index: 0;
  min-height: 0;
  overflow: hidden;
}

.tab-content > :is(.chat-workspace-view, .chat-view-layout, .collab-view, .knowledge-view, .asset-view, .agent-view, .settings-panel) {
  min-width: 0;
  min-height: 0;
  contain: layout paint;
}

.tab-loading-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 13px;
}

.tab-loading-state.is-error {
  color: var(--status-danger-fg);
}

.tab-plugin-warn {
  -webkit-app-region: no-drag;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px;
  height: 23px;
  margin-left: 8px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--status-danger-bg) 78%, var(--panel-bg) 22%);
  border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color) 28%);
  color: var(--status-danger-fg);
  cursor: pointer;
  font: inherit;
  box-shadow: 0 4px 12px rgba(15, 23, 42, 0.08);
  transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
  white-space: nowrap;
}

.tab-plugin-warn:hover:not(:disabled),
.tab-plugin-warn:focus-visible {
  background: color-mix(in srgb, var(--status-danger-bg) 88%, var(--panel-bg) 12%);
  border-color: color-mix(in srgb, var(--status-danger-fg) 42%, var(--status-danger-border) 58%);
}

.tab-plugin-warn:focus-visible {
  outline: none;
}

.tab-plugin-warn:disabled {
  cursor: progress;
  opacity: 0.82;
}

.tab-plugin-icon {
  flex-shrink: 0;
  opacity: 0.86;
}

.tab-plugin-spinner {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, currentColor 18%, transparent);
  border-top-color: currentColor;
  animation: tab-plugin-spin 0.8s linear infinite;
}

.tab-plugin-label {
  font-size: 12px;
  font-weight: 500;
  color: currentColor;
  white-space: nowrap;
}

.tab-plugin-action {
  font-size: 11px;
  color: color-mix(in srgb, currentColor 76%, var(--text-secondary) 24%);
  white-space: nowrap;
}

@keyframes tab-plugin-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes workspace-switch-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
