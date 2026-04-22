<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import type { PendingToolConfirm } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import ToolConfirmCard from "./ToolConfirmCard.vue";
import ToolConfirmFeedbackForm from "./ToolConfirmFeedbackForm.vue";
import { subtitleForPendingToolConfirm, titleForPendingToolConfirm } from "./toolConfirmLabels";

const props = defineProps<{
  toolConfirms: PendingToolConfirm[];
}>();

const emit = defineEmits<{
  answer: [payload: { questionId: string; answer: string }];
  answerMany: [payload: { questionIds: string[]; answer: string }];
}>();

const selectedQuestionId = ref<string>("");

watch(
  () => props.toolConfirms.map((item) => item.questionId),
  (questionIds) => {
    if (questionIds.length === 0) {
      selectedQuestionId.value = "";
      return;
    }
    if (!questionIds.includes(selectedQuestionId.value)) {
      selectedQuestionId.value = questionIds[0] ?? "";
    }
  },
  { immediate: true },
);

const selectedToolConfirm = computed(() =>
  props.toolConfirms.find((item) => item.questionId === selectedQuestionId.value) ?? props.toolConfirms[0] ?? null,
);

const batchQuestionIds = computed(() => props.toolConfirms.map((item) => item.questionId));
const batchCountLabel = computed(() => t("chat.toolConfirm.batchCount", props.toolConfirms.length));

function answerAll(answer: string) {
  if (batchQuestionIds.value.length === 0) return;
  emit("answerMany", {
    questionIds: batchQuestionIds.value,
    answer,
  });
}
</script>

<template>
  <div class="tool-confirm-batch-card">
    <div class="tool-confirm-batch-header">
      <div class="tool-confirm-batch-heading">
        <div class="tool-confirm-batch-title">{{ t("chat.toolConfirm.batchTitle") }}</div>
        <div class="tool-confirm-batch-subtitle">{{ batchCountLabel }}</div>
      </div>
      <div class="tool-confirm-batch-actions">
        <BaseButton
          class="tool-confirm-batch-btn"
          variant="primary"
          size="md"
          @click="answerAll('allow')"
        >
          {{ t("chat.toolConfirm.allowAll") }}
        </BaseButton>
        <BaseButton
          class="tool-confirm-batch-btn"
          size="md"
          @click="answerAll('deny')"
        >
          {{ t("chat.toolConfirm.denyAll") }}
        </BaseButton>
      </div>
    </div>

    <ToolConfirmFeedbackForm
      :label="t('chat.toolConfirm.batchFeedbackLabel')"
      :placeholder="t('chat.toolConfirm.batchFeedbackPlaceholder')"
      :submit-label="t('chat.toolConfirm.batchFeedbackSubmit')"
      @submit="answerAll($event)"
    />

    <div class="tool-confirm-batch-body">
      <div class="tool-confirm-batch-list" role="tablist" :aria-label="t('chat.toolConfirm.batchListLabel')">
        <button
          v-for="(toolConfirm, index) in toolConfirms"
          :key="toolConfirm.questionId"
          type="button"
          class="tool-confirm-batch-item"
          :class="{ active: toolConfirm.questionId === selectedQuestionId }"
          @click="selectedQuestionId = toolConfirm.questionId"
        >
          <span class="tool-confirm-batch-item-index">{{ index + 1 }}</span>
          <span class="tool-confirm-batch-item-main">
            <span class="tool-confirm-batch-item-title">{{ titleForPendingToolConfirm(toolConfirm) }}</span>
            <span class="tool-confirm-batch-item-subtitle">{{ subtitleForPendingToolConfirm(toolConfirm) }}</span>
          </span>
        </button>
      </div>

      <div class="tool-confirm-batch-detail">
        <ToolConfirmCard
          v-if="selectedToolConfirm"
          :tool-confirm="selectedToolConfirm"
          @answer="emit('answer', { questionId: selectedToolConfirm.questionId, answer: $event })"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-confirm-batch-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px 16px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-radius: 12px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.tool-confirm-batch-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.tool-confirm-batch-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.tool-confirm-batch-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.tool-confirm-batch-subtitle {
  font-size: 12px;
  color: var(--text-secondary);
}

.tool-confirm-batch-actions {
  display: flex;
  gap: 8px;
}

.tool-confirm-batch-btn {
  min-width: 88px;
}

.tool-confirm-batch-body {
  display: grid;
  grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
  gap: 12px;
  min-height: 0;
}

.tool-confirm-batch-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 520px;
  overflow: auto;
  padding-right: 2px;
}

.tool-confirm-batch-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  width: 100%;
  padding: 9px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--sidebar-bg) 18%);
  color: var(--text-color);
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.tool-confirm-batch-item:hover {
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--hover-bg) 50%, var(--panel-bg) 50%);
}

.tool-confirm-batch-item.active {
  border-color: color-mix(in srgb, var(--accent-color) 40%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 8%, var(--panel-bg) 92%);
}

.tool-confirm-batch-item-index {
  flex: none;
  min-width: 18px;
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.6;
}

.tool-confirm-batch-item-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tool-confirm-batch-item-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.tool-confirm-batch-item-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono-identifier);
}

.tool-confirm-batch-detail {
  min-width: 0;
  min-height: 0;
}

.tool-confirm-batch-detail :deep(.ask-user-card),
.tool-confirm-batch-detail :deep(.knowledge-confirm-card) {
  margin: 0;
}

@media (max-width: 900px) {
  .tool-confirm-batch-header,
  .tool-confirm-batch-actions {
    flex-direction: column;
    align-items: stretch;
  }

  .tool-confirm-batch-body {
    grid-template-columns: minmax(0, 1fr);
  }

  .tool-confirm-batch-list {
    max-height: 220px;
  }
}
</style>
