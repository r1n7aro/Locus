<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ExternalLink, X } from "lucide";
import type { FileDiffPayload, FileDiffRequest } from "../types";
import { diffSingleFile, invalidateDiffCache, parseDiffRequestKey } from "../services/diff";
import { openFileExternal } from "../services/unity";
import { normalizeAppError } from "../services/errors";
import {
  CHAT_DIFF_REVIEW_WINDOW_EVENT,
  getChatDiffReviewWindowPayload,
  type ChatDiffReviewWindowPayload,
} from "../services/chatDiffReviewWindow";
import { canOpenInEditor } from "../composables/useHideMeta";
import { useDiffProgress } from "../composables/useDiffProgress";
import { t } from "../i18n";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import LucideIcon from "./icons/LucideIcon.vue";

const appWindow = getCurrentWindow();
const diffProgress = useDiffProgress();
const payload = ref<FileDiffPayload | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
const fileDiffViewerRef = ref<InstanceType<typeof FileDiffViewer> | null>(null);
const requestSeq = ref(0);
const currentRequest = ref<FileDiffRequest | null>(null);
const fullContext = ref(false);

let unlistenPayload: UnlistenFn | null = null;

const diffTabOptions = computed(() => [
  { value: "semantic", label: t("diff.tabs.semantic") },
  { value: "text", label: t("diff.tabs.text") },
]);

const titlePath = computed(() => {
  if (!payload.value) return t("chat.changes.reviewWindowTitle");
  return payload.value.oldPath
    ? `${payload.value.oldPath} -> ${payload.value.filePath}`
    : payload.value.filePath;
});

const statsLabel = computed(() => {
  const stats = payload.value?.stats;
  if (!stats) return null;
  return {
    additions: `+${stats.additions}`,
    deletions: `-${stats.deletions}`,
  };
});

const canToggleFullTextCompare = computed(() =>
  !!payload.value
  && !!currentRequest.value
  && !payload.value.isBinary
  && payload.value.contentState.type !== "lfsNotFetched",
);

function normalizeReviewRequest(
  request: FileDiffRequest,
): FileDiffRequest {
  return {
    ...request,
    detail: "full",
    fullContext: Boolean(request.fullContext),
  };
}

function applyCurrentRequest(request: FileDiffRequest | null) {
  currentRequest.value = request ? normalizeReviewRequest(request) : null;
  fullContext.value = Boolean(currentRequest.value?.fullContext);
}

async function selectTextTabIfAvailable() {
  await nextTick();
  const viewer = fileDiffViewerRef.value;
  if (!viewer) return;
  if (viewer.hasSemanticAndText) {
    viewer.activeTab = "text";
  }
}

async function loadRequest(
  request: FileDiffRequest,
  options?: { invalidateKey?: string; preferTextTab?: boolean },
): Promise<boolean> {
  const normalizedRequest = normalizeReviewRequest(request);
  const seq = ++requestSeq.value;
  applyCurrentRequest(normalizedRequest);
  loading.value = true;
  error.value = null;
  payload.value = null;
  diffProgress.reset();
  if (options?.invalidateKey) {
    invalidateDiffCache(options.invalidateKey);
  }
  try {
    const nextPayload = await diffSingleFile(normalizedRequest);
    if (seq !== requestSeq.value) return false;
    payload.value = nextPayload;
    if (options?.preferTextTab) {
      await selectTextTabIfAvailable();
    }
    return true;
  } catch (cause) {
    if (seq !== requestSeq.value) return false;
    error.value = normalizeAppError(cause).message;
    return false;
  } finally {
    if (seq === requestSeq.value) {
      loading.value = false;
    }
  }
}

async function loadDiffKey(diffKey: string): Promise<boolean> {
  const request = parseDiffRequestKey(diffKey);
  if (!request) {
    requestSeq.value += 1;
    applyCurrentRequest(null);
    payload.value = null;
    loading.value = false;
    error.value = t("chat.changes.reviewMissing");
    return false;
  }
  return loadRequest(request, {
    invalidateKey: diffKey,
  });
}

function applyWindowPayload(next: ChatDiffReviewWindowPayload) {
  if (next.payload) {
    const request = parseDiffRequestKey(next.payload.key);
    requestSeq.value += 1;
    applyCurrentRequest(request);
    payload.value = next.payload;
    error.value = null;
    loading.value = false;
    return;
  }
  if (next.request) {
    void loadRequest(next.request);
    return;
  }
  if (next.diffKey?.trim()) {
    void loadDiffKey(next.diffKey.trim());
    return;
  }
  error.value = t("chat.changes.reviewMissing");
}

async function onLfsPulled() {
  if (!payload.value) return;
  await loadDiffKey(payload.value.key);
}

async function toggleFullTextCompare() {
  if (!currentRequest.value || loading.value) return;
  const nextFullContext = !fullContext.value;
  await loadRequest({
    ...currentRequest.value,
    fullContext: nextFullContext,
  }, { preferTextTab: true });
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through
  }
  await appWindow.destroy().catch(() => {});
}

onMounted(async () => {
  applyWindowPayload(getChatDiffReviewWindowPayload());
  unlistenPayload = await listen<ChatDiffReviewWindowPayload>(
    CHAT_DIFF_REVIEW_WINDOW_EVENT,
    (event) => applyWindowPayload(event.payload),
  );
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  requestSeq.value += 1;
});
</script>

<template>
  <div class="chat-diff-review-window-root">
    <div class="chat-diff-review-titlebar">
      <div class="chat-diff-review-title">
        <span class="chat-diff-review-title-main">{{ t("chat.changes.reviewWindowTitle") }}</span>
        <span class="chat-diff-review-title-path" :title="titlePath">{{ titlePath }}</span>
      </div>
      <button
        type="button"
        class="chat-diff-review-close"
        :title="t('app.win.close')"
        @click="closeWindow"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div v-if="payload" class="chat-diff-review-header">
      <div class="chat-diff-review-meta">
        <span class="chat-diff-review-status" :class="'status-' + (payload.status ?? '').toLowerCase()">
          {{ payload.status }}
        </span>
        <span class="chat-diff-review-file" :title="titlePath">{{ titlePath }}</span>
        <span v-if="statsLabel" class="chat-diff-review-stats">
          <span class="stat-add">{{ statsLabel.additions }}</span>
          <span class="stat-del">{{ statsLabel.deletions }}</span>
        </span>
      </div>
      <div class="chat-diff-review-actions">
        <BaseSegmented
          v-if="fileDiffViewerRef?.hasSemanticAndText"
          class="chat-diff-review-tabs"
          size="sm"
          :model-value="fileDiffViewerRef.activeTab"
          :options="diffTabOptions"
          @update:model-value="fileDiffViewerRef.activeTab = $event as 'semantic' | 'text'"
        />
        <button
          v-if="!payload.isBinary && canOpenInEditor(payload.filePath)"
          type="button"
          class="chat-diff-review-action"
          :title="t('common.openInEditor')"
          @click="openFileExternal(payload.filePath)"
        >
          <LucideIcon :icon="ExternalLink" :size="13" />
          <span>{{ t("common.openInEditor") }}</span>
        </button>
        <button
          v-if="canToggleFullTextCompare"
          type="button"
          class="chat-diff-review-action"
          :class="{ active: fullContext }"
          :disabled="loading"
          :title="t('diff.mode.fullTextCompare')"
          @click="toggleFullTextCompare"
        >
          <span>{{ t("diff.mode.fullTextCompare") }}</span>
        </button>
        <button
          v-if="fileDiffViewerRef?.hasTextDisplayModeControl"
          type="button"
          class="chat-diff-review-action"
          :class="{ active: fileDiffViewerRef.textDisplayMode === 'side-by-side' }"
          :title="t('diff.mode.sideBySide')"
          @click="fileDiffViewerRef.toggleTextDisplayMode()"
        >
          <span>{{ t("diff.mode.sideBySide") }}</span>
        </button>
      </div>
    </div>

    <div class="chat-diff-review-body">
      <FileDiffViewer
        v-if="payload"
        ref="fileDiffViewerRef"
        :payload="payload"
        :hide-builtin-tabs="true"
        :hide-text-display-controls="true"
        @lfs-pulled="onLfsPulled"
      />
      <div v-else-if="loading" class="chat-diff-review-loading">
        <span>{{ diffProgress.phaseLabel.value }}</span>
        <div class="chat-diff-review-progress">
          <div class="chat-diff-review-progress-fill" :style="{ width: `${diffProgress.progress.value * 100}%` }"></div>
        </div>
      </div>
      <div v-else class="chat-diff-review-error">
        {{ error || t("chat.changes.reviewMissing") }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-diff-review-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.chat-diff-review-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.chat-diff-review-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.chat-diff-review-title-main {
  flex-shrink: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.chat-diff-review-title-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.chat-diff-review-close,
.chat-diff-review-action {
  -webkit-app-region: no-drag;
}

.chat-diff-review-close {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.chat-diff-review-close:hover,
.chat-diff-review-close:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.chat-diff-review-header {
  min-height: 42px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.chat-diff-review-meta,
.chat-diff-review-actions,
.chat-diff-review-stats,
.chat-diff-review-action {
  display: flex;
  align-items: center;
}

.chat-diff-review-meta {
  min-width: 0;
  gap: 8px;
}

.chat-diff-review-actions {
  flex-shrink: 0;
  gap: 8px;
}

.chat-diff-review-status {
  flex-shrink: 0;
  min-width: 20px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 700;
  line-height: 18px;
  text-align: center;
}

.chat-diff-review-status.status-m {
  color: var(--git-status-modified);
}

.chat-diff-review-status.status-a,
.chat-diff-review-status.status-\? {
  color: var(--git-status-added);
}

.chat-diff-review-status.status-d {
  color: var(--git-status-deleted);
}

.chat-diff-review-status.status-r {
  color: var(--git-status-renamed);
}

.chat-diff-review-file {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.chat-diff-review-stats {
  flex-shrink: 0;
  gap: 6px;
  font-size: 12px;
  font-family: var(--font-mono-identifier);
}

.stat-add {
  color: var(--git-status-added);
}

.stat-del {
  color: var(--git-status-deleted);
}

.chat-diff-review-action {
  min-height: 26px;
  gap: 6px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.chat-diff-review-action:hover,
.chat-diff-review-action:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
  outline: none;
}

.chat-diff-review-action:disabled {
  opacity: 0.6;
  cursor: default;
}

.chat-diff-review-action.active {
  background: var(--accent-color);
  border-color: var(--accent-color);
  color: var(--text-on-accent, #fff);
}

.chat-diff-review-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.chat-diff-review-loading,
.chat-diff-review-error {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.chat-diff-review-loading {
  flex-direction: column;
  gap: 10px;
}

.chat-diff-review-error {
  color: var(--status-danger-fg);
}

.chat-diff-review-progress {
  width: min(360px, 60vw);
  height: 3px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--border-color);
}

.chat-diff-review-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent-color);
  transition: width 0.15s ease;
}
</style>
