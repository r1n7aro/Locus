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
    <span class="chat-waiting-indicator-label" :class="{ 'locus-running-text': active }">{{ label }}</span>
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
}

/* Breathes through the global .locus-running-text (opacity only); the former
   background-position shimmer repainted the window on the main thread. The
   breathing layer loses ClearType, so full --text-color reads as bold; this mix
   keeps the brightness at the old shimmer's average. */
.chat-waiting-indicator.active .chat-waiting-indicator-label {
  color: color-mix(in srgb, var(--text-color) 30%, var(--text-secondary));
}

@keyframes chat-waiting-indicator-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
