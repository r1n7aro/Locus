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
  canSend?: boolean;
  sendLabel?: string;
  cancelLabel?: string;
  maxHeight?: number;
  submitMode?: ChatSubmitMode;
  compact?: boolean;
  showAction?: boolean;
  showHeader?: boolean | null;
  extendTop?: boolean;
}>(), {
  placeholder: "",
  disabled: false,
  isStreaming: false,
  canSend: false,
  sendLabel: "",
  cancelLabel: "",
  maxHeight: 200,
  submitMode: "enter-send",
  compact: false,
  showAction: true,
  showHeader: null,
  extendTop: false,
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "send"): void;
  (e: "cancel"): void;
  (e: "keydown", event: KeyboardEvent): void;
  (e: "keyup", event: KeyboardEvent): void;
  (e: "input", event: Event): void;
  (e: "paste", event: ClipboardEvent): void;
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
const textareaDisabled = computed(() => props.disabled || props.isStreaming);
const actionDisabled = computed(() => !props.isStreaming && (props.disabled || !props.canSend));
const textareaStyle = computed(() => ({
  maxHeight: `${props.maxHeight}px`,
}));
const actionLabel = computed(() => (
  props.isStreaming
    ? (props.cancelLabel || t("common.cancel"))
    : (props.sendLabel || t("common.send"))
));

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
  if (props.isStreaming) {
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
  <div class="chat-composer" :class="{ 'is-compact': compact, 'has-top-extension': extendTop }">
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
        :class="{ 'is-cancel': isStreaming }"
        :disabled="actionDisabled"
        :title="actionLabel"
        :aria-label="actionLabel"
        type="button"
        @click="handleActionClick"
      >
        <span v-if="isStreaming" class="chat-composer-stop-icon" aria-hidden="true">&#9632;</span>
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
          :class="{ 'is-cancel': isStreaming }"
          :disabled="actionDisabled"
          :title="actionLabel"
          :aria-label="actionLabel"
          type="button"
          @click="handleActionClick"
        >
          <span v-if="isStreaming" class="chat-composer-stop-icon" aria-hidden="true">&#9632;</span>
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
