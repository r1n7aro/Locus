<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Maximize2, Minus, X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getSubWindowClaimedQuery } from "../services/subWindow";
import {
  getToolFilePreviewWindowPayload,
  resolveToolFilePreviewHighlightRanges,
  TOOL_FILE_PREVIEW_WINDOW_EVENT,
  TOOL_FILE_PREVIEW_WINDOW_LABEL,
  type ToolFilePreviewWindowPayload,
} from "../services/toolFilePreviewWindow";
import { previewWorkspaceFile, type WorkspaceFilePreview } from "../services/unity";
import AssetTextViewer from "./asset/AssetTextViewer.vue";
import LucideIcon from "./icons/LucideIcon.vue";

const appWindow = getCurrentWindow();
const filePath = ref("");
const activePayload = ref<ToolFilePreviewWindowPayload | null>(null);
const preview = ref<WorkspaceFilePreview | null>(null);
const loading = ref(false);
const error = ref("");
let unlistenPayload: UnlistenFn | null = null;
let loadSeq = 0;

const fileName = computed(() => (
  filePath.value.replace(/\\/g, "/").split("/").pop() || t("tool.filePreview.title")
));
const lineCount = computed(() => preview.value?.snippet?.split("\n").length ?? 0);
const highlightLineRanges = computed(() => {
  const current = preview.value;
  if (current?.kind !== "text" || current.snippet === undefined) return [];
  return resolveToolFilePreviewHighlightRanges(
    current.snippet,
    current.snippetStartLine,
    activePayload.value?.highlight,
  );
});
const focusLine = computed(() => highlightLineRanges.value[0]?.startLine ?? null);

function formatSize(bytes?: number): string {
  if (bytes == null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function loadFile(payload: ToolFilePreviewWindowPayload) {
  const nextPath = payload.filePath.trim();
  if (!nextPath) return;
  const seq = ++loadSeq;
  filePath.value = nextPath;
  activePayload.value = payload;
  preview.value = null;
  loading.value = true;
  error.value = "";
  void appWindow.setTitle(`Locus - ${fileName.value}`).catch(() => {});

  try {
    const nextPreview = await previewWorkspaceFile(nextPath, undefined, true);
    if (seq !== loadSeq) return;
    preview.value = nextPreview;
  } catch (cause) {
    if (seq !== loadSeq) return;
    error.value = normalizeAppError(cause).message;
  } finally {
    if (seq === loadSeq) loading.value = false;
  }
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // Fall through to forced teardown when the native close request fails.
  }
  await appWindow.destroy().catch(() => {});
}

onMounted(async () => {
  unlistenPayload = await listen<ToolFilePreviewWindowPayload>(
    TOOL_FILE_PREVIEW_WINDOW_EVENT,
    (event) => void loadFile(event.payload),
  );
  const claimedQuery = await getSubWindowClaimedQuery(
    TOOL_FILE_PREVIEW_WINDOW_LABEL,
  ).catch(() => null);
  const payload = getToolFilePreviewWindowPayload(
    claimedQuery ? `?${claimedQuery}` : window.location.search,
  );
  if (payload) {
    void loadFile(payload);
  } else {
    error.value = t("tool.filePreview.notFound");
  }
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  loadSeq += 1;
});
</script>

<template>
  <main class="tool-file-preview-window">
    <header class="tool-file-preview-titlebar" data-tauri-drag-region>
      <div class="tool-file-preview-heading" data-tauri-drag-region>
        <span class="tool-file-preview-name">{{ fileName }}</span>
        <span class="tool-file-preview-path" :title="filePath">{{ filePath }}</span>
      </div>
      <div class="tool-file-preview-window-controls" data-window-no-drag>
        <button
          type="button"
          class="tool-file-preview-window-control"
          :title="t('app.win.minimize')"
          @click="appWindow.minimize()"
        >
          <LucideIcon :icon="Minus" :size="13" />
        </button>
        <button
          type="button"
          class="tool-file-preview-window-control"
          :title="t('app.win.maximize')"
          @click="appWindow.toggleMaximize()"
        >
          <LucideIcon :icon="Maximize2" :size="12" />
        </button>
        <button
          type="button"
          class="tool-file-preview-window-control is-close"
          :title="t('app.win.close')"
          @click="closeWindow"
        >
          <LucideIcon :icon="X" :size="14" />
        </button>
      </div>
    </header>

    <section class="tool-file-preview-content">
      <div v-if="error" class="tool-file-preview-state is-error">{{ error }}</div>
      <div v-else-if="loading" class="tool-file-preview-state">{{ t("common.loading") }}</div>
      <div v-else-if="!preview?.exists" class="tool-file-preview-state">
        {{ t("tool.filePreview.notFound") }}
      </div>
      <div v-else-if="preview.previewSuppressed === 'largeFile'" class="tool-file-preview-state">
        {{ t("tool.filePreview.tooLarge", formatSize(preview.fileSize)) }}
      </div>
      <div v-else-if="preview.kind === 'binary'" class="tool-file-preview-state">
        {{ t("tool.filePreview.binary", formatSize(preview.fileSize)) }}
      </div>
      <div v-else-if="preview.kind === 'text' && !preview.snippet" class="tool-file-preview-state">
        {{ t("tool.filePreview.empty") }}
      </div>
      <AssetTextViewer
        v-else-if="preview.kind === 'text' && preview.snippet !== undefined"
        :snippet="preview.snippet"
        :truncated="preview.truncated"
        :total-lines="lineCount"
        :start-line="preview.snippetStartLine"
        :focus-line="focusLine"
        :highlight-line-ranges="highlightLineRanges"
        :language="preview.language"
      />
    </section>
  </main>
</template>

<style scoped>
.tool-file-preview-window {
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

.tool-file-preview-titlebar {
  -webkit-app-region: drag;
  flex: 0 0 38px;
  min-width: 0;
  display: flex;
  align-items: center;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.tool-file-preview-heading {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding-left: 14px;
}

.tool-file-preview-name {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
}

.tool-file-preview-path {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-file-preview-window-controls {
  -webkit-app-region: no-drag;
  flex: 0 0 126px;
  height: 100%;
  display: flex;
  margin-left: auto;
}

.tool-file-preview-window-control {
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

.tool-file-preview-window-control:hover,
.tool-file-preview-window-control:focus-visible {
  outline: none;
  background: var(--hover-bg);
  color: var(--text-color);
}

.tool-file-preview-window-control.is-close:hover,
.tool-file-preview-window-control.is-close:focus-visible {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.tool-file-preview-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.tool-file-preview-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  color: var(--text-secondary);
  font-size: 13px;
}

.tool-file-preview-state.is-error {
  color: var(--status-danger-fg);
}
</style>
