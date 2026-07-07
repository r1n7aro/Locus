
<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { t } from "../i18n";
import { STREAMING_RENDER_THROTTLE_MS } from "../composables/streamingRenderThrottle";
import type { StreamingTextSource } from "../composables/streamingTextChunks";

/**
 * Live thinking viewer. Streaming input arrives as an append-only chunk
 * buffer (`stream`) rendered as frozen spans plus a growing tail span, so an
 * update only lays out the tail instead of replacing (and re-laying-out) the
 * whole accumulated text — the previous whole-string interpolation was an
 * O(n) DOM rebuild per delta. Growth is consumed at the shared streaming
 * cadence rather than per delta. `text` shows fixed content (history
 * viewing) and wins over the stream when set.
 */
const props = defineProps<{
  stream?: StreamingTextSource | null;
  text?: string;
  isThinking: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const contentRef = ref<HTMLElement | null>(null);

const liveStream = computed(() => (props.text ? null : props.stream ?? null));

/** Throttled projection of the buffer: frozen parts diff away in the keyed
 * v-for, so a flush re-renders only the active tail span. */
const liveParts = shallowRef<{ frozen: readonly string[]; active: string } | null>(null);
let liveFlushTimer: ReturnType<typeof setTimeout> | null = null;

function clearLiveFlushTimer() {
  if (liveFlushTimer === null) return;
  clearTimeout(liveFlushTimer);
  liveFlushTimer = null;
}

function flushLiveParts() {
  clearLiveFlushTimer();
  const stream = liveStream.value;
  liveParts.value = stream && stream.length > 0
    ? { frozen: stream.frozenParts, active: stream.activePart }
    : null;
  scheduleScrollToBottom();
}

watch(
  () => liveStream.value?.version.value,
  () => {
    if (liveFlushTimer !== null) return;
    liveFlushTimer = setTimeout(flushLiveParts, STREAMING_RENDER_THROTTLE_MS);
  },
);

// Stream identity or mode changes swap the content outright: flush
// immediately so stale parts never linger.
watch([liveStream, () => props.text], flushLiveParts, { immediate: true });

let scrollFrame: number | null = null;

function scheduleScrollToBottom() {
  if (scrollFrame !== null) return;
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = null;
    const el = contentRef.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

onBeforeUnmount(() => {
  clearLiveFlushTimer();
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
  }
});
</script>

<template>
  <aside class="thinking-panel">
    <div class="panel-header">
      <span class="panel-title">
        <span v-if="isThinking" class="thinking-dot" />
        {{ t("thinking.panel.title") }}
      </span>
      <button class="close-btn" @click="emit('close')" :title="t('thinking.panel.close')">&times;</button>
    </div>
    <div ref="contentRef" class="thinking-content">
      <pre v-if="text" class="thinking-text">{{ text }}</pre>
      <pre
        v-else-if="liveParts"
        class="thinking-text"
      ><span
        v-for="(part, index) in liveParts.frozen"
        :key="index"
      >{{ part }}</span><span>{{ liveParts.active }}</span></pre>
      <div v-else class="empty-hint">{{ t("thinking.panel.empty") }}</div>
    </div>
  </aside>
</template>

<style scoped>
.thinking-panel {
  width: 340px;
  min-width: 340px;
  height: 100%;
  background: var(--sidebar-bg);
  border-left: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  user-select: text;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.panel-title {
  flex: 1;
  font-size: 14px;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 8px;
}

.thinking-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #3b82f6;
  animation: pulse 1.2s ease-in-out infinite;
  flex-shrink: 0;
}

@keyframes pulse {
  0%, 100% { opacity: 0.4; transform: scale(0.9); }
  50% { opacity: 1; transform: scale(1.1); }
}

.close-btn {
  width: 24px;
  height: 24px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 16px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  box-shadow: none;
  flex-shrink: 0;
}

.close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.thinking-content {
  flex: 1;
  overflow-y: auto;
  padding: 12px 16px;
}

.thinking-text {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--font-prose);
  margin: 0;
}

.empty-hint {
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px 0;
}
</style>
