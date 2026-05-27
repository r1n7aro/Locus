<script setup lang="ts">
import { ref, watch } from "vue";
import {
  tryParseUnitySerializedEditValue,
  unitySerializedValueToEditText,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  propertyType?: string;
  disabled?: boolean;
  readonly?: boolean;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
}>(), {
  propertyType: "Float",
  disabled: false,
  readonly: false,
  placeholder: "",
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  edit: [value: string];
  commit: [value: unknown];
}>();

const text = ref(unitySerializedValueToEditText(props.propertyType, props.modelValue));

watch(
  () => [props.modelValue, props.propertyType] as const,
  () => {
    text.value = unitySerializedValueToEditText(props.propertyType, props.modelValue);
  },
);

function parsedValue() {
  return tryParseUnitySerializedEditValue(props.propertyType, text.value);
}

function updateFromInput(event: Event) {
  const target = event.target as HTMLInputElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  const parsed = parsedValue();
  if (parsed.ok) emit("update:modelValue", parsed.value);
}

function commitFromInput() {
  if (props.disabled || props.readonly) return;
  const parsed = parsedValue();
  if (!parsed.ok) return;
  emit("update:modelValue", parsed.value);
  emit("commit", parsed.value);
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}
</script>

<template>
  <input
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
  />
</template>

<style scoped>
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

.unity-number-field:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-number-field:disabled,
.unity-number-field:read-only {
  opacity: 0.65;
}
</style>
