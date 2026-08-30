<script setup lang="ts">
import type { Text } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { computed, nextTick, ref, watch } from "vue";
import { Save } from "lucide";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { previewWorkspaceAsset } from "../../services/asset";
import type { WorkspaceRef } from "../../services/project";
import {
  projectExplorerPreviewFile,
  projectExplorerWriteFile,
  workspaceFilePreview,
  workspaceFileWrite,
} from "../../services/workspaceExplorer";
import type { AssetPreviewPayload } from "../../types";
import type { ProjectExplorerFilePreview } from "../../types/workbench";
import WorkspaceAssetPreview from "../asset/WorkspaceAssetPreview.vue";
import AssetTextViewer from "../asset/AssetTextViewer.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import type { MarkdownEditorDocumentChange } from "../ui/markdown-editor/markdownEditorDocumentChange";

const props = defineProps<{
  projectId?: string;
  path: string;
  workspaceRef?: WorkspaceRef | null;
  active?: boolean;
}>();

const emit = defineEmits<{
  (event: "dirtyChange", dirty: boolean): void;
}>();

const preview = ref<ProjectExplorerFilePreview | null>(null);
const loading = ref(false);
const error = ref("");
const assetPayload = ref<AssetPreviewPayload | null>(null);
const sourceText = ref("");
const editorDocument = ref<Text | null>(null);
const dirty = ref(false);
const saving = ref(false);
const saveStatus = ref<"idle" | "saved">("idle");
const originalLineEnding = ref<"\n" | "\r\n" | "\r">("\n");
const sourceEditor = ref<InstanceType<typeof BaseMarkdownEditor> | null>(null);
const pendingPosition = ref<{ line: number; column: number } | null>(null);
let requestEpoch = 0;

const normalizedSourceText = computed(() => (
  sourceText.value.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
));
const editorContentKey = computed(() => [
  props.workspaceRef?.checkoutId ?? props.projectId ?? "workspace-file",
  props.workspaceRef?.expectedGeneration ?? "current",
  props.path,
  preview.value?.contentHash ?? "unloaded",
].join(":"));
const editorStatus = computed(() => {
  if (saving.value) return t("development.editor.saving");
  if (error.value && preview.value) return error.value;
  if (dirty.value) return t("development.editor.unsaved");
  if (saveStatus.value === "saved") return t("development.editor.saved");
  return "";
});

const language = computed(() => {
  const extension = preview.value?.extension ?? "";
  return ({
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    cs: "csharp",
    py: "python",
    rs: "rust",
    md: "markdown",
    yml: "yaml",
    sh: "bash",
    ps1: "powershell",
  } as Record<string, string>)[extension] ?? extension;
});

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

async function loadPreview(): Promise<void> {
  const epoch = ++requestEpoch;
  loading.value = true;
  error.value = "";
  preview.value = null;
  assetPayload.value = null;
  sourceText.value = "";
  editorDocument.value = null;
  setDirty(false);
  saveStatus.value = "idle";
  try {
    const next = props.workspaceRef
      ? await workspaceFilePreview(props.path, props.workspaceRef)
      : await projectExplorerPreviewFile(props.projectId ?? "", props.path);
    if (epoch !== requestEpoch) return;
    preview.value = next;
    sourceText.value = next.text ?? "";
    originalLineEnding.value = next.text?.includes("\r\n")
      ? "\r\n"
      : next.text?.includes("\r")
        ? "\r"
        : "\n";
    if (
      next.kind === "unity"
      && next.checkoutId
      && next.workspaceRelativePath
    ) {
      assetPayload.value = await previewWorkspaceAsset(
        next.workspaceRelativePath,
        undefined,
        {
          checkoutId: next.checkoutId,
          expectedGeneration: next.workspaceGeneration,
        },
      );
    }
    if (epoch === requestEpoch) void applyPendingPosition();
  } catch (cause) {
    if (epoch !== requestEpoch) return;
    error.value = normalizeAppError(cause).message;
  } finally {
    if (epoch === requestEpoch) loading.value = false;
  }
}

async function applyPendingPosition(): Promise<boolean> {
  const position = pendingPosition.value;
  if (!position || preview.value?.kind !== "text") return false;
  await nextTick();
  const view = sourceEditor.value?.getEditorView();
  if (!view) return false;
  const lineNumber = Math.min(view.state.doc.lines, Math.max(1, Math.floor(position.line)));
  const line = view.state.doc.line(lineNumber);
  const columnOffset = Math.min(line.length, Math.max(0, Math.floor(position.column) - 1));
  const anchor = line.from + columnOffset;
  view.dispatch({
    selection: { anchor },
    effects: EditorView.scrollIntoView(anchor, { y: "center" }),
  });
  view.focus();
  pendingPosition.value = null;
  return true;
}

async function revealPosition(line: number, column = 1): Promise<boolean> {
  pendingPosition.value = {
    line: Math.max(1, Math.floor(line || 1)),
    column: Math.max(1, Math.floor(column || 1)),
  };
  return applyPendingPosition();
}

function setDirty(value: boolean): void {
  if (dirty.value === value) return;
  dirty.value = value;
  emit("dirtyChange", value);
}

function onEditorDocumentChange(change: MarkdownEditorDocumentChange): void {
  editorDocument.value = change.doc;
  saveStatus.value = "idle";
  setDirty(change.doc.toString() !== normalizedSourceText.value);
}

function serializedEditorText(): string {
  const normalized = editorDocument.value?.toString() ?? normalizedSourceText.value;
  if (originalLineEnding.value === "\r\n") return normalized.replace(/\n/g, "\r\n");
  if (originalLineEnding.value === "\r") return normalized.replace(/\n/g, "\r");
  return normalized;
}

async function saveFile(): Promise<boolean> {
  const current = preview.value;
  if (
    !current
    || current.kind !== "text"
    || !current.editable
    || !current.contentHash
    || saving.value
  ) return false;
  if (!dirty.value) return true;
  saving.value = true;
  error.value = "";
  try {
    const next = props.workspaceRef
      ? await workspaceFileWrite(
        props.path,
        serializedEditorText(),
        current.contentHash,
        props.workspaceRef,
      )
      : await projectExplorerWriteFile(
        props.projectId ?? "",
        props.path,
        serializedEditorText(),
        current.contentHash,
      );
    preview.value = next;
    sourceText.value = next.text ?? "";
    editorDocument.value = null;
    saveStatus.value = "saved";
    setDirty(false);
    return true;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    return false;
  } finally {
    saving.value = false;
  }
}

watch(
  () => [
    props.projectId,
    props.path,
    props.workspaceRef?.checkoutId,
    props.workspaceRef?.expectedGeneration,
  ] as const,
  loadPreview,
  { immediate: true },
);
watch(() => props.active, (active) => {
  if (active) void applyPendingPosition();
});

defineExpose({ saveFile, revealPosition });
</script>

<template>
  <section class="workspace-file-preview">
    <div v-if="loading && !preview" class="workspace-file-preview-state">
      {{ t("development.preview.loading") }}
    </div>
    <div v-else-if="error && !preview" class="workspace-file-preview-state error">{{ error }}</div>

    <WorkspaceAssetPreview
      v-else-if="preview?.kind === 'unity'"
      :workspace-ref="preview.checkoutId ? {
        checkoutId: preview.checkoutId,
        expectedGeneration: preview.workspaceGeneration,
      } : null"
      :path="preview.workspaceRelativePath || preview.path"
      :title="preview.name"
      :payload="assetPayload"
      :loading="loading"
      :error="error"
      :auto-load-preview="false"
    />

    <template v-else-if="preview">
      <header class="workspace-file-preview-header">
        <span class="workspace-file-preview-title">{{ preview.name }}</span>
        <span class="workspace-file-preview-path" :title="preview.path">{{ preview.path }}</span>
        <span
          v-if="editorStatus"
          class="workspace-file-preview-status"
          :class="{ error: !!error && !!preview }"
          :title="editorStatus"
        >{{ editorStatus }}</span>
        <BaseButton
          v-if="preview.kind === 'text' && preview.editable"
          class="workspace-file-preview-save"
          :disabled="!dirty || saving"
          :title="t('common.save')"
          @click="saveFile"
        >
          <LucideIcon :icon="Save" :size="12" :stroke-width="2" />
          {{ t("common.save") }}
        </BaseButton>
      </header>
      <div class="workspace-file-preview-body">
        <BaseMarkdownEditor
          v-if="preview.kind === 'text' && preview.editable"
          ref="sourceEditor"
          :model-value="normalizedSourceText"
          :content-key="editorContentKey"
          :content-path="preview.path"
          :workspace-ref="workspaceRef"
          :active="active !== false"
          view-mode="native"
          transaction-model
          @document-change="onEditorDocumentChange"
          @shortcut-save="saveFile"
        />
        <AssetTextViewer
          v-else-if="preview.kind === 'text'"
          :snippet="preview.text || ''"
          :truncated="preview.truncated"
          :total-lines="preview.totalLines || 1"
          :language="language"
        />
        <div v-else-if="preview.kind === 'image'" class="workspace-media-preview image">
          <img :src="preview.dataUrl" :alt="preview.name" />
        </div>
        <iframe
          v-else-if="preview.kind === 'pdf'"
          class="workspace-pdf-preview"
          :src="preview.dataUrl"
          :title="preview.name"
        />
        <div v-else-if="preview.kind === 'audio'" class="workspace-media-preview">
          <audio :src="preview.dataUrl" controls />
        </div>
        <div v-else-if="preview.kind === 'video'" class="workspace-media-preview video">
          <video :src="preview.dataUrl" controls />
        </div>
        <div v-else class="workspace-binary-preview">
          <div>{{ preview.name }}</div>
          <dl>
            <dt>{{ t("development.preview.path") }}</dt><dd>{{ preview.path }}</dd>
            <dt>{{ t("development.preview.size") }}</dt><dd>{{ formatSize(preview.size) }}</dd>
            <dt>{{ t("development.preview.type") }}</dt><dd>{{ preview.mimeType }}</dd>
          </dl>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.workspace-file-preview {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
}

.workspace-file-preview-state {
  margin: auto;
  color: var(--text-secondary);
  font-size: 12px;
}

.workspace-file-preview-state.error {
  color: var(--status-error-fg, var(--text-color));
}

.workspace-file-preview-header {
  min-height: 38px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  border-bottom: 1px solid var(--border-color);
}

.workspace-file-preview-title {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
}

.workspace-file-preview-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-file-preview-status {
  max-width: min(260px, 30%);
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-file-preview-status.error {
  color: var(--status-error-fg, var(--text-color));
}

.workspace-file-preview-save {
  flex-shrink: 0;
}

.workspace-file-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.workspace-media-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  overflow: auto;
}

.workspace-media-preview.image img,
.workspace-media-preview.video video {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}

.workspace-media-preview audio {
  width: min(520px, 100%);
}

.workspace-pdf-preview {
  flex: 1;
  width: 100%;
  min-height: 0;
  border: 0;
  background: var(--panel-bg);
}

.workspace-binary-preview {
  width: min(620px, calc(100% - 36px));
  margin: 24px auto auto;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  font-size: 12px;
}

.workspace-binary-preview > div {
  margin-bottom: 10px;
  font-weight: 600;
}

.workspace-binary-preview dl {
  margin: 0;
  display: grid;
  grid-template-columns: 58px minmax(0, 1fr);
  gap: 6px 10px;
}

.workspace-binary-preview dt {
  color: var(--text-secondary);
}

.workspace-binary-preview dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
  font-family: var(--font-mono-identifier);
}
</style>
