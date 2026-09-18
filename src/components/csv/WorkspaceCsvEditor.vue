<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { t } from "../../i18n";
import { useDocumentFileSession } from "../../composables/useDocumentFileSession";
import { useFileChangeRevalidation } from "../../composables/useFileChangeRevalidation";
import { documentSessionKey } from "../../document/documentIdentity";
import { resolveToolFilePreviewHighlightRanges, type ToolFilePreviewHighlight } from "../../services/toolFilePreviewWindow";
import { normalizeAppError } from "../../services/errors";
import type { WorkspaceRef } from "../../services/project";
import { projectExplorerPreviewFile, projectExplorerWriteFile, workspaceFilePreview, workspaceFileWrite,
  projectExplorerFileRevision, workspaceFileRevision } from "../../services/workspaceExplorer";
import { readCsvViewFile, writeCsvViewFile, relocateCsvFile, type CsvViewFile } from "../../services/csvDocument";
import type { ProjectExplorerFilePreview, ProjectExplorerFileRevision, WorkbenchEditorTransferSnapshot } from "../../types/workbench";
import { applyCsvCellEdits, changeCsvColumn, deleteCsvRows, insertCsvRow, parseCsvDocument, serializeCsvDocument,
  type CsvCellEdit, type CsvDocument } from "../../document/csv/csvDocument";
import { defaultCsvView, parseCsvView, reconcileCsvView, serializeCsvView, type CsvView } from "../../document/csv/csvView";
import { applyCsvNormalizedTextEdit } from "../../document/csv/csvSourceEditing";
import { csvColumnLabel } from "../../document/csv/csvGridProjection";
import { adjustCsvRowDimensions, adjustCsvStyleRows, reconcileCsvStyles } from "../../document/csv/csvStyles";
import { adjustCsvMerges, mergeCsvSelection, unmergeCsvSelection } from "../../document/csv/csvMerges";
import { normalizeTextDocumentLineEndings } from "../../document/textDocumentFormat";
import { csvEditorSessions } from "./csvEditorSessions";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import type { MarkdownEditorDocumentChange } from "../ui/markdown-editor/markdownEditorDocumentChange";
import CsvGrid, { type CsvGridSnapshot } from "./CsvGrid.vue";

const props = withDefaults(defineProps<{ path: string; projectId?: string; workspaceRef?: WorkspaceRef | null;
  active?: boolean; readOnly?: boolean; content?: string; fileActions?: boolean }>(), { active: true });
const emit = defineEmits<{ dirtyChange: [dirty: boolean]; pathChange: [path: string]; ready: [] }>();
const key = computed(() => documentSessionKey(props.workspaceRef, ["csv", props.projectId ?? "", props.path]));
const scope = () => ({ filePath: props.path, projectId: props.projectId, workspaceRef: props.workspaceRef });
const view = shallowRef<CsvView>(defaultCsvView());
const viewFile = shallowRef<CsvViewFile>({});
const viewBaseline = ref("");
const viewError = ref("");
const viewWriteError = ref("");
const parseError = ref("");
const actionError = ref("");
const diskChanged = ref(false);
const viewChanged = ref(false);
const viewSaving = ref(false);
const parsing = ref(false);
const mode = ref("table");
const parsed = shallowRef<CsvDocument | null>(null);
const editorRoot = ref<HTMLElement | null>(null);
const grid = ref<InstanceType<typeof CsvGrid> | null>(null);
const source = ref<InstanceType<typeof BaseMarkdownEditor> | null>(null);
const selection = ref({ row: 0, column: 0 });
const zoom = ref(1);
const contextMenu = ref<{ x: number; y: number } | null>(null);
let gridSnapshot: CsvGridSnapshot | undefined;
const search = ref("");
const replacement = ref("");
const showFind = ref(false);
const settingsOpen = ref(false);
const headerDraft = ref("");
const targetPath = ref("");
const editingView = ref(false);
const viewSource = ref("");
const history = shallowRef<Array<{ text: string; view: CsvView }>>([]);
const historyIndex = ref(-1);
let worker: Worker | null = null;
let parseId = 0;
let viewTimer: ReturnType<typeof setTimeout> | null = null;
let disposed = false;
let loadingEpoch = 0;
let sourceHistoryTimer: ReturnType<typeof setTimeout> | null = null;
let discardOnUnmount = false;
let pendingEditingRefresh = false;
let viewEditRevision = 0;
let pendingLoadingRefresh = false;
let pendingSavingRefresh = false;

const session = useDocumentFileSession<ProjectExplorerFilePreview, string>({
  key: () => key.value, emptyDraft: () => "",
  read: async () => {
    if (props.readOnly && props.content !== undefined) return {
      path: props.path, name: props.path.split(/[\\/]/).pop() ?? props.path, extension: "csv",
      size: props.content.length, kind: "text", text: props.content, mimeType: "text/csv", editable: false,
      truncated: false, revision: { exists: true, size: props.content.length, key: props.content, modifiedAtNanos: "0" },
    };
    return props.workspaceRef ? workspaceFilePreview(props.path, props.workspaceRef)
      : projectExplorerPreviewFile(props.projectId ?? "", props.path);
  },
  write: (document, text) => props.workspaceRef
    ? workspaceFileWrite(props.path, text, document.contentHash!, props.workspaceRef)
    : projectExplorerWriteFile(props.projectId ?? "", props.path, text, document.contentHash!),
  toDraft: (document) => document.text ?? "", equals: (left, right) => left === right,
  canSave: (document) => document.editable && !!document.contentHash && !props.readOnly,
  errorMessage: (error) => normalizeAppError(error).message,
});
const readOnly = computed(() => props.readOnly || !session.document.value?.editable);
const viewText = computed(() => serializeCsvView(view.value));
const viewDirty = computed(() => !!viewBaseline.value && viewText.value !== viewBaseline.value);
const dirty = computed(() => session.dirty.value || viewDirty.value);
const selectedColumn = computed(() => view.value.columnOrder.find((id) => view.value.columns[id]!.sourceIndex === selection.value.column));
const sourceModel = computed(() => normalizeTextDocumentLineEndings(session.modelDraft.value));

function message(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.startsWith("csv.") ? t(text) : normalizeAppError(error).message;
}

function parsedResult(document: CsvDocument): void {
  const onlyDefaultView = !!viewBaseline.value && !viewDirty.value && viewFile.value.contentHash == null && viewFile.value.text == null;
  parsed.value = document;
  view.value = reconcileCsvView(view.value, document);
  if (onlyDefaultView) viewBaseline.value = serializeCsvView(view.value);
  parseError.value = "";
  parsing.value = false;
}
function parseText(text = session.draft.value): void {
  const id = ++parseId;
  parseError.value = "";
  if (text.length < 128 * 1024 || typeof Worker === "undefined") {
    try { parsedResult(parseCsvDocument(text)); }
    catch (error) { parsed.value = null; parseError.value = message(error); parsing.value = false; mode.value = "source"; }
    return;
  }
  parsing.value = true;
  if (!worker) {
    worker = new Worker(new URL("../../document/csv/csvParse.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (event: MessageEvent<{ id: number; document?: CsvDocument; error?: string }>) => {
      if (event.data.id !== parseId || disposed) return;
      if (event.data.document) parsedResult(event.data.document);
      else { parsed.value = null; parsing.value = false; parseError.value = t(event.data.error ?? "csv.invalidCsv"); mode.value = "source"; }
    };
    worker.onerror = () => { parsing.value = false; parseError.value = t("csv.parseFailed"); mode.value = "source"; };
  }
  worker.postMessage({ id, source: text });
}

function remember(): void {
  const entry = { text: session.draft.value, view: view.value };
  const previous = history.value[historyIndex.value];
  if (previous?.text === entry.text && serializeCsvView({ ...previous.view, schema: "locus.csv-view.v3" })
    === serializeCsvView({ ...entry.view, schema: "locus.csv-view.v3" })) return;
  const entries = history.value.slice(0, historyIndex.value + 1);
  entries.push(entry);
  while (entries.length > 1 && (entries.length > 80 || entries.reduce((sum, item) => sum + item.text.length * 2, 0) > 48 * 1024 * 1024)) entries.shift();
  history.value = entries;
  historyIndex.value = entries.length - 1;
}
function commit(text: string, nextView = view.value, nextDocument?: CsvDocument): void {
  if (readOnly.value || text === session.draft.value && serializeCsvView(nextView) === viewText.value) return;
  try {
    if (nextView !== view.value) parseCsvView(serializeCsvView(nextView));
    const document = nextDocument ?? parseCsvDocument(text);
    session.updateDraft(text);
    session.modelDraft.value = text;
    view.value = nextView;
    viewEditRevision++;
    parsedResult(document);
    remember();
    actionError.value = "";
  } catch (error) { actionError.value = message(error); }
}
function edit(edits: CsvCellEdit[]): void {
  if (!parsed.value || parsing.value) return;
  try {
    const result = applyCsvCellEdits(parsed.value, session.draft.value, edits);
    commit(result.text, view.value, result.document);
  } catch (error) { actionError.value = message(error); }
}
function undo(redo = false): void {
  if (readOnly.value) return;
  remember();
  const index = historyIndex.value + (redo ? 1 : -1);
  const entry = history.value[index];
  if (!entry) return;
  historyIndex.value = index;
  session.updateDraft(entry.text); session.modelDraft.value = entry.text;
  view.value = { ...entry.view, schema: entry.view.schema > view.value.schema ? entry.view.schema : view.value.schema };
  viewEditRevision++;
  parseText();
}
function sourceChange(change: MarkdownEditorDocumentChange): void {
  if (readOnly.value) return;
  const raw = session.draft.value;
  const next = applyCsvNormalizedTextEdit(raw, change.doc.toString(), parsed.value?.newline ?? "\n");
  session.updateDraft(next);
  parseText(next);
  if (sourceHistoryTimer) clearTimeout(sourceHistoryTimer);
  sourceHistoryTimer = setTimeout(remember, 400);
}

function updateView(next: CsvView): void {
  if (readOnly.value || viewError.value || viewChanged.value) return;
  if (next.headerRows !== view.value.headerRows && parsed.value) {
    next = { ...next, columns: Object.fromEntries(Object.entries(next.columns).map(([id, column]) =>
      [id, { ...column, header: next.headerRows ? parsed.value!.records[0]?.fields[column.sourceIndex]?.value ?? "" : "" }])) };
  }
  view.value = next;
  viewEditRevision++;
  remember();
}
async function reloadView(prefetched?: CsvViewFile): Promise<void> {
  const requestKey = key.value;
  const epoch = loadingEpoch;
  try {
    const next = prefetched ?? (props.readOnly && props.content !== undefined ? {} : await readCsvViewFile(scope()));
    if (requestKey !== key.value || epoch !== loadingEpoch || disposed) return;
    viewFile.value = next;
    const config = next.text == null ? defaultCsvView() : parseCsvView(next.text);
    if (next.text == null && parsed.value?.records.length === 0) config.headerRows = 0;
    view.value = parsed.value ? reconcileCsvView(config, parsed.value) : config;
    viewBaseline.value = serializeCsvView(view.value);
    viewError.value = ""; viewWriteError.value = ""; viewChanged.value = false;
  } catch (error) { if (requestKey === key.value && epoch === loadingEpoch) viewError.value = message(error); }
}
function editViewSource(): void {
  viewSource.value = viewFile.value.text ?? viewText.value;
  editingView.value = true;
}
async function saveViewSource(): Promise<void> {
  if (readOnly.value || session.dirty.value || !session.document.value?.contentHash) return;
  const requestKey = key.value;
  const epoch = loadingEpoch;
  try {
    const config = parseCsvView(viewSource.value);
    const next = await writeCsvViewFile(scope(), viewSource.value, viewFile.value.contentHash ?? null, session.document.value.contentHash);
    if (requestKey !== key.value || epoch !== loadingEpoch || disposed) return;
    viewFile.value = next;
    view.value = parsed.value ? reconcileCsvView(config, parsed.value) : config;
    viewBaseline.value = viewText.value;
    viewError.value = ""; viewWriteError.value = ""; viewChanged.value = false; editingView.value = false;
  } catch (error) { if (requestKey === key.value && epoch === loadingEpoch && !disposed) viewError.value = message(error); }
}
async function saveView(): Promise<boolean> {
  if (!viewDirty.value) return true;
  if (readOnly.value || session.dirty.value || diskChanged.value || viewError.value || viewChanged.value || viewSaving.value
    || !session.document.value?.contentHash) return false;
  const requestKey = key.value;
  const epoch = loadingEpoch;
  const text = viewText.value;
  viewSaving.value = true;
  try {
    const next = await writeCsvViewFile(scope(), text, viewFile.value.contentHash ?? null, session.document.value.contentHash);
    if (requestKey !== key.value || epoch !== loadingEpoch || disposed) return false;
    viewFile.value = next; viewBaseline.value = text;
    viewWriteError.value = "";
    return viewText.value === text;
  } catch (error) { if (requestKey === key.value && epoch === loadingEpoch && !disposed) viewWriteError.value = message(error); return false; }
  finally { if (requestKey === key.value && epoch === loadingEpoch) viewSaving.value = false; }
}
async function saveFile(): Promise<boolean> {
  const active = document.activeElement;
  const finishCell = active instanceof HTMLElement && active.classList.contains("csv-cell-input") && editorRoot.value?.contains(active);
  if (finishCell) active.blur();
  await nextTick();
  if (finishCell) grid.value?.focus();
  if (readOnly.value || parseError.value || parsing.value || diskChanged.value) return false;
  const requestKey = key.value;
  if (session.dirty.value && !await session.save()) return false;
  if (requestKey !== key.value) return false;
  return saveView();
}
async function relocate(copy: boolean): Promise<void> {
  if (!props.workspaceRef || readOnly.value || !targetPath.value.trim() || !await saveFile()) return;
  const requestKey = key.value;
  try {
    const next = await relocateCsvFile(scope(), targetPath.value.trim(), copy, session.document.value!.contentHash!);
    if (requestKey === key.value) { csvEditorSessions.delete(requestKey); emit("pathChange", next); settingsOpen.value = false; }
  } catch (error) { actionError.value = message(error); }
}

function capture(cacheKey = key.value): void {
  if (!session.document.value) return;
  csvEditorSessions.set(cacheKey, { text: session.draft.value, hash: session.document.value.contentHash ?? "",
    view: view.value, viewBaseline: viewBaseline.value, viewHash: viewFile.value.contentHash ?? null,
    dirty: dirty.value || diskChanged.value, mode: mode.value, grid: grid.value?.getSnapshot(),
    history: history.value, historyIndex: historyIndex.value });
}
async function load(keepLocal = false, useDisk = false): Promise<void> {
  const epoch = ++loadingEpoch;
  const cached = keepLocal ? { text: session.draft.value, view: view.value } : undefined;
  const saved = !useDisk && !keepLocal ? csvEditorSessions.get(key.value) : undefined;
  ++parseId;
  parseError.value = ""; viewError.value = ""; viewWriteError.value = ""; actionError.value = ""; viewChanged.value = false;
  viewSaving.value = false;
  if (!keepLocal && !useDisk) { parsed.value = null; viewFile.value = {}; editingView.value = false; settingsOpen.value = false; }
  viewBaseline.value = ""; view.value = defaultCsvView();
  if (!await session.load({ keepCurrent: keepLocal || useDisk, keepDraft: keepLocal }) || epoch !== loadingEpoch) return;
  if (!session.document.value?.text && session.document.value?.kind !== "text") {
    parseError.value = t("csv.encodingUnsupported"); return;
  }
  const retained = cached ?? (saved?.dirty ? saved : undefined);
  if (retained) { session.updateDraft(retained.text); session.modelDraft.value = retained.text; }
  parseText();
  while (parsing.value && epoch === loadingEpoch && !disposed) await new Promise((resolve) => setTimeout(resolve, 16));
  if (epoch !== loadingEpoch || disposed) return;
  await reloadView();
  if (epoch !== loadingEpoch) return;
  if (retained) view.value = retained.view;
  if (saved?.dirty) { viewBaseline.value = saved.viewBaseline; viewFile.value = { contentHash: saved.viewHash }; }
  diskChanged.value = !!saved?.dirty && saved.hash !== session.document.value?.contentHash;
  mode.value = saved?.mode ?? (parseError.value ? "source" : "table");
  const reuseHistory = saved && (saved.dirty || saved.hash === session.document.value?.contentHash);
  history.value = reuseHistory ? saved.history : [{ text: session.draft.value, view: view.value }];
  historyIndex.value = reuseHistory ? saved.historyIndex : 0;
  if (saved?.grid) { await nextTick(); await grid.value?.applySnapshot(saved.grid); }
  if (pendingLoadingRefresh) { pendingLoadingRefresh = false; scheduleRefresh("event"); }
}

function hasUncommittedInput(): boolean {
  return !!editorRoot.value?.querySelector(".csv-cell-input") || editingView.value;
}
function resumeExternalRefresh(): void {
  if (!pendingEditingRefresh) return;
  // Cell commits happen on blur; recheck after their draft changes are applied.
  queueMicrotask(() => {
    if (disposed || hasUncommittedInput()) return;
    pendingEditingRefresh = false;
    scheduleRefresh("event");
  });
}
function combinedRevision(revision: ProjectExplorerFileRevision, file: CsvViewFile): ProjectExplorerFileRevision {
  return { ...revision, key: JSON.stringify([revision.key, file.contentHash ?? null]) };
}
let probedFiles: { key: string; epoch: number; revision: ProjectExplorerFileRevision; view: CsvViewFile } | null = null;

async function applyExternalFiles(): Promise<void> {
  const files = probedFiles;
  if (!files || files.key !== key.value || files.epoch !== loadingEpoch) return;
  if (session.loading.value || !viewBaseline.value) { pendingLoadingRefresh = true; return; }
  const csvChanged = files.revision.key !== session.document.value?.revision.key;
  const sidecarChanged = (files.view.contentHash ?? null) !== (viewFile.value.contentHash ?? null);
  if (hasUncommittedInput()) { pendingEditingRefresh = true; return; }
  if (session.saving.value || viewSaving.value) { pendingSavingRefresh = true; return; }
  if (dirty.value) {
    if (csvChanged) diskChanged.value = true;
    if (sidecarChanged) viewChanged.value = true;
    return;
  }
  if (csvChanged) {
    const before = session.document.value;
    const previousView = view.value;
    const previousViewEdit = viewEditRevision;
    const loaded = await session.load({ keepCurrent: true,
      canApply: () => !hasUncommittedInput() && !dirty.value && !viewSaving.value && view.value === previousView });
    if (files.key !== key.value || files.epoch !== loadingEpoch || disposed) return;
    if (!loaded) {
      if (hasUncommittedInput()) pendingEditingRefresh = true;
      else if (dirty.value) diskChanged.value = true;
      return;
    }
    diskChanged.value = false;
    // Metadata-only notifications and our own save echoes need no parse/render.
    if (before?.contentHash !== session.document.value?.contentHash) {
      parseText();
      while (parsing.value && files.epoch === loadingEpoch && !disposed) await new Promise((resolve) => setTimeout(resolve, 16));
      if (files.key !== key.value || files.epoch !== loadingEpoch || disposed) return;
      if (!session.dirty.value) {
        history.value = [{ text: session.draft.value, view: view.value }]; historyIndex.value = 0;
      }
    }
    // Reconciled column identities/layout are a disk baseline, not a local edit.
    if (!session.dirty.value && previousViewEdit === viewEditRevision) viewBaseline.value = viewText.value;
  }
  if (sidecarChanged) {
    if (session.dirty.value || viewDirty.value || hasUncommittedInput()) viewChanged.value = true;
    else await reloadView(files.view);
  }
}

const { checkNow: refreshIfChanged, scheduleCheck: scheduleRefresh } = useFileChangeRevalidation({
  active: () => props.active !== false && !(props.readOnly && props.content !== undefined),
  currentRevision: () => session.document.value ? combinedRevision(session.document.value.revision, viewFile.value) : null,
  workspaceRef: () => props.workspaceRef, workspacePaths: () => [props.path, `${props.path}.view`],
  debounceMs: 80, maxWaitMs: 240,
  probe: async () => {
    const requestKey = key.value, epoch = loadingEpoch;
    const [revision, nextView] = await Promise.all([
      props.workspaceRef ? workspaceFileRevision(props.path, props.workspaceRef)
        : projectExplorerFileRevision(props.projectId ?? "", props.path),
      readCsvViewFile(scope()),
    ]);
    probedFiles = { key: requestKey, epoch, revision, view: nextView };
    return combinedRevision(revision, nextView);
  },
  onBaseline: () => { pendingLoadingRefresh = true; },
  onChanged: applyExternalFiles,
});

function rowAction(remove: boolean, after = false): void {
  if (!parsed.value) return;
  const selected = grid.value?.selectedRows() ?? [];
  const rows = selected.length ? selected : [selection.value.row];
  const index = Math.min(parsed.value.records.length, after ? rows.reduce((maximum, row) => Math.max(maximum, row), 0) + 1 : rows.reduce((minimum, row) => Math.min(minimum, row), selection.value.row));
  commit(remove ? deleteCsvRows(parsed.value, rows) : insertCsvRow(parsed.value, index),
    { ...view.value, ...(view.value.rowDimensions ? { rowDimensions: adjustCsvRowDimensions(view.value.rowDimensions, remove ? null : index, remove ? rows : []) } : {}),
      ...(view.value.merges ? { merges: adjustCsvMerges(view.value.merges, "rows", remove ? null : index, remove ? rows : []) } : {}),
      ...(view.value.styles ? { styles: adjustCsvStyleRows(view.value.styles, remove ? null : index,
      remove ? rows.filter((row) => row < parsed.value!.records.length) : []) } : {}) });
}
function columnAction(remove: boolean, after = false): void {
  if (!parsed.value) return;
  const selected = grid.value?.selectedColumns() ?? [];
  const indices = selected.length ? selected : [selection.value.column];
  const index = Math.min(parsed.value.columnCount, after ? Math.max(...indices) + 1 : Math.min(...indices));
  const removed = remove ? new Set(indices) : new Set<number>();
  const columns = Object.fromEntries(Object.entries(view.value.columns)
    .filter(([, column]) => !removed.has(column.sourceIndex))
    .map(([id, column]) => [id, { ...column, sourceIndex: remove ? column.sourceIndex - indices.filter((index) => index < column.sourceIndex).length
      : column.sourceIndex >= index ? column.sourceIndex + 1 : column.sourceIndex }]));
  const nextView = { ...view.value, columns, columnOrder: view.value.columnOrder.filter((id) => id in columns),
    ...(view.value.merges ? { merges: adjustCsvMerges(view.value.merges, "columns", remove ? null : index, remove ? indices : []) } : {}),
    sort: view.value.sort?.filter((item) => item.columnId in columns), filters: view.value.filters?.filter((item) => item.columnId in columns),
    ...(view.value.styles ? { styles: reconcileCsvStyles(view.value.styles, columns) } : {}) };
  let document = parsed.value;
  if (remove) for (const column of [...indices].sort((left, right) => right - left)) {
    if (column < document.columnCount) document = parseCsvDocument(changeCsvColumn(document, column, true));
  }
  commit(remove ? serializeCsvDocument(document)
    : !document.records.length ? ",\n" : changeCsvColumn(document, index, false), nextView);
}
function mergeAction(remove = false): void {
  if (readOnly.value || !parsed.value || parsing.value || viewError.value || viewChanged.value || !grid.value) return;
  if (remove) commit(session.draft.value, unmergeCsvSelection(view.value, grid.value.selectedRows(), grid.value.selectedColumns()));
  else {
    const range = grid.value.mergeSelection();
    if (range) commit(session.draft.value, mergeCsvSelection(view.value, range));
  }
}
function openContextMenu(event: MouseEvent): void { contextMenu.value = { x: event.clientX, y: event.clientY }; }
async function menuAction(action: () => unknown): Promise<void> {
  contextMenu.value = null;
  try { await action(); } catch (error) { actionError.value = message(error); }
  await nextTick();
  if (!settingsOpen.value && !showFind.value) grid.value?.focus();
}
function sourceContextMenu(event: MouseEvent): void {
  if (mode.value !== "source") return;
  event.preventDefault(); event.stopPropagation(); openContextMenu(event);
}
function sortColumn(direction: "asc" | "desc"): void {
  if (selectedColumn.value) updateView({ ...view.value, sort: [{ columnId: selectedColumn.value, direction }] });
}
function filterColumn(value: string): void {
  if (!selectedColumn.value) return;
  const filters = (view.value.filters ?? []).filter((item) => item.columnId !== selectedColumn.value);
  if (value) filters.push({ columnId: selectedColumn.value, value });
  updateView({ ...view.value, filters });
}
const menuItems = computed(() => {
  const blocked = readOnly.value || !parsed.value || parsing.value;
  const mergeRange = contextMenu.value ? grid.value?.mergeSelection() : null;
  const mergeRows = contextMenu.value ? grid.value?.selectedRows() ?? [] : [];
  const mergeColumns = contextMenu.value ? grid.value?.selectedColumns() ?? [] : [];
  const canUnmerge = (view.value.merges ?? []).some((merge) => mergeRows.some((row) => row >= merge.rows[0] && row <= merge.rows[1])
    && mergeColumns.some((column) => column >= merge.columns[0] && column <= merge.columns[1]));
  return [
    { label: t("common.save"), shortcut: "Ctrl+S", disabled: readOnly.value || !dirty.value, action: saveFile },
    { label: t("csv.undo"), shortcut: "Ctrl+Z", disabled: readOnly.value || historyIndex.value <= 0, action: () => undo() },
    { label: t("csv.redo"), shortcut: "Ctrl+Y", disabled: readOnly.value || historyIndex.value >= history.value.length - 1, action: () => undo(true) },
    ...(mode.value === "table" ? [
      { separator: true },
      { label: t("editor.cut"), shortcut: "Ctrl+X", disabled: blocked, action: () => grid.value?.clipboardAction("cut") },
      { label: t("editor.copy"), shortcut: "Ctrl+C", action: () => grid.value?.clipboardAction("copy") },
      { label: t("editor.paste"), shortcut: "Ctrl+V", disabled: blocked, action: () => grid.value?.clipboardAction("paste") },
      { label: t("csv.clearCells"), shortcut: "Delete", disabled: blocked, action: () => grid.value?.clearSelection() },
      { separator: true },
      { label: t("csv.mergeCells"), disabled: blocked || !!viewError.value || viewChanged.value || !mergeRange, action: () => mergeAction() },
      { label: t("csv.unmergeCells"), disabled: blocked || !!viewError.value || viewChanged.value || !canUnmerge, action: () => mergeAction(true) },
      { separator: true },
      { label: t("csv.autoFitColumns"), disabled: blocked || !!viewError.value || viewChanged.value, action: () => grid.value?.autoFitColumns() },
      { label: t("csv.autoFitRows"), disabled: blocked || !!viewError.value || viewChanged.value, action: () => grid.value?.autoFitRows() },
      { separator: true },
      { label: t("editor.table.row-before"), disabled: blocked, action: () => rowAction(false) },
      { label: t("editor.table.row-after"), disabled: blocked, action: () => rowAction(false, true) },
      { label: t("csv.deleteRows"), disabled: blocked, action: () => rowAction(true) },
      { label: t("editor.table.column-before"), disabled: blocked, action: () => columnAction(false) },
      { label: t("editor.table.column-after"), disabled: blocked, action: () => columnAction(false, true) },
      { label: t("csv.deleteColumn"), disabled: blocked, action: () => columnAction(true) },
      { separator: true },
      { label: t("csv.sortAsc"), disabled: blocked || !selectedColumn.value, action: () => sortColumn("asc") },
      { label: t("csv.sortDesc"), disabled: blocked || !selectedColumn.value, action: () => sortColumn("desc") },
      { label: t("csv.resetOrder"), disabled: !view.value.sort?.length && !view.value.filters?.length, action: () => updateView({ ...view.value, sort: [], filters: [] }) },
      { label: t("csv.find"), shortcut: "Ctrl+F", action: () => { showFind.value = !showFind.value; } },
    ] : []),
    { separator: true },
    { label: t(mode.value === "table" ? "csv.source" : "csv.table"), disabled: mode.value === "source" && (!parsed.value || parsing.value), action: () => { mode.value = mode.value === "table" ? "source" : "table"; } },
    { label: t("csv.options"), action: () => { settingsOpen.value = !settingsOpen.value; } },
  ];
});
function renameHeader(): void {
  if (!selectedColumn.value || !parsed.value) return;
  const nextView = { ...view.value, columns: { ...view.value.columns,
    [selectedColumn.value]: { ...view.value.columns[selectedColumn.value]!, header: headerDraft.value } } };
  const result = applyCsvCellEdits(parsed.value, session.draft.value,
    [{ row: 0, column: selection.value.column, value: headerDraft.value }]);
  commit(result.text, nextView, result.document);
}
async function findNext(replaceAll = false): Promise<void> {
  if (!parsed.value || !search.value) return;
  const matches: CsvCellEdit[] = [];
  parsed.value.records.forEach((row, rowIndex) => row.fields.forEach((field, column) => {
    if (field.value.includes(search.value)) matches.push({ row: rowIndex, column,
      value: field.value.split(search.value).join(replacement.value) });
  }));
  if (replaceAll) { edit(matches); return; }
  const next = matches.find((match) => match.row > selection.value.row ||
    (match.row === selection.value.row && match.column > selection.value.column)) ?? matches[0];
  if (!next) { actionError.value = t("csv.noMatches"); return; }
  selection.value = next;
  await grid.value?.applySnapshot({ ...next, scrollTop: Math.max(0, (next.row - view.value.headerRows) * view.value.rowHeight), scrollLeft: 0 });
}
function exportTransferSnapshot(): WorkbenchEditorTransferSnapshot {
  return { kind: "workspaceFile", text: session.draft.value, contentHash: session.document.value?.contentHash ?? "",
    originalLineEnding: "\n", csv: { view: viewText.value, viewBaseline: viewBaseline.value,
      viewHash: viewFile.value.contentHash ?? null, mode: mode.value, grid: grid.value?.getSnapshot() } };
}
async function applyTransferSnapshot(snapshot: WorkbenchEditorTransferSnapshot): Promise<boolean> {
  if (snapshot.kind !== "workspaceFile") return false;
  const deadline = Date.now() + 5000;
  while (session.loading.value && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 16));
  if (snapshot.contentHash !== session.document.value?.contentHash) { diskChanged.value = true; return false; }
  session.updateDraft(snapshot.text); session.modelDraft.value = snapshot.text;
  if (snapshot.csv) {
    view.value = parseCsvView(snapshot.csv.view); viewBaseline.value = snapshot.csv.viewBaseline;
    viewFile.value = { contentHash: snapshot.csv.viewHash }; mode.value = snapshot.csv.mode;
  }
  parseText(); remember();
  await nextTick();
  if (snapshot.csv?.grid) await grid.value?.applySnapshot(snapshot.csv.grid);
  return true;
}
async function revealPosition(line: number, column = 1): Promise<boolean> {
  const deadline = Date.now() + 6000;
  while (session.loading.value && Date.now() < deadline && !disposed) await new Promise((resolve) => setTimeout(resolve, 16));
  mode.value = "source"; await nextTick();
  const editor = source.value?.getEditorView();
  if (!editor) return false;
  const target = editor.state.doc.line(Math.max(1, Math.min(editor.state.doc.lines, line)));
  editor.dispatch({ selection: { anchor: Math.min(target.to, target.from + column - 1) }, scrollIntoView: true });
  editor.focus(); return true;
}
async function revealToolFileHighlight(highlight?: ToolFilePreviewHighlight): Promise<boolean> {
  if (!highlight) return true;
  const range = resolveToolFilePreviewHighlightRanges(session.draft.value, 1, highlight)[0];
  return revealPosition(range?.startLine ?? 1);
}

watch(key, async (current, previous) => {
  if (previous) capture(previous);
  discardOnUnmount = false;
  await load();
  await nextTick();
  if (!disposed && key.value === current) emit("ready");
}, { immediate: true });
watch(dirty, (value) => emit("dirtyChange", value));
watch([session.saving, viewSaving], ([savingData, savingView]) => {
  if (!pendingSavingRefresh || savingData || savingView) return;
  pendingSavingRefresh = false; scheduleRefresh("event");
});
watch(editingView, (editing) => { if (!editing) resumeExternalRefresh(); });
watch(mode, async (_, previous) => {
  if (previous === "table") gridSnapshot = grid.value?.getSnapshot();
  remember(); session.modelDraft.value = session.draft.value;
  await nextTick();
  if (mode.value === "table" && gridSnapshot) await grid.value?.applySnapshot(gridSnapshot);
}, { flush: "pre" });
watch(selectedColumn, () => { headerDraft.value = selectedColumn.value ? view.value.columns[selectedColumn.value]!.header : ""; });
watch(viewText, () => {
  if (viewTimer) clearTimeout(viewTimer);
  if (viewDirty.value && !session.dirty.value) viewTimer = setTimeout(() => void saveView(), 650);
});
onBeforeUnmount(() => {
  if (!discardOnUnmount) capture();
  disposed = true; loadingEpoch++; parseId++; worker?.terminate();
  if (viewTimer) clearTimeout(viewTimer);
  if (sourceHistoryTimer) clearTimeout(sourceHistoryTimer);
});
function discardChanges(): void { discardOnUnmount = true; csvEditorSessions.delete(key.value); }
defineExpose({ saveFile, refreshIfChanged, exportTransferSnapshot, applyTransferSnapshot, revealPosition, revealToolFileHighlight, discardChanges });
</script>

<template>
  <section ref="editorRoot" class="csv-editor" @keydown.ctrl.s.prevent="saveFile" @focusout="resumeExternalRefresh">
    <BaseContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" :aria-label="t('editor.contextMenu')" @close="contextMenu = null">
      <template v-for="(item, index) in menuItems" :key="index">
        <div v-if="item.separator" class="base-context-menu-separator" role="separator" />
        <button v-else type="button" role="menuitem" :disabled="item.disabled" @click="menuAction(item.action!)"><span>{{ item.label }}</span><span class="csv-menu-shortcut">{{ item.shortcut }}</span></button>
      </template>
    </BaseContextMenu>
    <div v-if="diskChanged" class="csv-message" role="status">
      <span>{{ t('development.editor.diskChanged') }}</span>
      <BaseButton size="sm" @click="load(false, true)">{{ t('development.editor.useDisk') }}</BaseButton>
      <BaseButton size="sm" @click="load(true, true)">{{ t('development.editor.keepLocal') }}</BaseButton>
    </div>
    <div v-if="viewError || viewChanged" class="csv-message" role="status">
      <span>{{ viewError || t('csv.viewChanged') }}</span><BaseButton size="sm" :disabled="readOnly" @click="editViewSource">{{ t('csv.editView') }}</BaseButton><BaseButton size="sm" @click="reloadView">{{ t('csv.reloadView') }}</BaseButton>
    </div>
    <div v-if="viewWriteError" class="csv-message" role="status"><span>{{ viewWriteError }}</span><BaseButton size="sm" :disabled="viewSaving || session.dirty.value" @click="saveView">{{ t('common.save') }}</BaseButton><BaseButton size="sm" @click="reloadView">{{ t('csv.reloadView') }}</BaseButton></div>
    <div v-if="parseError || actionError || session.error.value" class="csv-message error" role="alert">{{ parseError || actionError || session.error.value }}</div>
    <div v-if="settingsOpen" class="csv-options">
      <label v-if="fileActions !== false && workspaceRef">{{ t('csv.targetPath') }}<input v-model="targetPath" :disabled="readOnly" placeholder="items-copy.csv" /><BaseButton size="sm" :disabled="readOnly" @click="relocate(false)">{{ t('csv.moveFile') }}</BaseButton><BaseButton size="sm" :disabled="readOnly" @click="relocate(true)">{{ t('csv.copyFile') }}</BaseButton></label>
      <BaseButton size="sm" :disabled="readOnly" @click="editViewSource">{{ t('csv.editView') }}</BaseButton>
      <label><BaseCheckbox :model-value="!!view.headerRows" :disabled="readOnly || !!viewError" @update:model-value="updateView({ ...view, headerRows: $event ? 1 : 0 })" />{{ t('csv.firstRowHeader') }}</label>
      <label><BaseCheckbox :model-value="view.wrapText" :disabled="readOnly || !!viewError" @update:model-value="updateView({ ...view, wrapText: $event })" />{{ t('csv.wrap') }}</label>
      <label>{{ t('csv.freeze') }}<input type="number" min="0" :max="view.columnOrder.length" :value="view.frozenColumns" :disabled="readOnly" @change="updateView({ ...view, frozenColumns: Math.max(0, Math.min(view.columnOrder.length, Number(($event.target as HTMLInputElement).value))) })" /></label>
      <label>{{ t('csv.rowHeight') }}<input type="number" min="20" max="120" :value="view.rowHeight" :disabled="readOnly" @change="updateView({ ...view, rowHeight: Math.max(20, Math.min(120, Number(($event.target as HTMLInputElement).value))) })" /></label>
      <label v-if="view.headerRows">{{ t('csv.header') }}<input v-model="headerDraft" :disabled="readOnly" @keydown.enter="renameHeader" /><BaseButton size="sm" :disabled="readOnly" @click="renameHeader">{{ t('common.save') }}</BaseButton></label>
      <label v-if="selectedColumn">{{ t('csv.filter') }}<input :value="view.filters?.find(item => item.columnId === selectedColumn)?.value ?? ''" :disabled="readOnly" @change="filterColumn(($event.target as HTMLInputElement).value)" /></label>
      <label v-for="id in view.columnOrder" :key="id"><BaseCheckbox :model-value="!view.columns[id]!.hidden" :disabled="readOnly || !!viewError" @update:model-value="updateView({ ...view, columns: { ...view.columns, [id]: { ...view.columns[id]!, hidden: !$event } } })" />{{ csvColumnLabel(view.columns[id]!.sourceIndex) }}{{ view.columns[id]!.header ? ` · ${view.columns[id]!.header}` : '' }}</label>
      <BaseButton size="sm" @click="settingsOpen = false">{{ t('common.close') }}</BaseButton>
    </div>
    <div v-if="editingView" class="csv-view-source">
      <label>{{ `${path}.view` }}</label>
      <textarea v-model="viewSource" spellcheck="false" :aria-label="`${path}.view`" />
      <div><BaseButton size="sm" :disabled="readOnly || session.dirty.value" @click="saveViewSource">{{ t('common.save') }}</BaseButton><BaseButton size="sm" @click="editingView = false">{{ t('common.cancel') }}</BaseButton></div>
    </div>
    <div v-if="showFind && mode === 'table'" class="csv-find">
      <input v-model="search" :placeholder="t('csv.find')" :aria-label="t('csv.find')" @keydown.enter="findNext()" />
      <BaseButton size="sm" @click="findNext()">{{ t('csv.findNext') }}</BaseButton>
      <input v-model="replacement" :placeholder="t('csv.replace')" :aria-label="t('csv.replace')" />
      <BaseButton size="sm" :disabled="readOnly" @click="findNext(true)">{{ t('csv.replaceAll') }}</BaseButton>
      <BaseButton size="sm" @click="showFind = false">{{ t('common.close') }}</BaseButton>
    </div>
    <div v-if="session.loading.value && !session.document.value" class="csv-empty">{{ t('development.preview.loading') }}</div>
    <div v-else class="csv-body" @contextmenu.capture="sourceContextMenu">
      <CsvGrid v-if="mode === 'table' && parsed" ref="grid" :document="parsed" :view="view" :read-only="readOnly || parsing" :active="active"
        @edit="edit" @view-change="updateView" @selection-change="selection = $event" @zoom-change="zoom = $event" @context-menu="openContextMenu" @find="showFind = true" @error="actionError = message($event)" @undo="undo()" @redo="undo(true)" @save="saveFile" />
      <BaseMarkdownEditor v-else ref="source" :model-value="sourceModel" :content-key="key" :content-path="path" :workspace-ref="workspaceRef"
        :disabled="readOnly" :active="active !== false" view-mode="native" transaction-model @document-change="sourceChange" @shortcut-save="saveFile" />
    </div>
    <footer class="csv-status"><span>{{ csvColumnLabel(selection.column) }}{{ selection.row + 1 }}</span><span>{{ parsed ? `${parsed.records.length} × ${parsed.columnCount}` : '' }}</span><span v-if="parsing">{{ t('csv.parsing') }}</span><span v-if="readOnly">{{ t('csv.readOnly') }}</span><BaseButton v-if="mode === 'table'" class="csv-zoom" size="sm" :title="t('csv.resetZoom')" :aria-label="t('csv.resetZoom')" @click="grid?.setZoom(1)">{{ Math.round(zoom * 100) }}%</BaseButton></footer>
  </section>
</template>

<style scoped>
.csv-editor { display: flex; flex: 1; flex-direction: column; min-width: 0; min-height: 0; background: var(--panel-bg); color: var(--text-color); }
.csv-find, .csv-options { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border-color); flex-shrink: 0; }
.csv-menu-shortcut { margin-left: auto; padding-left: 24px; color: var(--text-secondary); font-size: 11px; }
.csv-options { flex-wrap: wrap; max-height: 180px; overflow-y: auto; align-items: center; }
.csv-options label { display: inline-flex; align-items: center; gap: 6px; margin-right: 8px; font-size: 12px; }
.csv-options input, .csv-find input { min-width: 0; width: 140px; padding: 4px 6px; border: 1px solid var(--border-color); border-radius: 4px; background: var(--bg-color); color: var(--text-color); font: 12px var(--font-ui); }
.csv-options input[type=number] { width: 56px; }
.csv-view-source { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-bottom: 1px solid var(--border-color); font-size: 12px; }
.csv-view-source textarea { height: 180px; resize: vertical; background: var(--bg-color); color: var(--text-color); border: 1px solid var(--border-color); border-radius: 4px; font-family: var(--font-mono-editor); }
.csv-view-source > div { display: flex; gap: 6px; }
.csv-message { display: flex; align-items: center; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--border-color); color: var(--status-warn-fg); font-size: 12px; }
.csv-message span { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.csv-message.error { color: var(--status-error-fg); }
.csv-body { flex: 1; min-height: 0; min-width: 0; display: flex; overflow: hidden; }
.csv-empty { margin: auto; color: var(--text-secondary); font-size: 12px; }
.csv-status { display: flex; align-items: center; gap: 12px; padding: 4px 10px; border-top: 1px solid var(--border-color); color: var(--text-secondary); font-size: 11px; }
.csv-status .csv-zoom { margin-left: auto; min-height: 18px; padding: 0 4px; font-size: 11px; }
</style>
