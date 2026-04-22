<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import {
  knowledgeCancelUnityReferenceImport,
  knowledgeCloseUnityReferenceImportProgressWindow,
  knowledgeGetUnityReferenceImportStatus,
  knowledgeImportUnityReferenceDocs,
} from "../services/knowledge";
import {
  getUnityReferenceImportWindowPayload,
  UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  type UnityReferenceImportWindowPayload,
} from "../services/unityReferenceImportWindow";
import type { UnityReferenceImportLocale, UnityReferenceImportStatus } from "../types";
import BaseButton from "./ui/BaseButton.vue";
import BaseDropdown from "./ui/BaseDropdown.vue";

type CloseReason = "success" | "error" | "cancelled" | null;
type ImportStageKey =
  | "resolving_source"
  | "downloading"
  | "extracting"
  | "converting"
  | "reconciling";

const IMPORT_STAGE_ORDER: ImportStageKey[] = [
  "resolving_source",
  "downloading",
  "extracting",
  "converting",
  "reconciling",
];
const UNITY_REFERENCE_MANAGED_DIR = "unity-official-docs";

const appWindow = getCurrentWindow();
const initialPayload = getUnityReferenceImportWindowPayload();
const targetPath = ref(initialPayload.targetPath?.trim() || "");
const requestedProjectVersion = ref(initialPayload.projectVersion?.trim() || "");
const requestedDocsVersion = ref(initialPayload.docsVersion?.trim() || "");
const selectedLocale = ref<UnityReferenceImportLocale>(initialPayload.locale ?? "en");
const statusSnapshot = ref<UnityReferenceImportStatus | null>(null);
const closeReason = ref<CloseReason>(null);
const pollError = ref("");
const hasSeenRunning = ref(!!initialPayload.running);
const lastActiveStageKey = ref<ImportStageKey>("resolving_source");
const downloadSpeedBytesPerSecond = ref<number | null>(null);
const lastDownloadSampleBytes = ref<number | null>(null);
const lastDownloadSampleAt = ref<number | null>(null);
const cancelling = ref(false);
const startPending = ref(false);

let pollTimer: ReturnType<typeof setTimeout> | null = null;
let closeTimer: ReturnType<typeof setTimeout> | null = null;
let statusEventUnlisten: UnlistenFn | null = null;
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

function schedulePoll(delay = 260) {
  clearPollTimer();
  pollTimer = setTimeout(() => {
    pollTimer = null;
    void refreshStatus();
  }, delay);
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

function formatBytesPerSecond(bytesPerSecond: number | null | undefined): string {
  if (!bytesPerSecond || !Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return "—";
  }
  return `${formatBytes(bytesPerSecond)}/s`;
}

function clampProgress(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function requestTargetPath(): string | undefined {
  const normalized = targetPath.value.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  if (!normalized || normalized === UNITY_REFERENCE_MANAGED_DIR) {
    return undefined;
  }
  return normalized;
}

function isImportStage(stage: string | null | undefined): stage is ImportStageKey {
  return IMPORT_STAGE_ORDER.includes(stage as ImportStageKey);
}

function stageProgressRatioForStatus(status: UnityReferenceImportStatus | null): number | null {
  if (!status) return null;
  if (status.stage === "ready" || status.state === "ready") return 1;
  switch (status.stage) {
    case "resolving_source":
      return status.running ? 0.65 : null;
    case "downloading":
      if (typeof status.progress === "number") return clampProgress(status.progress);
      if (status.totalBytes && typeof status.downloadedBytes === "number") {
        return clampProgress(status.downloadedBytes / status.totalBytes);
      }
      return null;
    case "extracting":
      if (typeof status.progress === "number") return clampProgress(status.progress);
      return status.running ? 0.5 : null;
    case "converting":
      if (typeof status.progress === "number") return clampProgress(status.progress);
      if (status.totalDocs && status.processedDocs > 0) {
        return clampProgress(status.processedDocs / status.totalDocs);
      }
      return null;
    case "reconciling":
      if (typeof status.progress === "number") return clampProgress(status.progress);
      return status.running ? 0.55 : null;
    default:
      return null;
  }
}

function stageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "idle":
      return t("knowledge.referenceImport.stage.idle");
    case "resolving_source":
      return t("knowledge.referenceImport.stage.resolvingSource");
    case "downloading":
      return t("knowledge.referenceImport.stage.downloading");
    case "extracting":
      return t("knowledge.referenceImport.stage.extracting");
    case "converting":
      return t("knowledge.referenceImport.stage.converting");
    case "reconciling":
      return t("knowledge.referenceImport.stage.reconciling");
    case "ready":
      return t("knowledge.referenceImport.stage.ready");
    case "error":
      return t("knowledge.referenceImport.stage.error");
    default:
      return t("knowledge.referenceImport.window.waitingStage");
  }
}

function localeLabel(locale: UnityReferenceImportLocale | null | undefined): string {
  switch (locale) {
    case "zh-CN":
      return t("knowledge.referenceImport.locale.zhCn");
    case "en":
    default:
      return t("knowledge.referenceImport.locale.en");
  }
}

function resetImportSession(payload: UnityReferenceImportWindowPayload = {}) {
  targetPath.value = payload.targetPath?.trim() || "";
  requestedProjectVersion.value = payload.projectVersion?.trim() || "";
  requestedDocsVersion.value = payload.docsVersion?.trim() || "";
  selectedLocale.value = payload.locale ?? selectedLocale.value ?? "en";
  statusSnapshot.value = null;
  closeReason.value = null;
  pollError.value = "";
  hasSeenRunning.value = !!payload.running;
  lastActiveStageKey.value = "resolving_source";
  downloadSpeedBytesPerSecond.value = null;
  lastDownloadSampleBytes.value = null;
  lastDownloadSampleAt.value = null;
  cancelling.value = false;
  startPending.value = false;
  clearCloseTimer();
  schedulePoll(140);
}

function updateDownloadSpeed(status: UnityReferenceImportStatus) {
  if (status.stage !== "downloading" || typeof status.downloadedBytes !== "number") {
    downloadSpeedBytesPerSecond.value = null;
    lastDownloadSampleBytes.value = null;
    lastDownloadSampleAt.value = null;
    return;
  }

  const now = Date.now();
  const previousBytes = lastDownloadSampleBytes.value;
  const previousAt = lastDownloadSampleAt.value;
  lastDownloadSampleBytes.value = status.downloadedBytes;
  lastDownloadSampleAt.value = now;

  if (previousBytes == null || previousAt == null) return;
  if (status.downloadedBytes < previousBytes) {
    downloadSpeedBytesPerSecond.value = null;
    return;
  }

  const elapsedMs = now - previousAt;
  if (elapsedMs <= 0) return;

  const deltaBytes = status.downloadedBytes - previousBytes;
  if (deltaBytes > 0) {
    const instantaneous = (deltaBytes * 1000) / elapsedMs;
    downloadSpeedBytesPerSecond.value = downloadSpeedBytesPerSecond.value == null
      ? instantaneous
      : downloadSpeedBytesPerSecond.value * 0.68 + instantaneous * 0.32;
    return;
  }

  if (elapsedMs >= 900) {
    downloadSpeedBytesPerSecond.value = null;
  }
}

async function destroyWindow() {
  clearPollTimer();
  clearCloseTimer();
  allowWindowClose = true;
  closeRequestUnlisten?.();
  closeRequestUnlisten = null;
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
    return;
  } catch {
    // fall back to backend close when local handles fail
  }

  try {
    await knowledgeCloseUnityReferenceImportProgressWindow();
  } catch {
    // ignore teardown failures after local close attempts
  }
}

async function cancelImport() {
  if (cancelling.value || closeReason.value) return;
  cancelling.value = true;
  pollError.value = "";
  try {
    const status = await knowledgeCancelUnityReferenceImport(requestTargetPath());
    statusSnapshot.value = status;
    hasSeenRunning.value = true;
    schedulePoll(120);
  } catch (cause) {
    pollError.value = normalizeAppError(cause).message;
    cancelling.value = false;
  }
}

async function startImport() {
  if (startPending.value || cancelling.value || closeReason.value) return;
  startPending.value = true;
  pollError.value = "";
  clearCloseTimer();
  try {
    const status = await knowledgeImportUnityReferenceDocs(
      requestTargetPath(),
      selectedLocale.value,
    );
    statusSnapshot.value = status;
    hasSeenRunning.value = hasSeenRunning.value || status.running;
    if (status.selectedLocale) {
      selectedLocale.value = status.selectedLocale;
    }
    updateDownloadSpeed(status);
    if (isImportStage(status.stage)) {
      lastActiveStageKey.value = status.stage;
    }
    schedulePoll(status.running ? 120 : 260);
  } catch (cause) {
    pollError.value = normalizeAppError(cause).message;
  } finally {
    startPending.value = false;
  }
}

function scheduleAutoClose(reason: Exclude<CloseReason, null>) {
  if (closeReason.value === reason || closeTimer) return;
  closeReason.value = reason;
  clearPollTimer();
  closeTimer = setTimeout(() => {
    closeTimer = null;
    void destroyWindow();
  }, reason === "error" ? 2600 : 1200);
}

async function refreshStatus() {
  try {
    const nextStatus = await knowledgeGetUnityReferenceImportStatus(requestTargetPath());
    statusSnapshot.value = nextStatus;
    pollError.value = "";
    updateDownloadSpeed(nextStatus);
    if (nextStatus.selectedLocale) {
      selectedLocale.value = nextStatus.selectedLocale;
    }

    if (nextStatus.running || nextStatus.state === "running") {
      hasSeenRunning.value = true;
    }
    if (isImportStage(nextStatus.stage)) {
      lastActiveStageKey.value = nextStatus.stage;
    }

    if (hasSeenRunning.value && !nextStatus.running && nextStatus.lastOutcome === "cancelled") {
      cancelling.value = false;
      scheduleAutoClose("cancelled");
      return;
    }

    if (hasSeenRunning.value && !nextStatus.running && nextStatus.state === "ready") {
      scheduleAutoClose("success");
      return;
    }

    if (hasSeenRunning.value && !nextStatus.running && nextStatus.state === "error") {
      scheduleAutoClose("error");
      return;
    }

    schedulePoll();
  } catch (cause) {
    pollError.value = normalizeAppError(cause).message;
    schedulePoll(600);
  }
}

const stageProgressRatio = computed(() => stageProgressRatioForStatus(statusSnapshot.value));
const stageProgressLabel = computed(() => {
  if (stageProgressRatio.value == null) {
    return statusSnapshot.value?.running
      ? t("knowledge.referenceImport.window.stageProgressRunning")
      : "—";
  }
  return formatPercent(stageProgressRatio.value);
});
const currentStageLabel = computed(() => stageLabel(statusSnapshot.value?.stage));
const stageTrackRatio = computed(() => stageProgressRatio.value ?? 0);
const selectedLocaleLabel = computed(() => localeLabel(selectedLocale.value));
const localeOptions = computed(() => ([
  {
    value: "en",
    label: t("knowledge.referenceImport.locale.en"),
  },
  {
    value: "zh-CN",
    label: t("knowledge.referenceImport.locale.zhCn"),
  },
]));
const titlebarStatus = computed(() => {
  if (closeReason.value === "success") return t("common.done");
  if (closeReason.value === "error") return t("knowledge.genBar.failed");
  if (closeReason.value === "cancelled") return t("knowledge.referenceImport.window.cancelledTitle");
  return currentStageLabel.value;
});
const stageTimeline = computed(() => {
  const currentKey = isImportStage(statusSnapshot.value?.stage)
    ? statusSnapshot.value?.stage
    : lastActiveStageKey.value;
  const currentIndex = IMPORT_STAGE_ORDER.findIndex((item) => item === currentKey);
  const isReady = closeReason.value === "success" || statusSnapshot.value?.state === "ready";
  const isError = closeReason.value === "error" || statusSnapshot.value?.state === "error";
  return IMPORT_STAGE_ORDER.map((item, index) => {
    const complete = isReady || index < currentIndex;
    const current = !isReady && index === currentIndex;
    const error = isError && index === currentIndex;
    const progress = complete ? 1 : current ? (stageProgressRatio.value ?? 0) : 0;
    return {
      key: item,
      label: stageLabel(item),
      complete,
      current,
      error,
      progress,
      statusText: error
        ? t("knowledge.genBar.failed")
        : complete
          ? t("common.done")
          : current
            ? stageProgressLabel.value
            : t("knowledge.referenceImport.window.waitingStage"),
    };
  });
});
const transferredLabel = computed(() => {
  const status = statusSnapshot.value;
  if (status?.downloadedBytes == null && status?.totalBytes == null) {
    return "—";
  }
  return `${formatBytes(status?.downloadedBytes)} / ${formatBytes(status?.totalBytes)}`;
});
const downloadSpeedLabel = computed(() => {
  if (statusSnapshot.value?.stage !== "downloading") return "—";
  return formatBytesPerSecond(downloadSpeedBytesPerSecond.value);
});
const processedLabel = computed(() => {
  const status = statusSnapshot.value;
  if (!status) return "—";
  if (status.totalDocs == null) return `${status.processedDocs}`;
  return `${status.processedDocs} / ${status.totalDocs}`;
});
const currentProjectVersion = computed(() =>
  statusSnapshot.value?.projectVersion?.trim()
  || requestedProjectVersion.value
  || "—",
);
const currentDocsVersion = computed(() =>
  statusSnapshot.value?.docsVersion?.trim()
  || requestedDocsVersion.value
  || "—",
);
const managedPathLabel = computed(() =>
  statusSnapshot.value?.managedPath?.trim()
  || (targetPath.value ? `reference/${targetPath.value}` : "—"),
);
const canStartImport = computed(() =>
  !closeReason.value
  && !cancelling.value
  && !startPending.value
  && !statusSnapshot.value?.running
  && currentDocsVersion.value !== "—"
  && statusSnapshot.value?.state !== "unavailable",
);
const statusHeading = computed(() => {
  if (closeReason.value === "success") return t("knowledge.referenceImport.window.doneTitle");
  if (closeReason.value === "error") return t("knowledge.referenceImport.window.errorTitle");
  if (closeReason.value === "cancelled") {
    return t("knowledge.referenceImport.window.cancelledTitle");
  }
  if (!hasSeenRunning.value && !statusSnapshot.value?.running) {
    return t("knowledge.referenceImport.window.idleTitle");
  }
  return t("knowledge.referenceImport.window.title");
});
const windowSubtitle = computed(() => {
  if (closeReason.value === "success") {
    return t("knowledge.referenceImport.window.autoCloseSuccess");
  }
  if (closeReason.value === "error") {
    return t("knowledge.referenceImport.window.autoCloseError");
  }
  if (closeReason.value === "cancelled") {
    return t("knowledge.referenceImport.window.autoCloseCancelled");
  }
  if (pollError.value) return pollError.value;
  if (!hasSeenRunning.value && !statusSnapshot.value?.running) {
    return statusSnapshot.value?.message?.trim()
      || t("knowledge.referenceImport.window.setupHint");
  }
  return statusSnapshot.value?.message?.trim()
    || t("knowledge.referenceImport.window.waiting", currentDocsVersion.value);
});
const stageDetail = computed(() => {
  if (pollError.value) return pollError.value;
  if (statusSnapshot.value?.error?.trim()) return statusSnapshot.value.error.trim();
  if (!hasSeenRunning.value && !statusSnapshot.value?.running) {
    return statusSnapshot.value?.message?.trim()
      || t("knowledge.referenceImport.window.setupDetail");
  }
  switch (statusSnapshot.value?.stage) {
    case "downloading":
      return statusSnapshot.value?.message?.trim()
        || t("knowledge.referenceImport.window.downloadingDetail");
    case "extracting":
      return statusSnapshot.value?.message?.trim()
        || t("knowledge.referenceImport.window.extractingDetail");
    case "converting":
      return statusSnapshot.value?.message?.trim()
        || t("knowledge.referenceImport.window.convertingDetail");
    case "reconciling":
      return statusSnapshot.value?.message?.trim()
        || t("knowledge.referenceImport.window.reconcilingDetail");
    default:
      return statusSnapshot.value?.message?.trim()
        || t("knowledge.referenceImport.window.preparing");
  }
});
const currentPath = computed(() => statusSnapshot.value?.currentPath?.trim() || "");
const showTransferMetrics = computed(() =>
  statusSnapshot.value?.stage === "downloading"
  || statusSnapshot.value?.downloadedBytes != null
  || statusSnapshot.value?.totalBytes != null,
);
const showProcessedMetrics = computed(() =>
  statusSnapshot.value?.stage === "converting"
  || (statusSnapshot.value?.processedDocs ?? 0) > 0
  || statusSnapshot.value?.totalDocs != null,
);
const showSetupActions = computed(() =>
  !closeReason.value && !statusSnapshot.value?.running,
);
const showRunningCancelAction = computed(() =>
  !closeReason.value && !!statusSnapshot.value?.running,
);
const canCloseWindow = computed(() =>
  !statusSnapshot.value?.running
  && !startPending.value
  && !cancelling.value,
);

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
      if (
        allowWindowClose
        || (!statusSnapshot.value?.running && !startPending.value && !cancelling.value)
      ) {
        return;
      }
      event.preventDefault();
    });
    statusEventUnlisten = await appWindow.listen<UnityReferenceImportWindowPayload>(
      UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      (event) => {
        resetImportSession(event.payload ?? {});
      },
    );
  } catch {
    // keep polling even if window event hooks are unavailable
  }

  schedulePoll(140);
}

onMounted(() => {
  void initializeWindow();
});

watch(statusHeading, (nextTitle) => {
  void appWindow.setTitle(nextTitle).catch(() => {
    // ignore unsupported title updates
  });
}, { immediate: true });

onUnmounted(() => {
  clearPollTimer();
  clearCloseTimer();
  statusEventUnlisten?.();
  closeRequestUnlisten?.();
});
</script>

<template>
  <div class="reference-import-window-root">
    <div class="reference-import-window-titlebar">
      <div class="reference-import-window-titlebar-label">{{ statusHeading }}</div>
      <div class="reference-import-window-titlebar-actions">
        <div class="reference-import-window-titlebar-progress">{{ titlebarStatus }}</div>
        <button
          class="reference-import-window-close"
          type="button"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          :disabled="!canCloseWindow"
          @click="void requestWindowClose()"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14" aria-hidden="true">
            <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="reference-import-window-body">
      <div class="reference-import-window-scroll">
        <div class="reference-import-window-summary">{{ windowSubtitle }}</div>

        <div class="reference-import-window-config">
          <div class="reference-import-window-config-copy">
            <div class="reference-import-window-config-label">
              {{ t("knowledge.referenceImport.window.language") }}
            </div>
            <div class="reference-import-window-config-hint">
              {{ t("knowledge.referenceImport.window.languageHint") }}
            </div>
          </div>
          <BaseDropdown
            v-model="selectedLocale"
            class="reference-import-window-locale"
            size="md"
            :disabled="!!statusSnapshot?.running || cancelling || startPending || !!closeReason"
            :options="localeOptions"
            :aria-label="t('knowledge.referenceImport.window.language')"
          />
        </div>

        <div class="reference-import-window-hero">
          <div class="reference-import-window-hero-copy">
            <div class="reference-import-window-progress">{{ currentStageLabel }}</div>
            <div class="reference-import-window-progress-caption">
              {{ t("knowledge.referenceImport.window.stageProgress") }}
            </div>
          </div>
          <div class="reference-import-window-stage-value">{{ stageProgressLabel }}</div>
        </div>

        <div class="reference-import-window-track" aria-hidden="true">
          <div
            class="reference-import-window-track-fill"
            :style="{ width: `${Math.round(stageTrackRatio * 100)}%` }"
          ></div>
        </div>

        <div class="reference-import-window-stage-list">
          <div
            v-for="item in stageTimeline"
            :key="item.key"
            class="reference-import-window-stage-item"
            :class="{
              'is-complete': item.complete,
              'is-current': item.current,
              'is-error': item.error,
            }"
          >
            <div class="reference-import-window-stage-head">
              <span class="reference-import-window-stage-dot"></span>
              <span class="reference-import-window-stage-name">{{ item.label }}</span>
            </div>
            <div class="reference-import-window-stage-status">{{ item.statusText }}</div>
            <div class="reference-import-window-stage-track" aria-hidden="true">
              <div
                class="reference-import-window-stage-track-fill"
                :style="{ width: `${Math.round(item.progress * 100)}%` }"
              ></div>
            </div>
          </div>
        </div>

        <div class="reference-import-window-meta">
          <div class="reference-import-window-row">
            <span>{{ t("knowledge.referenceImport.managedPath") }}</span>
            <span>{{ managedPathLabel }}</span>
          </div>
          <div class="reference-import-window-row">
            <span>{{ t("knowledge.referenceImport.projectVersion") }}</span>
            <span>{{ currentProjectVersion }}</span>
          </div>
          <div class="reference-import-window-row">
            <span>{{ t("knowledge.referenceImport.docsVersion") }}</span>
            <span>{{ currentDocsVersion }}</span>
          </div>
          <div class="reference-import-window-row">
            <span>{{ t("knowledge.referenceImport.locale") }}</span>
            <span>{{ selectedLocaleLabel }}</span>
          </div>
          <div class="reference-import-window-row">
            <span>{{ t("knowledge.dashboard.knowledge.rebuildStage") }}</span>
            <span>{{ currentStageLabel }}</span>
          </div>
          <div
            v-if="showTransferMetrics"
            class="reference-import-window-row"
          >
            <span>{{ t("knowledge.referenceImport.window.transferred") }}</span>
            <span>{{ transferredLabel }}</span>
          </div>
          <div
            v-if="showTransferMetrics"
            class="reference-import-window-row"
          >
            <span>{{ t("knowledge.referenceImport.window.downloadSpeed") }}</span>
            <span>{{ downloadSpeedLabel }}</span>
          </div>
          <div
            v-if="showProcessedMetrics"
            class="reference-import-window-row"
          >
            <span>{{ t("knowledge.referenceImport.window.processed") }}</span>
            <span>{{ processedLabel }}</span>
          </div>
        </div>

        <div class="reference-import-window-detail">
          {{ stageDetail }}
        </div>

        <div
          v-if="currentPath"
          class="reference-import-window-path"
        >
          <div class="reference-import-window-path-label">
            {{ t("knowledge.referenceImport.window.currentPath") }}
          </div>
          <div class="reference-import-window-path-value">{{ currentPath }}</div>
        </div>
      </div>

      <div
        v-if="showSetupActions || showRunningCancelAction"
        class="reference-import-window-actions"
      >
        <BaseButton
          v-if="showSetupActions"
          :disabled="startPending"
          @click="void destroyWindow()"
        >
          {{ t("common.cancel") }}
        </BaseButton>
        <BaseButton
          v-if="showSetupActions"
          variant="primary"
          :disabled="!canStartImport"
          @click="void startImport()"
        >
          {{ startPending ? t("knowledge.referenceImport.window.starting") : t("knowledge.referenceImport.action.import") }}
        </BaseButton>
        <BaseButton
          v-if="showRunningCancelAction"
          :disabled="cancelling"
          @click="void cancelImport()"
        >
          {{ cancelling ? t("knowledge.referenceImport.window.cancelling") : t("common.cancel") }}
        </BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.reference-import-window-root {
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

.reference-import-window-titlebar {
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

.reference-import-window-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-import-window-titlebar-progress {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.reference-import-window-titlebar-actions {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  -webkit-app-region: no-drag;
}

.reference-import-window-close {
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
  transition: background 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.reference-import-window-close:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.reference-import-window-close:disabled {
  opacity: 0.45;
  cursor: default;
}

.reference-import-window-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  padding: 16px 18px 18px;
  background: var(--panel-bg);
  overflow: hidden;
}

.reference-import-window-scroll {
  display: flex;
  flex-direction: column;
  gap: 16px;
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding-right: 4px;
  margin-right: -4px;
}

.reference-import-window-summary {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-import-window-config {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
  padding: 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--input-bg) 16%);
}

.reference-import-window-config-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.reference-import-window-config-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-import-window-config-hint {
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.reference-import-window-locale {
  width: 180px;
  flex-shrink: 0;
}

.reference-import-window-hero {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.reference-import-window-hero-copy {
  min-width: 0;
}

.reference-import-window-progress {
  font-size: 24px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
}

.reference-import-window-progress-caption {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-secondary);
}

.reference-import-window-stage-value {
  flex-shrink: 0;
  font-size: 28px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.reference-import-window-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.reference-import-window-track-fill {
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

.reference-import-window-stage-list {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 8px;
}

.reference-import-window-stage-item {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--input-bg) 16%);
  color: var(--text-secondary);
}

.reference-import-window-stage-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.reference-import-window-stage-item.is-complete,
.reference-import-window-stage-item.is-current {
  color: var(--text-color);
}

.reference-import-window-stage-item.is-current {
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--accent-soft) 30%);
}

.reference-import-window-stage-item.is-error {
  border-color: color-mix(in srgb, var(--danger-color, #d9534f) 28%, var(--border-color));
}

.reference-import-window-stage-dot {
  width: 7px;
  height: 7px;
  flex-shrink: 0;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 60%, transparent);
}

.reference-import-window-stage-item.is-complete .reference-import-window-stage-dot,
.reference-import-window-stage-item.is-current .reference-import-window-stage-dot {
  background: color-mix(in srgb, var(--accent-color) 76%, white 24%);
}

.reference-import-window-stage-item.is-error .reference-import-window-stage-dot {
  background: var(--danger-color, #d9534f);
}

.reference-import-window-stage-name {
  min-width: 0;
  font-size: 11px;
  line-height: 1.4;
}

.reference-import-window-stage-status {
  font-size: 11px;
  line-height: 1.3;
  color: var(--text-secondary);
}

.reference-import-window-stage-item.is-complete .reference-import-window-stage-status,
.reference-import-window-stage-item.is-current .reference-import-window-stage-status {
  color: var(--text-color);
}

.reference-import-window-stage-track {
  position: relative;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 82%, var(--border-color) 18%);
  overflow: hidden;
}

.reference-import-window-stage-track-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent-color) 78%, white 22%);
  transition: width 0.18s ease;
}

.reference-import-window-meta {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-import-window-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-import-window-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.reference-import-window-detail {
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-secondary);
  min-height: 40px;
}

.reference-import-window-actions {
  flex-shrink: 0;
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-import-window-path {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--input-bg) 16%);
}

.reference-import-window-path-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.reference-import-window-path-value {
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

@media (max-width: 640px) {
  .reference-import-window-config {
    flex-direction: column;
    align-items: stretch;
  }

  .reference-import-window-locale {
    width: 100%;
  }

  .reference-import-window-stage-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
