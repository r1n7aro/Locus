<script setup lang="ts">
import { EditorView } from "@codemirror/view";
import { computed, nextTick, ref, watch } from "vue";
import { AlertTriangle } from "lucide";
import { t } from "../../i18n";
import { useDocumentFileSession } from "../../composables/useDocumentFileSession";
import { documentSessionKey } from "../../document/documentIdentity";
import {
  detectTextDocumentLineEnding,
  normalizeTextDocumentLineEndings,
  serializeTextDocumentLineEndings,
  type TextDocumentLineEnding,
} from "../../document/textDocumentFormat";
import { useFileChangeRevalidation } from "../../composables/useFileChangeRevalidation";
import { useMarkdownDocumentOutline } from "../../composables/useMarkdownDocumentOutline";
import { normalizeAppError } from "../../services/errors";
import { previewWorkspaceAsset } from "../../services/asset";
import type { WorkspaceRef } from "../../services/project";
import {
  resolveToolFilePreviewHighlightRanges,
  type ToolFilePreviewHighlight,
} from "../../services/toolFilePreviewWindow";
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
import WorkspaceCsvEditor from "../csv/WorkspaceCsvEditor.vue";
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
  (event: "pathChange", path: string): void;
}>();

const isCsv = computed(() => /\.csv$/i.test(props.path));
const csvEditor = ref<InstanceType<typeof WorkspaceCsvEditor> | null>(null);

interface FileDocument {
  preview: ProjectExplorerFilePreview;
  assetPayload: AssetPreviewPayload | null;
}

interface FileDocumentDraft {
  text: string;
  lineEnding: TextDocumentLineEnding;
}

const resourceKey = computed(() => documentSessionKey(props.workspaceRef, [
  "workspaceFile", props.projectId ?? "", props.path,
]));
const fileSession = useDocumentFileSession<FileDocument, FileDocumentDraft>({
  key: () => resourceKey.value,
  emptyDraft: () => ({ text: "", lineEnding: "\n" }),
  read: readDocument,
  write: async (current, draft) => {
    const text = serializeTextDocumentLineEndings(draft.text, draft.lineEnding);
    const hash = current.preview.contentHash!;
    const next = props.workspaceRef
      ? await workspaceFileWrite(props.path, text, hash, props.workspaceRef)
      : await projectExplorerWriteFile(props.projectId ?? "", props.path, text, hash);
    return { preview: next, assetPayload: null };
  },
  toDraft: ({ preview }) => ({
    text: normalizeTextDocumentLineEndings(preview.text ?? ""),
    lineEnding: detectTextDocumentLineEnding(preview.text ?? ""),
  }),
  equals: (left, right) => left.text === right.text && left.lineEnding === right.lineEnding,
  canSave: ({ preview }) => preview.kind === "text" && preview.editable && !!preview.contentHash,
  errorMessage: (cause) => normalizeAppError(cause).message,
});
const { loading, error, dirty, saving } = fileSession;
const preview = computed(() => fileSession.document.value?.preview ?? null);
const assetPayload = computed(() => fileSession.document.value?.assetPayload ?? null);
const diskChanged = ref(false);
const observedDiskRevision = ref<ProjectExplorerFileRevision | null>(null);
const sourceEditor = ref<InstanceType<typeof BaseMarkdownEditor> | null>(null);
const documentScrollerRef = ref<HTMLElement | null>(null);
const documentPageRef = ref<HTMLElement | null>(null);
const documentBodyRef = ref<HTMLElement | null>(null);
const pendingPosition = ref<{
  line: number;
  column: number;
  highlight?: ToolFilePreviewHighlight;
} | null>(null);
const normalizedSourceText = computed(() => fileSession.modelDraft.value.text);
const editorContentKey = computed(() => JSON.stringify([
  resourceKey.value, preview.value?.contentHash ?? "unloaded",
]));
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
const isMarkdownDocument = computed(() => (
  preview.value?.kind === "text" && preview.value.editable && editorViewMode.value === "rendered"
));
const documentTitle = computed(() => preview.value?.name.replace(/\.(?:md|markdown)$/iu, "") ?? "");
const {
  documentOutlineItems,
  activeOutlineId,
  documentOutlineMarginTop,
  documentOutlineMaxHeight,
  outlineItemPadding,
  scrollToDocumentOutlineItem,
  scheduleDocumentOutlineActiveUpdate,
} = useMarkdownDocumentOutline({
  documentKey: () => editorContentKey.value,
  source: () => isMarkdownDocument.value
    ? fileSession.draft.value.text
    : "",
  active: () => props.active !== false,
  scroller: documentScrollerRef,
  page: documentPageRef,
  body: documentBodyRef,
  editor: sourceEditor,
});

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

async function readDocument(): Promise<FileDocument> {
  const next = await readPreview();
  const payload = next.kind === "unity" && next.checkoutId && next.workspaceRelativePath
    ? await previewWorkspaceAsset(next.workspaceRelativePath, undefined, {
        checkoutId: next.checkoutId,
        expectedGeneration: next.workspaceGeneration,
        expectedMaterializationEpoch: next.materializationEpoch,
      })
    : null;
  return { preview: next, assetPayload: payload };
}

async function loadPreview(options: { keepCurrent?: boolean; keepDraft?: boolean } = {}): Promise<boolean> {
  const key = resourceKey.value;
  const before = fileSession.document.value;
  const loaded = await fileSession.load(options);
  if (key !== resourceKey.value) return false;
  if (loaded) {
    diskChanged.value = false;
    observedDiskRevision.value = null;
    void applyPendingPosition();
  } else if (options.keepCurrent && fileSession.document.value === before && dirty.value) {
    diskChanged.value = true;
  }
  return loaded;
}

async function probeFileRevision(): Promise<ProjectExplorerFileRevision> {
  return props.workspaceRef
    ? await workspaceFileRevision(props.path, props.workspaceRef)
    : await projectExplorerFileRevision(props.projectId ?? "", props.path);
}

const { checkNow: refreshIfChanged } = useFileChangeRevalidation({
  active: () => props.active !== false && !isCsv.value,
  currentRevision: () => observedDiskRevision.value ?? preview.value?.revision ?? null,
  probe: probeFileRevision,
  workspaceRef: () => props.workspaceRef,
  workspacePath: () => props.workspaceRef ? props.path : null,
  onChanged: async (revision) => {
    observedDiskRevision.value = revision;
    if (dirty.value || saving.value) {
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
  const viewBefore = sourceEditor.value?.getEditorView();
  const selection = viewBefore ? {
    anchor: viewBefore.state.selection.main.anchor,
    head: viewBefore.state.selection.main.head,
  } : null;
  const loaded = await loadPreview({ keepCurrent: true, keepDraft: true });
  if (!loaded || preview.value?.kind !== "text" || !preview.value.editable) return;
  await nextTick();
  const view = sourceEditor.value?.getEditorView();
  if (!view) return;
  const normalized = fileSession.draft.value.text;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: normalized },
    selection: selection ? {
      anchor: Math.min(normalized.length, selection.anchor),
      head: Math.min(normalized.length, selection.head),
    } : undefined,
  });
}

async function applyPendingPosition(): Promise<boolean> {
  const position = pendingPosition.value;
  if (!position || preview.value?.kind !== "text") return false;
  await nextTick();
  const view = sourceEditor.value?.getEditorView();
  if (!view) return false;
  const highlight = position.highlight
    ? resolveToolFilePreviewHighlightRanges(view.state.doc.toString(), 1, position.highlight)[0]
    : null;
  const lineNumber = Math.min(view.state.doc.lines, Math.max(1, Math.floor(highlight?.startLine ?? position.line)));
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
  if (isCsv.value) return await csvEditor.value?.revealPosition(line, column) ?? false;
  pendingPosition.value = {
    line: Math.max(1, Math.floor(line || 1)),
    column: Math.max(1, Math.floor(column || 1)),
  };
  return applyPendingPosition();
}

async function revealToolFileHighlight(highlight?: ToolFilePreviewHighlight): Promise<boolean> {
  if (isCsv.value) { await nextTick(); return await csvEditor.value?.revealToolFileHighlight(highlight) ?? false; }
  pendingPosition.value = { line: 1, column: 1, highlight };
  return applyPendingPosition();
}

function onEditorDocumentChange(change: MarkdownEditorDocumentChange): void {
  fileSession.updateDraft({ ...fileSession.draft.value, text: change.doc.toString() });
}

function serializedEditorText(): string {
  const draft = fileSession.draft.value;
  return serializeTextDocumentLineEndings(draft.text, draft.lineEnding);
}

async function saveFile(): Promise<boolean> {
  if (isCsv.value) return await csvEditor.value?.saveFile() ?? false;
  return fileSession.save();
}

function exportTransferSnapshot(): WorkbenchEditorTransferSnapshot {
  if (isCsv.value && csvEditor.value) return csvEditor.value.exportTransferSnapshot();
  const view = sourceEditor.value?.getEditorView();
  return {
    kind: "workspaceFile",
    text: serializedEditorText(),
    contentHash: preview.value?.contentHash ?? "",
    originalLineEnding: fileSession.draft.value.lineEnding,
    selection: view ? {
      anchor: view.state.selection.main.anchor,
      head: view.state.selection.main.head,
    } : null,
    scrollTop: documentScrollerRef.value?.scrollTop ?? view?.scrollDOM.scrollTop ?? null,
  };
}

async function applyTransferSnapshot(snapshot: WorkbenchEditorTransferSnapshot): Promise<boolean> {
  if (isCsv.value) {
    await nextTick();
    return await csvEditor.value?.applyTransferSnapshot(snapshot) ?? false;
  }
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
  const normalized = normalizeTextDocumentLineEndings(snapshot.text);
  fileSession.updateDraft({ text: normalized, lineEnding: snapshot.originalLineEnding });
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
  if (snapshot.scrollTop != null) {
    await nextTick();
    const scroller = documentScrollerRef.value ?? view.scrollDOM;
    scroller.scrollTop = snapshot.scrollTop;
  }
  return true;
}

watch(dirty, (value) => emit("dirtyChange", value));
watch(fileSession.document, () => {
  diskChanged.value = false;
  observedDiskRevision.value = null;
}, { flush: "sync" });
watch(resourceKey, () => {
  diskChanged.value = false;
  observedDiskRevision.value = null;
  if (!isCsv.value) void loadPreview();
}, { immediate: true });
watch(() => props.active, (active) => {
  if (active) void applyPendingPosition();
});

defineExpose({
  discardChanges: () => csvEditor.value?.discardChanges(),
  saveFile,
  revealPosition,
  revealToolFileHighlight,
  refreshIfChanged: () => isCsv.value ? csvEditor.value?.refreshIfChanged() : refreshIfChanged(),
  exportTransferSnapshot,
  applyTransferSnapshot,
});
</script>

<template>
  <WorkspaceCsvEditor v-if="isCsv" ref="csvEditor" :path="path" :project-id="projectId" :workspace-ref="workspaceRef" :active="active" @dirty-change="emit('dirtyChange', $event)" @path-change="emit('pathChange', $event)" />
  <section v-else class="workspace-file-preview">
    <div v-if="loading && !preview" class="workspace-file-preview-state">
      {{ t("development.preview.loading") }}
    </div>
    <div v-else-if="error && !preview" class="workspace-file-preview-state error">{{ error }}</div>

    <WorkspaceAssetPreview
      v-else-if="preview?.kind === 'unity'"
      :workspace-ref="preview.checkoutId ? {
        checkoutId: preview.checkoutId,
        expectedGeneration: preview.workspaceGeneration,
        expectedMaterializationEpoch: preview.materializationEpoch,
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
        <div
          v-if="isMarkdownDocument"
          ref="documentScrollerRef"
          class="document-scroller"
          @scroll.passive="scheduleDocumentOutlineActiveUpdate"
        >
          <div class="document-workspace" :class="{ 'has-outline': documentOutlineItems.length > 0 }">
            <aside
              v-if="documentOutlineItems.length"
              class="document-outline"
              :style="{
                marginTop: documentOutlineMarginTop,
                maxHeight: documentOutlineMaxHeight,
              }"
            >
              <nav class="document-outline-nav" :aria-label="t('knowledge.preview.outline')">
                <button
                  v-for="item in documentOutlineItems"
                  :key="item.id"
                  type="button"
                  class="document-outline-item"
                  :class="{ active: activeOutlineId === item.id }"
                  :style="{ paddingInlineStart: outlineItemPadding(item) }"
                  :title="item.text"
                  :aria-current="activeOutlineId === item.id ? 'location' : undefined"
                  @click="scrollToDocumentOutlineItem(item)"
                >
                  <span>{{ item.text }}</span>
                </button>
              </nav>
            </aside>
            <article ref="documentPageRef" class="document-page">
              <header class="document-heading">
                <h1 class="document-title">{{ documentTitle }}</h1>
              </header>
              <section ref="documentBodyRef" class="document-body">
                <BaseMarkdownEditor
                  ref="sourceEditor"
                  :model-value="normalizedSourceText"
                  :content-key="editorContentKey"
                  :content-path="preview.path"
                  :workspace-ref="workspaceRef"
                  :active="active !== false"
                  :view-mode="editorViewMode"
                  auto-grow
                  :min-height="360"
                  transaction-model
                  @document-change="onEditorDocumentChange"
                  @shortcut-save="saveFile"
                />
              </section>
            </article>
          </div>
        </div>
        <BaseMarkdownEditor
          v-else-if="preview.kind === 'text' && preview.editable"
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
  min-width: 0;
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

<style scoped src="../ui/markdown-document.css" />
