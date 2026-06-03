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
  width: 13px;
  height: 13px;
  min-width: 13px;
  min-height: 13px;
  margin: 0;
  padding: 0;
  display: inline-grid;
  place-content: center;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 46%, var(--panel-bg) 54%);
  border-radius: 2px;
  background: color-mix(in srgb, var(--input-bg) 72%, var(--panel-bg) 28%);
  appearance: none;
}

.unity-bool-field::after {
  width: 7px;
  height: 4px;
  border-left: 2px solid var(--bg-color);
  border-bottom: 2px solid var(--bg-color);
  transform: translateY(-1px) rotate(-45deg);
  content: "";
  opacity: 0;
}

.unity-bool-field:checked {
  border-color: color-mix(in srgb, var(--text-secondary) 82%, var(--panel-bg) 18%);
  background: color-mix(in srgb, var(--text-secondary) 78%, var(--panel-bg) 22%);
}

.unity-bool-field:checked::after {
  opacity: 1;
}

.unity-bool-field:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--text-secondary) 60%, transparent);
  outline-offset: 2px;
}

.unity-bool-field:disabled {
  opacity: 0.55;
}
</style>
