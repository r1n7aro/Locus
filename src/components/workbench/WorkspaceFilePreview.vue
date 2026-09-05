<script setup lang="ts">
import type { Text } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { computed, nextTick, ref, watch } from "vue";
import { AlertTriangle } from "lucide";
import { t } from "../../i18n";
import { useFileChangeRevalidation } from "../../composables/useFileChangeRevalidation";
import { normalizeAppError } from "../../services/errors";
import { previewWorkspaceAsset } from "../../services/asset";
import type { WorkspaceRef } from "../../services/project";
import {
  projectExplorerPreviewFile,
  projectExplorerFileRevision,
  projectExplorerWriteFile,
  workspaceFilePreview,
  workspaceFileRevision,
  workspaceFileWrite,
} from "../../services/workspaceExplorer";
import type { AssetPreviewPayload } from "../../types";
import type {
  ProjectExplorerFilePreview,
  ProjectExplorerFileRevision,
} from "../../types/workbench";
import type { WorkbenchEditorTransferSnapshot } from "../../types/workbench";
import WorkspaceAssetPreview from "../asset/WorkspaceAssetPreview.vue";
import AssetTextViewer from "../asset/AssetTextViewer.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import type { MarkdownEditorDocumentChange } from "../ui/markdown-editor/markdownEditorDocumentChange";
import type { MarkdownEditorViewMode } from "../ui/markdownEditorViewMode";

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
const diskChanged = ref(false);
const observedDiskRevision = ref<ProjectExplorerFileRevision | null>(null);
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
const editorViewMode = computed<MarkdownEditorViewMode>(() => (
  preview.value?.extension === "md" || preview.value?.extension === "markdown"
    ? "rendered"
    : "native"
));

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

async function readPreview(): Promise<ProjectExplorerFilePreview> {
  return props.workspaceRef
    ? await workspaceFilePreview(props.path, props.workspaceRef)
    : await projectExplorerPreviewFile(props.projectId ?? "", props.path);
}

async function loadPreview(options: { keepCurrent?: boolean } = {}): Promise<boolean> {
  const epoch = ++requestEpoch;
  loading.value = true;
  error.value = "";
  if (!options.keepCurrent) {
    preview.value = null;
    assetPayload.value = null;
    sourceText.value = "";
    editorDocument.value = null;
    setDirty(false);
  }
  try {
    const next = await readPreview();
    if (epoch !== requestEpoch) return false;
    let nextAssetPayload: AssetPreviewPayload | null = null;
    if (
      next.kind === "unity"
      && next.checkoutId
      && next.workspaceRelativePath
    ) {
      nextAssetPayload = await previewWorkspaceAsset(
        next.workspaceRelativePath,
        undefined,
        {
          checkoutId: next.checkoutId,
          expectedGeneration: next.workspaceGeneration,
        },
      );
    }
    if (epoch !== requestEpoch) return false;
    preview.value = next;
    assetPayload.value = nextAssetPayload;
    sourceText.value = next.text ?? "";
    editorDocument.value = null;
    originalLineEnding.value = next.text?.includes("\r\n")
      ? "\r\n"
      : next.text?.includes("\r")
        ? "\r"
        : "\n";
    diskChanged.value = false;
    observedDiskRevision.value = null;
    setDirty(false);
    if (epoch === requestEpoch) void applyPendingPosition();
    return true;
  } catch (cause) {
    if (epoch !== requestEpoch) return false;
    error.value = normalizeAppError(cause).message;
    return false;
  } finally {
    if (epoch === requestEpoch) loading.value = false;
  }
}

async function probeFileRevision(): Promise<ProjectExplorerFileRevision> {
  return props.workspaceRef
    ? await workspaceFileRevision(props.path, props.workspaceRef)
    : await projectExplorerFileRevision(props.projectId ?? "", props.path);
}

const { checkNow: refreshIfChanged } = useFileChangeRevalidation({
  active: () => props.active !== false,
  currentRevision: () => observedDiskRevision.value ?? preview.value?.revision ?? null,
  probe: probeFileRevision,
  workspaceRef: () => props.workspaceRef,
  workspacePath: () => props.workspaceRef ? props.path : null,
  onChanged: async (revision) => {
    observedDiskRevision.value = revision;
    if (dirty.value) {
      diskChanged.value = true;
      return;
    }
    await loadPreview({ keepCurrent: true });
  },
});

async function useDiskVersion(): Promise<void> {
  await loadPreview({ keepCurrent: true });
}

async function keepLocalVersion(): Promise<void> {
  const localText = serializedEditorText();
  const localLineEnding = originalLineEnding.value;
  const viewBefore = sourceEditor.value?.getEditorView();
  const selection = viewBefore ? {
    anchor: viewBefore.state.selection.main.anchor,
    head: viewBefore.state.selection.main.head,
  } : null;
  const loaded = await loadPreview({ keepCurrent: true });
  if (!loaded || preview.value?.kind !== "text" || !preview.value.editable) return;
  await nextTick();
  const view = sourceEditor.value?.getEditorView();
  if (!view) return;
  const normalized = localText.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: normalized },
    selection: selection ? {
      anchor: Math.min(normalized.length, selection.anchor),
      head: Math.min(normalized.length, selection.head),
    } : undefined,
  });
  originalLineEnding.value = localLineEnding;
  diskChanged.value = false;
  observedDiskRevision.value = null;
  setDirty(normalized !== normalizedSourceText.value);
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
    diskChanged.value = false;
    observedDiskRevision.value = null;
    setDirty(false);
    return true;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    return false;
  } finally {
    saving.value = false;
  }
}

function exportTransferSnapshot(): WorkbenchEditorTransferSnapshot {
  const view = sourceEditor.value?.getEditorView();
  return {
    kind: "workspaceFile",
    text: serializedEditorText(),
    contentHash: preview.value?.contentHash ?? "",
    originalLineEnding: originalLineEnding.value,
    selection: view ? {
      anchor: view.state.selection.main.anchor,
      head: view.state.selection.main.head,
    } : null,
    scrollTop: view?.scrollDOM.scrollTop ?? null,
  };
}

async function applyTransferSnapshot(snapshot: WorkbenchEditorTransferSnapshot): Promise<boolean> {
  if (snapshot.kind !== "workspaceFile") return false;
  const deadline = Date.now() + 4_000;
  while (!sourceEditor.value?.getEditorView() && Date.now() < deadline) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 16));
  }
  const view = sourceEditor.value?.getEditorView();
  if (!view || preview.value?.kind !== "text" || !preview.value.editable) return false;
  if (snapshot.contentHash && preview.value.contentHash !== snapshot.contentHash) {
    error.value = t("development.editor.transferConflict");
    return false;
  }
  originalLineEnding.value = snapshot.originalLineEnding;
  const normalized = snapshot.text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const selection = snapshot.selection
    ? {
        anchor: Math.min(normalized.length, Math.max(0, snapshot.selection.anchor)),
        head: Math.min(normalized.length, Math.max(0, snapshot.selection.head)),
      }
    : undefined;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: normalized },
    selection,
  });
  if (snapshot.scrollTop != null) view.scrollDOM.scrollTop = snapshot.scrollTop;
  return true;
}

watch(
  () => [
    props.projectId,
    props.path,
    props.workspaceRef?.checkoutId,
    props.workspaceRef?.expectedGeneration,
  ] as const,
  () => void loadPreview(),
  { immediate: true },
);
watch(() => props.active, (active) => {
  if (active) void applyPendingPosition();
});

defineExpose({
  saveFile,
  revealPosition,
  refreshIfChanged,
  exportTransferSnapshot,
  applyTransferSnapshot,
});
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
      :active="active !== false"
      :payload="assetPayload"
      :preview-revision="preview.revision.key"
      :loading="loading"
      :error="error"
      :auto-load-preview="false"
      :show-header="false"
    />

    <template v-else-if="preview">
      <div v-if="diskChanged" class="workspace-file-preview-conflict" role="status">
        <span class="workspace-file-preview-conflict-text">
          <LucideIcon :icon="AlertTriangle" :size="14" :stroke-width="1.8" />
          {{ t("development.editor.diskChanged") }}
        </span>
        <span class="workspace-file-preview-conflict-actions">
          <BaseButton size="sm" @click="useDiskVersion">
            {{ t("development.editor.useDisk") }}
          </BaseButton>
          <BaseButton size="sm" @click="keepLocalVersion">
            {{ t("development.editor.keepLocal") }}
          </BaseButton>
        </span>
      </div>
      <div v-if="error" class="workspace-file-preview-inline-error">{{ error }}</div>
      <div class="workspace-file-preview-body">
        <BaseMarkdownEditor
          v-if="preview.kind === 'text' && preview.editable"
          ref="sourceEditor"
          :model-value="normalizedSourceText"
          :content-key="editorContentKey"
          :content-path="preview.path"
          :workspace-ref="workspaceRef"
          :active="active !== false"
          :view-mode="editorViewMode"
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

.workspace-file-preview-inline-error {
  flex-shrink: 0;
  padding: 7px 10px;
  border-bottom: 1px solid var(--border-color);
  color: var(--status-error-fg, var(--text-color));
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-file-preview-conflict {
  flex-shrink: 0;
  min-height: 38px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 5px 8px 5px 10px;
  border-bottom: 1px solid var(--status-warn-border, var(--border-color));
  background: var(--status-warn-bg, var(--sidebar-bg));
  color: var(--status-warn-fg, var(--text-color));
  font-size: 12px;
}

.workspace-file-preview-conflict-text {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 7px;
}

.workspace-file-preview-conflict-actions {
  display: inline-flex;
  gap: 6px;
  margin-left: auto;
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
