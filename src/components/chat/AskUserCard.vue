
<script setup lang="ts">
import type { PendingQuestion } from "../../types";
import { ref } from "vue";
import BaseButton from "../ui/BaseButton.vue";

defineProps<{
  question: PendingQuestion;
}>();

const emit = defineEmits<{
  answer: [answer: string];
}>();

const customAnswer = ref("");
</script>

<template>
  <div class="ask-user-card">
    <div class="ask-question">{{ question.question }}</div>
    <div class="ask-options">
      <BaseButton
        v-for="(opt, idx) in question.options.slice(0, -1)"
        :key="idx"
        class="ask-option-btn"
        block
        size="md"
        @click="emit('answer', opt.label)"
      >
        <span class="ask-option-label">{{ opt.label }}</span>
        <span class="ask-option-desc">{{ opt.description }}</span>
      </BaseButton>
      <div v-if="question.options.length > 0" class="ask-custom">
        <span class="ask-custom-label">{{ question.options[question.options.length - 1].label }}</span>
        <div class="ask-custom-input-row">
          <input
            v-model="customAnswer"
            class="ask-custom-input"
            :placeholder="question.options[question.options.length - 1].description"
            @keydown.enter="customAnswer.trim() && emit('answer', customAnswer.trim())"
          />
          <BaseButton
            class="ask-custom-send"
            variant="primary"
            size="md"
            :disabled="!customAnswer.trim()"
            @click="emit('answer', customAnswer.trim())"
          >&#8593;</BaseButton>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ask-option-btn {
  min-width: 0;
}

.ask-option-label,
.ask-option-desc {
  display: block;
  width: 100%;
  min-width: 0;
  white-space: normal;
}

.ask-option-desc {
  overflow-wrap: anywhere;
}
</style>
