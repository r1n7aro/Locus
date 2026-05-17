<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n";
import BaseButton from "./ui/BaseButton.vue";
import {
  knowledgeCancelLocalEmbeddingModelDownload,
  knowledgeCloseDownloadProgressWindow,
  knowledgeGetEmbeddingStatus,
} from "../services/knowledge";
import { normalizeAppError } from "../services/errors";
import {
  getKnowledgeDownloadWindowModelId,
  KNOWLEDGE_DOWNLOAD_WINDOW_MODEL_EVENT,
  KNOWLEDGE_DOWNLOAD_WINDOW_TITLE,
} from "../services/knowledgeDownloadWindow";
import type { EmbeddingStatus } from "../types";

type CloseReason = "success" | "error" | "cancelled" | null;
type MetaSummary = {
  primary: string;
  secondary: string;
};

const appWindow = getCurrentWindow();
const requestedModelId = ref(getKnowledgeDownloadWindowModelId());
const statusSnapshot = ref<EmbeddingStatus | null>(null);
const closeReason = ref<CloseReason>(null);
const pollError = ref("");
const hasSeenActiveStage = ref(false);
const consecutiveErrorPolls = ref(0);
const cancelPending = ref(false);

let pollTimer: ReturnType<typeof setTimeout> | null = null;
let closeTimer: ReturnType<typeof setTimeout> | null = null;
let modelEventUnlisten: UnlistenFn | null = null;
let closeRequestUnlisten: UnlistenFn | null = null;
let allowWindowClose = false;

function clearPollTimer() {
  if (!pollTimer) return;
  clearTimeout(pollTimer);
  pollTimer = null;
}

function clearCloseTimer() {
  if (!closeTimer) return;
  clearTimeout(closeTimer);
  closeTimer = null;
}

function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function formatPercent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

function stageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "preparing":
      return t("settings.knowledge.stage.preparing");
    case "downloading_model":
      return t("settings.knowledge.stage.downloadingModel");
    case "cancelling":
      return t("settings.knowledge.stage.cancelling");
    case "cancelled":
      return t("settings.knowledge.stage.cancelled");
    case "initializing_runtime":
      return t("settings.knowledge.stage.initializingRuntime");
    case "ready":
      return t("settings.knowledge.stage.ready");
    case "error":
      return t("settings.knowledge.stage.error");
    default:
      return t("knowledge.retrieval.downloadWindowWaitingStage");
  }
}

function downloadSourceLabel(source: string | null | undefined): string {
  return source === "hf-mirror"
    ? t("knowledge.retrieval.downloadSourceMirror")
    : t("knowledge.retrieval.downloadSourceOfficial");
}

function resetDownloadSession(modelId: string) {
  requestedModelId.value = modelId.trim();
  statusSnapshot.value = null;
  closeReason.value = null;
  pollError.value = "";
  hasSeenActiveStage.value = false;
  consecutiveErrorPolls.value = 0;
  cancelPending.value = false;
  clearCloseTimer();
  schedulePoll(220);
}

function schedulePoll(delay = 260) {
  clearPollTimer();
  pollTimer = setTimeout(() => {
    pollTimer = null;
    void refreshStatus();
  }, delay);
}

function summarizeProxyStatus(): MetaSummary {
  const route = statusSnapshot.value?.downloadNetwork;
  if (!route) {
    return {
      primary: "—",
      secondary: "",
    };
  }

  switch (route.proxyState) {
    case "environment":
      return {
        primary: t("knowledge.retrieval.downloadWindowProxyEnvironment"),
        secondary: [route.proxyEnvKey, route.proxyUrl].filter(Boolean).join(" · "),
      };
    case "manual":
      return {
        primary: t("knowledge.retrieval.downloadWindowProxyManual"),
        secondary: route.proxyUrl || "",
      };
    case "manual_unsupported":
      return {
        primary: t("knowledge.retrieval.downloadWindowProxyUnsupported"),
        secondary: route.proxyUrl
          ? `${route.proxyUrl} · ${t("knowledge.retrieval.downloadWindowProxyUnsupportedDetail")}`
          : t("knowledge.retrieval.downloadWindowProxyUnsupportedDetail"),
      };
    case "system":
      return {
        primary: t("knowledge.retrieval.downloadWindowProxySystem"),
        secondary: route.proxyUrl || "",
      };
    case "system_unsupported":
      return {
        primary: t("knowledge.retrieval.downloadWindowProxyUnsupported"),
        secondary: route.proxyUrl
          ? `${route.proxyUrl} · ${t("knowledge.retrieval.downloadWindowProxyUnsupportedDetail")}`
          : t("knowledge.retrieval.downloadWindowProxyUnsupportedDetail"),
      };
    default:
      return {
        primary: t("knowledge.retrieval.downloadWindowProxyDirect"),
        secondary: "",
      };
  }
}

async function destroyWindow() {
  clearPollTimer();
  clearCloseTimer();
  allowWindowClose = true;
  closeRequestUnlisten?.();
  closeRequestUnlisten = null;
  try {
    await knowledgeCloseDownloadProgressWindow();
    return;
  } catch {
    // fall back to local window handles when the command is unavailable
  }
  try {
    await appWindow.setClosable(true);
  } catch {
    // ignore unsupported close state changes on teardown
  }
  try {
    await appWindow.close();
    return;
  } catch {
    // fallback to destroy if close is unavailable
  }
  try {
    await appWindow.destroy();
  } catch {
    // ignore destroy failures on teardown
  }
}

function scheduleAutoClose(reason: Exclude<CloseReason, null>) {
  if (closeReason.value === reason || closeTimer) return;
  closeReason.value = reason;
  clearPollTimer();
  closeTimer = setTimeout(() => {
    closeTimer = null;
    void destroyWindow();
  }, reason === "success" ? 1200 : 2200);
}

function detailIncludesRequestedModel(detail: string | null | undefined): boolean {
  const modelId = requestedModelId.value.trim();
  if (!modelId || !detail) return false;
  return detail.includes(modelId);
}

async function cancelDownload() {
  if (cancelPending.value || closeReason.value) return;
  cancelPending.value = true;
  pollError.value = "";
  try {
    await knowledgeCancelLocalEmbeddingModelDownload();
    schedulePoll(140);
  } catch (cause) {
    cancelPending.value = false;
    pollError.value = normalizeAppError(cause).message;
  }
}

async function refreshStatus() {
  try {
    const nextStatus = await knowledgeGetEmbeddingStatus();
    statusSnapshot.value = nextStatus;
    pollError.value = "";

    if (
      nextStatus.activating
      || nextStatus.stage === "preparing"
      || nextStatus.stage === "downloading_model"
      || nextStatus.stage === "initializing_runtime"
    ) {
      hasSeenActiveStage.value = true;
    }

    if (!nextStatus.activating && (nextStatus.stage === "error" || !!nextStatus.error)) {
      consecutiveErrorPolls.value += 1;
    } else {
      consecutiveErrorPolls.value = 0;
    }

    const downloadSucceeded =
      !nextStatus.activating
      && nextStatus.stage === "ready"
      && (hasSeenActiveStage.value || detailIncludesRequestedModel(nextStatus.detail));

    const downloadFailed =
      !nextStatus.activating
      && (nextStatus.stage === "error" || !!nextStatus.error)
      && (hasSeenActiveStage.value || consecutiveErrorPolls.value >= 2);

    const downloadCancelled =
      !nextStatus.activating
      && nextStatus.stage === "cancelled"
      && (hasSeenActiveStage.value || cancelPending.value);

    if (downloadSucceeded) {
      scheduleAutoClose("success");
      return;
    }

    if (downloadCancelled) {
      cancelPending.value = false;
      scheduleAutoClose("cancelled");
      return;
    }

    if (downloadFailed) {
      cancelPending.value = false;
      scheduleAutoClose("error");
      return;
    }

    schedulePoll();
  } catch (cause) {
    pollError.value = normalizeAppError(cause).message;
    cancelPending.value = false;
    schedulePoll(600);
  }
}

const progressRatio = computed(() => {
  if (closeReason.value === "success") return 1;
  const status = statusSnapshot.value;
  if (!status) return 0;
  if (typeof status.modelDownloadProgress === "number") {
    return Math.min(1, Math.max(0, status.modelDownloadProgress));
  }
  if (status.totalBytes && typeof status.downloadedBytes === "number") {
    return Math.min(1, Math.max(0, status.downloadedBytes / status.totalBytes));
  }
  return 0;
});

const progressLabel = computed(() => formatPercent(progressRatio.value));
const waitingForCurrentSession = computed(() => {
  const status = statusSnapshot.value;
  if (!status) return true;
  if (closeReason.value || hasSeenActiveStage.value) return false;
  if (status.stage === "ready" && !detailIncludesRequestedModel(status.detail)) return true;
  if (status.stage === "error" && consecutiveErrorPolls.value < 2) return true;
  return false;
});
const currentStageLabel = computed(() => {
  const status = statusSnapshot.value;
  if (!status) return stageLabel(null);
  if (waitingForCurrentSession.value) return stageLabel(null);
  return stageLabel(status.stage);
});
const downloadedBytesLabel = computed(() => {
  const status = statusSnapshot.value;
  if (status?.downloadedBytes == null && status?.totalBytes == null) {
    return "—";
  }
  return `${formatBytes(status?.downloadedBytes)} / ${formatBytes(status?.totalBytes)}`;
});
const downloadSourceSummary = computed<MetaSummary>(() => {
  const route = statusSnapshot.value?.downloadNetwork;
  if (!route) {
    return {
      primary: "—",
      secondary: "",
    };
  }
  return {
    primary: downloadSourceLabel(route.source),
    secondary: route.endpoint || "",
  };
});
const proxySummary = computed<MetaSummary>(() => summarizeProxyStatus());
const statusHeading = computed(() => {
  if (closeReason.value === "success") return t("knowledge.retrieval.downloadWindowDoneTitle");
  if (closeReason.value === "cancelled") return t("knowledge.retrieval.downloadWindowCancelledTitle");
  if (closeReason.value === "error") return t("knowledge.retrieval.downloadWindowErrorTitle");
  return t("knowledge.retrieval.downloadWindowTitle");
});
const windowSubtitle = computed(() => {
  if (closeReason.value === "success") {
    return t("knowledge.retrieval.downloadWindowAutoCloseSuccess");
  }
  if (closeReason.value === "cancelled") {
    return t("knowledge.retrieval.downloadWindowAutoCloseCancelled");
  }
  if (closeReason.value === "error") {
    return t("knowledge.retrieval.downloadWindowAutoCloseError");
  }
  if (pollError.value) return pollError.value;
  if (waitingForCurrentSession.value) {
    return t("knowledge.retrieval.downloadWindowWaiting", requestedModelId.value || "—");
  }
  return statusSnapshot.value?.detail?.trim()
    || t("knowledge.retrieval.downloadWindowWaiting", requestedModelId.value || "—");
});
const stageDetail = computed(() => {
  if (pollError.value) return pollError.value;
  if (closeReason.value === "cancelled") {
    return t("knowledge.retrieval.downloadWindowAutoCloseCancelled");
  }
  if (waitingForCurrentSession.value) {
    return t("knowledge.retrieval.downloadWindowPreparing");
  }
  if (statusSnapshot.value?.stage === "cancelling") {
    return t("knowledge.retrieval.downloadWindowCancellingDetail");
  }
  if (statusSnapshot.value?.stage === "downloading_model") {
    return t("knowledge.retrieval.downloadWindowDownloadingDetail");
  }
  if (statusSnapshot.value?.stage === "initializing_runtime") {
    return t("knowledge.retrieval.downloadWindowInitializingDetail");
  }
  if (statusSnapshot.value?.error) return statusSnapshot.value.error;
  return statusSnapshot.value?.detail?.trim() || t("knowledge.retrieval.downloadWindowPreparing");
});
const canCancelDownload = computed(() =>
  !closeReason.value && !cancelPending.value && !waitingForCurrentSession.value,
);
const cancelLabel = computed(() =>
  cancelPending.value
    ? t("knowledge.retrieval.downloadWindowCancelling")
    : t("common.cancel"),
);

async function initializeWindow() {
  try {
    await appWindow.setTitle(KNOWLEDGE_DOWNLOAD_WINDOW_TITLE);
  } catch {
    // ignore unsupported title updates
  }
  try {
    await appWindow.setClosable(false);
  } catch {
    // ignore unsupported close state changes
  }

  try {
    closeRequestUnlisten = await appWindow.onCloseRequested((event) => {
      if (allowWindowClose) return;
      event.preventDefault();
    });
    modelEventUnlisten = await appWindow.listen<{ modelId?: string }>(
      KNOWLEDGE_DOWNLOAD_WINDOW_MODEL_EVENT,
      (event) => {
        const nextModelId = event.payload?.modelId?.trim() || "";
        if (!nextModelId) return;
        resetDownloadSession(nextModelId);
      },
    );
  } catch {
    // keep polling even if window event hooks are unavailable
  }

  schedulePoll(220);
}

onMounted(() => {
  void initializeWindow();
});

onUnmounted(() => {
  clearPollTimer();
  clearCloseTimer();
  modelEventUnlisten?.();
  closeRequestUnlisten?.();
});
</script>

<template>
  <div class="download-window-root">
    <div class="download-window-titlebar">
      <div class="download-window-titlebar-label">{{ KNOWLEDGE_DOWNLOAD_WINDOW_TITLE }}</div>
      <div class="download-window-titlebar-progress">{{ progressLabel }}</div>
    </div>

    <div class="download-window-body-shell">
      <div class="download-window-shell">
        <div class="download-window-header">
          <div class="download-window-title">{{ statusHeading }}</div>
          <div class="download-window-subtitle">{{ windowSubtitle }}</div>
        </div>

        <div class="download-window-body">
          <div class="download-window-content">
            <div class="download-window-hero">
              <div class="download-window-progress">{{ progressLabel }}</div>
              <div class="download-window-progress-caption">
                {{ t("knowledge.retrieval.downloadWindowProgressCaption") }}
              </div>
            </div>

            <div class="download-window-track" aria-hidden="true">
              <div class="download-window-track-fill" :style="{ width: `${Math.round(progressRatio * 100)}%` }"></div>
            </div>

            <div class="download-window-meta">
              <div class="download-window-row">
                <span>{{ t("knowledge.retrieval.modelLabel") }}</span>
                <span class="truncate">{{ requestedModelId || "—" }}</span>
              </div>
              <div class="download-window-row">
                <span>{{ t("knowledge.dashboard.knowledge.rebuildStage") }}</span>
                <span>{{ currentStageLabel }}</span>
              </div>
              <div class="download-window-row">
                <span>{{ t("knowledge.retrieval.downloadWindowTransferred") }}</span>
                <span>{{ downloadedBytesLabel }}</span>
              </div>
              <div class="download-window-row download-window-row-multiline">
                <span>{{ t("knowledge.retrieval.downloadSource") }}</span>
                <div class="download-window-row-value">
                  <span>{{ downloadSourceSummary.primary }}</span>
                  <span
                    v-if="downloadSourceSummary.secondary"
                    class="download-window-row-note"
                  >
                    {{ downloadSourceSummary.secondary }}
                  </span>
                </div>
              </div>
              <div class="download-window-row download-window-row-multiline">
                <span>{{ t("knowledge.retrieval.downloadWindowProxy") }}</span>
                <div class="download-window-row-value">
                  <span>{{ proxySummary.primary }}</span>
                  <span
                    v-if="proxySummary.secondary"
                    class="download-window-row-note"
                  >
                    {{ proxySummary.secondary }}
                  </span>
                </div>
              </div>
            </div>

            <div class="download-window-detail">
              {{ stageDetail }}
            </div>
          </div>

          <div class="download-window-footer">
            <BaseButton
              :disabled="!canCancelDownload"
              @click="void cancelDownload()"
            >
              {{ cancelLabel }}
            </BaseButton>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.download-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  overflow: hidden;
}

.download-window-titlebar {
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

.download-window-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.download-window-titlebar-progress {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.download-window-body-shell {
  flex: 1;
  min-height: 0;
  padding: 14px;
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--bg-color) 8%);
}

.download-window-shell {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
  overflow: hidden;
}

.download-window-header {
  padding: 16px 18px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.download-window-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-color);
}

.download-window-subtitle {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.download-window-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  padding: 18px;
  min-height: 0;
  gap: 16px;
}

.download-window-content {
  display: flex;
  flex-direction: column;
  gap: 16px;
  flex: 1;
  min-height: 0;
}

.download-window-hero {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.download-window-progress {
  font-size: 32px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.download-window-progress-caption {
  font-size: 12px;
  color: var(--text-secondary);
}

.download-window-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.download-window-track-fill {
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

.download-window-meta {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.download-window-row {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  align-items: flex-start;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.download-window-row > span:first-child {
  padding-top: 1px;
}

.download-window-row > span:last-child,
.download-window-row > .download-window-row-value {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
  min-width: 0;
}

.download-window-row-value {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 3px;
  min-width: 0;
  width: 100%;
  line-height: 1.45;
}

.download-window-row-multiline .download-window-row-value {
  align-items: flex-start;
  text-align: left;
}

.download-window-row-multiline .download-window-row-value > span:first-child {
  width: 100%;
  font-weight: 600;
}

.download-window-row-note {
  display: block;
  max-width: 100%;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
  font-weight: 500;
  font-family: var(--font-mono-identifier);
  overflow-wrap: anywhere;
  word-break: normal;
  white-space: normal;
}

.download-window-detail {
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-secondary);
  min-height: 40px;
}

.download-window-footer {
  flex-shrink: 0;
  display: flex;
  justify-content: flex-end;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.truncate {
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
