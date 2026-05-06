<script setup lang="ts">
import { computed } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import { t } from "../../i18n";
import type { KnowledgeProposal } from "../../types";

const props = defineProps<{
  proposal: KnowledgeProposal;
}>();

const emit = defineEmits<{
  apply: [proposalId: string];
  ignore: [proposalId: string];
}>();

const summaryText = computed(() => {
  const count = props.proposal.items.length;
  const confidence = Math.round(props.proposal.confidence * 100);
  return t("knowledge.proposal.summary", count, confidence);
});

const reminderText = computed(() => t("knowledge.proposal.reminder"));

function labelForItemKind(kind: string): string {
  return kind === "memory" ? "Memory" : "Knowledge";
}

function labelForItemMode(mode: string): string {
  switch (mode) {
    case "replace":
      return t("knowledge.proposal.mode.replace");
    case "create_source":
      return t("knowledge.proposal.mode.createSource");
    case "update_source":
      return t("knowledge.proposal.mode.updateSource");
    default:
      return mode;
  }
}
</script>

<template>
  <div class="knowledge-card">
    <div class="knowledge-card-header">
      <div class="knowledge-card-title">{{ t("knowledge.proposal.title") }}</div>
      <div class="knowledge-card-meta">
        <span>{{ summaryText }}</span>
      </div>
    </div>
    <div class="knowledge-card-reminder">
      {{ reminderText }}
    </div>

    <div class="knowledge-card-items">
      <div
        v-for="(item, index) in proposal.items"
        :key="`${proposal.proposalId}-${index}`"
        class="knowledge-card-item"
      >
        <div class="knowledge-card-item-main">
          <span class="knowledge-card-item-kind">{{ labelForItemKind(item.kind) }}</span>
          <span class="knowledge-card-item-target">{{ item.target }}</span>
        </div>
        <div class="knowledge-card-item-mode">{{ labelForItemMode(item.mode) }}</div>
      </div>
    </div>

    <details class="knowledge-card-preview">
      <summary>{{ t("knowledge.proposal.previewDraft") }}</summary>
      <div class="knowledge-card-preview-list">
        <section
          v-for="(item, index) in proposal.items"
          :key="`${proposal.proposalId}-preview-${index}`"
          class="knowledge-card-preview-item"
        >
          <div class="knowledge-card-preview-label">
            {{ labelForItemKind(item.kind) }} · {{ item.target }}
          </div>
          <pre>{{ item.draft }}</pre>
        </section>
      </div>
    </details>

    <div class="knowledge-card-actions">
      <template v-if="proposal.status === 'pending'">
        <BaseButton variant="neutral" @click="emit('ignore', proposal.proposalId)">{{ t("knowledge.proposal.ignore") }}</BaseButton>
        <BaseButton variant="primary" @click="emit('apply', proposal.proposalId)">{{ t("knowledge.proposal.apply") }}</BaseButton>
      </template>
      <template v-else-if="proposal.status === 'applying'">
        <span class="knowledge-card-status">{{ t("knowledge.proposal.applying") }}</span>
      </template>
      <template v-else-if="proposal.status === 'applied'">
        <span class="knowledge-card-status success">{{ t("knowledge.proposal.applied") }}</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
.knowledge-card {
  margin: 6px 0 2px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  padding: 12px;
}

.knowledge-card-header {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.knowledge-card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.knowledge-card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.knowledge-card-reminder {
  margin-top: 10px;
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--bg-color) 72%, transparent);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.knowledge-card-items {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.knowledge-card-item {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--bg-color) 68%, transparent);
}

.knowledge-card-item-main {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.knowledge-card-item-kind {
  flex: none;
  font-size: 11px;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.knowledge-card-item-target {
  min-width: 0;
  font-size: 13px;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.knowledge-card-item-mode {
  flex: none;
  font-size: 12px;
  color: var(--text-secondary);
}

.knowledge-card-preview {
  margin-top: 10px;
  border-top: 1px solid var(--border-color);
  padding-top: 10px;
}

.knowledge-card-preview summary {
  cursor: pointer;
  font-size: 12px;
  color: var(--text-secondary);
  user-select: none;
}

.knowledge-card-preview-list {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.knowledge-card-preview-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.knowledge-card-preview-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.knowledge-card-preview pre {
  margin: 0;
  padding: 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--bg-color) 74%, transparent);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--font-mono-block);
}

.knowledge-card-actions {
  margin-top: 12px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.knowledge-card-status {
  font-size: 12px;
  color: var(--text-secondary);
}

.knowledge-card-status.success {
  color: var(--accent-color);
}
</style>
