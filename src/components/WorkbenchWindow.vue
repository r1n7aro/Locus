<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { LogicalPosition, LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";
import { useAppBootstrap } from "../composables/useAppBootstrap";
import { provideDiffOverlay } from "../composables/useDiffOverlay";
import {
  WORKBENCH_WINDOW_POOL_CLAIM_EVENT,
  getWorkbenchAuxWindowRecord,
  isWorkbenchWindowPoolLocation,
  isWorkbenchWindowRestoreLocation,
  markWorkbenchWindowPoolReady,
  persistWorkbenchWindowBounds,
  recordWorkbenchWindowMetric,
  registerWorkbenchAuxWindow,
  unregisterWorkbenchAuxWindow,
  workbenchTransferTokenFromLocation,
  type WorkbenchWindowPoolClaimPayload,
} from "../services/workbenchWindow";
import {
  canStartWindowDragFromTarget,
  showCurrentTauriWindow,
  startCurrentWindowDragging,
} from "../services/tauriRuntime";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useWorkbenchStore } from "../stores/workbench";
import {
  removeSharedWorkbenchWindowHost,
  type SharedWorkbenchWindowHost,
} from "../services/sharedWorkbenchWindow";
import TopBannerHost from "./TopBannerHost.vue";

const props = defineProps<{
  sharedHost?: SharedWorkbenchWindowHost;
}>();

const developmentWorkbenchModule = import("./workbench/DevelopmentWorkbench.vue");
const DevelopmentWorkbench = defineAsyncComponent(
  () => developmentWorkbenchModule.then((module) => module.default),
);
const FileDiffOverlay = defineAsyncComponent(() => import("./diff/FileDiffOverlay.vue"));

const appWindow = props.sharedHost?.appWindow ?? getCurrentWindow();
const windowId = props.sharedHost?.label ?? appWindow.label;
const initiallyPool = !props.sharedHost && isWorkbenchWindowPoolLocation();
const restoring = props.sharedHost?.restoring ?? isWorkbenchWindowRestoreLocation();
const transferToken = ref(props.sharedHost?.transferToken ?? workbenchTransferTokenFromLocation());
const claimed = ref(!initiallyPool);
const bootstrapped = ref(!!props.sharedHost);
const bootstrapError = ref("");
const isMaximized = ref(false);
const workbenchMounted = computed(() => bootstrapped.value);
let allowNativeClose = false;
let closeRequest: Promise<void> | null = null;
let boundsTimer: ReturnType<typeof setTimeout> | null = null;
let unlistenPoolClaim: (() => void) | null = null;
let unlistenCloseRequested: (() => void) | null = null;
let unlistenMoved: (() => void) | null = null;
let unlistenResized: (() => void) | null = null;
let poolReadyMarked = false;
let windowRevealed = false;

const workspaceContextStore = useWorkspaceContextStore();
const workbenchStore = useWorkbenchStore();
const { bootstrapCritical, registerListeners, cleanup } = useAppBootstrap({
  syncActiveSessionSelection: false,
  handleExternalScriptOpen: false,
});
const diffOverlay = provideDiffOverlay();

function handleTitlebarPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || event.detail > 1) return;
  if (!canStartWindowDragFromTarget(event.target)) return;
  event.preventDefault();
  if (props.sharedHost) void appWindow.startDragging().catch(() => undefined);
  else startCurrentWindowDragging();
}

function scheduleBoundsPersist(): void {
  if (!claimed.value || boundsTimer) return;
  boundsTimer = setTimeout(() => {
    boundsTimer = null;
    void persistWorkbenchWindowBounds(appWindow).catch(() => undefined);
  }, 120);
}

async function revealWorkbenchWindow(
  token = transferToken.value,
  startedAt?: number,
): Promise<void> {
  if (windowRevealed) {
    void appWindow.setFocus().catch(() => undefined);
    return;
  }
  registerWorkbenchAuxWindow(windowId);
  await appWindow.show();
  windowRevealed = true;
  recordWorkbenchWindowMetric(props.sharedHost ? "shared-window-shown" : "window-shown", {
    token,
    startedAt,
    detail: { windowId, pooled: props.sharedHost?.pooled ?? false },
  });
  void appWindow.setFocus().catch(() => undefined);
  void persistWorkbenchWindowBounds(appWindow).catch(() => undefined);
}

async function applyPoolClaim(payload: WorkbenchWindowPoolClaimPayload): Promise<void> {
  if (claimed.value || !payload.token) return;
  await Promise.all([
    appWindow.setPosition(new LogicalPosition(payload.geometry.x, payload.geometry.y)),
    appWindow.setSize(new LogicalSize(payload.geometry.width, payload.geometry.height)),
  ]);
  transferToken.value = payload.token;
  claimed.value = true;
  registerWorkbenchAuxWindow(windowId, payload.geometry);
  await nextTick();
  await revealWorkbenchWindow(payload.token, payload.startedAt);
}

async function closeWorkbenchWindow(options: { discardDirty?: boolean; empty?: boolean } = {}): Promise<void> {
  if (closeRequest) return closeRequest;
  closeRequest = (async () => {
    const state = workbenchStore.ensureWindow(windowId);
    const dirty = Object.values(state.groups).flatMap((group) => group.tabs)
      .some((editor) => editor.dirty);
    if (dirty && options.discardDirty !== true) {
      const approved = await confirm(t("workbench.window.closeDirty"), {
        title: "Locus",
        kind: "warning",
      });
      if (!approved) return;
    }
    unregisterWorkbenchAuxWindow(windowId);
    workbenchStore.deleteWindow(windowId, { removeStorage: true });
    await workspaceContextStore.disposeWindow(windowId).catch(() => 0);
    allowNativeClose = true;
    if (props.sharedHost) await removeSharedWorkbenchWindowHost(windowId);
    else await appWindow.close();
  })().finally(() => {
    closeRequest = null;
  });
  return closeRequest;
}

async function toggleMaximize(): Promise<void> {
  await appWindow.toggleMaximize();
  isMaximized.value = await appWindow.isMaximized().catch(() => false);
  scheduleBoundsPersist();
}

function handleWorkbenchReady(): void {
  if (props.sharedHost?.pooled) return;
  if (initiallyPool && !claimed.value) {
    if (!poolReadyMarked) {
      poolReadyMarked = true;
      markWorkbenchWindowPoolReady(windowId);
    }
    return;
  }
  if (transferToken.value) return;
  if (!workbenchStore.hasEditors(windowId)) {
    void closeWorkbenchWindow({ discardDirty: true, empty: true });
    return;
  }
  void revealWorkbenchWindow();
}

function handleTransferReady(token: string, startedAt: number): void {
  if (windowRevealed) {
    recordWorkbenchWindowMetric("window-content-ready", {
      token,
      startedAt,
      detail: { windowId },
    });
    void persistWorkbenchWindowBounds(appWindow).catch(() => undefined);
    return;
  }
  recordWorkbenchWindowMetric("window-content-ready", {
    token,
    startedAt,
    detail: { windowId },
  });
  void nextTick(() => revealWorkbenchWindow(token, startedAt));
}

onMounted(async () => {
  try {
    if (!props.sharedHost) {
      unlistenPoolClaim = await appWindow.listen<WorkbenchWindowPoolClaimPayload>(
        WORKBENCH_WINDOW_POOL_CLAIM_EVENT,
        (event) => void applyPoolClaim(event.payload),
      );
    }
    unlistenCloseRequested = await appWindow.onCloseRequested((event) => {
      if (allowNativeClose) return;
      event.preventDefault();
      void closeWorkbenchWindow();
    });
    unlistenMoved = await appWindow.onMoved(scheduleBoundsPersist);
    unlistenResized = await appWindow.onResized(scheduleBoundsPersist);
    isMaximized.value = await appWindow.isMaximized().catch(() => false);
    if (restoring && getWorkbenchAuxWindowRecord(windowId)?.maximized) {
      await appWindow.maximize().catch(() => undefined);
      isMaximized.value = await appWindow.isMaximized().catch(() => true);
    }
    if (!props.sharedHost) await workspaceContextStore.initialize(windowId, "main");
    workbenchStore.switchWorkspaceScope(windowId, null);
    if (!props.sharedHost) {
      await bootstrapCritical();
      await registerListeners();
    }
    bootstrapped.value = true;
  } catch (error) {
    bootstrapError.value = error instanceof Error ? error.message : String(error);
    if (!initiallyPool) await showCurrentTauriWindow().catch(() => undefined);
  }
});

onUnmounted(() => {
  if (boundsTimer) clearTimeout(boundsTimer);
  boundsTimer = null;
  unlistenPoolClaim?.();
  unlistenPoolClaim = null;
  unlistenCloseRequested?.();
  unlistenCloseRequested = null;
  unlistenMoved?.();
  unlistenMoved = null;
  unlistenResized?.();
  unlistenResized = null;
    if (!props.sharedHost) cleanup();
});
</script>

<template>
  <main class="workbench-window-root">
    <header
      class="workbench-window-titlebar"
      data-tauri-drag-region
      @pointerdown="handleTitlebarPointerDown"
      @dblclick="toggleMaximize"
    >
      <span class="workbench-window-title" data-tauri-drag-region>Locus</span>
      <div class="workbench-window-drag-region" data-tauri-drag-region />
      <div class="workbench-window-controls" data-window-no-drag @dblclick.stop>
        <button
          type="button"
          class="workbench-window-control"
          :title="t('app.win.minimize')"
          @click="appWindow.minimize()"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1" y="5.5" width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          type="button"
          class="workbench-window-control"
          :title="t('app.win.maximize')"
          @click="toggleMaximize"
        >
          <svg v-if="!isMaximized" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
          <svg v-else viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1" />
            <rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1" />
          </svg>
        </button>
        <button
          type="button"
          class="workbench-window-control is-close"
          :title="t('app.win.close')"
          @click="closeWorkbenchWindow()"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </header>

    <TopBannerHost />
    <div v-if="bootstrapError" class="workbench-window-state is-error">{{ bootstrapError }}</div>
    <div v-else-if="!bootstrapped" class="workbench-window-state">{{ t("common.loading") }}</div>
    <DevelopmentWorkbench
      v-else-if="workbenchMounted"
      class="workbench-window-content"
      :window-id="windowId"
      :initial-transfer-token="transferToken"
      :native-window="appWindow"
      :owner-window="sharedHost?.browserWindow"
      :prewarm="sharedHost?.pooled"
      auxiliary
      :show-explorer="false"
      @ready="handleWorkbenchReady"
      @transfer-ready="handleTransferReady"
      @empty="closeWorkbenchWindow({ discardDirty: true, empty: true })"
    />

    <FileDiffOverlay v-if="diffOverlay.visible.value" />
  </main>
</template>

<style scoped>
.workbench-window-root {
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

.workbench-window-titlebar {
  -webkit-app-region: drag;
  position: relative;
  z-index: 120;
  flex: 0 0 32px;
  min-width: 0;
  display: flex;
  align-items: center;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.workbench-window-title {
  padding-left: 10px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  user-select: none;
}

.workbench-window-drag-region {
  flex: 1;
  align-self: stretch;
}

.workbench-window-controls {
  -webkit-app-region: no-drag;
  display: flex;
  height: 100%;
}

.workbench-window-control {
  width: 42px;
  height: 100%;
  padding: 0;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: default;
}

.workbench-window-control:hover,
.workbench-window-control:focus-visible {
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.workbench-window-control.is-close:hover,
.workbench-window-control.is-close:focus-visible {
  background: var(--status-error-fg);
  color: var(--bg-color);
}

.workbench-window-state,
.workbench-window-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.workbench-window-state {
  display: grid;
  place-items: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.workbench-window-state.is-error {
  color: var(--status-error-fg);
}
</style>
