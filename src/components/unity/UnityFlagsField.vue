<script setup lang="ts">
import { computed } from "vue";
import {
  normalizeUnityOptions,
  unityEnumNumericValue,
  type UnitySelectOption,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  enumOptions?: UnitySelectOption[];
  enumValueFlag?: number;
  disabled?: boolean;
  readonly?: boolean;
  title?: string;
  ariaLabel?: string;
}>(), {
  enumOptions: () => [],
  enumValueFlag: 0,
  disabled: false,
  readonly: false,
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  commit: [value: unknown];
}>();

const normalizedOptions = computed(() =>
  normalizeUnityOptions(props.enumOptions).filter((option) => option.numericValue != null),
);
const currentMask = computed(() => unityEnumNumericValue(props.modelValue, props.enumValueFlag));

function isChecked(option: UnitySelectOption): boolean {
  const value = option.numericValue ?? 0;
  if (value === 0) return currentMask.value === 0;
  return (currentMask.value & value) === value;
}

function toggle(option: UnitySelectOption) {
  if (props.disabled || props.readonly) return;
  const value = option.numericValue ?? 0;
  const nextMask = value === 0
    ? 0
    : isChecked(option)
      ? currentMask.value & ~value
      : currentMask.value | value;
  const next = {
    action: "setFlags",
    numericValue: nextMask,
    flagValue: nextMask,
  };
  emit("update:modelValue", next);
  emit("commit", next);
}
</script>

<template>
  <div
    class="unity-flags-field"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
  >
    <button
      v-for="option in normalizedOptions"
      :key="option.value"
      type="button"
      class="flag-option"
      :class="{ active: isChecked(option) }"
      :disabled="disabled || readonly"
      @click="toggle(option)"
    >
      <span class="flag-check" aria-hidden="true"></span>
      <span class="flag-label">{{ option.label }}</span>
    </button>
  </div>
</template>

<style scoped>
.unity-flags-field {
  width: 100%;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.flag-option {
  min-width: 0;
  min-height: 24px;
  display: inline-grid;
  grid-template-columns: 12px minmax(0, auto);
  align-items: center;
  gap: 4px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 11px;
  text-align: left;
}

.flag-option.active {
  border-color: color-mix(in srgb, var(--text-secondary) 62%, var(--border-color) 38%);
  background: color-mix(in srgb, var(--text-secondary) 12%, var(--input-bg) 88%);
}

.flag-option:disabled {
  opacity: 0.65;
}

.flag-check,
.flag-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.flag-check {
  width: 10px;
  height: 10px;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 46%, var(--panel-bg) 54%);
  border-radius: 2px;
  background: color-mix(in srgb, var(--input-bg) 72%, var(--panel-bg) 28%);
}

.flag-option.active .flag-check {
  border-color: color-mix(in srgb, var(--text-secondary) 82%, var(--panel-bg) 18%);
  background: color-mix(in srgb, var(--text-secondary) 78%, var(--panel-bg) 22%);
}
</style>
