<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import {
  knowledgeCloseLexicalProgressWindow,
  knowledgeGetLexicalRebuildStatus,
} from "../services/knowledge";
import {
  KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
  KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE,
} from "../services/knowledgeLexicalProgressWindow";
import type { LexicalRebuildStatus } from "../types";
import LucideIcon from "./icons/LucideIcon.vue";

type CloseReason = "success" | "error" | null;

const appWindow = getCurrentWindow();
const statusSnapshot = ref<LexicalRebuildStatus | null>(null);
const closeReason = ref<CloseReason>(null);
const statusError = ref("");

let closeTimer: ReturnType<typeof setTimeout> | null = null;
let statusUnlisten: RuntimeUnsubscribe | null = null;

function clearCloseTimer() {
  if (!closeTimer) return;
  clearTimeout(closeTimer);
  closeTimer = null;
}

function clampProgress(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function formatPercent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

async function destroyWindow() {
  clearCloseTimer();
  statusUnlisten?.();
  statusUnlisten = null;
  try {
    await appWindow.setClosable(true);
  } catch {
    // ignore unsupported close state changes on teardown
  }
  try {
    await knowledgeCloseLexicalProgressWindow();
  } catch {
    // fall back to local window handles when the command is unavailable
  }
  try {
    await appWindow.destroy();
    return;
  } catch {
    // fallback to close if destroy is unavailable
  }
  try {
    await appWindow.close();
    return;
  } catch {
    // ignore close failures on teardown
  }
}

function scheduleAutoClose(reason: Exclude<CloseReason, null>) {
  if (closeReason.value === reason || closeTimer) return;
  closeReason.value = reason;
  closeTimer = setTimeout(() => {
    closeTimer = null;
    void destroyWindow();
  }, reason === "success" ? 1200 : 2600);
}

async function requestWindowClose() {
  await destroyWindow();
}

function stageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "preparing":
      return t("knowledge.dashboard.knowledge.stagePreparing");
    case "cleaning":
      return t("knowledge.dashboard.knowledge.stageCleaning");
    case "indexing":
      return t("knowledge.dashboard.knowledge.stageIndexing");
    case "committing":
      return t("knowledge.dashboard.knowledge.stageCommitting");
    case "completed":
      return t("knowledge.dashboard.knowledge.stageCompleted");
    case "error":
      return t("settings.knowledge.stage.error");
    default:
      return t("knowledge.dashboard.knowledge.stageIdle");
  }
}

function applyStatus(nextStatus: LexicalRebuildStatus) {
  statusSnapshot.value = nextStatus;
  statusError.value = "";

  if (nextStatus.running) {
    closeReason.value = null;
    clearCloseTimer();
    return;
  }

  if (nextStatus.stage === "completed") {
    scheduleAutoClose("success");
    return;
  }

  if (nextStatus.stage === "error" || nextStatus.error) {
    scheduleAutoClose("error");
  }
}

async function loadInitialStatus() {
  try {
    const nextStatus = await knowledgeGetLexicalRebuildStatus();
    applyStatus(nextStatus);
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
  }
}

const progressRatio = computed(() => {
  if (closeReason.value === "success") return 1;
  const status = statusSnapshot.value;
  if (!status) return 0;
  if (typeof status.progress === "number") {
    return clampProgress(status.progress);
  }
  if (
    typeof status.processedDocs === "number"
    && typeof status.totalDocs === "number"
    && status.totalDocs > 0
  ) {
    return clampProgress(status.processedDocs / status.totalDocs);
  }
  switch (status.stage) {
    case "preparing":
      return 0.04;
    case "cleaning":
      return 0.12;
    case "committing":
      return 0.94;
    case "completed":
      return 1;
    default:
      return 0;
  }
});

const progressLabel = computed(() => formatPercent(progressRatio.value));
const currentStageLabel = computed(() => stageLabel(statusSnapshot.value?.stage));
const processedCaption = computed(() => {
  switch (statusSnapshot.value?.stage) {
    case "preparing":
      return t("knowledge.lexicalWindow.scanned");
    case "cleaning":
      return t("knowledge.lexicalWindow.cleaned");
    default:
      return t("knowledge.lexicalWindow.processed");
  }
});
const statusHeading = computed(() => {
  if (closeReason.value === "success") return t("knowledge.lexicalWindow.doneTitle");
  if (closeReason.value === "error") return t("knowledge.lexicalWindow.errorTitle");
  return t("knowledge.lexicalWindow.title");
});
const windowSubtitle = computed(() => {
  if (closeReason.value === "success") return t("knowledge.lexicalWindow.autoCloseSuccess");
  if (closeReason.value === "error") return t("knowledge.lexicalWindow.autoCloseError");
  if (statusError.value) return statusError.value;
  return statusSnapshot.value?.detail?.trim() || t("knowledge.lexicalWindow.waiting");
});
const processedLabel = computed(() => {
  const status = statusSnapshot.value;
  if (status?.processedDocs == null && status?.totalDocs == null) {
    return "—";
  }
  if (status?.totalDocs == null) return `${status?.processedDocs ?? 0}`;
  return `${status?.processedDocs ?? 0} / ${status.totalDocs}`;
});
const currentFileLabel = computed(() =>
  statusSnapshot.value?.currentFile?.trim() || t("knowledge.lexicalWindow.currentFileFallback"),
);

async function initializeWindow() {
  try {
    await appWindow.setTitle(KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE);
  } catch {
    // ignore unsupported title updates
  }
  try {
    await appWindow.setClosable(true);
  } catch {
    // ignore unsupported close state changes
  }

  try {
    statusUnlisten = await getLocusRuntime().subscribe<LexicalRebuildStatus>(
      KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
      applyStatus,
    );
  } catch {
    // initial status still renders if event subscription is unavailable
  }

  await loadInitialStatus();
}

onMounted(() => {
  void initializeWindow();
});

onUnmounted(() => {
  clearCloseTimer();
  statusUnlisten?.();
});
</script>

<template>
  <div class="lexical-window-root">
    <div class="lexical-window-titlebar">
      <div class="lexical-window-titlebar-label">{{ KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE }}</div>
      <button
        class="lexical-window-close"
        type="button"
        :aria-label="t('common.close')"
        :title="t('common.close')"
        @pointerdown.stop
        @mousedown.stop
        @click.stop="void requestWindowClose()"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div class="lexical-window-body-shell">
      <div class="lexical-window-shell">
        <div class="lexical-window-header">
          <div class="lexical-window-title">{{ statusHeading }}</div>
          <div class="lexical-window-subtitle">{{ windowSubtitle }}</div>
        </div>

        <div class="lexical-window-body">
          <div class="lexical-window-hero">
            <div class="lexical-window-progress">{{ progressLabel }}</div>
            <div class="lexical-window-progress-caption">
              {{ t("knowledge.lexicalWindow.progressCaption") }}
            </div>
          </div>

          <div class="lexical-window-track" aria-hidden="true">
            <div class="lexical-window-track-fill" :style="{ width: `${Math.round(progressRatio * 100)}%` }"></div>
          </div>

          <div class="lexical-window-meta">
            <div class="lexical-window-row">
              <span>{{ t("knowledge.dashboard.knowledge.rebuildStage") }}</span>
              <span>{{ currentStageLabel }}</span>
            </div>
            <div class="lexical-window-row">
              <span>{{ processedCaption }}</span>
              <span>{{ processedLabel }}</span>
            </div>
            <div class="lexical-window-row">
              <span>{{ t("settings.knowledge.currentFile") }}</span>
              <span class="truncate">{{ currentFileLabel }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lexical-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  overflow: hidden;
}

.lexical-window-titlebar {
  -webkit-app-region: drag;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 38px;
  padding: 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.lexical-window-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.lexical-window-close {
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
  transition: background 0.15s ease, color 0.15s ease;
}

.lexical-window-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.lexical-window-close:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 1px;
}

.lexical-window-close,
.lexical-window-close * {
  -webkit-app-region: no-drag;
}

.lexical-window-body-shell {
  flex: 1;
  min-height: 0;
  padding: 14px;
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--bg-color) 8%);
}

.lexical-window-shell {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
  overflow: hidden;
}

.lexical-window-header {
  padding: 16px 18px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.lexical-window-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-color);
}

.lexical-window-subtitle {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.lexical-window-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 18px;
  min-height: 0;
}

.lexical-window-hero {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.lexical-window-progress {
  font-size: 32px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.lexical-window-progress-caption {
  font-size: 12px;
  color: var(--text-secondary);
}

.lexical-window-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.lexical-window-track-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  border-radius: inherit;
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--accent-color) 74%, #ffffff 26%),
    var(--accent-color)
  );
  transition: width 0.18s ease;
}

.lexical-window-meta {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.lexical-window-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.lexical-window-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
}

.truncate {
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
