<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { Component } from "vue";
import { emit as emitTauriEvent } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";
import { provideDiffOverlay } from "../composables/useDiffOverlay";
import { useWorkspacePageBootstrap } from "../composables/useWorkspacePageBootstrap";
import { setLocusAssetInspectorPanelHostAvailable } from "../composables/useLocusAssetInspectorPanel";
import { normalizeAppError } from "../services/errors";
import {
  getWorkspacePageWindowPayload,
  WORKSPACE_PAGE_RESET_ONBOARDING_EVENT,
  type WorkspacePageId,
} from "../services/workspacePageWindow";
import {
  canStartWindowDragFromTarget,
  hasTauriWindowRuntime,
  showCurrentTauriWindow,
  startCurrentWindowDragging,
} from "../services/tauriRuntime";
import TopBannerHost from "./TopBannerHost.vue";

const PAGE_COMPONENTS = {
  collab: defineAsyncComponent(() => import("./CollabView.vue")),
  knowledge: defineAsyncComponent(() => import("./KnowledgeView.vue")),
  asset: defineAsyncComponent(() => import("./AssetView.vue")),
  views: defineAsyncComponent(() => import("./ViewPackageView.vue")),
  plugins: defineAsyncComponent(() => import("./PluginView.vue")),
  agent: defineAsyncComponent(() => import("./AgentView.vue")),
  settings: defineAsyncComponent(() => import("./SettingsView.vue")),
} as const;
const FileDiffOverlay = defineAsyncComponent(() => import("./diff/FileDiffOverlay.vue"));

const payload = getWorkspacePageWindowPayload();
const page = ref<WorkspacePageId | null>(payload?.page ?? null);
const pageTitle = ref(payload?.title ?? "Locus");
const bootstrapped = ref(false);
const bootstrapError = ref("");

const {
  uiStore,
  modelStore,
  agentStore,
  projectStore,
  bootstrap,
  refreshAuthAndModels,
  cleanup,
} = useWorkspacePageBootstrap();
const diffOverlay = provideDiffOverlay();
setLocusAssetInspectorPanelHostAvailable(false);

const pageComponent = computed<Component | null>(() =>
  page.value ? PAGE_COMPONENTS[page.value] : null,
);
const pageProps = computed<Record<string, unknown>>(() => {
  switch (page.value) {
    case "collab":
      return {
        workingDir: projectStore.workingDir,
        isActive: true,
        selectedModelId: modelStore.selectedModelId,
        selectedAgentId: agentStore.selectedAgentId,
        models: modelStore.availableModels,
        onSelectModel: (id: string) => modelStore.selectModel(id),
      };
    case "knowledge":
      return {
        workingDir: projectStore.workingDir,
        selectedModelId: modelStore.selectedModelId,
        modelDefaults: modelStore.modelDefaults,
      };
    case "agent":
      return {
        workingDir: projectStore.workingDir,
        agentList: [...agentStore.agents, ...agentStore.subagents],
      };
    case "settings":
      return {
        allModels: modelStore.availableModels,
        agents: agentStore.agents,
        subagents: agentStore.subagents,
        onAuthChanged: handleSettingsAuthChanged,
        onModelDefaultsChanged: modelStore.applyModelDefaults,
        onCodexTransportChanged: modelStore.applyCodexModelConfig,
        onCustomProvidersChanged: modelStore.applyCustomProviders,
        onResetOnboarding: requestResetOnboarding,
      };
    case "asset":
    case "views":
    case "plugins":
      return { workingDir: projectStore.workingDir };
    default:
      return {};
  }
});

async function handleSettingsAuthChanged() {
  await refreshAuthAndModels();
}

async function requestResetOnboarding() {
  if (hasTauriWindowRuntime()) {
    await emitTauriEvent(WORKSPACE_PAGE_RESET_ONBOARDING_EVENT);
    return;
  }
  projectStore.resetWorkspaceState();
  uiStore.resetOnboarding();
}

function handleTitlebarPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.detail > 1) return;
  if (!canStartWindowDragFromTarget(event.target)) return;
  event.preventDefault();
  startCurrentWindowDragging();
}

watch(pageTitle, (title) => {
  if (!hasTauriWindowRuntime()) return;
  void getCurrentWindow().setTitle(`Locus - ${title}`).catch(() => {});
}, { immediate: true });

onMounted(async () => {
  await nextTick();
  if (hasTauriWindowRuntime()) {
    void showCurrentTauriWindow().catch(() => {});
  }
  if (!page.value) {
    bootstrapError.value = t("app.tab.windowUnavailable");
    return;
  }
  try {
    await bootstrap(page.value);
    bootstrapped.value = true;
  } catch (cause) {
    bootstrapError.value = normalizeAppError(cause).message;
  }
});

onUnmounted(() => cleanup());
</script>

<template>
  <main class="workspace-page-window-root">
    <header
      class="workspace-page-window-titlebar"
      data-tauri-drag-region
      @pointerdown="handleTitlebarPointerDown"
    >
      <span class="workspace-page-window-title" data-tauri-drag-region>{{ pageTitle }}</span>
      <div class="workspace-page-window-drag-region" data-tauri-drag-region></div>
      <div class="workspace-page-window-controls" data-window-no-drag>
        <button
          type="button"
          class="workspace-page-window-control"
          :title="t('app.win.minimize')"
          @click="uiStore.winMinimize"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1" y="5.5" width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          type="button"
          class="workspace-page-window-control"
          :title="t('app.win.maximize')"
          @click="uiStore.winToggleMaximize"
        >
          <svg v-if="!uiStore.isMaximized" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
          <svg v-else viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1" />
            <rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1" />
          </svg>
        </button>
        <button
          type="button"
          class="workspace-page-window-control is-close"
          :title="t('app.win.close')"
          @click="uiStore.winClose"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </header>

    <TopBannerHost />
    <div v-if="bootstrapError" class="workspace-page-window-state is-error">
      {{ bootstrapError }}
    </div>
    <div v-else-if="!bootstrapped" class="workspace-page-window-state">
      {{ t("common.loading") }}
    </div>
    <section v-else class="workspace-page-window-content">
      <component
        :is="pageComponent"
        v-if="pageComponent"
        v-bind="pageProps"
      />
    </section>

    <FileDiffOverlay v-if="diffOverlay.visible.value" />
  </main>
</template>

<style scoped>
.workspace-page-window-root {
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  background: var(--panel-bg);
  color: var(--text-color);
}

.workspace-page-window-titlebar {
  -webkit-app-region: drag;
  position: relative;
  z-index: 120;
  flex: 0 0 38px;
  width: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.workspace-page-window-title {
  flex: 0 1 auto;
  min-width: 0;
  margin-left: 20px;
  overflow: hidden;
  color: var(--text-color);
  font-size: 14px;
  font-weight: 600;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-page-window-drag-region {
  -webkit-app-region: drag;
  flex: 1 1 40px;
  min-width: 24px;
  align-self: stretch;
}

.workspace-page-window-controls {
  -webkit-app-region: no-drag;
  position: relative;
  z-index: 2;
  flex: 0 0 126px;
  min-width: 126px;
  height: 100%;
  display: flex;
  align-items: stretch;
  margin-left: auto;
}

.workspace-page-window-control {
  width: 42px;
  height: 100%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.workspace-page-window-control:hover,
.workspace-page-window-control:focus-visible {
  outline: none;
  background: var(--hover-bg);
  color: var(--text-color);
}

.workspace-page-window-control.is-close:hover,
.workspace-page-window-control.is-close:focus-visible {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.workspace-page-window-state,
.workspace-page-window-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.workspace-page-window-state {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  color: var(--text-secondary);
  font-size: 13px;
}

.workspace-page-window-state.is-error {
  color: var(--status-danger-fg);
}

.workspace-page-window-content {
  display: flex;
  overflow: hidden;
}

.workspace-page-window-content > :deep(*) {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

:deep(.top-banner-host) {
  top: 44px;
}
</style>
