<script setup lang="ts">
import { ChangeSet, Compartment, EditorSelection, EditorState, Prec, StateEffect, Transaction, type Extension, type Text } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { openSearchPanel } from "@codemirror/search";
import { useSearchScope } from "../../composables/useSearchScope";
import { markdownEditorSearch } from "./markdown-editor/markdownEditorSearch";
import { computed, getCurrentInstance, inject, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { WORKBENCH_FILE_OPEN_KEY, openWorkbenchFileTab } from "../../services/workbenchFile";
import { openMarkdownReference, relativeMarkdownPath } from "../../services/markdownReference";
import { useOpenView } from "../../composables/useOpenView";
import { extractKnowledgeDocumentOutline } from "../knowledge/knowledgeDocumentOutline";
import { openUrl } from "@tauri-apps/plugin-opener";
import { normalizeExternalMarkdownHref } from "../../composables/markdownExternalLinks";
import "../../styles/markdown-links.css";
import BaseButton from "./BaseButton.vue";
import { useDocumentRecovery } from "../../composables/useDocumentRecovery";
import { markdownImagePaste, type MarkdownImageFolderRequest } from "./markdown-editor/markdownImagePaste";
import MarkdownImageFolderPopover from "./markdown-editor/MarkdownImageFolderPopover.vue";
import { markdownMentions } from "./markdown-editor/markdownMentions";
import { getLocusRuntime } from "../../services/locusRuntime";
import { rebaseDocumentText } from "../../document/documentText";
import MarkdownEditorControls from "./markdown-editor/MarkdownEditorControls.vue";
import { openImageLightbox } from "./imageLightboxService";
import MarkdownCommentPopover from "./markdown-editor/MarkdownCommentPopover.vue";
import MarkdownFootnotePopover from "./markdown-editor/MarkdownFootnotePopover.vue";
import { markdownCommentExtensions } from "./markdown-editor/markdownCommentEditing";
import type { MarkdownCommentController } from "./markdown-editor/markdownCommentController";
import { insertMarkdownFootnote, markdownFootnoteExtensions } from "./markdown-editor/markdownFootnoteEditing";
import type { FloatingAnchorRect } from "./markdown-editor/markdownFloating";
import MarkdownProperties from "./MarkdownProperties.vue";
import { markdownFrontmatterField, markdownFrontmatterHiding, type MarkdownFrontmatter } from "./markdown-editor/markdownFrontmatter";
import { isolateHistory } from "@codemirror/commands";
import MarkdownEditorContextMenu from "./markdown-editor/MarkdownEditorContextMenu.vue";
import {
  captureMarkdownEditorSelection,
  syncMarkdownDOMSelection,
  type MarkdownEditorSelection,
} from "./markdown-editor/markdownEditorSelection";
import { useNotificationStore } from "../../stores/notification";
import { t } from "../../i18n";
import {
  markdownEditorBaseExtensions,
  markdownEditorLanguageExtension,
  markdownEditorLanguageFromPath,
  markdownEditorModeExtension,
  markdownEditorPlaceholderExtension,
  markdownEditorReadOnlyExtension,
} from "./markdown-editor/codeMirrorMarkdownExtensions";
import {
  MarkdownEditorSessionCache,
  type MarkdownEditorSessionStore,
} from "./markdown-editor/markdownEditorSessionCache";
import { createMinimalTextChange } from "./markdown-editor/markdownEditorTransactions";
import type { MarkdownEditorDocumentChange } from "./markdown-editor/markdownEditorDocumentChange";
import type { MarkdownImageResolver, MarkdownLivePreviewOptions } from "./markdown-editor/markdownComplexWidgets";
import type { MarkdownReferenceToken } from "./markdown-editor/markdownComplexTokens";
import { normalizeMarkdownEditorLineEndings } from "./markdownEditorFormatting";
import type { MarkdownEditorViewMode } from "./markdownEditorViewMode";
import { resolveMarkdownImage, rewriteMarkdownImageLinks } from "../../services/markdownImage";
import type { WorkspaceRef } from "../../services/project";
import { subscribeWorkspaceFileChanges } from "../../services/workspaceExplorer";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import { useTextViewerZoom } from "../../composables/useTextViewerZoom";
import { workspaceFileChangeInScope } from "../../composables/useFileChangeRevalidation";
import { useDocumentEditingSettings } from "../../composables/useDocumentEditingSettings";
import { markdownPunctuationPairing } from "./markdown-editor/markdownPunctuationPairing";
import {
  captureReadingAnchor as captureAnchorAt,
  restoreReadingAnchor as restoreAnchorAt,
  type ReadingAnchor,
} from "../../document/readingAnchor";
import {
  getReadingPosition,
  putReadingPosition,
  readingTargetKey,
  registerReadingPositionCapture,
  type ReadingTarget,
} from "../../services/readingPosition";

import { markdownDependencyPath } from "../../composables/markdownFileDependencies";

// Track only images that have actually entered the preview cache. This also
// retains off-screen image dependencies without scanning the editor document
// on every filesystem event. Absolute paths use conservative suffix matching
// because this component has a WorkspaceRef but no checkout root.
let imageReferences = new Set<string>();
function rememberImageReference(references: Set<string>, source: string, contentPath = "") {
  const relative = markdownDependencyPath(source, "", contentPath, true);
  if (relative) { references.add(relative); return; }
  let path = source.replace(/\\/g, "/")
    .replace(/^file:\/\//i, "").replace(/^\/([a-z]:\/)/i, "$1").replace(/[?#].*$/, "");
  try { path = decodeURIComponent(path); } catch { /* Literal path. */ }
  const absolute = relativeMarkdownPath(contentPath, path);
  if (/^(?:[a-z]:\/|\/)/i.test(absolute)) references.add(absolute.toLocaleLowerCase());
}
const workspaceMarkdownImageResolver: MarkdownImageResolver = async (source, context) => {
  if (!context.workspaceRef) return null;
  const references = imageReferences;
  // The backend supports both document-relative and workspace-relative paths.
  rememberImageReference(references, source);
  rememberImageReference(references, source, context.contentPath);
  const resolved = await resolveMarkdownImage(context.workspaceRef, source, context.contentPath);
  references.add(resolved.displayPath.replace(/\\/g, "/").toLocaleLowerCase());
  return resolved;
};

const props = withDefaults(defineProps<{
  modelValue: string;
  savedValue?: string;
  disabled?: boolean;
  placeholder?: string;
  viewMode?: MarkdownEditorViewMode;
  contentPath?: string;
  /** Physical document location when contentPath names a logical knowledge field. */
  resourcePath?: string;
  contentKey?: string;
  sessionCache?: MarkdownEditorSessionStore | null;
  sessionPinned?: boolean;
  workspaceRef?: WorkspaceRef | null;
  active?: boolean;
  autoGrow?: boolean;
  minHeight?: number;
  canQuoteSelection?: boolean;
  /** Offers to open a new conversation beside the editor with the selection quoted. */
  canQuoteSelectionInNewSession?: boolean;
  showProperties?: boolean;
  searchPriority?: number;
  /**
   * Emits immutable CodeMirror Text/ChangeSet payloads for local edits. This
   * avoids allocating the whole document string on every transaction. The
   * model is materialized at an explicit shortcut-save boundary.
   */
  transactionModel?: boolean;
  /**
   * Durable reading position of the shown document. Restored when no session
   * of this editor exists; recorded when scrolling settles, on hide and on exit.
   * Set it only once the document content is loaded.
   */
  readingPosition?: ReadingTarget | null;
  /** Comment UI of this editor; without it comments stay hidden but intact. */
  comments?: MarkdownCommentController | null;
}>(), {
  readingPosition: null,
  comments: null,
  disabled: false,
  placeholder: "",
  viewMode: "rendered",
  contentPath: "",
  contentKey: "",
  sessionCache: null,
  sessionPinned: false,
  workspaceRef: null,
  active: true,
  autoGrow: false,
  minHeight: 80,
  canQuoteSelection: false,
  canQuoteSelectionInNewSession: false,
  showProperties: false,
  transactionModel: false,
});

const emit = defineEmits<{
  (e: "quoteSelection", selection: MarkdownEditorSelection): void;
  (e: "quoteSelectionInNewSession", selection: MarkdownEditorSelection): void;
  /** Settled user selection; null once it collapses or the editor shows another document. */
  (e: "selectionChange", selection: MarkdownEditorSelection | null): void;
  (e: "update:modelValue", value: string): void;
  (e: "documentChange", value: MarkdownEditorDocumentChange): void;
  (e: "shortcutSave"): void;
  (e: "externalChange"): void;
  (e: "referenceOpen", reference: MarkdownReferenceToken): void;
  (e: "referencePointerDown", payload: {
    reference: MarkdownReferenceToken;
    event: PointerEvent;
    element: HTMLElement;
  }): void;
}>();

const mountRef = ref<HTMLDivElement | null>(null);
const instance = getCurrentInstance();
const openWorkbenchFile = inject(WORKBENCH_FILE_OPEN_KEY, openWorkbenchFileTab);
const openViewPackage = useOpenView();
const searchRoot = ref<HTMLElement | null>(null);
useSearchScope(searchRoot, () => { if (editorView) openSearchPanel(editorView); }, {
  enabled: () => props.active && !!editorView,
  priority: props.searchPriority ?? 1,
});
const controlsRef = ref<InstanceType<typeof MarkdownEditorControls> | null>(null);
const contextMenuRef = ref<InstanceType<typeof MarkdownEditorContextMenu> | null>(null);
const controlsView = shallowRef<EditorView | null>(null);
const frontmatter = shallowRef<MarkdownFrontmatter | null>(null);
const propertiesCompartment = new Compartment();

function propertiesExtensions(): Extension {
  if (!props.showProperties || currentLanguage() !== "markdown") return [];
  return [markdownFrontmatterField, ...(props.viewMode === "rendered" ? markdownFrontmatterHiding : [])];
}

function updateProperties(yaml: string): void {
  const view = editorView;
  if (!view || view.state.readOnly) return;
  const current = view.state.field(markdownFrontmatterField, false);
  if (!current || current.invalid) return;
  const range = current.range;
  const change = range
    ? { from: range.yamlFrom, to: range.yamlTo, insert: yaml }
    : { from: view.state.doc.sliceString(0, 1) === "\uFEFF" ? 1 : 0, insert: `---\n${yaml}---\n\n` };
  view.dispatch({ changes: change, annotations: isolateHistory.of("full"), userEvent: "input.property" });
}
const languageCompartment = new Compartment();
const modeCompartment = new Compartment();
const readOnlyCompartment = new Compartment();
const placeholderCompartment = new Compartment();
const punctuationCompartment = new Compartment();
const annotationCompartment = new Compartment();

interface FootnotePopoverState {
  key: string;
  label: string;
  anchor: FloatingAnchorRect;
  insertedDoc: Text | null;
}
const footnotePopover = shallowRef<FootnotePopoverState | null>(null);

// First image paste of a project without an image folder. One prompt per
// editor; pastes made while it is open wait for the same answer.
interface ImageFolderPromptState extends MarkdownImageFolderRequest {
  anchor: FloatingAnchorRect;
}
const imageFolderPrompt = shallowRef<ImageFolderPromptState | null>(null);
let imageFolderAnswer: { promise: Promise<boolean>; resolve: (saved: boolean) => void } | null = null;

function chooseImageFolder(request: MarkdownImageFolderRequest): Promise<boolean> {
  if (imageFolderAnswer) return imageFolderAnswer.promise;
  let resolve!: (saved: boolean) => void;
  const promise = new Promise<boolean>((done) => { resolve = done; });
  imageFolderAnswer = { promise, resolve };
  const rect = request.anchor ?? editorView?.contentDOM.getBoundingClientRect() ?? null;
  imageFolderPrompt.value = {
    ...request,
    anchor: rect ? { left: rect.left, top: rect.top, bottom: rect.bottom } : { left: 16, top: 16, bottom: 16 },
  };
  return promise;
}

/** Closes the prompt; pending pastes continue only when a folder was saved. */
function cancelImageFolderPrompt(saved = false): void {
  const answer = imageFolderAnswer;
  imageFolderAnswer = null;
  imageFolderPrompt.value = null;
  answer?.resolve(saved);
}

function onImageFolderDone(saved: boolean): void {
  cancelImageFolderPrompt(saved);
  editorView?.focus();
}

function annotationExtensions(): Extension {
  if (currentLanguage() !== "markdown") return [];
  return [
    markdownFootnoteExtensions(),
    markdownCommentExtensions({ hideBlock: props.viewMode === "rendered", sync: props.comments ?? null }),
    Prec.high(keymap.of([
      { key: "Mod-Alt-m", run: () => startComment() },
      { key: "Mod-Alt-f", run: () => insertFootnoteAtCursor() },
    ])),
  ];
}

function openFootnote(target: { key: string; label: string; element: HTMLElement }): void {
  const rect = target.element.getBoundingClientRect();
  footnotePopover.value = {
    key: target.key,
    label: target.label,
    anchor: { left: rect.left, top: rect.top, bottom: rect.bottom },
    insertedDoc: null,
  };
}

function insertFootnoteAtCursor(): boolean {
  const view = editorView;
  if (!view || view.state.readOnly || currentLanguage() !== "markdown") return false;
  const key = insertMarkdownFootnote(view);
  if (!key) return false;
  const insertedDoc = view.state.doc;
  view.requestMeasure({
    read: () => view.coordsAtPos(view.state.selection.main.head),
    write: (coords) => {
      if (editorView !== view || view.state.doc !== insertedDoc) return;
      const rect = coords ?? view.contentDOM.getBoundingClientRect();
      footnotePopover.value = { key, label: key, anchor: { left: rect.left, top: rect.top, bottom: rect.bottom }, insertedDoc };
    },
  });
  return true;
}

function startComment(): boolean {
  const controller = props.comments;
  if (!controller || !editorView || editorView.state.readOnly) return false;
  const result = controller.startDraft();
  if (result === "empty") useNotificationStore().addNotice("info", t("editor.comment.emptySelection"));
  else if (result === "tooLong") useNotificationStore().addNotice("warning", t("editor.comment.selectionTooLong"));
  else if (result === "readOnly" && !controller.writable.value) useNotificationStore().addNotice("info", t("editor.comment.readOnlyFormat"));
  return result !== "readOnly";
}
const { state: documentEditingSettings } = useDocumentEditingSettings();
const localSessionCache = new MarkdownEditorSessionCache();
const imageCacheEpoch = ref(0);
let imageRefreshTimer: ReturnType<typeof setTimeout> | null = null;
const {
  textViewerZoomScale,
  textViewerZoomStyle,
  handleTextViewerZoomWheel,
} = useTextViewerZoom();

let editorView: EditorView | null = null;
let activeSessionKey = normalizeSessionKey(props.contentKey);
let activeSessionCache = props.sessionCache ?? localSessionCache;
let activeSessionPinned = props.sessionPinned;
let applyingExternalModel = false;
let composing = false;
/**
 * A model value that arrived during IME composition. `changes` is relative to
 * the document at arrival; `local` collects the edits made since, so a flush
 * merges both instead of replacing text committed by the composition.
 */
let pendingExternalModel: { changes: ChangeSet; local: ChangeSet } | null = null;
let compositionFlushToken = 0;
let currentScrollTop = 0;
let currentScrollLeft = 0;
let removeScrollTracking: (() => void) | null = null;
let activeScrollElement: HTMLElement | null = null;
let releaseWorkspaceFileChanges: RuntimeUnsubscribe | null = null;
let editorUnmounted = false;
const READING_CAPTURE_DELAY_MS = 1000;
/** Reading target of the document currently in the view. */
let readingTarget: ReadingTarget | null = null;
let readingCaptureTimer: ReturnType<typeof setTimeout> | null = null;
/** Bumped to abandon a pending restore; a restore applies only for its own token. */
let readingRestoreToken = 0;
let readingRestorePending = false;
/** Shown without a session before its reading target was known. */
let readingRestoreAwaitingTarget = false;
let lastReadingAnchor: ReadingAnchor | null = null;
let releaseReadingCapture: (() => void) | null = null;
const physicalPath = computed(() => props.resourcePath || props.contentPath);
const recovery = useDocumentRecovery({
  key: () => JSON.stringify([props.workspaceRef?.checkoutId ?? "local", props.workspaceRef?.expectedMaterializationEpoch ?? 0, physicalPath.value, props.contentKey.match(/:(body|summary|maintenanceRules)$/)?.[1] ?? "body"]),
  base: () => normalizeMarkdown(props.savedValue ?? props.modelValue),
  text: () => editorView?.state.doc ?? props.modelValue,
  enabled: () => !props.disabled && !!physicalPath.value && currentLanguage() === "markdown",
  apply: (text) => {
    const view = editorView;
    if (!view || view.state.readOnly) return;
    const changes = createMinimalTextChange(view.state.doc.toString(), text);
    if (changes) view.dispatch({ changes, annotations: isolateHistory.of("full"), userEvent: "input.restore" });
    view.focus();
  },
});
let releaseImageRenames: RuntimeUnsubscribe | null = null;
const SELECTION_CHANGE_DELAY_MS = 150;
let selectionChangeTimer: ReturnType<typeof setTimeout> | null = null;
// Identifies the last emitted selection ("" = none), so cursor moves and
// repeated DOM syncs do not re-emit an unchanged selection.
let emittedSelectionKey = "";
let selectionDocument: Document | null = null;

function hasSelectionListener(): boolean {
  return !!instance?.vnode.props?.onSelectionChange;
}

function selectionKey(state: EditorState): string {
  const ranges = state.selection.ranges.filter((range) => !range.empty);
  return ranges.length
    ? `${state.doc.length}|${ranges.map((range) => `${range.from}:${range.to}`).join(",")}`
    : "";
}

function emitSelectionChange(): void {
  if (selectionChangeTimer) clearTimeout(selectionChangeTimer);
  selectionChangeTimer = null;
  const view = editorView;
  if (!view) return;
  const key = selectionKey(view.state);
  if (key === emittedSelectionKey) return;
  emittedSelectionKey = key;
  emit("selectionChange", key ? captureMarkdownEditorSelection(view.state) : null);
}

function scheduleSelectionChange(): void {
  if (!hasSelectionListener()) return;
  if (selectionChangeTimer) clearTimeout(selectionChangeTimer);
  selectionChangeTimer = setTimeout(emitSelectionChange, SELECTION_CHANGE_DELAY_MS);
}

/** Another document replaces this one; its selection no longer applies. */
function resetSelectionChange(): void {
  if (selectionChangeTimer) clearTimeout(selectionChangeTimer);
  selectionChangeTimer = null;
  if (!emittedSelectionKey) return;
  emittedSelectionKey = "";
  emit("selectionChange", null);
}

/** Non-editable content keeps the browser selection; mirror it into the editor state. */
function handleDocumentSelectionChange(): void {
  const view = editorView;
  if (!view || !view.state.readOnly) return;
  const selection = view.dom.ownerDocument.getSelection();
  if (!selection?.anchorNode || !view.contentDOM.contains(selection.anchorNode)) return;
  if (!selection.isCollapsed) {
    syncMarkdownDOMSelection(view);
    return;
  }
  if (view.state.selection.ranges.some((range) => !range.empty)) {
    view.dispatch({ selection: EditorSelection.cursor(view.state.selection.main.head) });
  }
}

function normalizeMarkdown(value: string): string {
  return normalizeMarkdownEditorLineEndings(value);
}

function normalizeSessionKey(value: string): string {
  return value.trim() || "__default__";
}

function currentLanguage() {
  return markdownEditorLanguageFromPath(props.contentPath);
}

function handleReferenceOpen(reference: MarkdownReferenceToken): void {
  if (instance?.vnode.props?.onReferenceOpen) emit("referenceOpen", reference);
  else if (props.workspaceRef) void openMarkdownReference(reference, props.workspaceRef, openWorkbenchFile, undefined, openViewPackage).catch(handleContextMenuError);
}

function openLink(url: string): void {
  const external = normalizeExternalMarkdownHref(url);
  if (external) { void openUrl(external).catch(handleContextMenuError); return; }
  const decode = (value: string) => { try { return decodeURIComponent(value); } catch { return value; } };
  if (url.startsWith("#") && editorView) {
    const heading = decode(url.slice(1));
    const item = extractKnowledgeDocumentOutline(editorView.state.doc.toString()).find(item => item.text === heading || item.text.toLowerCase().replace(/\s+/g, "-") === heading.toLowerCase());
    if (item) editorView.dispatch({ effects: EditorView.scrollIntoView(item.from, { y: "start" }) });
    return;
  }
  if (!props.workspaceRef || (/^[a-z][a-z0-9+.-]*:/i.test(url) && !/^[a-z]:[\\/]/i.test(url))) return;
  const filePath = relativeMarkdownPath(physicalPath.value, decode(url.split("#", 1)[0] ?? url));
  void openWorkbenchFile({ workspaceRef: props.workspaceRef, filePath }).catch(handleContextMenuError);
}

function handleReferencePointerDown(
  reference: MarkdownReferenceToken,
  event: PointerEvent,
  element: HTMLElement,
): void {
  emit("referencePointerDown", { reference, event, element });
}

function currentLivePreviewOptions(): MarkdownLivePreviewOptions {
  const workspaceRef = props.workspaceRef;
  const options: MarkdownLivePreviewOptions = {
    onEditTarget: (target) => controlsRef.value?.open(target),
    onImageOpen: openImageLightbox,
    onReferenceOpen: handleReferenceOpen,
    onReferencePointerDown: handleReferencePointerDown,
    onLinkOpen: openLink,
    onFootnoteOpen: openFootnote,
  };
  if (!workspaceRef) return options;
  return {
    ...options,
    imageResolver: workspaceMarkdownImageResolver,
    imageContext: {
      cacheKey: `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}:${imageCacheEpoch.value}`,
      contentPath: physicalPath.value,
      workspaceRef: {
        checkoutId: workspaceRef.checkoutId,
        expectedGeneration: workspaceRef.expectedGeneration,
        expectedMaterializationEpoch: workspaceRef.expectedMaterializationEpoch,
      },
    },
  };
}

/** Model text for `doc`, including a model value still queued behind IME composition. */
function modelText(doc: Text): string {
  const pending = pendingExternalModel;
  return normalizeMarkdown((pending ? pending.changes.map(pending.local).apply(doc) : doc).toString());
}

function emitCurrentValue(): void {
  if (!editorView) return;
  emit("update:modelValue", modelText(editorView.state.doc));
}

function emitShortcutSave(): void {
  emitCurrentValue();
  emit("shortcutSave");
}

function flushPendingExternalModel(): void {
  const pending = pendingExternalModel;
  pendingExternalModel = null;
  const view = editorView;
  if (pending === null || !view) return;
  const merged = !pending.local.empty;
  // Without local edits the document equals the model value afterwards. Merged
  // text is new to the parent, which only saw the local edits, so publish it.
  applyingExternalModel = !merged;
  try {
    view.dispatch({
      changes: merged ? pending.changes.map(pending.local) : pending.changes,
      annotations: Transaction.addToHistory.of(false),
    });
  } finally {
    applyingExternalModel = false;
  }
}

function resetCompositionState(flushPending: boolean): void {
  compositionFlushToken += 1;
  composing = false;
  if (flushPending) flushPendingExternalModel();
  else pendingExternalModel = null;
}

function handleCompositionStart(): boolean {
  compositionFlushToken += 1;
  composing = true;
  return false;
}

function handleCompositionEnd(): boolean {
  composing = false;
  const flushToken = ++compositionFlushToken;
  void Promise.resolve().then(() => {
    if (composing || flushToken !== compositionFlushToken) return;
    flushPendingExternalModel();
  });
  return false;
}

function editorExtensions(): Extension[] {
  const language = currentLanguage();
  return [
    ...markdownEditorBaseExtensions(emitShortcutSave),
    markdownImagePaste({
      context: () => props.workspaceRef && physicalPath.value && currentLanguage() === "markdown"
        ? { workspaceRef: { ...props.workspaceRef }, contentPath: physicalPath.value, key: activeSessionKey } : null,
      onError: handleContextMenuError,
      chooseFolder: chooseImageFolder,
    }),
    markdownEditorSearch(),
    markdownMentions({
      context: () => props.active && props.workspaceRef && currentLanguage() === "markdown"
        ? { workspaceRef: props.workspaceRef, contentPath: physicalPath.value, key: activeSessionKey } : null,
    }),
    propertiesCompartment.of(propertiesExtensions()),
    languageCompartment.of(markdownEditorLanguageExtension(language)),
    modeCompartment.of(markdownEditorModeExtension(
      props.viewMode,
      language,
      currentLivePreviewOptions(),
    )),
    readOnlyCompartment.of(markdownEditorReadOnlyExtension(props.disabled)),
    placeholderCompartment.of(markdownEditorPlaceholderExtension(props.placeholder)),
    punctuationCompartment.of(punctuationExtensions()),
    annotationCompartment.of(annotationExtensions()),
    EditorView.domEventHandlers({
      compositionstart: handleCompositionStart,
      compositionend: handleCompositionEnd,
    }),
    EditorView.updateListener.of((update) => {
      frontmatter.value = update.state.field(markdownFrontmatterField, false) ?? null;
      if (update.docChanged || update.selectionSet) {
        contextMenuRef.value?.close(false);
        scheduleSelectionChange();
      }
      controlsRef.value?.update(update);
      if (!update.docChanged) return;
      if (pendingExternalModel) pendingExternalModel.local = pendingExternalModel.local.compose(update.changes);
      recovery.changed();
      if (applyingExternalModel) return;
      if (props.transactionModel) {
        emit("documentChange", {
          doc: update.state.doc,
          changes: update.changes,
        });
        return;
      }
      // A v-model parent stores what it receives, so keep its queued value.
      emit("update:modelValue", modelText(update.state.doc));
    }),
  ];
}

function createEditorState(value: string): EditorState {
  return EditorState.create({
    doc: normalizeMarkdown(value),
    extensions: editorExtensions(),
  });
}

function configurationEffects() {
  const language = currentLanguage();
  return [
    propertiesCompartment.reconfigure(propertiesExtensions()),
    languageCompartment.reconfigure(markdownEditorLanguageExtension(language)),
    modeCompartment.reconfigure(markdownEditorModeExtension(
      props.viewMode,
      language,
      currentLivePreviewOptions(),
    )),
    readOnlyCompartment.reconfigure(markdownEditorReadOnlyExtension(props.disabled)),
    placeholderCompartment.reconfigure(markdownEditorPlaceholderExtension(props.placeholder)),
    punctuationCompartment.reconfigure(punctuationExtensions()),
    annotationCompartment.reconfigure(annotationExtensions()),
  ];
}

function punctuationExtensions(): Extension {
  return documentEditingSettings.punctuationPairing && currentLanguage() === "markdown"
    ? markdownPunctuationPairing() : [];
}

function stateWithFreshConfiguration(state: EditorState): EditorState {
  return state.update({
    effects: StateEffect.reconfigure.of(editorExtensions()),
    annotations: Transaction.addToHistory.of(false),
  }).state;
}

function stateWithExternalValue(state: EditorState, value: string): EditorState {
  const normalized = normalizeMarkdown(value);
  const change = createMinimalTextChange(state.doc.toString(), normalized);
  if (!change) return state;
  return state.update({
    changes: change,
    annotations: Transaction.addToHistory.of(false),
  }).state;
}

function syncExternalModel(value: string): void {
  const view = editorView;
  if (!view) return;
  const normalized = normalizeMarkdown(value);
  const change = createMinimalTextChange(view.state.doc.toString(), normalized);
  if (!change) return;

  applyingExternalModel = true;
  try {
    view.dispatch({
      changes: change,
      annotations: Transaction.addToHistory.of(false),
    });
  } finally {
    applyingExternalModel = false;
  }
}

function syncOrQueueExternalModel(value: string): void {
  const normalized = normalizeMarkdown(value);
  const view = editorView;
  if (composing && view) {
    // The newest model value supersedes a queued one. A value equal to the
    // document, such as an autosave or v-model echo, leaves nothing to apply.
    const doc = view.state.doc;
    const change = createMinimalTextChange(doc.toString(), normalized);
    pendingExternalModel = change
      ? { changes: ChangeSet.of(change, doc.length), local: ChangeSet.empty(doc.length) }
      : null;
    return;
  }
  compositionFlushToken += 1;
  pendingExternalModel = null;
  syncExternalModel(normalized);
}

function saveActiveSession(
  modelValue = props.modelValue,
  pinned = activeSessionPinned,
): void {
  const view = editorView;
  if (!view) return;
  activeSessionCache.set(activeSessionKey, {
    state: view.state,
    scrollTop: currentScrollTop,
    scrollLeft: currentScrollLeft,
    modelValue: normalizeMarkdown(modelValue),
    pinned,
  });
}

function isScrollableOverflow(element: HTMLElement): boolean {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  if (!style) return false;
  return /^(?:auto|scroll|overlay)$/.test(style.overflowY)
    || /^(?:auto|scroll|overlay)$/.test(style.overflow);
}

function resolveScrollElement(view: EditorView): HTMLElement {
  if (props.autoGrow) {
    let ancestor = view.dom.parentElement;
    while (ancestor) {
      if (isScrollableOverflow(ancestor)) return ancestor;
      ancestor = ancestor.parentElement;
    }
    const documentScroller = view.dom.ownerDocument.scrollingElement;
    if (documentScroller instanceof HTMLElement) return documentScroller;
  }
  return view.scrollDOM;
}

function startScrollTracking(view: EditorView): void {
  removeScrollTracking?.();
  const scrollElement = resolveScrollElement(view);
  activeScrollElement = scrollElement;
  const handleScroll = () => {
    currentScrollTop = scrollElement.scrollTop;
    currentScrollLeft = scrollElement.scrollLeft;
    scheduleReadingCapture();
  };
  // The user taking over the viewport abandons a pending reading restore.
  const handleUserScroll = () => {
    if (readingRestorePending || readingRestoreAwaitingTarget) cancelReadingRestore();
  };
  const userEvents = ["wheel", "pointerdown", "keydown", "touchstart"] as const;
  scrollElement.addEventListener("scroll", handleScroll, { passive: true });
  for (const type of userEvents) scrollElement.addEventListener(type, handleUserScroll, { passive: true });
  removeScrollTracking = () => {
    scrollElement.removeEventListener("scroll", handleScroll);
    for (const type of userEvents) scrollElement.removeEventListener(type, handleUserScroll);
    if (activeScrollElement === scrollElement) activeScrollElement = null;
    removeScrollTracking = null;
  };
}

function restoreTrackedScroll(view: EditorView, top: number, left: number): void {
  const scrollElement = activeScrollElement ?? resolveScrollElement(view);
  scrollElement.scrollTop = top;
  scrollElement.scrollLeft = left;
}

/** Anchor at the viewport top; undefined while it is unknown (no view, restore pending). */
function captureReadingNow(): ReadingAnchor | null | undefined {
  const view = editorView;
  if (!view || !readingTarget || readingRestorePending) return undefined;
  return captureAnchorAt(view, activeScrollElement ?? resolveScrollElement(view));
}

function storeReadingPosition(): void {
  if (readingCaptureTimer !== null) clearTimeout(readingCaptureTimer);
  readingCaptureTimer = null;
  const anchor = captureReadingNow();
  if (anchor === undefined || !readingTarget) return;
  lastReadingAnchor = anchor;
  putReadingPosition(readingTarget, anchor);
}

function scheduleReadingCapture(): void {
  if (!readingTarget || readingRestorePending) return;
  if (readingCaptureTimer !== null) clearTimeout(readingCaptureTimer);
  readingCaptureTimer = setTimeout(storeReadingPosition, READING_CAPTURE_DELAY_MS);
}

/** Explicit navigation or user scrolling wins over a restore still in flight. */
function cancelReadingRestore(): void {
  readingRestoreToken += 1;
  readingRestorePending = false;
  readingRestoreAwaitingTarget = false;
}

/** Restores the durable position of a document shown without an editor session. */
function requestReadingRestore(view: EditorView): void {
  readingRestoreToken += 1;
  readingRestorePending = false;
  const target = readingTarget;
  // The document may still be loading; restore once its target is known.
  readingRestoreAwaitingTarget = !target;
  if (!target) return;
  const token = readingRestoreToken;
  readingRestorePending = true;
  void getReadingPosition(target).then((anchor) => {
    if (token !== readingRestoreToken) return;
    readingRestorePending = false;
    if (editorView !== view) return;
    lastReadingAnchor = anchor;
    if (anchor) restoreAnchorAt(view, activeScrollElement ?? resolveScrollElement(view), anchor);
    else restoreTrackedScroll(view, 0, 0);
  });
}

function restoreReadingAnchor(anchor: ReadingAnchor | null): boolean {
  const view = editorView;
  if (!view) return false;
  cancelReadingRestore();
  lastReadingAnchor = anchor;
  if (!anchor) return true;
  return restoreAnchorAt(view, activeScrollElement ?? resolveScrollElement(view), anchor);
}

function bindReadingTarget(): void {
  readingTarget = props.readingPosition ?? null;
  lastReadingAnchor = null;
}

function stateWithSessionModel(
  state: EditorState,
  cachedModelValue: string | undefined,
  nextModelValue: string,
): EditorState {
  if (
    props.transactionModel
    && cachedModelValue !== undefined
    && normalizeMarkdown(cachedModelValue) === normalizeMarkdown(nextModelValue)
  ) {
    return state;
  }
  return stateWithExternalValue(state, nextModelValue);
}

function restoreSession(contentKey: string, nextModelValue: string, previousModelValue: string): void {
  contextMenuRef.value?.close(false);
  footnotePopover.value = null;
  cancelImageFolderPrompt();
  const view = editorView;
  if (!view) {
    activeSessionKey = normalizeSessionKey(contentKey);
    return;
  }

  resetCompositionState(true);
  resetSelectionChange();
  storeReadingPosition();
  cancelReadingRestore();
  saveActiveSession(previousModelValue, activeSessionPinned);
  activeSessionKey = normalizeSessionKey(contentKey);
  activeSessionCache = props.sessionCache ?? localSessionCache;
  activeSessionPinned = props.sessionPinned;
  activeSessionCache.setPinned(activeSessionKey, activeSessionPinned);
  const cached = activeSessionCache.get(activeSessionKey);
  // A newly created state already contains the current document and every
  // compartment. Re-applying both here duplicates the most expensive work for
  // large documents (full-text comparison plus language/plugin setup).
  const nextState = cached
    ? stateWithFreshConfiguration(stateWithSessionModel(cached.state, cached.modelValue, nextModelValue))
    : createEditorState(nextModelValue);
  view.setState(nextState);
  frontmatter.value = nextState.field(markdownFrontmatterField, false) ?? null;
  bindReadingTarget();

  const scrollTop = cached?.scrollTop ?? 0;
  const scrollLeft = cached?.scrollLeft ?? 0;
  currentScrollTop = scrollTop;
  currentScrollLeft = scrollLeft;
  const restoredKey = activeSessionKey;
  // A session in memory is exact; otherwise use the durable reading position.
  if (!cached) requestReadingRestore(view);
  view.requestMeasure({
    read: () => null,
    write() {
      if (!editorView || activeSessionKey !== restoredKey || readingRestorePending) return;
      restoreTrackedScroll(editorView, scrollTop, scrollLeft);
    },
  });
}

function mountEditor(): void {
  const parent = mountRef.value;
  if (!parent || editorView || !props.active) return;

  activeSessionCache = props.sessionCache ?? localSessionCache;
  activeSessionPinned = props.sessionPinned;
  activeSessionCache.setPinned(activeSessionKey, activeSessionPinned);
  const cached = activeSessionCache.get(activeSessionKey);
  const state = cached
    ? stateWithFreshConfiguration(stateWithSessionModel(cached.state, cached.modelValue, props.modelValue))
    : createEditorState(props.modelValue);
  editorView = new EditorView({ state, parent });
  frontmatter.value = state.field(markdownFrontmatterField, false) ?? null;
  controlsView.value = editorView;
  startScrollTracking(editorView);
  bindReadingTarget();

  const scrollTop = cached?.scrollTop ?? 0;
  const scrollLeft = cached?.scrollLeft ?? 0;
  currentScrollTop = scrollTop;
  currentScrollLeft = scrollLeft;
  const restoredKey = activeSessionKey;
  if (!cached) requestReadingRestore(editorView);
  editorView.requestMeasure({
    read: () => null,
    write() {
      if (!editorView || activeSessionKey !== restoredKey || readingRestorePending) return;
      restoreTrackedScroll(editorView, scrollTop, scrollLeft);
    },
  });
}

function suspendEditor(): void {
  cancelImageFolderPrompt();
  if (!editorView) return;
  footnotePopover.value = null;
  // A hidden tab keeps its last selection; deliver one that is still settling.
  if (selectionChangeTimer) emitSelectionChange();
  resetCompositionState(true);
  storeReadingPosition();
  cancelReadingRestore();
  saveActiveSession();
  removeScrollTracking?.();
  editorView.destroy();
  editorView = null;
  controlsView.value = null;
}

function reconfigureEditor(): void {
  contextMenuRef.value?.close(false);
  const view = editorView;
  if (!view) return;
  applyingExternalModel = true;
  try {
    view.dispatch({
      effects: configurationEffects(),
      annotations: Transaction.addToHistory.of(false),
    });
  } finally {
    applyingExternalModel = false;
  }
  view.requestMeasure();
}

function handleEditorWheel(event: WheelEvent): void {
  handleTextViewerZoomWheel(event);
}

function handleContextMenuError(error: unknown): void {
  useNotificationStore().addNotice("error", `${t("editor.actionFailed")}: ${error instanceof Error ? error.message : String(error)}`);
}

onMounted(() => {
  mountEditor();
  // App exit records the position of the visible document.
  releaseReadingCapture = registerReadingPositionCapture(storeReadingPosition);
  if (hasSelectionListener()) {
    selectionDocument = mountRef.value?.ownerDocument ?? null;
    selectionDocument?.addEventListener("selectionchange", handleDocumentSelectionChange);
  }
  void getLocusRuntime().subscribe<{ checkoutId?: string; workspaceGeneration?: number; materializationEpoch?: number; oldPath: string; newPath: string }>("markdown-image-renamed", async (event) => {
    const workspaceRef = props.workspaceRef;
    const view = editorView;
    if (!workspaceRef || !view || view.state.readOnly || currentLanguage() !== "markdown" || (event.checkoutId && event.checkoutId !== workspaceRef.checkoutId)) return;
    if (event.workspaceGeneration != null && workspaceRef.expectedGeneration != null && event.workspaceGeneration !== workspaceRef.expectedGeneration) return;
    if (event.materializationEpoch != null && workspaceRef.expectedMaterializationEpoch != null && event.materializationEpoch !== workspaceRef.expectedMaterializationEpoch) return;
    const key = activeSessionKey;
    const before = view.state.doc.toString();
    try {
      const after = await rewriteMarkdownImageLinks(workspaceRef, physicalPath.value, before, event.oldPath, event.newPath);
      if (editorView !== view || activeSessionKey !== key || after === before) return;
      const rebased = rebaseDocumentText(before, view.state.doc.toString(), after);
      if (rebased.conflicts.length) return;
      const changes = createMinimalTextChange(view.state.doc.toString(), rebased.text);
      if (changes) view.dispatch({ changes, annotations: Transaction.addToHistory.of(false) });
      emit("externalChange");
    } catch (error) { handleContextMenuError(error); }
  }).then(release => { if (editorUnmounted) release(); else releaseImageRenames = release; }).catch(handleContextMenuError);
  void subscribeWorkspaceFileChanges((event) => {
    if (!workspaceFileChangeInScope(event, props.workspaceRef) || !imageReferences.size) return;
    const path = event.payload.path.replace(/\\/g, "/").replace(/^\.\//, "").replace(/\/$/, "").toLocaleLowerCase();
    if (path !== "." && ![...imageReferences].some(reference => reference === path
      || reference.startsWith(`${path}/`) || reference.endsWith(`/${path}`) || reference.includes(`/${path}/`))) return;
    if (imageRefreshTimer !== null) return;
    imageRefreshTimer = setTimeout(() => {
      imageRefreshTimer = null;
      if (editorUnmounted) return;
      imageCacheEpoch.value += 1;
      reconfigureEditor();
    }, 100);
  }, () => props.workspaceRef).then((release) => {
    if (editorUnmounted) release();
    else releaseWorkspaceFileChanges = release;
  });
});

watch(
  () => [props.contentKey, physicalPath.value, props.workspaceRef?.checkoutId,
    props.workspaceRef?.expectedGeneration, props.workspaceRef?.expectedMaterializationEpoch],
  () => {
    imageReferences = new Set();
    imageCacheEpoch.value += 1;
    if (imageRefreshTimer !== null) clearTimeout(imageRefreshTimer);
    imageRefreshTimer = null;
  },
  { flush: "sync" },
);

watch(
  () => props.active,
  (active) => {
    if (!active) {
      suspendEditor();
      return;
    }
    activeSessionKey = normalizeSessionKey(props.contentKey);
    activeSessionCache = props.sessionCache ?? localSessionCache;
    activeSessionPinned = props.sessionPinned;
    mountEditor();
  },
);

watch(
  () => [props.contentKey, props.modelValue] as const,
  ([nextKey, nextValue], [previousKey, previousValue]) => {
    if (nextKey !== previousKey) {
      restoreSession(nextKey, nextValue, previousValue);
      return;
    }
    if (nextValue !== previousValue) syncOrQueueExternalModel(nextValue);
  },
);

watch(
  () => [
    props.viewMode,
    documentEditingSettings.punctuationPairing,
    props.comments,
    props.showProperties,
    props.disabled,
    props.placeholder,
    props.contentPath,
    props.resourcePath,
    props.workspaceRef?.checkoutId,
    props.workspaceRef?.expectedGeneration,
    props.workspaceRef?.expectedMaterializationEpoch,
  ] as const,
  () => reconfigureEditor(),
);

watch(
  () => props.sessionPinned,
  (pinned) => {
    activeSessionPinned = pinned;
    if (editorView) {
      saveActiveSession(props.modelValue, pinned);
      return;
    }
    activeSessionCache.setPinned(activeSessionKey, pinned);
  },
);

watch(
  () => [props.autoGrow, props.minHeight] as const,
  () => {
    void nextTick(() => {
      if (!editorView) return;
      startScrollTracking(editorView);
      editorView.requestMeasure();
    });
  },
);

watch(textViewerZoomScale, () => {
  void nextTick(() => editorView?.requestMeasure());
});

onBeforeUnmount(() => {
  editorUnmounted = true;
  selectionDocument?.removeEventListener("selectionchange", handleDocumentSelectionChange);
  selectionDocument = null;
  if (selectionChangeTimer) clearTimeout(selectionChangeTimer);
  selectionChangeTimer = null;
  releaseWorkspaceFileChanges?.();
  releaseWorkspaceFileChanges = null;
  if (imageRefreshTimer !== null) clearTimeout(imageRefreshTimer);
  imageRefreshTimer = null;
  releaseImageRenames?.();
  suspendEditor();
  releaseReadingCapture?.();
  releaseReadingCapture = null;
  if (readingCaptureTimer !== null) clearTimeout(readingCaptureTimer);
  readingCaptureTimer = null;
  localSessionCache.clear();
});

// A renamed document keeps its reading position under the new identity.
watch(
  () => props.readingPosition ? readingTargetKey(props.readingPosition) : "",
  (next, previous) => {
    const target = props.readingPosition ?? null;
    const old = readingTarget;
    // A content switch already bound the new target.
    if (!editorView || next === previous || (old ? readingTargetKey(old) : "") === next) return;
    readingTarget = target;
    if (!old && target && readingRestoreAwaitingTarget) {
      requestReadingRestore(editorView);
      return;
    }
    if (!old || !target || readingRestorePending) return;
    const anchor = captureReadingNow();
    if (anchor === undefined) return;
    putReadingPosition(target, anchor);
    putReadingPosition(old, null);
  },
  { flush: "post" },
);

defineExpose({
  getEditorView: () => editorView,
  getBodyElement: () => mountRef.value,
  /** Fresh anchor at the viewport top; null at the top or without a view. */
  captureReadingAnchor: (): ReadingAnchor | null => captureReadingNow() ?? null,
  /** Last anchor recorded or restored, without reading layout. */
  lastReadingAnchor: (): ReadingAnchor | null => lastReadingAnchor,
  restoreReadingAnchor,
  cancelReadingRestore,
});
</script>

<template>
  <div
    class="base-markdown-editor"
    ref="searchRoot"
    :class="{
      disabled,
      'auto-grow': autoGrow,
      'is-rendered': viewMode === 'rendered',
      'is-source': viewMode === 'native',
    }"
    :style="{
      '--markdown-editor-min-height': `${minHeight}px`,
      ...textViewerZoomStyle,
    }"
    @wheel="handleEditorWheel"
  >
    <div v-if="recovery.pending.value" class="markdown-recovery-bar" role="status">
      <span>有未保存的草稿</span>
      <BaseButton size="sm" @click="recovery.restore(recovery.pending.value!)">恢复草稿</BaseButton>
      <BaseButton size="sm" @click="recovery.dismiss">保留当前内容</BaseButton>
    </div>
    <div v-if="recovery.error.value" class="markdown-recovery-bar" role="alert">{{ recovery.error.value }}</div>
    <section v-if="recovery.historyOpen.value" class="markdown-recovery-history" aria-label="版本历史">
      <header><span>版本历史</span><BaseButton size="sm" @click="recovery.historyOpen.value = false">关闭</BaseButton></header>
      <p v-if="!recovery.snapshots.value.length">暂无历史版本</p>
      <details v-for="snapshot in [...recovery.snapshots.value].reverse()" :key="snapshot.at">
        <summary>{{ new Date(snapshot.at).toLocaleString() }}</summary>
        <pre>{{ snapshot.text }}</pre><BaseButton size="sm" :disabled="disabled" @click="recovery.restore(snapshot)">恢复此版本</BaseButton>
      </details>
    </section>
    <MarkdownProperties
      v-if="showProperties && frontmatter && viewMode === 'rendered' && (!disabled || frontmatter.range)"
      :key="contentKey"
      :yaml="frontmatter.yaml"
      :disabled="disabled"
      :error="frontmatter.invalid ? t('editor.properties.invalidYaml') : ''"
      @change="updateProperties"
      @shortcut-save="emitShortcutSave"
    />
    <div ref="mountRef" class="base-markdown-editor-host" @contextmenu="contextMenuRef?.open($event)" @keydown="contextMenuRef?.onKeydown($event)" />
    <MarkdownEditorContextMenu v-if="controlsView" ref="contextMenuRef" :view="controlsView" :can-history="!!physicalPath && currentLanguage() === 'markdown'" :can-quote="canQuoteSelection" :can-quote-in-new-session="canQuoteSelectionInNewSession" @history="recovery.openHistory" @quote-selection="emit('quoteSelection', $event)" @quote-selection-in-new-session="emit('quoteSelectionInNewSession', $event)" @action-error="handleContextMenuError" :can-comment="!!comments && currentLanguage() === 'markdown' && !disabled" :can-footnote="currentLanguage() === 'markdown' && !disabled" @comment="startComment" @footnote="insertFootnoteAtCursor" />
    <MarkdownCommentPopover v-if="comments && controlsView && currentLanguage() === 'markdown'" :controller="comments" />
    <MarkdownFootnotePopover
      v-if="footnotePopover && controlsView"
      :key="`${footnotePopover.key}:${footnotePopover.anchor.left}:${footnotePopover.anchor.top}`"
      :view="controlsView"
      :footnote-key="footnotePopover.key"
      :label="footnotePopover.label"
      :anchor="footnotePopover.anchor"
      :inserted-doc="footnotePopover.insertedDoc"
      @close="footnotePopover = null"
    />
    <MarkdownImageFolderPopover
      v-if="imageFolderPrompt"
      :workspace-ref="imageFolderPrompt.workspaceRef"
      :content-path="imageFolderPrompt.contentPath"
      :anchor="imageFolderPrompt.anchor"
      @done="onImageFolderDone"
    />
    <MarkdownEditorControls v-if="controlsView && viewMode === 'rendered' && currentLanguage() === 'markdown' && !disabled" :key="contentKey" ref="controlsRef" :view="controlsView" :workspace-ref="workspaceRef" :content-path="physicalPath" />
  </div>
</template>

<style scoped>
.markdown-recovery-bar { display: flex; align-items: center; gap: 8px; padding: 6px 12px; color: var(--text-secondary); font-size: 12px; }
.markdown-recovery-history { padding: 12px; border-bottom: 1px solid var(--border-color); font-size: 12px; }
.markdown-recovery-history header { display: flex; justify-content: space-between; align-items: center; }
.markdown-recovery-history details { margin-top: 8px; }
.markdown-recovery-history summary { cursor: pointer; }
.markdown-recovery-history pre { max-height: 240px; overflow: auto; white-space: pre-wrap; font-family: var(--font-mono-editor); }
.base-markdown-editor {
  --markdown-document-font-size: calc(14px * var(--text-viewer-font-scale, 1));
  --markdown-source-font-size: calc(13px * var(--text-viewer-font-scale, 1));
  --markdown-document-line-height: 1.68;
  --markdown-document-list-indent: 20px;
  --markdown-document-block-spacing: 8px;
  --markdown-document-padding-left: 16px;
  --markdown-document-padding-right: 14px;
  display: flex;
  flex: 1 1 0;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: transparent;
}

.base-markdown-editor-host {
  display: flex;
  flex: 1;
  width: 100%;
  min-width: 0;
  min-height: 0;
}

.base-markdown-editor :deep(.cm-editor) {
  flex: 1;
  width: 100%;
  min-width: 0;
  min-height: var(--markdown-editor-min-height);
  font-size: var(--markdown-document-font-size);
  background: transparent;
}

.base-markdown-editor :deep(.cm-scroller) {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
  font-family: var(--font-prose);
  font-size: inherit;
  line-height: var(--markdown-document-line-height);
}

.base-markdown-editor :deep(.cm-content) {
  min-height: 100%;
  padding: 14px 0 16px;
  color: var(--text-color);
  caret-color: var(--accent-color);
}

.base-markdown-editor :deep(.cm-line) {
  /* CodeMirror derives full-line selection bounds from .cm-line padding. */
  padding: 0 var(--markdown-document-padding-right) 0 var(--markdown-document-padding-left);
}

.base-markdown-editor :deep(.cm-line.cm-live-table-line) {
  padding: 0;
}

.base-markdown-editor :deep(.cm-live-preview .cm-widgetBuffer) {
  /* Keep the 1em caret target inside the line box. text-top alignment can
     enlarge it when hidden syntax is replaced beside mixed-font text. */
  vertical-align: -0.2em;
}

.base-markdown-editor.disabled,
.base-markdown-editor.disabled :deep(.cm-content) {
  cursor: default;
}

.base-markdown-editor :deep(.cm-editor.cm-source-mode) {
  font-size: var(--markdown-source-font-size);
}

.base-markdown-editor :deep(.cm-editor.cm-source-mode .cm-scroller) {
  font-family: var(--font-mono-editor);
  font-size: inherit;
  line-height: 1.65;
}

.base-markdown-editor.auto-grow {
  flex: none;
  height: auto;
  min-height: var(--markdown-editor-min-height);
}

.base-markdown-editor.auto-grow .base-markdown-editor-host,
.base-markdown-editor.auto-grow :deep(.cm-editor),
.base-markdown-editor.auto-grow :deep(.cm-scroller) {
  flex: none;
  height: auto;
  min-height: var(--markdown-editor-min-height);
  overflow: visible;
  overscroll-behavior: auto;
}

.base-markdown-editor.auto-grow :deep(.cm-content) {
  min-height: var(--markdown-editor-min-height);
}

.base-markdown-editor :deep(.cm-live-heading) {
  color: var(--text-color);
  font-weight: 600;
  line-height: 1.35;
  padding-top: 14px;
  padding-bottom: 6px;
}

.base-markdown-editor :deep(.cm-live-heading-1) {
  font-size: 1.4em;
  padding-bottom: 10px;
}

.base-markdown-editor :deep(.cm-live-heading-2) {
  font-size: 1.2em;
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
}

.base-markdown-editor :deep(.cm-live-heading-3) {
  font-size: 1.08em;
}

.base-markdown-editor :deep(.cm-live-heading-4),
.base-markdown-editor :deep(.cm-live-heading-5),
.base-markdown-editor :deep(.cm-live-heading-6) {
  font-size: 1em;
  color: var(--text-secondary);
}

.base-markdown-editor :deep(.cm-live-heading-marker),
.base-markdown-editor :deep(.cm-live-syntax-marker) {
  color: color-mix(in srgb, var(--accent-color) 68%, var(--text-secondary) 32%);
  font-weight: 400;
  font-style: normal;
  /* Revealing syntax may consume width, but must not add font leading. */
  line-height: 0;
}

.base-markdown-editor :deep(.cm-live-heading-marker) {
  font-family: var(--font-mono-inline);
  font-size: 0.85em;
}

.base-markdown-editor :deep(.cm-live-heading-marker span),
.base-markdown-editor :deep(.cm-live-syntax-marker span) {
  color: inherit;
  font-weight: inherit;
  font-style: inherit;
}

.base-markdown-editor :deep(.cm-live-strong) {
  font-weight: 600;
}

.base-markdown-editor :deep(.cm-live-emphasis) {
  color: color-mix(in srgb, var(--text-color) 82%, var(--text-secondary) 18%);
  font-style: italic;
}

.base-markdown-editor :deep(.cm-live-strikethrough) {
  text-decoration: line-through;
  text-decoration-color: color-mix(in srgb, var(--text-secondary) 78%, transparent);
}

.base-markdown-editor :deep(.cm-live-inline-code) {
  padding: 1px 5px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg) 52%, transparent);
  color: color-mix(in srgb, var(--text-color) 92%, var(--accent-color) 8%);
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
}

.base-markdown-editor :deep(.cm-live-list-marker) {
  display: inline-block;
  min-width: var(--markdown-document-list-indent);
  color: var(--text-secondary);
  font-weight: 600;
  text-align: center;
  user-select: none;
}

.base-markdown-editor :deep(.cm-live-task-checkbox) {
  width: 14px;
  height: 14px;
  margin: 0 6px 0 1px;
  vertical-align: -2px;
  accent-color: var(--accent-color);
}

.base-markdown-editor :deep(.cm-live-blockquote) {
  margin-right: var(--markdown-document-padding-right);
  margin-left: var(--markdown-document-padding-left);
  padding-right: 0;
  padding-left: 12px;
  border-left: 2px solid color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
  background: color-mix(in srgb, var(--sidebar-bg) 44%, transparent);
  color: var(--text-secondary);
}

.base-markdown-editor :deep(.cm-live-blockquote-start) {
  /* CodeMirror measures line boxes without margins for pointer/caret mapping. */
  padding-top: var(--markdown-document-block-spacing);
}

.base-markdown-editor :deep(.cm-live-blockquote-end) {
  padding-bottom: var(--markdown-document-block-spacing);
}

.base-markdown-editor :deep(.cm-live-horizontal-rule) {
  display: inline-block;
  width: 100%;
  height: 0.8em;
  border-top: 1px solid var(--border-color);
  opacity: 0.8;
  vertical-align: middle;
}

.base-markdown-editor :deep(.cm-live-fenced-code) {
  margin-right: var(--markdown-document-padding-right);
  margin-left: var(--markdown-document-padding-left);
  padding: 0 12px;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-left: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 76%, transparent);
  font-family: var(--font-mono-block);
  font-size: 13px;
  line-height: 1.55;
}

.base-markdown-editor :deep(.cm-live-fenced-code-start) {
  padding-top: 9px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px 8px 0 0;
}

.base-markdown-editor :deep(.cm-live-fenced-code-end) {
  padding-bottom: 9px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 0 0 8px 8px;
}
</style>
