<script setup lang="ts">
import type { KnowledgeSearchMatchKind, KnowledgeSearchResult } from "../../types";
import { t } from "../../i18n";

defineProps<{
  query: string;
  results: KnowledgeSearchResult[];
  searching: boolean;
  searchMode: KnowledgeSearchMatchKind | null;
  searchLatencyMs: number | null;
}>();

const emit = defineEmits<{
  (e: "selectResult", result: KnowledgeSearchResult): void;
}>();

function labelForType(type: KnowledgeSearchResult["type"]): string {
  switch (type) {
    case "design":
      return t("knowledge.type.design");
    case "memory":
      return t("knowledge.type.memory");
    case "skill":
      return t("knowledge.type.skill");
    case "reference":
      return t("knowledge.type.reference");
    default:
      return type;
  }
}

function labelForStoredScope(result: KnowledgeSearchResult): string {
  return result.storageSource === "app"
    ? t("knowledge.scope.user")
    : t("knowledge.scope.project");
}

function labelForMatchMode(kind: KnowledgeSearchResult["matchKind"]): string {
  switch (kind) {
    case "hybrid":
    case "grepHybrid":
      return t("knowledge.search.hybrid");
    case "semantic":
      return t("knowledge.search.semantic");
    case "grep":
      return t("knowledge.search.grep");
    default:
      return t("knowledge.search.lexical");
  }
}

</script>

<template>
  <div class="knowledge-search-results">
    <div v-if="query.trim() && searchLatencyMs !== null" class="search-results-header">
      <span class="search-results-title">{{ t("knowledge.search.resultsTitle", query.trim()) }}</span>
      <span class="search-results-meta">
        <template v-if="searchMode">{{ labelForMatchMode(searchMode) }} · </template>{{ searchLatencyMs }} ms
      </span>
    </div>

    <div class="search-results-list">
      <div v-if="searching" class="search-empty">{{ t("common.loading") }}</div>
      <div v-else-if="!results.length" class="search-empty">{{ t("knowledge.search.noResults") }}</div>
      <button
        v-for="result in results"
        :key="`${result.id}-${result.path}`"
        class="search-result-card"
        type="button"
        @click="emit('selectResult', result)"
      >
        <div class="result-header">
          <span class="result-title">{{ result.title }}</span>
          <span class="result-meta">{{ labelForType(result.type) }} · {{ labelForStoredScope(result) }}</span>
        </div>
        <div class="result-submeta">
          <span class="result-subpath">{{ result.path }}</span>
        </div>
        <div class="result-snippet">{{ result.snippet }}</div>
      </button>
    </div>
  </div>
</template>

<style scoped>
.knowledge-search-results {
  display: flex;
  flex-direction: column;
  flex: 1;
  overflow: hidden;
}

.search-results-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 10px 10px;
}

.search-results-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 2px 12px 10px;
  flex-shrink: 0;
}

.search-results-title {
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-results-meta {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
}

.search-empty {
  padding: 24px 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.search-result-card {
  width: 100%;
  margin-bottom: 8px;
  padding: 12px 12px 11px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  cursor: pointer;
  text-align: left;
}

.search-result-card:hover {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, var(--border-color) 70%, var(--text-secondary));
}

.result-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.result-title {
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-meta,
.result-subpath {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.result-submeta {
  display: flex;
  align-items: center;
  margin-top: 4px;
}

.result-snippet {
  margin-top: 8px;
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
}
</style>
