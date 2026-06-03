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
    class="base-switch"
    :class="{ on: modelValue }"
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :aria-label="ariaLabel || undefined"
    :aria-labelledby="ariaLabelledby || undefined"
    :disabled="disabled"
    @click="toggle"
  >
    <span class="base-switch-track">
      <span class="base-switch-knob" />
    </span>
  </button>
</template>

<style scoped>
.base-switch {
  display: inline-flex;
  align-items: center;
  justify-content: flex-start;
  width: 34px;
  height: 18px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  box-shadow: none;
}

.base-switch:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.base-switch-track {
  position: relative;
  width: 34px;
  height: 18px;
  border: 1px solid color-mix(in srgb, var(--border-strong) 82%, var(--text-secondary) 18%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--hover-bg) 24%);
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--surface-elevated, var(--panel-bg)) 18%, transparent);
  transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease;
}

.base-switch:hover:not(:disabled) .base-switch-track {
  border-color: color-mix(in srgb, var(--text-secondary) 44%, var(--border-strong));
  background: color-mix(in srgb, var(--hover-bg) 62%, var(--input-bg) 38%);
}

.base-switch.on .base-switch-track {
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-strong));
  background: color-mix(in srgb, var(--accent-soft) 54%, var(--input-bg) 46%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 10%, transparent);
}

.base-switch:focus-visible .base-switch-track {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 22%, transparent);
}

.base-switch-knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 12px;
  height: 12px;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 48%, var(--border-strong));
  border-radius: 4px;
  background: color-mix(in srgb, var(--text-secondary) 82%, var(--surface-elevated, var(--panel-bg)) 18%);
  box-shadow:
    0 1px 1px rgba(0, 0, 0, 0.24),
    inset 0 1px 0 color-mix(in srgb, var(--surface-elevated, var(--panel-bg)) 32%, transparent);
  transition: transform 0.15s ease, border-color 0.15s ease, background 0.15s ease;
}

.base-switch.on .base-switch-knob {
  transform: translateX(18px);
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-strong));
  background: color-mix(in srgb, var(--accent-color) 86%, var(--surface-elevated, var(--panel-bg)) 14%);
}
</style>
