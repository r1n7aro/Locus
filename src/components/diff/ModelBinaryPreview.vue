<script setup lang="ts">
import { computed, ref } from "vue";
import { refetchDiffByKey } from "../../services/diff";
import ModelViewer from "../model-preview/ModelViewer.vue";
import type { BinaryPreview } from "../../types";

const props = defineProps<{
  preview: BinaryPreview;
  diffKey: string;
  mode?: "diff" | "neutral";
  compact?: boolean;
}>();

const activeSide = ref<"before" | "after">(props.preview.after ? "after" : "before");

const activeRef = computed(() =>
  activeSide.value === "before" ? props.preview.before : props.preview.after,
);
const hasBoth = computed(() => !!props.preview.before && !!props.preview.after);
const statusLabel = computed(() => {
  if (props.mode === "neutral") return null;
  if (hasBoth.value) return null;
  return props.preview.after ? "Added" : "Deleted";
});
const showHeader = computed(() => !props.compact && (hasBoth.value || !!statusLabel.value));
</script>

<template>
  <div class="model-binary-preview" :class="{ compact }">
    <div v-if="showHeader" class="preview-controls">
      <div v-if="hasBoth" class="side-toggle">
        <button :class="{ active: activeSide === 'before' }" @click="activeSide = 'before'">Before</button>
        <button :class="{ active: activeSide === 'after' }" @click="activeSide = 'after'">After</button>
      </div>
      <span v-if="statusLabel" class="status-badge">{{ statusLabel }}</span>
    </div>
    <ModelViewer
      v-if="activeRef"
      :url="activeRef.url"
      :compact="compact"
      @fetch-error="refetchDiffByKey(diffKey)"
    />
  </div>
</template>

<style scoped>
.model-binary-preview {
  flex: 1;
  display: flex;
  flex-direction: column;
  width: 100%;
  min-height: 0;
  overflow: hidden;
}

.model-binary-preview.compact {
  min-height: 96px;
}

.preview-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-color, var(--border));
  background: var(--sidebar-bg, var(--bg-secondary));
  font-size: 12px;
  flex-shrink: 0;
}

.side-toggle {
  display: flex;
}

.side-toggle button {
  padding: 2px 10px;
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
}

.side-toggle button:first-child {
  border-radius: 4px 0 0 4px;
}

.side-toggle button:last-child {
  border-radius: 0 4px 4px 0;
  border-left: none;
}

.side-toggle button.active {
  background: var(--accent);
  color: var(--text-on-accent, #fff);
  border-color: var(--accent);
}

.status-badge {
  padding: 1px 6px;
  border-radius: 3px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font-size: 11px;
}
</style>
