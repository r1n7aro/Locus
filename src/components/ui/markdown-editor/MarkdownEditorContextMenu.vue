<script setup lang="ts">
import { redo, redoDepth, selectAll, undo, undoDepth } from "@codemirror/commands";
import type { EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { nextTick, onBeforeUnmount, ref, shallowRef, useId, watch } from "vue";
import {
  BetweenHorizontalEnd, BetweenHorizontalStart, BetweenVerticalEnd, BetweenVerticalStart,
  ClipboardPaste, Copy, History, MessageSquarePlus, MessageSquareQuote, Redo2, Scissors, SquareDashedMousePointer, Superscript, TextQuote, Trash2, Undo2,
  type IconNode,
} from "lucide";
import { t } from "../../../i18n";
import LucideIcon from "../../icons/LucideIcon.vue";
import BaseContextMenu from "../BaseContextMenu.vue";
import { normalizeMarkdownEditorLineEndings } from "../markdownEditorFormatting";
import { captureMarkdownEditorSelection, syncMarkdownDOMSelection, type MarkdownEditorSelection } from "./markdownEditorSelection";
import { editMarkdownTable, markdownTableTarget, type MarkdownTableAction, type MarkdownTableTarget } from "./markdownTableEditing";

const props = defineProps<{ view: EditorView; canQuote?: boolean; canQuoteInNewSession?: boolean; canHistory?: boolean; canComment?: boolean; canFootnote?: boolean }>();
const emit = defineEmits<{
  quoteSelection: [selection: MarkdownEditorSelection];
  quoteSelectionInNewSession: [selection: MarkdownEditorSelection];
  actionError: [error: unknown];
  history: [];
  comment: [];
  footnote: [];
}>();
const position = ref<{ x: number; y: number } | null>(null);
const menuId = useId();
const capturedState = shallowRef<EditorState | null>(null);
const hasSelection = ref(false);
const canUndo = ref(false);
const canRedo = ref(false);
const readOnly = ref(false);
const tableTarget = shallowRef<MarkdownTableTarget | null>(null);
const tableActions: MarkdownTableAction[] = ["row-before", "row-after", "row-delete", "column-before", "column-after", "column-delete", "table-delete"];
const actions = ["undo", "redo", "cut", "copy", "paste", "delete", "selectAll"] as const;
type EditorAction = typeof actions[number] | "quoteSelection" | "quoteSelectionInNewSession";
const actionIcons: Record<EditorAction, IconNode> = {
  undo: Undo2, redo: Redo2, cut: Scissors, copy: Copy, paste: ClipboardPaste,
  delete: Trash2, selectAll: SquareDashedMousePointer, quoteSelection: TextQuote,
  quoteSelectionInNewSession: MessageSquareQuote,
};
const tableActionIcons: Record<MarkdownTableAction, IconNode> = {
  "row-before": BetweenHorizontalStart,
  "row-after": BetweenHorizontalEnd,
  "row-delete": Trash2,
  "column-before": BetweenVerticalStart,
  "column-after": BetweenVerticalEnd,
  "column-delete": Trash2,
  "table-delete": Trash2,
};
const shortcuts: Record<string, string> = {
  undo: "Ctrl+Z", redo: "Ctrl+Y", cut: "Ctrl+X", copy: "Ctrl+C", paste: "Ctrl+V", delete: "Del", selectAll: "Ctrl+A",
};
let disposed = false;

function isDisabled(action: EditorAction): boolean {
  if (action === "copy" || action === "quoteSelection" || action === "quoteSelectionInNewSession") return !hasSelection.value;
  if (action === "selectAll") return false;
  if (readOnly.value) return true;
  if (action === "undo") return !canUndo.value;
  if (action === "redo") return !canRedo.value;
  return (action === "cut" || action === "delete") && !hasSelection.value;
}

function close(restoreFocus = true): void {
  position.value = null;
  capturedState.value = null;
  tableTarget.value = null;
  if (restoreFocus && !disposed) props.view.focus();
}

async function open(event: MouseEvent | KeyboardEvent): Promise<void> {
  const target = event.target;
  if (target instanceof Element && target.closest("input, textarea, button, a") && !target.closest(".cm-live-table-cell")) return;
  event.preventDefault();
  event.stopPropagation();
  syncMarkdownDOMSelection(props.view);
  const state = props.view.state;
  capturedState.value = state;
  hasSelection.value = state.selection.ranges.some((range) => !range.empty);
  canUndo.value = undoDepth(state) > 0;
  canRedo.value = redoDepth(state) > 0;
  readOnly.value = state.readOnly;
  tableTarget.value = state.readOnly ? null : markdownTableTarget(props.view, event instanceof MouseEvent && target instanceof Element ? target : undefined);
  const coords = event instanceof MouseEvent ? { x: event.clientX, y: event.clientY } : null;
  const caret = coords ? null : props.view.coordsAtPos(state.selection.main.head);
  position.value = coords ?? { x: caret?.left ?? 0, y: caret?.bottom ?? 0 };
  await nextTick();
  // The menu is teleported. Limit focus lookup to this menu's unique marker.
  document.getElementById(menuId)?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) void open(event);
}

function navigateMenu(event: KeyboardEvent): void {
  if (event.key === "Tab") { close(); return; }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const root = (event.target as HTMLElement).closest(".markdown-editor-context-menu");
  const buttons = Array.from(root?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
  if (!buttons.length) return;
  event.preventDefault();
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
    : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
  buttons[next]?.focus();
}

function runAnnotation(kind: "comment" | "footnote"): void {
  const state = capturedState.value;
  const view = props.view;
  close();
  if (!state || view.state !== state) return;
  if (kind === "comment") emit("comment");
  else emit("footnote");
}

function runTable(action: MarkdownTableAction): void {
  const state = capturedState.value;
  const target = tableTarget.value;
  const view = props.view;
  close();
  if (!state || view.state !== state || !target) return;
  editMarkdownTable(view, action, target);
}

async function run(action: EditorAction): Promise<void> {
  const state = capturedState.value;
  const view = props.view;
  close();
  if (!state || view.state !== state) return;
  const selection = captureMarkdownEditorSelection(state);
  try {
    if (action === "quoteSelection") {
      if (props.canQuote && selection.ranges.length) emit("quoteSelection", selection);
      return;
    }
    if (action === "quoteSelectionInNewSession") {
      if (props.canQuoteInNewSession && selection.ranges.length) emit("quoteSelectionInNewSession", selection);
      return;
    }
    if (action === "copy" || action === "cut") {
      if (!selection.ranges.length || (action === "cut" && state.readOnly)) return;
      await navigator.clipboard.writeText(selection.ranges.map((range) => range.text).join("\n"));
      // Clipboard access is asynchronous: never delete a different document/selection.
      if (action === "copy" || disposed || view.state !== state || view.state.readOnly) return;
    }
    if (action === "paste") {
      if (state.readOnly) return;
      let text = "";
      let html = "";
      if (navigator.clipboard.read) {
        try {
          const items = await navigator.clipboard.read();
          const images: File[] = [];
          for (const entry of items) {
            const type = entry.types.find(type => type.startsWith("image/"));
            if (type) images.push(new File([await entry.getType(type)], `image.${type.split("/")[1]}`, { type }));
          }
          if (images.length) {
            if (disposed || view.state !== state || view.state.readOnly) return;
            const clipboardData = new DataTransfer();
            for (const file of images) clipboardData.items.add(file);
            if (!view.contentDOM.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }))) return;
          }
          const item = items.find((candidate) => candidate.types.includes("text/plain") || candidate.types.includes("text/html"));
          if (item?.types.includes("text/plain")) text = await (await item.getType("text/plain")).text();
          if (item?.types.includes("text/html")) html = await (await item.getType("text/html")).text();
        } catch {
          // Some WebView2 versions expose only the plain-text clipboard API.
        }
      }
      if (!text && !html) text = await navigator.clipboard.readText();
      if (disposed || view.state !== state || view.state.readOnly) return;
      if (html) {
        // Let the existing Markdown/table paste extensions handle rich content,
        // just as they do for Ctrl+V, including their code-block exceptions.
        const clipboardData = new DataTransfer();
        clipboardData.setData("text/plain", text);
        clipboardData.setData("text/html", html);
        const event = new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true });
        if (!view.contentDOM.dispatchEvent(event)) return;
      }
      if (!text) return;
      view.dispatch({ ...state.replaceSelection(normalizeMarkdownEditorLineEndings(text)), userEvent: "input.paste", scrollIntoView: true });
    } else if (action === "selectAll") {
      selectAll(view);
    } else if (!state.readOnly) {
      if (action === "undo") undo(view);
      else if (action === "redo") redo(view);
      else if ((action === "cut" || action === "delete") && selection.ranges.length) {
        view.dispatch({ ...state.replaceSelection(""), userEvent: action === "cut" ? "delete.cut" : "delete.selection", scrollIntoView: true });
      }
    }
  } catch (error) {
    emit("actionError", error);
  }
}

watch(() => props.view, () => close(false));
onBeforeUnmount(() => { disposed = true; });
defineExpose({ open, onKeydown, close });
</script>

<template>
  <BaseContextMenu
    v-if="position"
    :id="menuId"
    class="markdown-editor-context-menu"
    :x="position.x"
    :y="position.y"
    :aria-label="t('editor.contextMenu')"
    @close="close()"
    @mousedown.prevent
    @keydown="navigateMenu"
  >
    <template v-if="tableTarget">
      <template v-for="action in tableActions" :key="action">
        <div v-if="action === 'column-before' || action === 'table-delete'" class="base-context-menu-separator" role="separator" />
        <button type="button" role="menuitem" :data-table-action="action" :class="{ danger: action.endsWith('-delete') }" @click="runTable(action)">
          <LucideIcon :icon="tableActionIcons[action]" :size="13" />
          <span>{{ t(`editor.table.${action}`) }}</span>
        </button>
      </template>
      <div class="base-context-menu-separator" role="separator" />
    </template>
    <button v-if="canQuote" type="button" role="menuitem" data-editor-action="quoteSelection" :disabled="!hasSelection" @click="run('quoteSelection')">
      <LucideIcon :icon="actionIcons.quoteSelection" :size="13" />
      <span>{{ t('editor.quoteSelection') }}</span>
    </button>
    <button v-if="canQuoteInNewSession" type="button" role="menuitem" data-editor-action="quoteSelectionInNewSession" :disabled="!hasSelection" @click="run('quoteSelectionInNewSession')">
      <LucideIcon :icon="actionIcons.quoteSelectionInNewSession" :size="13" />
      <span>{{ t('editor.quoteSelectionInNewSession') }}</span>
    </button>
    <div v-if="canQuote || canQuoteInNewSession" class="base-context-menu-separator" role="separator" />
    <template v-if="(canComment || canFootnote) && !readOnly">
      <button v-if="canComment" type="button" role="menuitem" data-editor-action="comment" :disabled="!hasSelection" @click="runAnnotation('comment')">
        <LucideIcon :icon="MessageSquarePlus" :size="13" />
        <span>{{ t('editor.comment.add') }}</span>
        <span class="editor-menu-shortcut">Ctrl+Alt+M</span>
      </button>
      <button v-if="canFootnote" type="button" role="menuitem" data-editor-action="footnote" @click="runAnnotation('footnote')">
        <LucideIcon :icon="Superscript" :size="13" />
        <span>{{ t('editor.footnote.add') }}</span>
        <span class="editor-menu-shortcut">Ctrl+Alt+F</span>
      </button>
      <div class="base-context-menu-separator" role="separator" />
    </template>
    <template v-for="action in actions" :key="action">
      <div v-if="action === 'cut' || action === 'selectAll'" class="base-context-menu-separator" role="separator" />
      <button
        type="button"
        role="menuitem"
        :data-editor-action="action"
        :class="{ danger: action === 'delete' }"
        :disabled="isDisabled(action)"
        @click="run(action)"
      >
        <LucideIcon :icon="actionIcons[action]" :size="13" />
        <span>{{ t(`editor.${action}`) }}</span>
        <span class="editor-menu-shortcut">{{ shortcuts[action] }}</span>
      </button>
    </template>
    <template v-if="canHistory">
      <div class="base-context-menu-separator" role="separator" />
      <button type="button" role="menuitem" @click="close(); emit('history')">
        <LucideIcon :icon="History" :size="13" />
        <span>版本历史</span>
      </button>
    </template>
  </BaseContextMenu>
</template>

<style scoped>
.editor-menu-shortcut {
  flex-shrink: 0;
  margin-left: auto;
  padding-left: 16px;
  color: var(--text-secondary);
  font-size: 11px;
}
</style>
