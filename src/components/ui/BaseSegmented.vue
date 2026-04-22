<script setup lang="ts">
import { computed } from "vue";

export interface SegmentedOption {
  value: string;
  label: string;
  hint?: string;
  disabled?: boolean;
}

const props = withDefaults(defineProps<{
  modelValue: string;
  options: SegmentedOption[];
  size?: "sm" | "md";
}>(), {
  size: "md",
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  press: [value: string];
  hover: [value: string | null];
}>();

const enabledOptions = computed(() => props.options.filter((option) => !option.disabled));

function select(value: string, disabled?: boolean) {
  if (disabled) return;
  emit("press", value);
  if (value === props.modelValue) return;
  emit("update:modelValue", value);
}

function moveSelection(step: 1 | -1) {
  const currentIndex = enabledOptions.value.findIndex((option) => option.value === props.modelValue);
  if (currentIndex === -1 || enabledOptions.value.length <= 1) return;
  const nextIndex = (currentIndex + step + enabledOptions.value.length) % enabledOptions.value.length;
  emit("update:modelValue", enabledOptions.value[nextIndex].value);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "ArrowRight" || event.key === "ArrowDown") {
    event.preventDefault();
    moveSelection(1);
    return;
  }
  if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
    event.preventDefault();
    moveSelection(-1);
  }
}

function emitHover(value: string | null) {
  emit("hover", value);
}
</script>

<template>
  <div class="base-segmented" :class="[`size-${size}`]" role="radiogroup">
    <button
      v-for="option in options"
      :key="option.value"
      class="base-segmented-item"
      :class="{ active: modelValue === option.value }"
      type="button"
      role="radio"
      :aria-checked="modelValue === option.value"
      :disabled="option.disabled"
      :title="option.hint || undefined"
      @keydown="onKeydown"
      @click="select(option.value, option.disabled)"
      @mouseenter="emitHover(option.value)"
      @mouseleave="emitHover(null)"
      @focus="emitHover(option.value)"
      @blur="emitHover(null)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.base-segmented {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--input-bg) 28%);
  overflow: hidden;
}

.base-segmented-item {
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
  box-shadow: none;
}

.base-segmented-item + .base-segmented-item {
  border-left: 1px solid var(--border-color);
}

.base-segmented-item:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.base-segmented-item.active {
  background: var(--accent-soft);
  color: var(--accent-color);
}

.base-segmented-item:focus-visible {
  position: relative;
  z-index: 1;
  outline: none;
  box-shadow: inset 0 0 0 1px var(--accent-color);
}

.base-segmented-item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.size-sm .base-segmented-item {
  min-height: 26px;
  padding: 0 12px;
  font-size: 12px;
}

.size-md .base-segmented-item {
  min-height: 30px;
  padding: 0 14px;
  font-size: 13px;
}
</style>
