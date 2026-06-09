<script setup lang="ts">
withDefaults(defineProps<{
  variant?: "neutral" | "primary" | "danger";
  size?: "sm" | "md";
  disabled?: boolean;
  block?: boolean;
  type?: "button" | "submit" | "reset";
}>(), {
  variant: "neutral",
  size: "sm",
  disabled: false,
  block: false,
  type: "button",
});
</script>

<template>
  <button
    class="base-button"
    :class="[variant, `size-${size}`, { block }]"
    :type="type"
    :disabled="disabled"
  >
    <slot />
  </button>
</template>

<style scoped>
.base-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-family: var(--font-ui);
  font-weight: 500;
  line-height: 1;
  cursor: pointer;
  box-shadow: none;
  white-space: nowrap;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease, filter 0.15s ease, opacity 0.15s ease;
}

.base-button:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.base-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.base-button.primary {
  background: var(--accent-color);
  border-color: var(--accent-color);
  color: #fff;
}

.base-button.primary:hover:not(:disabled) {
  background: var(--accent-color);
  border-color: var(--accent-color);
  color: #fff;
  filter: brightness(1.06);
}

.base-button.danger {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
}

.base-button.danger:hover:not(:disabled) {
  background: var(--status-danger-bg);
  border-color: var(--status-danger-fg);
  color: var(--status-danger-fg);
}

.base-button.block {
  width: 100%;
}

.size-sm {
  min-height: 28px;
  padding: 0 10px;
  font-size: 12px;
}

.size-md {
  min-height: 32px;
  padding: 0 14px;
  font-size: 13px;
}
</style>
