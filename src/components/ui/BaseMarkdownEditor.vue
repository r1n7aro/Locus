<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Vditor from "vditor";
import "vditor/dist/index.css";
import { semanticCodeLanguageFromPath } from "../../composables/semanticCodeRendering";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import SemanticCodeRenderer from "./SemanticCodeRenderer.vue";
import {
  canSyncMarkdownEditorWhileFocused,
  normalizeMarkdownEditorLineEndings,
  shouldPreferMarkdownPlainTextPaste,
} from "./markdownEditorFormatting";
import {
  applyMarkdownEditorPanelLayout,
  createMarkdownEditorResizeSync,
  MARKDOWN_EDITOR_PANEL_HEIGHT,
  MARKDOWN_EDITOR_PANEL_MAX_WIDTH,
} from "./markdownEditorLayout";
import type { MarkdownEditorViewMode } from "./markdownEditorViewMode";

const props = withDefaults(defineProps<{
  modelValue: string;
  disabled?: boolean;
  placeholder?: string;
  viewMode?: MarkdownEditorViewMode;
  contentPath?: string;
}>(), {
  disabled: false,
  placeholder: "",
  viewMode: "rendered",
  contentPath: "",
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "shortcutSave"): void;
}>();

const mountRef = ref<HTMLDivElement | null>(null);
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const editorReady = ref(false);
const focused = ref(false);
const syncing = ref(false);
const pendingModelValue = ref<string | null>(null);
const isNativeMode = computed(() => props.viewMode === "native");
const isReadonlyRenderedMode = computed(() => props.disabled && props.viewMode === "rendered");
const readonlyCodeLanguage = computed(() =>
  isReadonlyRenderedMode.value ? semanticCodeLanguageFromPath(props.contentPath) : null
);
const shouldUseVditor = computed(() => !isNativeMode.value && !isReadonlyRenderedMode.value);
let editor: Vditor | null = null;
let themeObserver: MutationObserver | null = null;
let layoutSync: { disconnect(): void } | null = null;
let pasteInterceptorCleanup: (() => void) | null = null;

const editorCdnBase = computed(() => {
  const base = import.meta.env.BASE_URL || "/";
  return new URL(`${base}vendor/vditor`, window.location.origin).toString().replace(/\/$/, "");
});

function normalizeMarkdown(value: string): string {
  return normalizeMarkdownEditorLineEndings(value);
}

function isSaveShortcut(event: KeyboardEvent) {
  return (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s";
}

function isDarkTheme() {
  return document.documentElement.dataset.theme === "dark";
}

function currentEditorValue() {
  return editor ? normalizeMarkdown(editor.getValue()) : normalizeMarkdown(props.modelValue);
}

function syncDisabledState() {
  if (!editor) return;
  if (props.disabled) {
    editor.disabled();
    return;
  }
  editor.enable();
}

function syncSpellcheckState() {
  const editable = mountRef.value?.querySelector<HTMLElement>(".vditor-ir .vditor-reset");
  if (!editable) return;
  editable.setAttribute("spellcheck", "false");
}

function installPasteInterceptor() {
  pasteInterceptorCleanup?.();
  pasteInterceptorCleanup = null;

  const editable = mountRef.value?.querySelector<HTMLElement>(".vditor-ir .vditor-reset");
  if (!editable) return;

  const onPaste = (event: ClipboardEvent) => {
    const html = event.clipboardData?.getData("text/html") ?? "";
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (!editor || props.disabled || !shouldPreferMarkdownPlainTextPaste(html, text)) {
      return;
    }

    // Avoid Vditor wrapping prose copied from preformatted HTML in a fenced code block.
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
    editor.insertMD(text);
    emitMarkdown(editor.getValue());
  };

  editable.addEventListener("paste", onPaste, true);
  pasteInterceptorCleanup = () => {
    editable.removeEventListener("paste", onPaste, true);
  };
}

function applyTheme() {
  if (!editor) return;
  editor.setTheme(isDarkTheme() ? "dark" : "classic");
}

function syncPanelLayout() {
  applyMarkdownEditorPanelLayout(mountRef.value);
}

function destroyEditor() {
  pasteInterceptorCleanup?.();
  pasteInterceptorCleanup = null;
  layoutSync?.disconnect();
  layoutSync = null;
  editorReady.value = false;
  focused.value = false;
  syncing.value = false;
  pendingModelValue.value = null;
  editor?.destroy();
  editor = null;
}

function syncFromModel(nextValue: string, clearStack = false) {
  if (!editor) return;
  const normalizedNext = normalizeMarkdown(nextValue);
  if (currentEditorValue() === normalizedNext) {
    pendingModelValue.value = null;
    return;
  }
  syncing.value = true;
  editor.setValue(nextValue, clearStack);
  syncing.value = false;
  pendingModelValue.value = null;
}

function emitMarkdown(value?: string) {
  if (syncing.value) return;
  const nextValue = normalizeMarkdown(value ?? currentEditorValue());
  emit("update:modelValue", nextValue);
}

function emitShortcutSave(value?: string) {
  emitMarkdown(value);
  emit("shortcutSave");
}

function handleEditorKeydown(event: KeyboardEvent) {
  if (!isSaveShortcut(event)) return;
  event.preventDefault();
  emitShortcutSave();
}

function handleNativeKeydown(event: KeyboardEvent) {
  if (!isSaveShortcut(event)) return;
  event.preventDefault();
  emitShortcutSave(textareaRef.value?.value ?? props.modelValue);
}

function handleNativeInput(event: Event) {
  emitMarkdown((event.target as HTMLTextAreaElement | null)?.value ?? "");
}

function mountEditor() {
  const target = mountRef.value;
  if (!target || editor) return;

  editor = new Vditor(target, {
    value: props.modelValue,
    height: MARKDOWN_EDITOR_PANEL_HEIGHT,
    minHeight: 0,
    mode: "ir",
    lang: "zh_CN",
    cdn: editorCdnBase.value,
    toolbar: [],
    cache: {
      enable: false,
    },
    counter: {
      enable: false,
    },
    outline: {
      enable: false,
      position: "right",
    },
    preview: {
      mode: "editor",
      maxWidth: MARKDOWN_EDITOR_PANEL_MAX_WIDTH,
      hljs: {
        enable: false,
        lineNumber: false,
      },
      markdown: {
        toc: false,
      },
    },
    placeholder: props.placeholder,
    icon: "ant",
    theme: isDarkTheme() ? "dark" : "classic",
    input(value) {
      emitMarkdown(value);
    },
    focus() {
      focused.value = true;
    },
    blur(value) {
      focused.value = false;
      emitMarkdown(value);
      if (pendingModelValue.value !== null) {
        if (canSyncMarkdownEditorWhileFocused(currentEditorValue(), pendingModelValue.value)) {
          pendingModelValue.value = null;
          return;
        }
        syncFromModel(pendingModelValue.value);
      }
    },
    keydown(event) {
      handleEditorKeydown(event);
    },
    after() {
      editorReady.value = true;
      syncDisabledState();
      syncSpellcheckState();
      installPasteInterceptor();
      applyTheme();
      syncPanelLayout();
      layoutSync?.disconnect();
      layoutSync = createMarkdownEditorResizeSync(mountRef.value, syncPanelLayout);
    },
  });
}

watch(
  () => props.modelValue,
  (nextValue) => {
    if (!shouldUseVditor.value || !editorReady.value || !editor) return;
    const currentValue = currentEditorValue();
    if (canSyncMarkdownEditorWhileFocused(currentValue, nextValue)) {
      pendingModelValue.value = null;
      return;
    }
    if (focused.value) {
      pendingModelValue.value = nextValue;
      return;
    }
    syncFromModel(nextValue);
  },
);

watch(
  shouldUseVditor,
  (nextValue) => {
    if (!nextValue) {
      destroyEditor();
      return;
    }
    void nextTick(() => {
      mountEditor();
    });
  },
);

watch(
  () => props.disabled,
  () => {
    if (!editorReady.value) return;
    syncDisabledState();
  },
);

watch(
  () => props.placeholder,
  (placeholder) => {
    const editable = mountRef.value?.querySelector<HTMLElement>(".vditor-ir .vditor-reset");
    if (!editable) return;
    editable.setAttribute("placeholder", placeholder);
  },
);

onMounted(() => {
  themeObserver = new MutationObserver(() => applyTheme());
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  if (shouldUseVditor.value) mountEditor();
});

onBeforeUnmount(() => {
  themeObserver?.disconnect();
  themeObserver = null;
  destroyEditor();
});
</script>

<template>
  <div class="base-markdown-editor" :class="{ disabled, 'is-native': isNativeMode }">
    <div v-if="isNativeMode" class="base-markdown-editor-native">
      <textarea
        ref="textareaRef"
        class="base-markdown-editor-textarea"
        :value="modelValue"
        :disabled="disabled"
        :placeholder="placeholder"
        spellcheck="false"
        @input="handleNativeInput"
        @keydown="handleNativeKeydown"
      />
    </div>
    <div v-else-if="isReadonlyRenderedMode" class="base-markdown-editor-rendered">
      <SemanticCodeRenderer
        v-if="readonlyCodeLanguage"
        :content="modelValue"
        :language="readonlyCodeLanguage"
      />
      <MarkdownRenderer v-else :content="modelValue" />
    </div>
    <div v-else ref="mountRef" class="base-markdown-editor-host" />
  </div>
</template>

<style scoped>
.base-markdown-editor {
  height: 100%;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: transparent;
}

.base-markdown-editor.disabled {
  cursor: default;
}

.base-markdown-editor-native {
  flex: 1;
  display: flex;
  min-width: 0;
  min-height: 0;
}

.base-markdown-editor-host {
  flex: 1;
  height: 100%;
  min-height: 0;
  min-width: 0;
}

.base-markdown-editor-rendered {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: 14px 14px 16px 16px;
  overscroll-behavior: contain;
}

.base-markdown-editor-rendered :deep(.markdown-body) {
  min-height: 100%;
}

.base-markdown-editor-textarea {
  flex: 1;
  width: 100%;
  min-width: 0;
  min-height: 0;
  margin: 0;
  padding: 14px 14px 16px 16px;
  border: none;
  outline: none;
  resize: none;
  overflow: auto;
  background: transparent;
  color: var(--text-color);
  font-family: var(--font-mono-editor);
  font-size: 13px;
  line-height: 1.65;
  white-space: pre;
  tab-size: 2;
}

.base-markdown-editor-textarea::placeholder {
  color: var(--text-secondary);
  opacity: 0.55;
}

.base-markdown-editor-textarea:disabled {
  opacity: 1;
  cursor: default;
}

.base-markdown-editor :deep(.vditor) {
  --border-color: color-mix(in srgb, var(--border-color) 88%, transparent);
  --second-color: color-mix(in srgb, var(--text-secondary) 50%, transparent);
  --panel-background-color: color-mix(in srgb, var(--panel-bg) 96%, var(--sidebar-bg) 4%);
  --panel-shadow: 0 10px 24px color-mix(in srgb, var(--text-color) 14%, transparent);
  --toolbar-background-color: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  --toolbar-icon-color: var(--text-secondary);
  --toolbar-icon-hover-color: var(--text-color);
  --textarea-background-color: transparent;
  --textarea-text-color: var(--text-color);
  --resize-icon-color: var(--text-secondary);
  --resize-background-color: var(--border-color);
  --resize-hover-icon-color: var(--text-color);
  --resize-hover-background-color: color-mix(in srgb, var(--hover-bg) 84%, var(--panel-bg) 16%);
  --count-background-color: color-mix(in srgb, var(--hover-bg) 72%, transparent);
  --heading-border-color: color-mix(in srgb, var(--border-color) 84%, transparent);
  --blockquote-color: var(--text-secondary);
  --ir-heading-color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
  --ir-title-color: var(--text-secondary);
  --ir-bi-color: color-mix(in srgb, var(--accent-color) 52%, var(--text-color) 48%);
  --ir-link-color: color-mix(in srgb, var(--accent-color) 66%, var(--text-color) 34%);
  --ir-bracket-color: color-mix(in srgb, var(--accent-color) 48%, var(--text-color) 52%);
  --ir-paren-color: var(--text-secondary);
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border: none;
  background: transparent;
  font-family: var(--font-prose);
}

.base-markdown-editor :deep(.vditor-toolbar) {
  display: none;
}

.base-markdown-editor :deep(.vditor-content) {
  flex: 1;
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}

.base-markdown-editor :deep(.vditor-ir) {
  flex: 1;
  min-height: 0;
  padding: 0 !important;
  overflow: hidden;
}

.base-markdown-editor :deep(.vditor-ir pre.vditor-reset) {
  height: auto;
  min-height: 100%;
  margin: 0;
  padding: 14px 14px 16px 16px !important;
  overflow: auto;
  background: transparent;
  color: var(--text-color);
  font-family: var(--font-prose);
  font-size: 14px;
  line-height: 1.68;
  white-space: pre-wrap;
}

.base-markdown-editor :deep(.vditor-ir pre.vditor-reset:focus) {
  background: transparent;
  outline: none;
}

.base-markdown-editor :deep(.vditor-ir pre.vditor-reset[contenteditable="false"]) {
  opacity: 1;
  cursor: default;
}

.base-markdown-editor :deep(.vditor-ir pre.vditor-reset:empty::before) {
  color: var(--text-secondary);
  opacity: 0.55;
}

.base-markdown-editor :deep(.vditor-ir .vditor-reset > h1::before),
.base-markdown-editor :deep(.vditor-ir .vditor-reset > h2::before),
.base-markdown-editor :deep(.vditor-ir .vditor-reset > h3::before),
.base-markdown-editor :deep(.vditor-ir .vditor-reset > h4::before),
.base-markdown-editor :deep(.vditor-ir .vditor-reset > h5::before),
.base-markdown-editor :deep(.vditor-ir .vditor-reset > h6::before),
.base-markdown-editor :deep(.vditor-ir div[data-type="link-ref-defs-block"]::before),
.base-markdown-editor :deep(.vditor-ir div[data-type="footnotes-block"]::before),
.base-markdown-editor :deep(.vditor-ir .vditor-toc::before) {
  content: none;
  display: none;
}

.base-markdown-editor :deep(.vditor-reset h1),
.base-markdown-editor :deep(.vditor-reset h2),
.base-markdown-editor :deep(.vditor-reset h3),
.base-markdown-editor :deep(.vditor-reset h4),
.base-markdown-editor :deep(.vditor-reset h5),
.base-markdown-editor :deep(.vditor-reset h6) {
  margin: 24px 0 10px;
  font-weight: 600;
  line-height: 1.35;
  letter-spacing: -0.01em;
  color: var(--text-color);
}

.base-markdown-editor :deep(.vditor-reset h1:first-child),
.base-markdown-editor :deep(.vditor-reset h2:first-child),
.base-markdown-editor :deep(.vditor-reset h3:first-child),
.base-markdown-editor :deep(.vditor-reset h4:first-child),
.base-markdown-editor :deep(.vditor-reset h5:first-child),
.base-markdown-editor :deep(.vditor-reset h6:first-child),
.base-markdown-editor :deep(.vditor-reset p:first-child),
.base-markdown-editor :deep(.vditor-reset ul:first-child),
.base-markdown-editor :deep(.vditor-reset ol:first-child),
.base-markdown-editor :deep(.vditor-reset blockquote:first-child),
.base-markdown-editor :deep(.vditor-reset pre:first-child),
.base-markdown-editor :deep(.vditor-reset table:first-child) {
  margin-top: 0;
}

.base-markdown-editor :deep(.vditor-reset h1) {
  font-size: 1.58em;
  margin-bottom: 14px;
}

.base-markdown-editor :deep(.vditor-reset h2) {
  font-size: 1.3em;
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
}

.base-markdown-editor :deep(.vditor-reset h3) {
  font-size: 1.12em;
}

.base-markdown-editor :deep(.vditor-reset h4),
.base-markdown-editor :deep(.vditor-reset h5),
.base-markdown-editor :deep(.vditor-reset h6) {
  font-size: 1em;
  color: var(--text-secondary);
}

.base-markdown-editor :deep(.vditor-reset p),
.base-markdown-editor :deep(.vditor-reset ul),
.base-markdown-editor :deep(.vditor-reset ol),
.base-markdown-editor :deep(.vditor-reset blockquote),
.base-markdown-editor :deep(.vditor-reset hr),
.base-markdown-editor :deep(.vditor-reset pre),
.base-markdown-editor :deep(.vditor-reset table) {
  margin: 0 0 12px;
}

.base-markdown-editor :deep(.vditor-reset ul),
.base-markdown-editor :deep(.vditor-reset ol) {
  padding-left: 14px;
}

.base-markdown-editor :deep(.vditor-reset li) {
  margin: 4px 0;
}

.base-markdown-editor :deep(.vditor-reset li > ul),
.base-markdown-editor :deep(.vditor-reset li > ol) {
  margin-top: 6px;
  margin-bottom: 6px;
}

.base-markdown-editor :deep(.vditor-reset blockquote) {
  padding: 8px 12px;
  border-left: 2px solid color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg) 44%, transparent);
  border-radius: 0 6px 6px 0;
}

.base-markdown-editor :deep(.vditor-reset a) {
  color: var(--accent-color);
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.16em;
  text-decoration-color: color-mix(in srgb, var(--accent-color) 40%, transparent);
}

.base-markdown-editor :deep(.vditor-reset hr) {
  border: none;
  border-top: 1px solid var(--border-color);
  opacity: 0.8;
}

.base-markdown-editor :deep(.vditor-reset table) {
  width: max-content;
  min-width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  table-layout: auto;
  font-size: 13px;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.base-markdown-editor :deep(.vditor-reset th),
.base-markdown-editor :deep(.vditor-reset td) {
  min-width: 120px;
  padding: 7px 10px;
  text-align: left;
  vertical-align: top;
  white-space: normal;
  overflow-wrap: anywhere;
  word-break: normal;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  color: var(--text-color) !important;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%) !important;
}

.base-markdown-editor :deep(.vditor-reset th) {
  background: color-mix(in srgb, var(--sidebar-bg) 68%, var(--panel-bg) 32%) !important;
  font-weight: 600;
  color: var(--text-secondary) !important;
}

.base-markdown-editor :deep(.vditor-reset tr:last-child td) {
  border-bottom: none;
}

.base-markdown-editor :deep(.vditor-reset th:last-child),
.base-markdown-editor :deep(.vditor-reset td:last-child) {
  border-right: none;
}

.base-markdown-editor :deep(.vditor-reset tbody tr:nth-child(even) td) {
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--hover-bg) 18%) !important;
}

.base-markdown-editor :deep(.vditor-reset code:not(.hljs):not(.highlight-chroma)) {
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 52%, transparent);
  color: color-mix(in srgb, var(--text-color) 92%, var(--accent-color) 8%);
}

.base-markdown-editor :deep(.vditor-reset pre) {
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 76%, transparent);
  overflow-x: auto;
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--panel-bg) 32%, transparent);
}

.base-markdown-editor :deep(.vditor-reset pre > code) {
  display: block;
  padding: 10px 12px;
  background: transparent;
  color: inherit;
  font-family: var(--font-mono-block);
  font-size: 13px;
  line-height: 1.55;
}

.base-markdown-editor :deep(.vditor-reset strong) {
  font-weight: 600;
}

.base-markdown-editor :deep(.vditor-reset em) {
  color: color-mix(in srgb, var(--text-color) 82%, var(--text-secondary) 18%);
}
</style>
