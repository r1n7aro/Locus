<script setup lang="ts">
import { EditorView } from "@codemirror/view";
import { Text } from "@codemirror/state";
import { markdownEditorTextFromString } from "../ui/markdown-editor/markdownEditorDocumentChange";
import { computed, defineAsyncComponent, inject, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { AlertTriangle } from "lucide";
import { t } from "../../i18n";
import { useDocumentFileSession } from "../../composables/useDocumentFileSession";
import { documentSessionKey } from "../../document/documentIdentity";
import { rebaseDocumentText } from "../../document/documentText";
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
  explorerFileAction,
  projectExplorerPreviewFile,
  projectExplorerFileRevision,
  projectExplorerWriteFile,
  releasePreviewBlob,
  workspaceFilePreview,
  workspaceFileRevision,
  workspaceFileWrite,
} from "../../services/workspaceExplorer";
import type { AssetPreviewPayload } from "../../types";
import type {
  ProjectExplorerFilePreview,
  ProjectExplorerFileRevision,
} from "../../types/workbench";
import type { EditorSnapshotOptions, WorkbenchEditorTransferSnapshot } from "../../types/workbench";
import type { ReadingTarget } from "../../services/readingPosition";
import WorkspaceAssetPreview from "../asset/WorkspaceAssetPreview.vue";
import AssetTextViewer from "../asset/AssetTextViewer.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import MarkdownCommentRail from "../ui/markdown-editor/MarkdownCommentRail.vue";
import { createMarkdownCommentController } from "../ui/markdown-editor/markdownCommentController";
import WorkspaceCsvEditor from "../csv/WorkspaceCsvEditor.vue";
import ModelViewer from "../model-preview/ModelViewer.vue";
import { modelFormatFromPath } from "../model-preview/modelFormat";
import type { MarkdownEditorDocumentChange } from "../ui/markdown-editor/markdownEditorDocumentChange";
import type { MarkdownEditorSelection } from "../ui/markdown-editor/markdownEditorSelection";
import type { MarkdownEditorViewMode } from "../ui/markdownEditorViewMode";
import {
  DOCUMENT_QUOTE_IN_NEW_SESSION_KEY,
  fileSelectionReference,
  normalizeSelectionFilePath,
  selectionLocationLabel,
  useDocumentSelectionReporter,
} from "../../composables/documentSelection";
import { useNotificationStore } from "../../stores/notification";
import { validResourceName } from "../explorer/explorerResourceActions";
import { WORKSPACE_FILE_PREVIEW_CACHE } from "./workspaceFilePreviewCache";
import { isExternalWorkspaceFilePath } from "./workbenchExternalFile";

// Loaded on the first `.tex` file only.
const LatexPreviewPane = defineAsyncComponent(() => import("./LatexPreviewPane.vue"));

const props = defineProps<{
  projectId?: string;
  path: string;
  workspaceRef?: WorkspaceRef | null;
  active?: boolean;
}>();

const emit = defineEmits<{
  (event: "dirtyChange", dirty: boolean): void;
  (event: "pathChange", path: string): void;
  (event: "ready"): void;
}>();

const previewCache = inject(WORKSPACE_FILE_PREVIEW_CACHE, null);
let cachedPreview: ProjectExplorerFilePreview | undefined;
let disposed = false;
onBeforeUnmount(() => { disposed = true; });
// One fresh blob lease per failure streak when an element finds its bytes gone.
let blobRetried = false;

const isCsv = computed(() => /\.csv$/i.test(props.path));
const csvEditor = ref<InstanceType<typeof WorkspaceCsvEditor> | null>(null);

interface FileDocument {
  preview: ProjectExplorerFilePreview;
  assetPayload: AssetPreviewPayload | null;
}

interface FileDocumentDraft {
  text: Text;
  lineEnding: TextDocumentLineEnding;
}

const filePath = ref(props.path);
const scopeKey = computed(() => documentSessionKey(props.workspaceRef, ["workspaceFile", props.projectId ?? ""]));
const resourceKey = computed(() => documentSessionKey(props.workspaceRef, [
  "workspaceFile", props.projectId ?? "", filePath.value,
]));
const normalizePath = (path: string) => {
  const normalized = path.replace(/\\/g, "/");
  return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized;
};
let localRename: { scope: string; paths: Set<string>; nextPath?: string } | null = null;
let renameRevision = 0;
const diskChanged = ref(false);
const fileSession = useDocumentFileSession<FileDocument, FileDocumentDraft>({
  key: () => resourceKey.value,
  emptyDraft: () => ({ text: Text.empty, lineEnding: "\n" }),
  read: readDocument,
  write: async (current, draft) => {
    const text = serializeTextDocumentLineEndings(draft.text.toString(), draft.lineEnding);
    const hash = current.preview.contentHash!;
    const next = props.workspaceRef
      ? await workspaceFileWrite(filePath.value, text, hash, props.workspaceRef)
      : await projectExplorerWriteFile(props.projectId ?? "", filePath.value, text, hash);
    return { preview: next, assetPayload: null };
  },
  toDraft: ({ preview }) => ({
    text: markdownEditorTextFromString(normalizeTextDocumentLineEndings(preview.text ?? "")),
    lineEnding: detectTextDocumentLineEnding(preview.text ?? ""),
  }),
  equals: (left, right) => left.text.eq(right.text) && left.lineEnding === right.lineEnding,
  autoSave: () => /\.(md|markdown)$/i.test(props.path) && !diskChanged.value,
  mergeDrafts: (base, local, remote) => {
    const merged = rebaseDocumentText(base.text.toString(), local.text.toString(), remote.text.toString());
    return merged.conflicts.length ? null : { text: markdownEditorTextFromString(merged.text), lineEnding: local.lineEnding };
  },
  canSave: ({ preview }) => preview.kind === "text" && preview.editable && !!preview.contentHash,
  errorMessage: (cause) => normalizeAppError(cause).message,
});
const { loading, error, dirty, saving, relocating: renaming } = fileSession;
const preview = computed(() => fileSession.document.value?.preview ?? null);
const assetPayload = computed(() => fileSession.document.value?.assetPayload ?? null);
/** Media and model bytes are parked in the backend binary cache; this preview owns the lease. */
const previewBlobUrl = computed(() => preview.value?.blobUrl ?? "");
watch(previewBlobUrl, (_next, previous) => { if (previous) void releasePreviewBlob(previous); });
onBeforeUnmount(() => { if (previewBlobUrl.value) void releasePreviewBlob(previewBlobUrl.value); });
const observedDiskRevision = ref<ProjectExplorerFileRevision | null>(null);
const sourceEditor = ref<InstanceType<typeof BaseMarkdownEditor> | null>(null);
/** Comments of the rendered document, shown in the page's right margin. */
const documentComments = createMarkdownCommentController();
const documentScrollerRef = ref<HTMLElement | null>(null);
const documentPageRef = ref<HTMLElement | null>(null);
const documentBodyRef = ref<HTMLElement | null>(null);
const pendingPosition = ref<{
  line: number;
  column: number;
  highlight?: ToolFilePreviewHighlight;
} | null>(null);
const normalizedSourceText = computed(() => fileSession.modelDraft.value.text.toString());
const savedSourceText = computed(() => fileSession.baseDraft.value.text.toString());
// A save or disk refresh changes the revision, not the editing session. Keep
// selection, history and scroll attached to the resource's workspace lifetime.
const editorContentKey = ref(resourceKey.value);
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
/**
 * `.tex` files open as the rendered preview, or as source when no TeX engine
 * is installed; either view is one click away. The preview pane and its
 * services load only for `.tex` files, so other files pay nothing for it.
 */
const isLatexDocument = computed(() => (
  preview.value?.kind === "text" && preview.value.extension?.toLowerCase() === "tex"
));
const latexView = ref<"preview" | "source">("preview");
/** Set once the user picks a view; until then a missing engine switches to source. */
const latexViewPinned = ref(false);
function selectLatexView(value: string) {
  latexViewPinned.value = true;
  latexView.value = value === "source" ? "source" : "preview";
}
function onLatexUnavailable() {
  if (!latexViewPinned.value) latexView.value = "source";
}
const latexViewOptions = computed(() => [
  { value: "preview", label: t("latexPreview.view.preview") },
  { value: "source", label: t("latexPreview.view.source") },
]);
/** Durable reading position; checkout paths share it across a project's worktrees. */
const readingTarget = computed<ReadingTarget | null>(() => {
  if (!isMarkdownDocument.value || loading.value) return null;
  const path = normalizePath(filePath.value);
  if (props.workspaceRef) return { checkoutId: props.workspaceRef.checkoutId, resource: ["file", path] };
  if (props.projectId && !/^(?:[a-z]:\/|\/)/i.test(path)) {
    return { checkoutId: null, resource: ["project-file", props.projectId, path] };
  }
  return { checkoutId: null, resource: ["file", path] };
});
const documentTitle = computed(() => preview.value?.name.replace(/\.(?:md|markdown)$/iu, "") ?? "");
const titleDraft = ref("");
const titleError = ref("");
// Workspace-scoped rename stays inside the workspace; files opened from
// outside it keep a read-only title. Mounted project files are unaffected.
const canRename = computed(() => !!props.projectId && isMarkdownDocument.value
  && !(props.workspaceRef && isExternalWorkspaceFilePath(filePath.value)));
const titleDirty = computed(() => canRename.value
  && titleDraft.value.trim().replace(/\.(?:md|markdown)$/iu, "") !== documentTitle.value);
watch(documentTitle, (title) => { titleDraft.value = title; }, { immediate: true });

const selectionReporter = useDocumentSelectionReporter("file");
watch(resourceKey, () => selectionReporter.clear());

function onEditorSelectionChange(selection: MarkdownEditorSelection | null): void {
  if (!selectionReporter.enabled) return;
  const current = preview.value;
  if (!selection?.ranges.length || !current || !isMarkdownDocument.value) {
    void selectionReporter.update(null);
    return;
  }
  // The editor holds the whole file, so its line numbers are physical lines.
  const path = normalizeSelectionFilePath(current.path);
  const content = fileSelectionReference(path, selection, {
    draftLabel: dirty.value ? t("editor.quoteDraft") : undefined,
  });
  void selectionReporter.update(() => ({
    checkoutId: props.workspaceRef?.checkoutId ?? null,
    filePath: path,
    name: current.name,
    location: selectionLocationLabel(path, content),
    content,
  }));
}

const quoteInNewSession = inject(DOCUMENT_QUOTE_IN_NEW_SESSION_KEY, null);
const quotingSelection = ref(false);

async function onQuoteSelectionInNewSession(selection: MarkdownEditorSelection): Promise<void> {
  const current = preview.value;
  if (!quoteInNewSession || quotingSelection.value || !current || !selection.ranges.length) return;
  const path = normalizeSelectionFilePath(current.path);
  quotingSelection.value = true;
  try {
    await quoteInNewSession(props.workspaceRef ? { ...props.workspaceRef } : null, {
      path,
      name: current.name,
      source: "file",
      content: fileSelectionReference(path, selection, {
        draftLabel: dirty.value ? t("editor.quoteDraft") : undefined,
      }),
    });
  } catch (cause) {
    useNotificationStore().addNotice("error", `${t("editor.quoteFailed")}: ${normalizeAppError(cause).message}`);
  } finally {
    quotingSelection.value = false;
  }
}

async function renameDocument(): Promise<void> {
  if (!canRename.value || renaming.value || !preview.value) return;
  const stem = titleDraft.value.trim().replace(/\.(?:md|markdown)$/iu, "");
  const extension = preview.value.name.match(/\.(?:md|markdown)$/iu)?.[0] ?? ".md";
  const name = `${stem}${extension}`;
  if (!stem || !validResourceName(stem) || !validResourceName(name)) {
    titleError.value = t(stem ? "development.invalidFileName" : "knowledge.preview.titleRequired");
    return;
  }
  titleError.value = "";
  if (name === preview.value.name) { titleDraft.value = documentTitle.value; return; }
  const projectId = props.projectId!;
  const workspaceRef = props.workspaceRef ? { ...props.workspaceRef } : null;
  const sourcePath = filePath.value;
  const sourceKey = resourceKey.value;
  const rename = {
    scope: scopeKey.value,
    paths: new Set([sourcePath, preview.value.path].flatMap((path) => [
      normalizePath(path), normalizePath(path.replace(/[^/\\]+$/, () => name)),
    ])),
    nextPath: undefined as string | undefined,
  };
  localRename = rename;
  renameRevision += 1;
  let renamed = false;
  try {
    renamed = await fileSession.relocate(async (current) => {
      const nextPath = await explorerFileAction(projectId, sourcePath, name, workspaceRef);
      if (!nextPath) throw new Error(t("development.invalidFileName"));
      previewCache?.delete(sourceKey);
      // A navigation or workspace switch may have happened while IPC was pending.
      if (localRename !== rename || scopeKey.value !== rename.scope) return current;
      // The command returns an absolute display path, while scoped file APIs
      // accept only workspace-relative paths (including symlinked workspaces).
      const nextFilePath = workspaceRef ? sourcePath.replace(/[^/\\]+$/, () => name) : nextPath;
      rename.nextPath = nextFilePath;
      rename.paths.add(normalizePath(nextPath));
      filePath.value = nextFilePath;
      emit("pathChange", nextFilePath);
      await nextTick();
      return {
        ...current,
        preview: {
          ...current.preview, path: nextPath, name,
          workspaceRelativePath: current.preview.workspaceRelativePath?.replace(/[^/\\]+$/, () => name),
        },
      };
    });
  } catch (cause) {
    if (localRename === rename) titleError.value = normalizeAppError(cause).message;
  }
  if (localRename !== rename) return;
  if (renamed) titleDraft.value = stem;
  else localRename = null;
}

function onTitleKeydown(event: KeyboardEvent): void {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === "Enter") { event.preventDefault(); void renameDocument(); }
  else if (event.key === "Escape") {
    event.preventDefault();
    titleDraft.value = documentTitle.value;
    titleError.value = "";
    (event.target as HTMLInputElement).blur();
  }
}
const {
  documentOutlineItems,
  activeOutlineId,
  documentOutlineMarginTop,
  documentOutlineMaxHeight,
  outlineItemPadding,
  scrollToDocumentOutlineItem,
  scheduleDocumentOutlineActiveUpdate,
} = useMarkdownDocumentOutline({
  documentKey: () => preview.value ? editorContentKey.value : "",
  source: () => isMarkdownDocument.value
    ? fileSession.draft.value.text
    : "",
  active: () => props.active !== false,
  scroller: documentScrollerRef,
  page: documentPageRef,
  body: documentBodyRef,
  bodyStart: () => sourceEditor.value?.getBodyElement?.() ?? null,
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
    ? await workspaceFilePreview(filePath.value, props.workspaceRef)
    : await projectExplorerPreviewFile(props.projectId ?? "", filePath.value);
}

async function readDocument(): Promise<FileDocument> {
  // Only the initial load may use a snapshot. Refreshes always read from disk.
  const cached = cachedPreview;
  cachedPreview = undefined;
  if (cached) return { preview: cached, assetPayload: null };
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

async function loadPreview(options: { keepCurrent?: boolean; keepDraft?: boolean; mergeDraft?: boolean } = {}): Promise<boolean> {
  if (renaming.value && options.keepCurrent) return false;
  const key = resourceKey.value;
  const requestRenameRevision = renameRevision;
  const before = fileSession.document.value;
  const loaded = await fileSession.load(options);
  if (key !== resourceKey.value || requestRenameRevision !== renameRevision) return false;
  if (loaded) {
    diskChanged.value = false;
    observedDiskRevision.value = null;
    void applyPendingPosition();
  } else if (options.keepCurrent && fileSession.document.value === before && dirty.value) {
    diskChanged.value = true;
  }
  return loaded;
}

function onBlobLoaded(): void {
  blobRetried = false;
}

/** The leased bytes were evicted or expired: read the file again for a new blob. */
async function onBlobFetchError(): Promise<void> {
  if (blobRetried || disposed) return;
  blobRetried = true;
  await loadPreview({ keepCurrent: true });
}

async function probeFileRevision(): Promise<ProjectExplorerFileRevision> {
  return props.workspaceRef
    ? await workspaceFileRevision(filePath.value, props.workspaceRef)
    : await projectExplorerFileRevision(props.projectId ?? "", filePath.value);
}

// A probe that overlaps our own write cannot tell it apart from an external
// change. Its result is checked again once the save settles, so autosave never
// flashes the disk conflict bar.
let probeOverlappedSave = false;
let recheckAfterSave = false;

const { checkNow: refreshIfChanged, scheduleCheck: scheduleFileCheck } = useFileChangeRevalidation({
  active: () => props.active !== false && !isCsv.value && !renaming.value,
  currentRevision: () => observedDiskRevision.value ?? preview.value?.revision ?? null,
  probe: async () => {
    const document = fileSession.document.value;
    const revision = await probeFileRevision();
    probeOverlappedSave = saving.value || fileSession.document.value !== document;
    return revision;
  },
  workspaceRef: () => props.workspaceRef,
  workspacePath: () => props.workspaceRef ? filePath.value : null,
  onChanged: async (revision) => {
    previewCache?.delete(resourceKey.value);
    if (probeOverlappedSave) {
      if (saving.value) recheckAfterSave = true;
      else scheduleFileCheck("event");
      return;
    }
    observedDiskRevision.value = revision;
    await loadPreview({ keepCurrent: true, mergeDraft: true });
  },
});

async function useDiskVersion(): Promise<void> {
  await loadPreview({ keepCurrent: true });
}

async function keepLocalVersion(): Promise<void> {
  // Only advance the disk baseline. Replacing the unchanged editor contents
  // would add a spurious undo step and discard secondary selections.
  await loadPreview({ keepCurrent: true, keepDraft: true });
}

async function applyPendingPosition(): Promise<boolean> {
  const position = pendingPosition.value;
  if (!position || preview.value?.kind !== "text") return false;
  await nextTick();
  const view = sourceEditor.value?.getEditorView();
  if (!view) return false;
  sourceEditor.value?.cancelReadingRestore?.();
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
  fileSession.updateDraft({ ...fileSession.draft.value, text: change.doc });
}

// Each edit installs a new draft object, so an unchanged draft reuses its text.
let serializedDraft: { draft: FileDocumentDraft; text: string } | null = null;
function serializedEditorText(): string {
  const draft = fileSession.draft.value;
  if (serializedDraft?.draft !== draft) {
    serializedDraft = { draft, text: serializeTextDocumentLineEndings(draft.text.toString(), draft.lineEnding) };
  }
  return serializedDraft.text;
}

async function saveFile(): Promise<boolean> {
  if (isCsv.value) return await csvEditor.value?.saveFile() ?? false;
  if (renaming.value) return false;
  if (titleDirty.value) {
    await renameDocument();
    if (titleDirty.value) return false;
  }
  return fileSession.save();
}

function exportTransferSnapshot(options: EditorSnapshotOptions = {}): WorkbenchEditorTransferSnapshot {
  if (isCsv.value && csvEditor.value) return csvEditor.value.exportTransferSnapshot(options);
  if (!preview.value || loading.value) return { kind: "resource" };
  const view = sourceEditor.value?.getEditorView();
  return {
    kind: "workspaceFile",
    dirty: dirty.value,
    conflicted: diskChanged.value,
    // Saved work state restores a saved file from disk; only an unsaved buffer is kept.
    ...(!options.workState || dirty.value || diskChanged.value ? { text: serializedEditorText() } : {}),
    contentHash: preview.value?.contentHash ?? "",
    originalLineEnding: fileSession.draft.value.lineEnding,
    selection: view ? {
      anchor: view.state.selection.main.anchor,
      head: view.state.selection.main.head,
    } : null,
    scrollTop: documentScrollerRef.value?.scrollTop ?? view?.scrollDOM.scrollTop ?? null,
    scrollAnchor: readingTarget.value ? sourceEditor.value?.lastReadingAnchor?.() ?? null : null,
  };
}

async function applyTransferSnapshot(snapshot: WorkbenchEditorTransferSnapshot, recovering = false): Promise<boolean> {
  if (isCsv.value) {
    await nextTick();
    return await csvEditor.value?.applyTransferSnapshot(snapshot, recovering) ?? false;
  }
  if (snapshot.kind !== "workspaceFile") return false;
  const deadline = Date.now() + 4_000;
  while (!sourceEditor.value?.getEditorView() && Date.now() < deadline) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 16));
  }
  const view = sourceEditor.value?.getEditorView();
  if (!view || preview.value?.kind !== "text" || !preview.value.editable) return false;
  const changedOnDisk = !!snapshot.contentHash && preview.value.contentHash !== snapshot.contentHash;
  // Without a buffer the snapshot was saved, so the loaded disk version is current.
  const buffer = snapshot.text;
  if (changedOnDisk && !recovering && buffer !== undefined) {
    error.value = t("development.editor.transferConflict");
    return false;
  }
  sourceEditor.value?.cancelReadingRestore?.();
  if (recovering && (snapshot.conflicted || (changedOnDisk && snapshot.dirty))) diskChanged.value = true;
  const replace = buffer !== undefined && !(recovering && changedOnDisk && !snapshot.dirty);
  const normalized = replace ? normalizeTextDocumentLineEndings(buffer) : "";
  if (replace) {
    fileSession.updateDraft({ text: markdownEditorTextFromString(normalized), lineEnding: snapshot.originalLineEnding });
  }
  const length = replace ? normalized.length : view.state.doc.length;
  const selection = snapshot.selection
    ? {
        anchor: Math.min(length, Math.max(0, snapshot.selection.anchor)),
        head: Math.min(length, Math.max(0, snapshot.selection.head)),
      }
    : undefined;
  if (replace) {
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: normalized },
      selection,
    });
  } else if (selection) {
    view.dispatch({ selection });
  }
  await nextTick();
  if (snapshot.scrollAnchor && readingTarget.value) {
    sourceEditor.value?.restoreReadingAnchor?.(snapshot.scrollAnchor);
  } else if (snapshot.scrollTop != null) {
    const scroller = documentScrollerRef.value ?? view.scrollDOM;
    scroller.scrollTop = snapshot.scrollTop;
  }
  return true;
}

watch(() => dirty.value || titleDirty.value || renaming.value, (value) => emit("dirtyChange", value));
watch(saving, (value, previous) => {
  if (value || !previous) return;
  const recheck = recheckAfterSave;
  recheckAfterSave = false;
  if (diskChanged.value) void loadPreview({ keepCurrent: true, mergeDraft: true });
  else if (recheck) scheduleFileCheck("event");
});
watch(fileSession.document, (document) => {
  if (document) previewCache?.set(resourceKey.value, document.preview);
  if (renaming.value) return;
  diskChanged.value = false;
  observedDiskRevision.value = null;
}, { flush: "sync" });
watch([scopeKey, () => props.path], ([scope, path]) => {
  if (localRename?.scope === scope && localRename.paths.has(normalizePath(path))) {
    // Backend events can reach the parent before the rename command resolves.
    // The command owns this transition; never reload the removed source path.
    if (!renaming.value && localRename.nextPath && normalizePath(path) === normalizePath(localRename.nextPath)) {
      localRename = null;
    }
    return;
  }
  localRename = null;
  filePath.value = path;
}, { flush: "sync" });
watch(resourceKey, async (key) => {
  if (renaming.value && localRename?.scope === scopeKey.value) return;
  blobRetried = false;
  diskChanged.value = false;
  observedDiskRevision.value = null;
  titleError.value = "";
  editorContentKey.value = resourceKey.value;
  if (isCsv.value) return;
  cachedPreview = previewCache?.get(key);
  const revalidate = !!cachedPreview;
  const loaded = await loadPreview();
  // Let the document and CodeMirror mount while the previous surface is still
  // visible. Failed loads also settle presentation, so navigation never hangs.
  await nextTick();
  if (disposed || key !== resourceKey.value) return;
  emit("ready");
  if (revalidate && loaded) void refreshIfChanged();
}, { immediate: true });
watch(() => props.active, (active) => {
  if (active) void applyPendingPosition();
});

defineExpose({
  isRenaming: () => renaming.value,
  /** Code, Markdown, CSV and other text; false for media, models and Unity previews. */
  isTextContent: () => isCsv.value || preview.value?.kind === "text",
  discardChanges: () => csvEditor.value?.discardChanges(),
  saveFile,
  revealPosition,
  revealToolFileHighlight,
  refreshIfChanged: () => isCsv.value ? csvEditor.value?.refreshIfChanged() : renaming.value ? Promise.resolve() : refreshIfChanged(),
  exportTransferSnapshot,
  applyTransferSnapshot,
  restoreWorkState: (snapshot: WorkbenchEditorTransferSnapshot) => applyTransferSnapshot(snapshot, true),
});
</script>

<template>
  <WorkspaceCsvEditor v-if="isCsv" ref="csvEditor" :path="path" :project-id="projectId" :workspace-ref="workspaceRef" :active="active" @ready="emit('ready')" @dirty-change="emit('dirtyChange', $event)" @path-change="emit('pathChange', $event)" />
  <section v-else class="workspace-file-preview" data-search-surface>
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
      <div v-if="isLatexDocument" class="workspace-latex-view-bar">
        <BaseSegmented
          :model-value="latexView"
          size="sm"
          :options="latexViewOptions"
          @update:model-value="selectLatexView"
        />
      </div>
      <div class="workspace-file-preview-body">
        <LatexPreviewPane
          v-if="isLatexDocument && latexView === 'preview'"
          :workspace-ref="workspaceRef ?? null"
          :path="preview.path"
          :active="active !== false"
          :auto-fallback="!latexViewPinned"
          @unavailable="onLatexUnavailable"
        />
        <div
          v-else-if="isMarkdownDocument"
          ref="documentScrollerRef"
          class="document-scroller"
          @scroll.passive="scheduleDocumentOutlineActiveUpdate"
        >
          <div class="document-workspace" :class="{ 'has-outline': documentOutlineItems.length > 0, 'has-comments': documentComments.hasRailContent.value }">
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
                <span v-if="canRename" class="document-title-input-shell" :data-value="titleDraft || ' '">
                  <input
                    v-model="titleDraft"
                    class="document-title-input"
                    type="text"
                    :readonly="renaming"
                    :placeholder="t('knowledge.preview.titlePlaceholder')"
                    :aria-label="t('knowledge.preview.titleLabel')"
                    :aria-invalid="!!titleError"
                    @input="titleError = ''"
                    @blur="renameDocument"
                    @keydown="onTitleKeydown"
                  />
                </span>
                <h1 v-else class="document-title">{{ documentTitle }}</h1>
                <div v-if="titleError" class="document-title-error" role="alert">{{ titleError }}</div>
              </header>
              <section ref="documentBodyRef" class="document-body document-body-with-properties">
                <BaseMarkdownEditor
                  ref="sourceEditor"
                  :model-value="normalizedSourceText"
                  :saved-value="savedSourceText"
                  :content-key="editorContentKey"
                  :content-path="preview.path"
                  show-properties
                  :workspace-ref="workspaceRef"
                  :active="active !== false"
                  :view-mode="editorViewMode"
                  :comments="documentComments"
                  :reading-position="readingTarget"
                  auto-grow
                  :min-height="360"
                  transaction-model
                  :can-quote-selection-in-new-session="!!quoteInNewSession && !quotingSelection"
                  @quote-selection-in-new-session="onQuoteSelectionInNewSession"
                  @selection-change="onEditorSelectionChange"
                  @document-change="onEditorDocumentChange"
                  @shortcut-save="saveFile"
                  @external-change="!renaming && refreshIfChanged()"
                />
              </section>
            </article>
            <MarkdownCommentRail :controller="documentComments" />
          </div>
        </div>
        <BaseMarkdownEditor
          v-else-if="preview.kind === 'text' && preview.editable"
          ref="sourceEditor"
          :model-value="normalizedSourceText"
          :saved-value="savedSourceText"
          :content-key="editorContentKey"
          :content-path="preview.path"
          show-properties
          :workspace-ref="workspaceRef"
          :active="active !== false"
          :view-mode="editorViewMode"
          transaction-model
          @document-change="onEditorDocumentChange"
          @shortcut-save="saveFile"
          @external-change="refreshIfChanged()"
        />
        <AssetTextViewer
          v-else-if="preview.kind === 'text'"
          :snippet="preview.text || ''"
          :truncated="preview.truncated"
          :total-lines="preview.totalLines || 1"
          :language="language"
        />
        <div v-else-if="preview.kind === 'image' && preview.blobUrl" class="workspace-media-preview image">
          <img :src="preview.blobUrl" :alt="preview.name" @load="onBlobLoaded" @error="onBlobFetchError" />
        </div>
        <iframe
          v-else-if="preview.kind === 'pdf' && preview.blobUrl"
          class="workspace-pdf-preview"
          :src="preview.blobUrl"
          :title="preview.name"
        />
        <div v-else-if="preview.kind === 'audio' && preview.blobUrl" class="workspace-media-preview">
          <audio :src="preview.blobUrl" controls @loadeddata="onBlobLoaded" @error="onBlobFetchError" />
        </div>
        <div v-else-if="preview.kind === 'video' && preview.blobUrl" class="workspace-media-preview video">
          <video :src="preview.blobUrl" controls @loadeddata="onBlobLoaded" @error="onBlobFetchError" />
        </div>
        <ModelViewer
          v-else-if="preview.kind === 'model' && preview.blobUrl"
          :url="preview.blobUrl"
          :format="modelFormatFromPath(preview.name)"
          :active="active !== false"
          @loaded="onBlobLoaded"
          @fetch-error="onBlobFetchError"
        />
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

.workspace-latex-view-bar {
  display: flex;
  align-items: center;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border-color);
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
