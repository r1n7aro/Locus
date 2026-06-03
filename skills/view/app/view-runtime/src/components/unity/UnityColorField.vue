<script setup lang="ts">
import { ref, watch } from "vue";
import {
  applyUnityRgbHexToColorText,
  formatUnityColorValue,
  parseUnityColorValue,
  unityColorTextToRgbHex,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  disabled?: boolean;
  readonly?: boolean;
  title?: string;
  ariaLabel?: string;
}>(), {
  disabled: false,
  readonly: false,
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  edit: [value: string];
  commit: [value: string];
}>();

const text = ref(formatUnityColorValue(props.modelValue));

watch(
  () => props.modelValue,
  () => {
    text.value = formatUnityColorValue(props.modelValue);
  },
);

function updateText(event: Event) {
  const target = event.target as HTMLInputElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  try {
    emit("update:modelValue", parseUnityColorValue(text.value));
  } catch {
    // Keep invalid color text local until commit.
  }
}

function updateColor(event: Event) {
  if (props.disabled || props.readonly) return;
  const rgb = (event.target as HTMLInputElement | null)?.value ?? "#000000";
  const value = applyUnityRgbHexToColorText(rgb, text.value);
  text.value = value;
  emit("update:modelValue", value);
  emit("commit", value);
}

function commitText() {
  if (props.disabled || props.readonly) return;
  try {
    const value = parseUnityColorValue(text.value);
    emit("update:modelValue", value);
    emit("commit", value);
  } catch {
    // Invalid color text remains editable.
  }
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}
</script>

<template>
  <div class="unity-color-field" :title="title || undefined" :aria-label="ariaLabel || undefined">
    <input
      class="unity-color-swatch-input"
      type="color"
      :value="unityColorTextToRgbHex(text)"
      :disabled="disabled || readonly"
      @input="updateColor"
    />
    <input
      class="unity-color-text-input"
      type="text"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      placeholder="#RRGGBBAA"
      @input="updateText"
      @change="commitText"
      @keydown.enter.prevent="blurOnEnter"
    />
  </div>
</template>

<style scoped>
.unity-color-field {
  width: 100%;
  min-width: 0;
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr);
  align-items: center;
  gap: 6px;
}

.unity-color-swatch-input {
  width: 28px;
  min-width: 28px;
  height: 26px;
  min-height: 26px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
}

.unity-color-text-input {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  box-sizing: border-box;
}

.unity-color-text-input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-color-swatch-input:disabled,
.unity-color-text-input:disabled,
.unity-color-text-input:read-only {
  opacity: 0.65;
}
</style>
