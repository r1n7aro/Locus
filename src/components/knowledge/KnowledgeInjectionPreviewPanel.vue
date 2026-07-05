<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { listAgentInjectedItems } from "../../services/agent";
import { normalizeAppError } from "../../services/errors";
import { useAgentStore } from "../../stores/agent";
import type { InjectedPromptItem } from "../../types";
import { estimateTextTokens } from "../../utils/tokenEstimate";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{
  workingDir: string;
}>();

const agentStore = useAgentStore();

const loading = ref(false);
const error = ref("");
const injectedItems = ref<InjectedPromptItem[]>([]);
const selectedItemId = ref("");

const selectedAgentId = computed(() => agentStore.selectedAgentId.trim());
const selectedAgent = computed(() =>
  agentStore.agents.find((agent) => agent.id === selectedAgentId.value) ?? null,
);

function normalizeSectionId(value: string): string {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "section";
}

function splitKnowledgeItem(item: InjectedPromptItem): InjectedPromptItem[] {
  if (item.id !== "knowledge_context") return [item];

  const lines = item.content.replace(/\r\n/g, "\n").trim().split("\n");
  if (!lines.length) return [item];

  const sections: InjectedPromptItem[] = [];
  let currentTitle = "";
  let currentSuffix = "";
  let currentLines: string[] = [];

  function flush() {
    if (!currentTitle) return;
    const content = currentLines.join("\n").trim();
    if (!content) return;
    sections.push({
      ...item,
      id: `${item.id}:${currentSuffix}`,
      title: currentTitle,
      content,
    });
  }

  for (const line of lines) {
    if (!currentTitle && line.trim() === "## Knowledge") {
      continue;
    }

    const match = line.match(/^###\s+(.+)$/);
    if (match) {
      flush();
      currentTitle = match[1].trim();
      currentSuffix = normalizeSectionId(currentTitle);
      currentLines = [line];
      continue;
    }

    if (!currentTitle) continue;
    currentLines.push(line);
  }
  flush();

  return sections.length ? sections : [item];
}

function isKnowledgeInjectionItem(item: InjectedPromptItem): boolean {
  return item.id === "knowledge_context" || item.id.startsWith("knowledge_rule::");
}

const knowledgeItems = computed(() =>
  injectedItems.value
    .filter(isKnowledgeInjectionItem)
    .flatMap(splitKnowledgeItem),
);

const selectedItem = computed(() =>
  knowledgeItems.value.find((item) => item.id === selectedItemId.value)
  ?? knowledgeItems.value[0]
  ?? null,
);

const totalEstimatedTokens = computed(() =>
  knowledgeItems.value.reduce((total, item) => total + estimateTextTokens(item.content), 0),
);

const formatter = new Intl.NumberFormat("zh-CN");

function formatCount(value: number): string {
  return formatter.format(value);
}

function formatTokenCount(value: number): string {
  return t("knowledge.injectionPreview.tokenCount", formatCount(value));
}

function itemCategoryLabel(item: InjectedPromptItem): string {
  if (item.id.startsWith("knowledge_context")) {
    return t("knowledge.injectionPreview.category.knowledge");
  }
  return item.kind === "rule"
    ? t("knowledge.injectionPreview.category.rule")
    : t("knowledge.injectionPreview.category.context");
}

async function loadInjectedItems() {
  if (!props.workingDir.trim() || !selectedAgentId.value) {
    injectedItems.value = [];
    error.value = "";
    return;
  }

  loading.value = true;
  try {
    injectedItems.value = await listAgentInjectedItems(selectedAgentId.value);
    error.value = "";
  } catch (cause) {
    injectedItems.value = [];
    error.value = normalizeAppError(cause).message;
  } finally {
    loading.value = false;
  }
}

watch(
  knowledgeItems,
  (items) => {
    if (!items.length) {
      selectedItemId.value = "";
      return;
    }
    if (items.some((item) => item.id === selectedItemId.value)) return;
    selectedItemId.value = items[0]?.id ?? "";
  },
  { immediate: true },
);

watch(
  () => `${props.workingDir}::${selectedAgentId.value}`,
  () => {
    void loadInjectedItems();
  },
  { immediate: true },
);
</script>

<template>
  <div class="injection-preview-panel">
    <div class="injection-preview-header">
      <div class="injection-preview-head">
        <div class="injection-preview-title">{{ t("knowledge.injectionPreview.title") }}</div>
        <div class="injection-preview-estimate">
          <span class="injection-preview-estimate-label">
            {{ t("knowledge.injectionPreview.estimatedTokens") }}
          </span>
          <span class="injection-preview-estimate-value">
            {{ formatTokenCount(totalEstimatedTokens) }}
          </span>
        </div>
      </div>
      <div class="injection-preview-actions">
        <div class="injection-agent">
          <span class="injection-agent-label">{{ t("knowledge.injectionPreview.agentLabel") }}</span>
          <span class="injection-agent-value">{{ selectedAgent?.name || selectedAgentId || "—" }}</span>
        </div>
        <BaseButton :disabled="loading || !selectedAgentId" @click="loadInjectedItems">
          {{ t("common.refresh") }}
        </BaseButton>
      </div>
    </div>

    <div v-if="error" class="injection-error">{{ error }}</div>

    <div v-if="loading && !knowledgeItems.length" class="injection-empty">
      {{ t("common.loading") }}
    </div>
    <div v-else-if="!selectedAgentId" class="injection-empty">
      {{ t("knowledge.injectionPreview.noAgent") }}
    </div>
    <div v-else-if="!knowledgeItems.length" class="injection-empty">
      <div class="injection-empty-title">{{ t("knowledge.injectionPreview.emptyTitle") }}</div>
      <div class="injection-empty-hint">{{ t("knowledge.injectionPreview.emptyHint") }}</div>
    </div>
    <div v-else class="injection-layout">
      <section class="injection-section-list">
        <div class="injection-panel-title">{{ t("knowledge.injectionPreview.sectionsTitle") }}</div>
        <div class="injection-section-items">
          <button
            v-for="item in knowledgeItems"
            :key="item.id"
            type="button"
            class="injection-section-item"
            :class="{ active: selectedItem?.id === item.id }"
            @click="selectedItemId = item.id"
          >
            <span class="injection-section-main">
              <span class="injection-section-name">{{ item.title }}</span>
              <span class="injection-section-meta">
                {{ itemCategoryLabel(item) }} · {{ formatTokenCount(estimateTextTokens(item.content)) }}
              </span>
            </span>
          </button>
        </div>
      </section>

      <section class="injection-detail-panel">
        <div v-if="selectedItem" class="injection-detail-shell">
          <div class="injection-detail-header">
            <div class="injection-detail-head">
              <div class="injection-detail-title">{{ selectedItem.title }}</div>
              <div class="injection-detail-meta">
                {{ itemCategoryLabel(selectedItem) }} · {{ formatTokenCount(estimateTextTokens(selectedItem.content)) }}
              </div>
            </div>
          </div>
          <div class="injection-detail-body">
            <MarkdownRenderer :content="selectedItem.content" />
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.injection-preview-panel {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
}

.injection-preview-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 16px 20px 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
}

.injection-preview-head {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.injection-preview-title {
  font-size: 18px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
}

.injection-preview-estimate {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.injection-preview-estimate-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.injection-preview-estimate-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.injection-preview-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

.injection-agent {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
}

.injection-agent-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.injection-agent-value {
  max-width: 220px;
  font-size: 12px;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.injection-error {
  padding: 10px 20px;
  font-size: 12px;
  color: var(--status-danger-fg);
  border-bottom: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
}

.injection-empty {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 6px;
  padding: 24px;
  color: var(--text-secondary);
  text-align: center;
  font-size: 12px;
}

.injection-empty-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.injection-empty-hint {
  max-width: 540px;
  line-height: 1.6;
}

.injection-layout {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(260px, 320px) minmax(0, 1fr);
}

.injection-section-list,
.injection-detail-panel {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.injection-section-list {
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 52%, var(--panel-bg) 48%);
}

.injection-panel-title {
  padding: 12px 16px 10px;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.injection-section-items {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.injection-section-item {
  appearance: none;
  width: 100%;
  min-width: 0;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  display: flex;
  align-items: flex-start;
  text-align: left;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s;
}

.injection-section-item:hover {
  background: var(--hover-bg);
}

.injection-section-item.active {
  background: color-mix(in srgb, var(--active-bg, var(--hover-bg)) 88%, var(--panel-bg) 12%);
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
}

.injection-section-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.injection-section-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.4;
}

.injection-section-meta {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.4;
}

.injection-detail-panel {
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
}

.injection-detail-shell {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.injection-detail-header {
  padding: 14px 18px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--bg-color) 14%);
}

.injection-detail-head {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.injection-detail-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.injection-detail-meta {
  font-size: 12px;
  color: var(--text-secondary);
}

.injection-detail-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 18px 20px 22px;
}

@media (max-width: 1100px) {
  .injection-preview-header {
    align-items: stretch;
    flex-direction: column;
  }

  .injection-layout {
    grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
  }
}
</style>
