<script setup lang="ts">
import { Compartment, EditorState, StateEffect, Transaction, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
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
import { resolveMarkdownImage } from "../../services/markdownImage";
import type { WorkspaceRef } from "../../services/project";
import { subscribeWorkspaceFileChanges } from "../../services/workspaceExplorer";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import { useTextViewerZoom } from "../../composables/useTextViewerZoom";
import { workspaceFileChangeMatches } from "../../composables/useFileChangeRevalidation";

const workspaceMarkdownImageResolver: MarkdownImageResolver = (source, context) => {
  if (!context.workspaceRef) return null;
  return resolveMarkdownImage(context.workspaceRef, source);
};

const props = withDefaults(defineProps<{
  modelValue: string;
  disabled?: boolean;
  placeholder?: string;
  viewMode?: MarkdownEditorViewMode;
  contentPath?: string;
  contentKey?: string;
  sessionCache?: MarkdownEditorSessionStore | null;
  sessionPinned?: boolean;
  workspaceRef?: WorkspaceRef | null;
  active?: boolean;
  autoGrow?: boolean;
  minHeight?: number;
  /**
   * Emits immutable CodeMirror Text/ChangeSet payloads for local edits. This
   * avoids allocating the whole document string on every transaction. The
   * model is materialized at an explicit shortcut-save boundary.
   */
  transactionModel?: boolean;
}>(), {
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
  transactionModel: false,
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "documentChange", value: MarkdownEditorDocumentChange): void;
  (e: "shortcutSave"): void;
  (e: "referenceOpen", reference: MarkdownReferenceToken): void;
  (e: "referencePointerDown", payload: {
    reference: MarkdownReferenceToken;
    event: PointerEvent;
    element: HTMLElement;
  }): void;
}>();

const mountRef = ref<HTMLDivElement | null>(null);
const languageCompartment = new Compartment();
const modeCompartment = new Compartment();
const readOnlyCompartment = new Compartment();
const placeholderCompartment = new Compartment();
const localSessionCache = new MarkdownEditorSessionCache();
const imageCacheEpoch = ref(0);
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
let pendingExternalModel: string | null = null;
let compositionFlushToken = 0;
let currentScrollTop = 0;
let currentScrollLeft = 0;
let removeScrollTracking: (() => void) | null = null;
let activeScrollElement: HTMLElement | null = null;
let releaseWorkspaceFileChanges: RuntimeUnsubscribe | null = null;
let editorUnmounted = false;

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
  emit("referenceOpen", reference);
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
    onReferenceOpen: handleReferenceOpen,
    onReferencePointerDown: handleReferencePointerDown,
  };
  if (!workspaceRef) return options;
  return {
    ...options,
    imageResolver: workspaceMarkdownImageResolver,
    imageContext: {
      cacheKey: `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}:${imageCacheEpoch.value}`,
      contentPath: props.contentPath,
      workspaceRef: {
        checkoutId: workspaceRef.checkoutId,
        expectedGeneration: workspaceRef.expectedGeneration,
      },
    },
  };
}

function emitCurrentValue(): void {
  if (!editorView) return;
  emit("update:modelValue", normalizeMarkdown(editorView.state.doc.toString()));
}

function emitShortcutSave(): void {
  emitCurrentValue();
  emit("shortcutSave");
}

function flushPendingExternalModel(): void {
  const pending = pendingExternalModel;
  pendingExternalModel = null;
  if (pending === null) return;
  syncExternalModel(pending);
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
    languageCompartment.of(markdownEditorLanguageExtension(language)),
    modeCompartment.of(markdownEditorModeExtension(
      props.viewMode,
      language,
      currentLivePreviewOptions(),
    )),
    readOnlyCompartment.of(markdownEditorReadOnlyExtension(props.disabled)),
    placeholderCompartment.of(markdownEditorPlaceholderExtension(props.placeholder)),
    EditorView.domEventHandlers({
      compositionstart: handleCompositionStart,
      compositionend: handleCompositionEnd,
    }),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged || applyingExternalModel) return;
      if (props.transactionModel) {
        emit("documentChange", {
          doc: update.state.doc,
          changes: update.changes,
        });
        return;
      }
      emit("update:modelValue", normalizeMarkdown(update.state.doc.toString()));
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
    languageCompartment.reconfigure(markdownEditorLanguageExtension(language)),
    modeCompartment.reconfigure(markdownEditorModeExtension(
      props.viewMode,
      language,
      currentLivePreviewOptions(),
    )),
    readOnlyCompartment.reconfigure(markdownEditorReadOnlyExtension(props.disabled)),
    placeholderCompartment.reconfigure(markdownEditorPlaceholderExtension(props.placeholder)),
  ];
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
  if (composing && editorView) {
    pendingExternalModel = normalized;
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
  };
  scrollElement.addEventListener("scroll", handleScroll, { passive: true });
  removeScrollTracking = () => {
    scrollElement.removeEventListener("scroll", handleScroll);
    if (activeScrollElement === scrollElement) activeScrollElement = null;
    removeScrollTracking = null;
  };
}

function restoreTrackedScroll(view: EditorView, top: number, left: number): void {
  const scrollElement = activeScrollElement ?? resolveScrollElement(view);
  scrollElement.scrollTop = top;
  scrollElement.scrollLeft = left;
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
  const view = editorView;
  if (!view) {
    activeSessionKey = normalizeSessionKey(contentKey);
    return;
  }

  resetCompositionState(true);
  saveActiveSession(previousModelValue, activeSessionPinned);
  activeSessionKey = normalizeSessionKey(contentKey);
  activeSessionCache = props.sessionCache ?? localSessionCache;
  activeSessionPinned = props.sessionPinned;
  activeSessionCache.setPinned(activeSessionKey, activeSessionPinned);
  const cached = activeSessionCache.get(activeSessionKey);
  // A newly created state already contains the current document and every
  // compartment. Re-applying both here duplicates the most expensive work for
  // large documents (full-text comparison plus language/plugin setup).
  let nextState = createEditorState(nextModelValue);
  if (cached) {
    nextState = stateWithSessionModel(cached.state, cached.modelValue, nextModelValue);
    nextState = stateWithFreshConfiguration(nextState);
  }
  view.setState(nextState);

  const scrollTop = cached?.scrollTop ?? 0;
  const scrollLeft = cached?.scrollLeft ?? 0;
  currentScrollTop = scrollTop;
  currentScrollLeft = scrollLeft;
  const restoredKey = activeSessionKey;
  view.requestMeasure({
    read: () => null,
    write() {
      if (!editorView || activeSessionKey !== restoredKey) return;
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
  let state = createEditorState(props.modelValue);
  if (cached) {
    state = stateWithSessionModel(cached.state, cached.modelValue, props.modelValue);
    state = stateWithFreshConfiguration(state);
  }
  editorView = new EditorView({ state, parent });
  startScrollTracking(editorView);

  const scrollTop = cached?.scrollTop ?? 0;
  const scrollLeft = cached?.scrollLeft ?? 0;
  currentScrollTop = scrollTop;
  currentScrollLeft = scrollLeft;
  const restoredKey = activeSessionKey;
  editorView.requestMeasure({
    read: () => null,
    write() {
      if (!editorView || activeSessionKey !== restoredKey) return;
      restoreTrackedScroll(editorView, scrollTop, scrollLeft);
    },
  });
}

function suspendEditor(): void {
  if (!editorView) return;
  resetCompositionState(true);
  saveActiveSession();
  removeScrollTracking?.();
  editorView.destroy();
  editorView = null;
}

function reconfigureEditor(): void {
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

onMounted(() => {
  mountEditor();
  void subscribeWorkspaceFileChanges((event) => {
    if (!workspaceFileChangeMatches(event, props.workspaceRef, event.payload.path)) return;
    if (!/\.(?:png|jpe?g|gif|bmp|webp|svg)$/i.test(event.payload.path)) return;
    const source = (editorView?.state.doc.toString() ?? props.modelValue).replace(/\\/g, "/");
    if (!source.toLocaleLowerCase().includes(event.payload.path.toLocaleLowerCase())) return;
    imageCacheEpoch.value += 1;
    reconfigureEditor();
  }).then((release) => {
    if (editorUnmounted) release();
    else releaseWorkspaceFileChanges = release;
  });
});

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
    props.disabled,
    props.placeholder,
    props.contentPath,
    props.workspaceRef?.checkoutId,
    props.workspaceRef?.expectedGeneration,
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
  releaseWorkspaceFileChanges?.();
  releaseWorkspaceFileChanges = null;
  suspendEditor();
  localSessionCache.clear();
});

defineExpose({
  getEditorView: () => editorView,
});
</script>

<template>
  <div
    class="base-markdown-editor"
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
    <div ref="mountRef" class="base-markdown-editor-host" />
  </div>
</template>

<style scoped>
.base-markdown-editor {
  --markdown-document-font-size: calc(14px * var(--text-viewer-font-scale, 1));
  --markdown-source-font-size: calc(13px * var(--text-viewer-font-scale, 1));
  --markdown-document-line-height: 1.68;
  --markdown-document-list-indent: 20px;
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
  font-size: 1.58em;
  padding-bottom: 10px;
}

.base-markdown-editor :deep(.cm-live-heading-2) {
  font-size: 1.3em;
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
}

.base-markdown-editor :deep(.cm-live-heading-3) {
  font-size: 1.12em;
}

.base-markdown-editor :deep(.cm-live-heading-4),
.base-markdown-editor :deep(.cm-live-heading-5),
.base-markdown-editor :deep(.cm-live-heading-6) {
  font-size: 1em;
  color: var(--text-secondary);
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

.base-markdown-editor :deep(.cm-live-link) {
  color: var(--accent-color);
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.16em;
  text-decoration-color: color-mix(in srgb, var(--accent-color) 40%, transparent);
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
