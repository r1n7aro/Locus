<script setup lang="ts">
import UnityNumberField from "./UnityNumberField.vue";

withDefaults(defineProps<{
  modelValue: unknown;
  disabled?: boolean;
  readonly?: boolean;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
}>(), {
  disabled: false,
  readonly: false,
  placeholder: "",
  title: "",
  ariaLabel: "",
});

defineEmits<{
  "update:modelValue": [value: unknown];
  edit: [value: string];
  commit: [value: unknown];
}>();
</script>

<template>
  <UnityNumberField
    class="unity-layer-mask-field"
    property-type="LayerMask"
    :model-value="modelValue"
    :disabled="disabled"
    :readonly="readonly"
    :placeholder="placeholder"
    :title="title"
    :aria-label="ariaLabel"
    @update:model-value="$emit('update:modelValue', $event)"
    @edit="$emit('edit', $event)"
    @commit="$emit('commit', $event)"
  />
</template>
