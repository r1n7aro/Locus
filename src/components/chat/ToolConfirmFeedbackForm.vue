<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import { encodeToolConfirmFeedback } from "./toolConfirmAnswer";

const emit = defineEmits<{
  submit: [answer: string];
}>();

const props = withDefaults(defineProps<{
  label?: string;
  placeholder?: string;
  submitLabel?: string;
}>(), {
  label: "",
  placeholder: "",
  submitLabel: "",
});

const feedback = ref("");

function submitFeedback() {
  const value = feedback.value.trim();
  if (!value) return;
  emit("submit", encodeToolConfirmFeedback(value));
  feedback.value = "";
}
</script>

<template>
  <div class="tool-confirm-feedback">
    <span class="tool-confirm-feedback-label">{{ props.label || t("chat.toolConfirm.feedbackLabel") }}</span>
    <div class="tool-confirm-feedback-row">
      <input
        v-model="feedback"
        class="tool-confirm-feedback-input"
        :placeholder="props.placeholder || t('chat.toolConfirm.feedbackPlaceholder')"
        @keydown.enter="submitFeedback"
      />
      <BaseButton
        class="tool-confirm-feedback-send"
        size="md"
        :disabled="!feedback.trim()"
        @click="submitFeedback"
      >
        {{ props.submitLabel || t("chat.toolConfirm.feedbackSubmit") }}
      </BaseButton>
    </div>
  </div>
</template>

<style scoped>
.tool-confirm-feedback {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-confirm-feedback-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.tool-confirm-feedback-row {
  display: flex;
  gap: 8px;
}

.tool-confirm-feedback-input {
  flex: 1;
  min-width: 0;
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--panel-bg));
  color: var(--text-color);
  font: inherit;
}

.tool-confirm-feedback-input:focus {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color));
}

.tool-confirm-feedback-send {
  flex: none;
  min-width: 88px;
}
</style>
