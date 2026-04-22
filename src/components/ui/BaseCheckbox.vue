<script setup lang="ts">
const props = withDefaults(defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  ariaLabelledby?: string;
}>(), {
  disabled: false,
  ariaLabel: "",
  ariaLabelledby: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: boolean];
}>();

function toggle() {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}
</script>

<template>
  <button
    class="base-checkbox"
    :class="{ checked: modelValue }"
    type="button"
    role="checkbox"
    :aria-checked="modelValue"
    :aria-label="ariaLabel || undefined"
    :aria-labelledby="ariaLabelledby || undefined"
    :disabled="disabled"
    @click="toggle"
  >
    <span class="base-checkbox-box">
      <svg v-if="modelValue" viewBox="0 0 16 16" width="12" height="12" fill="none" aria-hidden="true">
        <path d="M3.5 8.25 6.5 11.25 12.5 4.75" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </span>
  </button>
</template>

<style scoped>
.base-checkbox {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  box-shadow: none;
}

.base-checkbox:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.base-checkbox-box {
  width: 16px;
  height: 16px;
  border-radius: 4px;
  border: 1px solid var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--hover-bg) 26%);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: border-color 0.15s ease, background 0.15s ease, box-shadow 0.15s ease;
}

.base-checkbox.checked .base-checkbox-box {
  border-color: var(--accent-color);
  background: var(--accent-color);
}

.base-checkbox:focus-visible .base-checkbox-box {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}
</style>
