<script setup lang="ts">
withDefaults(defineProps<{
  label: string;
  compact?: boolean;
  active?: boolean;
}>(), {
  compact: false,
  active: true,
});
</script>

<template>
  <span
    class="chat-waiting-indicator"
    :class="{ compact, active }"
  >
    <span
      class="chat-waiting-indicator-spinner"
      :class="{ compact }"
      aria-hidden="true"
    ></span>
    <span class="chat-waiting-indicator-label">{{ label }}</span>
  </span>
</template>

<style scoped>
.chat-waiting-indicator {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: inherit;
}

.chat-waiting-indicator.compact {
  min-height: 20px;
}

.chat-waiting-indicator-spinner {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  border: 2px solid var(--border-color);
  border-top-color: currentColor;
  border-radius: 50%;
  animation: chat-waiting-indicator-spin 0.8s linear infinite;
}

.chat-waiting-indicator-spinner.compact {
  width: 12px;
  height: 12px;
}

.chat-waiting-indicator-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}

.chat-waiting-indicator.active .chat-waiting-indicator-label {
  background: linear-gradient(90deg, var(--text-secondary) 0%, var(--text-color) 50%, var(--text-secondary) 100%);
  background-size: 200% 100%;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
  animation: chat-waiting-indicator-shimmer 2s ease-in-out infinite;
}

@keyframes chat-waiting-indicator-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes chat-waiting-indicator-shimmer {
  0% {
    background-position: 100% 0;
  }

  100% {
    background-position: -100% 0;
  }
}
</style>
