<script setup lang="ts">
import { computed } from "vue";
import {
  normalizeUnityOptions,
  unityEnumIndexValue,
  unitySerializedValueToEditText,
  type UnitySelectOption,
} from "./unitySerializedValue";
import BaseDropdown, { type DropdownOption } from "../ui/BaseDropdown.vue";

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
const dropdownOptions = computed<DropdownOption[]>(() =>
  normalizedOptions.value.map((option) => ({ value: option.value, label: option.label })),
);
const selectedValue = computed(() => {
  const index = unityEnumIndexValue(props.modelValue, props.enumValueIndex);
  if (index >= 0 && normalizedOptions.value.some((option) => option.index === index || option.value === String(index))) {
    return String(index);
  }
  return unitySerializedValueToEditText("Enum", props.modelValue);
});
/* Out-of-list values (mixed/unknown enum states) still render as text. */
const selectedDisplayLabel = computed(() => {
  const match = dropdownOptions.value.find((option) => option.value === selectedValue.value);
  return match?.label ?? String(selectedValue.value ?? "");
});

function update(value: string) {
  if (props.disabled || props.readonly) return;
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
  <BaseDropdown
    class="unity-enum-field"
    :model-value="selectedValue"
    :options="dropdownOptions"
    :selected-label="selectedDisplayLabel"
    :disabled="disabled || readonly"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
    size="sm"
    menu-align="start"
    teleport
    @update:model-value="update"
  />
</template>

<style scoped>
.unity-enum-field {
  width: 100%;
  min-width: 0;
}

.unity-enum-field :deep(.base-dropdown-trigger) {
  min-width: 0;
  min-height: 26px;
  padding: 0 7px;
  background: var(--input-bg);
  font: inherit;
}
</style>
