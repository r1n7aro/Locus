<script setup lang="ts">
import { t } from "../../i18n";
import type { UnityRunStatesOutputPreview } from "../../composables/unityRunStatesPreview";

withDefaults(defineProps<{
  preview: UnityRunStatesOutputPreview;
  hidePrints?: boolean;
}>(), {
  hidePrints: false,
});
</script>

<template>
  <div class="unity-run-states-output">
    <div v-if="preview.fields.length > 0" class="unity-run-output-summary">
      <div
        v-for="field in preview.fields"
        :key="field.key"
        class="unity-run-output-field"
        :class="{ wide: field.key === 'message' }"
      >
        <span class="unity-run-output-label">{{ field.label }}</span>
        <code class="unity-run-output-value">{{ field.value || "-" }}</code>
      </div>
    </div>

    <div v-if="preview.prints && !hidePrints" class="unity-run-output-prints">
      <div class="unity-run-output-section-label">{{ t("tool.unityRunStates.prints") }}</div>
      <pre class="unity-run-output-print-text ui-select-text">{{ preview.prints }}</pre>
    </div>
  </div>
</template>

<style scoped>
.unity-run-states-output {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.unity-run-output-summary {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 6px 10px;
  padding: 6px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-radius: 6px;
  background: var(--hover-bg);
}

.unity-run-output-field {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.unity-run-output-field.wide {
  grid-column: 1 / -1;
}

.unity-run-output-label {
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.3;
}

.unity-run-output-value {
  min-width: 0;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-run-output-field.wide .unity-run-output-value {
  white-space: normal;
  word-break: break-word;
}

.unity-run-output-prints {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.unity-run-output-section-label {
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  line-height: 1.4;
}

.unity-run-output-print-text {
  margin: 0;
  max-height: 320px;
  overflow: auto;
  padding: 7px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-radius: 6px;
  background: var(--hover-bg);
  color: var(--text-color);
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  scrollbar-gutter: stable;
}

@media (max-width: 720px) {
  .unity-run-output-summary {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
