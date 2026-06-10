<script setup lang="ts">
import { computed, ref, useSlots, watch } from "vue";
import { t } from "../../i18n";
import {
  shouldSubmitOnEnter,
  type ChatSubmitMode,
} from "../../composables/useChatInputSettings";

const props = withDefaults(defineProps<{
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  cancelling?: boolean;
  canSend?: boolean;
  sendLabel?: string;
  cancelLabel?: string;
  maxHeight?: number;
  submitMode?: ChatSubmitMode;
  compact?: boolean;
  showAction?: boolean;
  showHeader?: boolean | null;
  extendTop?: boolean;
  dropActive?: boolean;
  dropLabel?: string;
}>(), {
  placeholder: "",
  disabled: false,
  isStreaming: false,
  cancelling: false,
  canSend: false,
  sendLabel: "",
  cancelLabel: "",
  maxHeight: 200,
  submitMode: "enter-send",
  compact: false,
  showAction: true,
  showHeader: null,
  extendTop: false,
  dropActive: false,
  dropLabel: "",
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "send"): void;
  (e: "cancel"): void;
  (e: "keydown", event: KeyboardEvent): void;
  (e: "keyup", event: KeyboardEvent): void;
  (e: "input", event: Event): void;
  (e: "paste", event: ClipboardEvent): void;
  (e: "dragenter", event: DragEvent): void;
  (e: "dragover", event: DragEvent): void;
  (e: "dragleave", event: DragEvent): void;
  (e: "drop", event: DragEvent): void;
  (e: "click", event: MouseEvent): void;
  (e: "mouseup", event: MouseEvent): void;
  (e: "focus", event: FocusEvent): void;
}>();

const slots = useSlots();
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const DEFAULT_TEXTAREA_MIN_HEIGHT = 42;
const COMPACT_TEXTAREA_MIN_HEIGHT = 28;

const hasHeader = computed(() => props.showHeader ?? !!slots.header);
const hasOverlay = computed(() => props.extendTop && !!slots.overlay);
const hasFooterStart = computed(() => !!slots["footer-start"]);
const hasFooterEnd = computed(() => !!slots["footer-end"]);
const hasFooter = computed(() =>
  !props.compact && (hasFooterStart.value || hasFooterEnd.value || props.showAction),
);
const showInlineAction = computed(() => props.compact && props.showAction);
const textareaDisabled = computed(() => props.disabled);
const isCancelAction = computed(() => props.isStreaming && !props.canSend);
const isCancellingAction = computed(() => isCancelAction.value && props.cancelling);
const actionDisabled = computed(() =>
  props.disabled
  || isCancellingAction.value
  || (!isCancelAction.value && !props.canSend));
const textareaStyle = computed(() => ({
  maxHeight: `${props.maxHeight}px`,
}));
const actionLabel = computed(() => {
  if (isCancellingAction.value) return t("common.cancelling");
  return isCancelAction.value
    ? (props.cancelLabel || t("common.cancel"))
    : (props.sendLabel || t("common.send"));
});

function resizeTextarea(textarea: HTMLTextAreaElement | null = textareaRef.value) {
  if (!textarea) return;
  const minHeight = props.compact ? COMPACT_TEXTAREA_MIN_HEIGHT : DEFAULT_TEXTAREA_MIN_HEIGHT;
  textarea.style.height = "auto";
  const contentHeight = textarea.scrollHeight;
  textarea.style.height = `${Math.max(minHeight, Math.min(contentHeight, props.maxHeight))}px`;
  textarea.style.overflowY = contentHeight > props.maxHeight ? "auto" : "hidden";
}

function handleInput(event: Event) {
  const target = event.target as HTMLTextAreaElement;
  resizeTextarea(target);
  emit("update:modelValue", target.value);
  emit("input", event);
}

function handleKeydown(event: KeyboardEvent) {
  emit("keydown", event);
  if (event.defaultPrevented) return;
  if (!shouldSubmitOnEnter(event, props.submitMode)) return;
  event.preventDefault();
  if (textareaDisabled.value || !props.canSend) return;
  emit("send");
}

function handleActionClick() {
  if (isCancelAction.value) {
    emit("cancel");
    return;
  }
  if (actionDisabled.value) return;
  emit("send");
}

function focus() {
  textareaRef.value?.focus();
}

function setSelectionRange(start: number, end: number) {
  textareaRef.value?.setSelectionRange(start, end);
}

function getTextarea() {
  return textareaRef.value;
}

defineExpose({
  focus,
  setSelectionRange,
  resizeTextarea,
  getTextarea,
});

watch(() => props.modelValue, () => {
  resizeTextarea();
}, {
  immediate: true,
  flush: "post",
});

watch(() => props.maxHeight, () => {
  resizeTextarea();
}, {
  flush: "post",
});

watch(() => props.compact, () => {
  resizeTextarea();
}, {
  flush: "post",
});
</script>

<template>
  <div
    class="chat-composer"
    :class="{ 'is-compact': compact, 'has-top-extension': extendTop, 'is-drop-active': dropActive }"
    @dragenter="emit('dragenter', $event as DragEvent)"
    @dragover="emit('dragover', $event as DragEvent)"
    @dragleave="emit('dragleave', $event as DragEvent)"
    @drop="emit('drop', $event as DragEvent)"
  >
    <Transition name="chat-composer-drop">
      <div
        v-if="dropActive"
        class="chat-composer-drop-overlay"
        role="status"
        aria-live="polite"
      >
        <span class="chat-composer-drop-label">{{ dropLabel }}</span>
      </div>
    </Transition>

    <div v-if="hasOverlay" class="chat-composer-overlay">
      <slot name="overlay" />
    </div>

    <div v-if="hasHeader" class="chat-composer-header">
      <slot name="header" />
    </div>

    <div class="chat-composer-body">
      <textarea
        ref="textareaRef"
        class="chat-composer-input"
        :value="modelValue"
        :style="textareaStyle"
        :disabled="textareaDisabled"
        :placeholder="placeholder"
        wrap="soft"
        rows="1"
        @input="handleInput"
        @keydown="handleKeydown"
        @keyup="emit('keyup', $event as KeyboardEvent)"
        @paste="emit('paste', $event as ClipboardEvent)"
        @click="emit('click', $event as MouseEvent)"
        @mouseup="emit('mouseup', $event as MouseEvent)"
        @focus="emit('focus', $event as FocusEvent)"
      />
      <button
        v-if="showInlineAction"
        class="chat-composer-action chat-composer-inline-action ui-select-none"
        :class="{ 'is-cancel': isCancelAction, 'is-cancelling': isCancellingAction }"
        :disabled="actionDisabled"
        :title="actionLabel"
        :aria-label="actionLabel"
        type="button"
        @click="handleActionClick"
      >
        <span v-if="isCancelAction" class="chat-composer-stop-icon" aria-hidden="true">&#9632;</span>
        <span v-else class="chat-composer-send-icon" aria-hidden="true">&#8593;</span>
      </button>
    </div>

    <div v-if="hasFooter" class="chat-composer-footer">
      <div class="chat-composer-footer-start" :class="{ empty: !hasFooterStart }">
        <slot name="footer-start" />
      </div>
      <div class="chat-composer-footer-end" :class="{ empty: !hasFooterEnd }">
        <slot name="footer-end" />
        <button
          v-if="showAction"
          class="chat-composer-action ui-select-none"
          :class="{ 'is-cancel': isCancelAction, 'is-cancelling': isCancellingAction }"
          :disabled="actionDisabled"
          :title="actionLabel"
          :aria-label="actionLabel"
          type="button"
          @click="handleActionClick"
        >
          <span v-if="isCancelAction" class="chat-composer-stop-icon" aria-hidden="true">&#9632;</span>
          <span v-else class="chat-composer-send-icon" aria-hidden="true">&#8593;</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-composer {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
  overflow: visible;
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  min-height: 92px;
  padding: 8px 12px;
  transition: border-color 0.2s ease;
}

.chat-composer.is-drop-active {
  border-color: color-mix(in srgb, var(--accent-color) 48%, var(--border-color));
  background: color-mix(in srgb, var(--accent-soft) 12%, var(--input-bg) 88%);
}

.chat-composer-drop-overlay {
  position: absolute;
  inset: 0;
  box-sizing: border-box;
  z-index: 4;
  display: grid;
  place-items: center;
  padding: 4px 8px;
  border-radius: inherit;
  background: color-mix(in srgb, var(--input-bg) 70%, var(--accent-soft) 30%);
  color: var(--text-color);
  pointer-events: none;
}

.chat-composer-drop-label {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  color: color-mix(in srgb, var(--text-color) 88%, var(--accent-color) 12%);
  font-size: 13px;
  font-weight: 600;
  line-height: 1.4;
  text-align: center;
  white-space: nowrap;
}

.chat-composer-drop-enter-active,
.chat-composer-drop-leave-active {
  transition: opacity 0.12s ease;
}

.chat-composer-drop-enter-from,
.chat-composer-drop-leave-to {
  opacity: 0;
}

.chat-composer-overlay {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: flex-start;
  max-width: 100%;
  min-height: 30px;
  margin-bottom: 2px;
  pointer-events: none;
}

.chat-composer.has-top-extension {
  min-height: 122px;
}

.chat-composer:focus-within {
  border-color: var(--accent-color);
}

.chat-composer.is-compact {
  box-sizing: border-box;
  min-height: 44px;
  padding: 7px 10px;
}

.chat-composer.is-compact.has-top-extension {
  min-height: 82px;
}

.chat-composer-header {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 6px;
  min-height: 24px;
}

.chat-composer-header:empty {
  margin-bottom: 0;
}

.chat-composer-body {
  position: relative;
  display: flex;
  flex: 1 1 auto;
  align-items: flex-start;
  min-height: 42px;
}

.chat-composer.is-compact .chat-composer-body {
  align-items: flex-end;
  gap: 8px;
  min-height: 28px;
}

.chat-composer-input {
  flex: 1;
  min-width: 0;
  min-height: 42px;
  overflow-y: hidden;
  padding: 4px 0 0;
  border: none;
  outline: none;
  resize: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 14px;
  line-height: 1.5;
  box-shadow: none;
  transition: height 0.1s ease;
}

.chat-composer-input::placeholder {
  color: var(--text-secondary);
}

.chat-composer-footer {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 28px;
  margin-top: 4px;
}

.chat-composer.is-compact .chat-composer-input {
  min-height: 28px;
  overflow: hidden;
  padding-top: 3px;
}

.chat-composer-footer-start {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.chat-composer-footer-start.empty {
  gap: 0;
}

.chat-composer-footer-end {
  flex: 0 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  flex-wrap: wrap;
}

.chat-composer-footer-end.empty {
  gap: 0;
}

.chat-composer-action {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: var(--accent-color);
  color: var(--text-on-accent, #fff);
  cursor: pointer;
  box-shadow: none;
  transition: opacity 0.15s ease, filter 0.15s ease, background 0.15s ease, border-color 0.15s ease;
}

.chat-composer-inline-action {
  width: 28px;
  height: 28px;
  border-radius: 6px;
}

.chat-composer-action:hover:not(:disabled) {
  filter: brightness(1.06);
}

.chat-composer-action:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.chat-composer-action.is-cancel {
  background: var(--status-danger-fg);
  border-color: var(--status-danger-fg);
  color: var(--text-on-accent, #fff);
  opacity: 1;
}

.chat-composer-action.is-cancel:hover:not(:disabled) {
  filter: brightness(0.94);
}

.chat-composer-action.is-cancelling {
  cursor: progress;
}

.chat-composer-action.is-cancelling .chat-composer-stop-icon {
  animation: chat-composer-cancelling-pulse 0.9s ease-in-out infinite;
}

@keyframes chat-composer-cancelling-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}

.chat-composer-send-icon {
  font-size: 16px;
  font-weight: 600;
  line-height: 1;
}

.chat-composer-stop-icon {
  font-size: 12px;
  line-height: 1;
}
</style>
