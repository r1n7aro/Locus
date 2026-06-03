<script setup lang="ts">
import { computed } from "vue";
import {
  normalizeUnityOptions,
  unityEnumIndexValue,
  unitySerializedValueToEditText,
  type UnitySelectOption,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  enumOptions?: UnitySelectOption[];
  enumValueIndex?: number;
  disabled?: boolean;
  readonly?: boolean;
  title?: string;
  ariaLabel?: string;
}>(), {
  enumOptions: () => [],
  enumValueIndex: -1,
  disabled: false,
  readonly: false,
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  commit: [value: unknown];
}>();

const normalizedOptions = computed(() => normalizeUnityOptions(props.enumOptions));
const selectedValue = computed(() => {
  const index = unityEnumIndexValue(props.modelValue, props.enumValueIndex);
  if (index >= 0 && normalizedOptions.value.some((option) => option.index === index || option.value === String(index))) {
    return String(index);
  }
  return unitySerializedValueToEditText("Enum", props.modelValue);
});

function update(event: Event) {
  if (props.disabled || props.readonly) return;
  const value = (event.target as HTMLSelectElement | null)?.value ?? "";
  const option = normalizedOptions.value.find((item) => item.value === value);
  const next = option && option.index != null
    ? {
      action: "setIndex",
      index: option.index,
      name: option.name,
      label: option.label,
      numericValue: option.numericValue,
    }
    : value;
  emit("update:modelValue", next);
  emit("commit", next);
}
</script>

<template>
  <select
    class="unity-enum-field"
    :value="selectedValue"
    :disabled="disabled || readonly"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
    @change="update"
  >
    <option v-for="option in normalizedOptions" :key="option.value" :value="option.value">
      {{ option.label }}
    </option>
  </select>
</template>

<style scoped>
.unity-enum-field {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 0 22px 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background:
    linear-gradient(45deg, transparent 50%, var(--text-secondary) 50%) right 11px center / 5px 5px no-repeat,
    linear-gradient(135deg, var(--text-secondary) 50%, transparent 50%) right 7px center / 5px 5px no-repeat,
    var(--input-bg);
  color: var(--text-color);
  font: inherit;
  box-sizing: border-box;
  appearance: none;
}

.unity-enum-field:focus {
  outline: none;
  border-color: color-mix(in srgb, var(--text-secondary) 68%, var(--border-color) 32%);
}

.unity-enum-field option {
  background: color-mix(in srgb, var(--input-bg) 76%, var(--panel-bg) 24%);
  color: var(--text-color);
}

.unity-enum-field option:checked {
  background: color-mix(in srgb, var(--text-secondary) 36%, var(--input-bg) 64%);
  color: var(--text-color);
}

.unity-enum-field:disabled {
  opacity: 0.65;
}
</style>
