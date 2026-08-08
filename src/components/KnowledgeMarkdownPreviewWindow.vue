<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { knowledgeRead } from "../services/knowledge";
import {
  getKnowledgeMarkdownPreviewWindowPayload,
  KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL,
  KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT,
  type KnowledgeMarkdownPreviewWindowPayload,
} from "../services/knowledgeMarkdownPreviewWindow";
import { getSubWindowClaimedQuery } from "../services/subWindow";
import LucideIcon from "./icons/LucideIcon.vue";
import MarkdownRenderer from "./MarkdownRenderer.vue";

const appWindow = getCurrentWindow();
const path = ref("");
const title = ref("");
const content = ref("");
const loading = ref(false);
const error = ref("");

let unlistenPayload: UnlistenFn | null = null;
let loadSeq = 0;

function fallbackTitle(documentPath: string): string {
  const fileName = documentPath.replace(/\\/g, "/").split("/").pop() || "Memory";
  return fileName.replace(/\.md$/i, "") || "Memory";
}

async function loadDocument(payload: KnowledgeMarkdownPreviewWindowPayload) {
  const seq = ++loadSeq;
  path.value = payload.path;
  title.value = fallbackTitle(payload.path);
  content.value = "";
  loading.value = true;
  error.value = "";
  void appWindow.setTitle(`Locus - ${title.value}`).catch(() => {});

  try {
    const result = await knowledgeRead({
      kind: "document",
      type: payload.docType,
      path: payload.path,
      part: "full",
    });
    if (seq !== loadSeq) return;
    const document = result.document;
    if (!document) throw new Error(t("knowledge.markdownPreview.notFound"));
    title.value = document.title.trim() || fallbackTitle(document.path);
    path.value = document.path;
    content.value = document.body;
    void appWindow.setTitle(`Locus - ${title.value}`).catch(() => {});
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
    // Fall through to a forced teardown when the native close request fails.
  }
  await appWindow.destroy().catch(() => {});
}

onMounted(async () => {
  unlistenPayload = await listen<KnowledgeMarkdownPreviewWindowPayload>(
    KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT,
    (event) => void loadDocument(event.payload),
  );
  const claimedQuery = await getSubWindowClaimedQuery(
    KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL,
  ).catch(() => null);
  const payload = getKnowledgeMarkdownPreviewWindowPayload(
    claimedQuery ? `?${claimedQuery}` : window.location.search,
  );
  if (payload) {
    void loadDocument(payload);
  } else {
    error.value = t("knowledge.markdownPreview.notFound");
  }
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  loadSeq += 1;
});
</script>

<template>
  <main class="markdown-preview-window-root">
    <header class="markdown-preview-titlebar" data-tauri-drag-region>
      <div class="markdown-preview-title" data-tauri-drag-region>
        <span class="markdown-preview-title-main">{{ title || t("knowledge.markdownPreview.title") }}</span>
        <span class="markdown-preview-title-path" :title="path">{{ path }}</span>
      </div>
      <button
        type="button"
        class="markdown-preview-close"
        :title="t('app.win.close')"
        @click="closeWindow"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </header>

    <section class="markdown-preview-body">
      <div v-if="error" class="markdown-preview-state is-error">{{ error }}</div>
      <div v-else-if="loading" class="markdown-preview-state">{{ t("common.loading") }}</div>
      <div v-else-if="!content.trim()" class="markdown-preview-state">
        {{ t("knowledge.markdownPreview.empty") }}
      </div>
      <MarkdownRenderer v-else :content="content" />
    </section>
  </main>
</template>

<style scoped>
.markdown-preview-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  background: var(--panel-bg);
  color: var(--text-color);
}

.markdown-preview-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex: 0 0 38px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.markdown-preview-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.markdown-preview-title-main {
  flex-shrink: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.markdown-preview-title-path {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.markdown-preview-close {
  -webkit-app-region: no-drag;
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

.markdown-preview-close:hover,
.markdown-preview-close:focus-visible {
  outline: none;
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.markdown-preview-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 18px 22px;
}

.markdown-preview-body :deep(.markdown-body) {
  max-width: 860px;
  margin: 0 auto;
  font-size: 13px;
  line-height: 1.7;
}

.markdown-preview-state {
  min-height: 120px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.markdown-preview-state.is-error {
  color: var(--status-danger-fg);
}
</style>
