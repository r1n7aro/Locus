<script setup lang="ts">
import { t } from "../../i18n";
import type { KnowledgeToolConfirmPreview as KnowledgeToolConfirmPreviewData } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import KnowledgeToolConfirmPreview from "./KnowledgeToolConfirmPreview.vue";
import ToolConfirmFeedbackForm from "./ToolConfirmFeedbackForm.vue";

defineProps<{
  preview: KnowledgeToolConfirmPreviewData;
}>();

const emit = defineEmits<{
  answer: [answer: string];
}>();
</script>

<template>
  <div class="knowledge-confirm-card">
    <KnowledgeToolConfirmPreview :preview="preview" />
    <ToolConfirmFeedbackForm @submit="emit('answer', $event)" />
    <div class="knowledge-confirm-actions">
      <BaseButton class="knowledge-confirm-btn" variant="primary" size="md" @click="emit('answer', 'allow')">
        {{ t("chat.toolConfirm.allow") }}
      </BaseButton>
      <BaseButton class="knowledge-confirm-btn" size="md" @click="emit('answer', 'deny')">
        {{ t("chat.toolConfirm.deny") }}
      </BaseButton>
    </div>
  </div>
</template>

<style scoped>
.knowledge-confirm-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: 12px;
  padding: 14px 16px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-radius: 12px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.knowledge-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.knowledge-confirm-btn {
  min-width: 72px;
}
</style>
