<script setup lang="ts">
import { ref, watch } from "vue";
import { unitySerializedValueToEditText } from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  displayValue?: string;
  disabled?: boolean;
  readonly?: boolean;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
}>(), {
  displayValue: "",
  disabled: false,
  readonly: false,
  placeholder: "Assets/...",
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  edit: [value: string];
  commit: [value: string];
}>();

const text = ref(unitySerializedValueToEditText("ObjectReference", props.modelValue, props.displayValue));

watch(
  () => [props.modelValue, props.displayValue] as const,
  () => {
    text.value = unitySerializedValueToEditText("ObjectReference", props.modelValue, props.displayValue);
  },
);

function updateText(event: Event) {
  const target = event.target as HTMLInputElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  emit("update:modelValue", text.value);
}

function commitText() {
  if (props.disabled || props.readonly) return;
  emit("update:modelValue", text.value);
  emit("commit", text.value);
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}
</script>

<template>
  <input
    class="unity-object-reference-field"
    type="text"
    :value="text"
    :disabled="disabled"
    :readonly="readonly"
    :placeholder="placeholder"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
    @input="updateText"
    @change="commitText"
    @keydown.enter.prevent="blurOnEnter"
  />
</template>

<style scoped>
.unity-object-reference-field {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  box-sizing: border-box;
}

.unity-object-reference-field:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-object-reference-field:disabled,
.unity-object-reference-field:read-only {
  opacity: 0.65;
}
</style>
