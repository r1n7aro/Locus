<script setup lang="ts">
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
  "update:modelValue": [value: boolean];
  commit: [value: boolean];
}>();

function update(event: Event) {
  if (props.disabled || props.readonly) return;
  const value = (event.target as HTMLInputElement | null)?.checked === true;
  emit("update:modelValue", value);
  emit("commit", value);
}
</script>

<template>
  <input
    class="unity-bool-field"
    type="checkbox"
    :checked="modelValue === true"
    :disabled="disabled || readonly"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
    @change="update"
  />
</template>

<style scoped>
.unity-bool-field {
  width: 16px;
  height: 16px;
  min-width: 16px;
  min-height: 16px;
  margin: 0;
  padding: 0;
  accent-color: var(--accent-color);
}

.unity-bool-field:disabled {
  opacity: 0.55;
}
</style>
