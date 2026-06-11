<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  UNITY_FLOAT_DRAG_STEP,
  constrainUnityNumberDragValue,
  constrainUnityNumberValue,
  formatUnityNumberValue,
  isUnityIntegerPropertyType,
  tryParseUnitySerializedEditValue,
  unitySerializedValueToEditText,
  type UnityNumberConstraintOptions,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  propertyType?: string;
  disabled?: boolean;
  readonly?: boolean;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
  hasRange?: boolean;
  rangeMin?: number;
  rangeMax?: number;
  numberStep?: number;
}>(), {
  propertyType: "Float",
  disabled: false,
  readonly: false,
  placeholder: "",
  title: "",
  ariaLabel: "",
  hasRange: false,
  rangeMin: 0,
  rangeMax: 0,
  numberStep: 0,
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  edit: [value: string];
  preview: [value: unknown];
  commit: [value: unknown];
}>();

const text = ref(unitySerializedValueToEditText(props.propertyType, props.modelValue));
const rangeMin = computed(() => Math.min(props.rangeMin, props.rangeMax));
const rangeMax = computed(() => Math.max(props.rangeMin, props.rangeMax));
const usesRange = computed(() =>
  props.hasRange &&
  Number.isFinite(props.rangeMin) &&
  Number.isFinite(props.rangeMax),
);
const rangeDisabled = computed(() => props.disabled || props.readonly);
const numberConstraints = computed<UnityNumberConstraintOptions>(() => ({
  hasRange: usesRange.value,
  rangeMin: rangeMin.value,
  rangeMax: rangeMax.value,
}));
const inputStep = computed<number | "any">(() => {
  if (isUnityIntegerPropertyType(props.propertyType)) return 1;
  if (props.numberStep > 0) return Math.max(UNITY_FLOAT_DRAG_STEP, props.numberStep);
  return UNITY_FLOAT_DRAG_STEP;
});
const sliderValue = computed(() => {
  const parsedText = parsedNumber(text.value);
  if (parsedText != null) return formatNumber(parsedText);
  const parsedModel = parsedNumber(props.modelValue);
  if (parsedModel != null) return formatNumber(parsedModel);
  return formatNumber(rangeMin.value);
});

watch(
  () => [props.modelValue, props.propertyType] as const,
  () => {
    text.value = unitySerializedValueToEditText(props.propertyType, props.modelValue);
  },
);

function parsedNumber(rawValue: unknown): number | null {
  const parsed = tryParseUnitySerializedEditValue(props.propertyType, rawValue);
  if (!parsed.ok || typeof parsed.value !== "number") return null;
  return constrainUnityNumberValue(props.propertyType, parsed.value, numberConstraints.value);
}

function draggedNumber(rawValue: unknown): number | null {
  const parsed = tryParseUnitySerializedEditValue(props.propertyType, rawValue);
  if (!parsed.ok || typeof parsed.value !== "number") return null;
  return constrainUnityNumberDragValue(props.propertyType, parsed.value, numberConstraints.value);
}

function formatNumber(value: number): string {
  return formatUnityNumberValue(props.propertyType, value, numberConstraints.value);
}

function emitNumberValue(value: number) {
  const nextText = formatNumber(value);
  text.value = nextText;
  emit("edit", nextText);
  emit("update:modelValue", value);
}

function updateFromInput(event: Event) {
  const target = event.target as HTMLInputElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  const parsed = parsedNumber(text.value);
  if (parsed != null) emit("update:modelValue", parsed);
}

function updateFromRange(event: Event) {
  const target = event.target as HTMLInputElement | null;
  const parsed = draggedNumber(target?.value ?? "");
  if (parsed == null) return;
  emitNumberValue(parsed);
  if (!props.disabled && !props.readonly) {
    emit("preview", parsed);
  }
}

function commitFromInput() {
  if (props.disabled || props.readonly) return;
  const parsed = parsedNumber(text.value);
  if (parsed == null) return;
  text.value = formatNumber(parsed);
  emit("update:modelValue", parsed);
  emit("commit", parsed);
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}

function restoreTextOnEscape(event: KeyboardEvent) {
  const input = event.target as HTMLInputElement | null;
  const original = unitySerializedValueToEditText(props.propertyType, props.modelValue);
  text.value = original;
  // Sync the DOM value before blurring so the change event does not fire.
  if (input) input.value = original;
  input?.blur();
}
</script>

<template>
  <div v-if="usesRange" class="unity-number-range-field">
    <input
      class="unity-number-range-slider"
      type="range"
      :value="sliderValue"
      :disabled="rangeDisabled"
      :min="rangeMin"
      :max="rangeMax"
      :step="inputStep"
      :title="title || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateFromRange"
      @change="commitFromInput"
    />
    <input
      class="unity-number-field unity-number-range-input"
      type="number"
      inputmode="decimal"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :min="rangeMin"
      :max="rangeMax"
      :step="inputStep"
      :title="title || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateFromInput"
      @change="commitFromInput"
      @keydown.enter.prevent="blurOnEnter"
      @keydown.esc.prevent="restoreTextOnEscape"
    />
  </div>
  <input
    v-else
    class="unity-number-field"
    type="text"
    inputmode="decimal"
    :value="text"
    :disabled="disabled"
    :readonly="readonly"
    :placeholder="placeholder"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
    @input="updateFromInput"
    @change="commitFromInput"
    @keydown.enter.prevent="blurOnEnter"
    @keydown.esc.prevent="restoreTextOnEscape"
  />
</template>

<style scoped>
.unity-number-range-field {
  width: 100%;
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(58px, 74px);
  align-items: center;
  gap: 6px;
}

.unity-number-range-slider {
  width: 100%;
  min-width: 0;
  height: 16px;
  margin: 0;
  background: transparent;
  cursor: default;
  appearance: none;
}

.unity-number-range-slider::-webkit-slider-runnable-track {
  height: 2px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, var(--panel-bg) 66%);
  cursor: default;
}

.unity-number-range-slider::-webkit-slider-thumb {
  width: 10px;
  height: 10px;
  margin-top: -4px;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 72%, var(--panel-bg) 28%);
  border-radius: 50%;
  background: color-mix(in srgb, var(--text-secondary) 78%, var(--panel-bg) 22%);
  cursor: default;
  appearance: none;
}

.unity-number-range-slider::-moz-range-track {
  height: 2px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, var(--panel-bg) 66%);
  cursor: default;
}

.unity-number-range-slider::-moz-range-progress {
  height: 2px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, var(--panel-bg) 66%);
  cursor: default;
}

.unity-number-range-slider::-moz-range-thumb {
  width: 10px;
  height: 10px;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 72%, var(--panel-bg) 28%);
  border-radius: 50%;
  background: color-mix(in srgb, var(--text-secondary) 78%, var(--panel-bg) 22%);
  cursor: default;
}

.unity-number-range-slider:hover::-webkit-slider-thumb,
.unity-number-range-slider:focus-visible::-webkit-slider-thumb {
  background: var(--text-secondary);
}

.unity-number-range-slider:hover::-moz-range-thumb,
.unity-number-range-slider:focus-visible::-moz-range-thumb {
  background: var(--text-secondary);
}

.unity-number-range-slider:focus-visible {
  outline: none;
}

.unity-number-range-slider:focus-visible::-webkit-slider-runnable-track {
  background: color-mix(in srgb, var(--text-secondary) 48%, var(--panel-bg) 52%);
}

.unity-number-range-slider:focus-visible::-moz-range-track {
  background: color-mix(in srgb, var(--text-secondary) 48%, var(--panel-bg) 52%);
}

.unity-number-field {
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

.unity-number-range-input {
  padding-right: 4px;
  appearance: textfield;
  -moz-appearance: textfield;
}

.unity-number-range-input::-webkit-outer-spin-button,
.unity-number-range-input::-webkit-inner-spin-button {
  margin: 0;
  appearance: none;
  -webkit-appearance: none;
}

.unity-number-field:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-number-field:disabled,
.unity-number-field:read-only,
.unity-number-range-slider:disabled {
  opacity: 0.65;
}

.unity-number-range-slider:disabled {
  cursor: default;
}
</style>
