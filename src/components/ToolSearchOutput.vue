<script setup lang="ts">
import { t } from "../i18n";

import type { ToolSearchToolSummary } from "./toolSearchOutput";

defineProps<{
  tools: readonly ToolSearchToolSummary[];
}>();
</script>

<template>
  <div class="tool-search-output ui-select-text">
    <div class="tool-search-output-summary">
      {{ tools.length > 0
        ? t("tool.toolSearch.loaded", tools.length)
        : t("tool.toolSearch.empty") }}
    </div>
    <div v-if="tools.length > 0" class="tool-search-list" role="list">
      <div
        v-for="tool in tools"
        :key="tool.name"
        class="tool-search-item"
        role="listitem"
      >
        <code class="tool-search-name" :title="tool.name">{{ tool.name }}</code>
        <span v-if="tool.description" class="tool-search-description">
          {{ tool.description }}
        </span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-search-output {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.tool-search-output-summary {
  margin-bottom: 5px;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
}

.tool-search-list {
  overflow: hidden;
  border-top: 1px solid var(--border-color);
  border-bottom: 1px solid var(--border-color);
}

.tool-search-item {
  display: grid;
  grid-template-columns: minmax(150px, 0.36fr) minmax(0, 1fr);
  align-items: baseline;
  gap: 12px;
  min-width: 0;
  padding: 7px 8px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
}

.tool-search-item:first-child {
  border-top: 0;
}

.tool-search-item:nth-child(even) {
  background: color-mix(in srgb, var(--hover-bg) 42%, transparent);
}

.tool-search-name {
  min-width: 0;
  overflow: hidden;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.45;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-search-description {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@media (max-width: 720px) {
  .tool-search-item {
    grid-template-columns: minmax(0, 1fr);
    gap: 2px;
  }
}
</style>
