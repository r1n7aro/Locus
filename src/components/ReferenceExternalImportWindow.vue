<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";
import ReferenceExternalImportPanel from "./knowledge/ReferenceExternalImportPanel.vue";
import {
  getReferenceExternalImportWindowPayload,
  REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
  REFERENCE_EXTERNAL_IMPORT_WINDOW_TITLE,
  type ReferenceExternalImportWindowPayload,
} from "../services/referenceExternalImportWindow";

const appWindow = getCurrentWindow();
const payload = ref<ReferenceExternalImportWindowPayload>(
  getReferenceExternalImportWindowPayload(),
);
const panelKey = ref(0);
const panelRunning = ref(false);
let payloadEventUnlisten: UnlistenFn | null = null;
let closeRequestUnlisten: UnlistenFn | null = null;
let allowWindowClose = false;

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

function normalizeRelativePath(path: string | null | undefined): string {
  return trimOrEmpty(path).replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function referencePathLabel(path: string | null | undefined): string {
  const normalized = normalizeRelativePath(path);
  return normalized ? `reference/${normalized}` : "reference";
}

function applyPayload(nextPayload: ReferenceExternalImportWindowPayload) {
  if (panelRunning.value) return;
  payload.value = {
    parentDir: normalizeRelativePath(nextPayload.parentDir),
    fixedTargetPath: normalizeRelativePath(nextPayload.fixedTargetPath),
    initialSource: nextPayload.initialSource === "unity" || nextPayload.initialSource === "local"
      ? nextPayload.initialSource
      : "feishu",
  };
  panelKey.value += 1;
}

const titlebarMeta = computed(() =>
  payload.value.fixedTargetPath
    ? referencePathLabel(payload.value.fixedTargetPath)
    : referencePathLabel(payload.value.parentDir),
);

const canCloseWindow = computed(() => !panelRunning.value);

async function destroyWindow() {
  allowWindowClose = true;
  closeRequestUnlisten?.();
  closeRequestUnlisten = null;
  try {
    await appWindow.setClosable(true);
  } catch {
    // ignore unsupported close state changes
  }
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through to destroy
  }
  try {
    await appWindow.destroy();
  } catch {
    // ignore destroy failures on teardown
  }
}

async function requestWindowClose() {
  if (!canCloseWindow.value) return;
  await destroyWindow();
}

async function initializeWindow() {
  try {
    await appWindow.setClosable(false);
  } catch {
    // ignore unsupported close state changes
  }

  try {
    closeRequestUnlisten = await appWindow.onCloseRequested((event) => {
      if (allowWindowClose || !panelRunning.value) return;
      event.preventDefault();
    });
    payloadEventUnlisten = await appWindow.listen<ReferenceExternalImportWindowPayload>(
      REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
      (event) => {
        applyPayload(event.payload ?? {});
      },
    );
  } catch {
    // keep the window usable even if event hooks are unavailable
  }
}

onMounted(() => {
  void initializeWindow();
});

watch(titlebarMeta, (value) => {
  const title = value
    ? `${REFERENCE_EXTERNAL_IMPORT_WINDOW_TITLE} · ${value}`
    : REFERENCE_EXTERNAL_IMPORT_WINDOW_TITLE;
  void appWindow.setTitle(title).catch(() => {
    // ignore unsupported title updates
  });
}, { immediate: true });

onUnmounted(() => {
  payloadEventUnlisten?.();
  closeRequestUnlisten?.();
});
</script>

<template>
  <div class="external-import-window-root">
    <div class="external-import-window-titlebar">
      <div class="external-import-window-titlebar-label">
        {{ t("knowledge.referenceFolder.external.createAction") }}
      </div>
      <div class="external-import-window-titlebar-actions">
        <div class="external-import-window-titlebar-meta">{{ titlebarMeta }}</div>
        <button
          type="button"
          class="external-import-window-close"
          :disabled="!canCloseWindow"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          @click="void requestWindowClose()"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14" aria-hidden="true">
            <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="external-import-window-scroll">
      <div class="external-import-window-body">
        <ReferenceExternalImportPanel
          :key="panelKey"
          mode="window"
          :parent-dir="payload.parentDir || ''"
          :fixed-target-path="payload.fixedTargetPath || null"
          :initial-source="payload.initialSource || 'feishu'"
          @close="void requestWindowClose()"
          @running-change="panelRunning = $event"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.external-import-window-root {
  position: relative;
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  border: 1px solid var(--border-strong);
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, white 8%, transparent),
    inset 0 0 0 1px color-mix(in srgb, var(--border-strong) 82%, transparent);
  overflow: hidden;
}

.external-import-window-titlebar {
  -webkit-app-region: drag;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 38px;
  padding: 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
  box-shadow: inset 0 1px 0 color-mix(in srgb, white 6%, transparent);
}

.external-import-window-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.external-import-window-titlebar-actions {
  min-width: 0;
  -webkit-app-region: no-drag;
  display: flex;
  align-items: center;
  gap: 12px;
}

.external-import-window-titlebar-meta {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
}

.external-import-window-close {
  -webkit-app-region: no-drag;
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
}

.external-import-window-close:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.external-import-window-close:disabled {
  opacity: 0.42;
  cursor: not-allowed;
}

.external-import-window-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.external-import-window-body {
  min-width: 0;
  padding: 18px;
}
</style>
