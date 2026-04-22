
<script setup lang="ts">
import { computed } from "vue";
import type { BasicToolConfirmDisplay, KnowledgeToolConfirmPreview, PendingToolConfirm } from "../../types";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import KnowledgeToolConfirmCard from "./KnowledgeToolConfirmCard.vue";
import ToolConfirmFeedbackForm from "./ToolConfirmFeedbackForm.vue";

const props = defineProps<{
  toolConfirm: PendingToolConfirm;
}>();

const emit = defineEmits<{
  answer: [answer: string];
}>();

function isKnowledgePreview(
  display: PendingToolConfirm["display"],
): display is KnowledgeToolConfirmPreview {
  return display.kind === "knowledge";
}

function isBasicDisplay(
  display: PendingToolConfirm["display"],
): display is BasicToolConfirmDisplay {
  return display.kind === "basic";
}

const knowledgeDisplay = computed(() =>
  isKnowledgePreview(props.toolConfirm.display) ? props.toolConfirm.display : null,
);

const basicDisplay = computed(() =>
  isBasicDisplay(props.toolConfirm.display) ? props.toolConfirm.display : null,
);

function formatToolArgs(raw: string): string {
  try {
    const obj = JSON.parse(raw);
    const pretty = JSON.stringify(obj, null, 2);
    return pretty.length > 500 ? pretty.slice(0, 500) + "\n..." : pretty;
  } catch {
    return raw.length > 500 ? raw.slice(0, 500) + "..." : raw;
  }
}
</script>

<template>
  <KnowledgeToolConfirmCard
    v-if="knowledgeDisplay"
    :preview="knowledgeDisplay"
    @answer="emit('answer', $event)"
  />
  <div v-else class="ask-user-card tool-confirm-card">
    <div class="tool-confirm-header">
      <span class="tool-confirm-icon">
        <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M8 1a3.5 3.5 0 0 0-3.5 3.5v1H3.25A1.25 1.25 0 0 0 2 6.75v7A1.25 1.25 0 0 0 3.25 15h9.5A1.25 1.25 0 0 0 14 13.75v-7A1.25 1.25 0 0 0 12.75 5.5H11.5v-1A3.5 3.5 0 0 0 8 1zm-2 4.5v-1a2 2 0 1 1 4 0v1H6z"/>
        </svg>
      </span>
      <span class="tool-confirm-title">{{ t('chat.toolConfirm.title') }}</span>
    </div>
    <template v-if="basicDisplay">
      <div class="tool-confirm-body">
        <div class="tool-confirm-name">{{ basicDisplay.toolName }}</div>
        <pre class="tool-confirm-args">{{ formatToolArgs(basicDisplay.arguments) }}</pre>
      </div>
    </template>
    <ToolConfirmFeedbackForm @submit="emit('answer', $event)" />
    <div class="tool-confirm-actions">
      <BaseButton class="tool-confirm-btn" variant="primary" size="md" @click="emit('answer', 'allow')">{{ t('chat.toolConfirm.allow') }}</BaseButton>
      <BaseButton class="tool-confirm-btn" size="md" @click="emit('answer', 'deny')">{{ t('chat.toolConfirm.deny') }}</BaseButton>
    </div>
  </div>
</template>
