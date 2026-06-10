
<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import type { PendingQuestion, SheetField } from "../../types";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{
  question: PendingQuestion;
}>();

const emit = defineEmits<{
  answer: [answer: string];
}>();

const fields = computed<SheetField[]>(() => props.question.sheet?.fields ?? []);
const values = reactive<Record<string, string>>({});
const feedback = ref("");

watch(
  () => props.question.questionId,
  () => {
    for (const key of Object.keys(values)) delete values[key];
    for (const field of fields.value) values[field.key] = field.value;
    feedback.value = "";
  },
  { immediate: true },
);

const confirmLabel = computed(
  () => props.question.sheet?.confirmLabel || t("chat.sheet.confirm"),
);
const hasFeedback = computed(() => feedback.value.trim().length > 0);

function fieldId(field: SheetField): string {
  return `sheet-field-${props.question.questionId}-${field.key}`;
}

function isChanged(field: SheetField): boolean {
  return !field.readonly && (values[field.key] ?? field.value) !== field.value;
}

function selectOptions(field: SheetField): string[] {
  const options = field.options ?? [];
  const current = values[field.key] ?? field.value;
  return options.includes(current) ? options : [current, ...options];
}

function submitConfirm() {
  const submitted: Record<string, string> = {};
  for (const field of fields.value) {
    if (!field.readonly) submitted[field.key] = values[field.key] ?? field.value;
  }
  emit(
    "answer",
    JSON.stringify({ action: "confirm", values: submitted, feedback: feedback.value.trim() }),
  );
}

function submitFeedback() {
  if (!hasFeedback.value) return;
  emit("answer", JSON.stringify({ action: "feedback", feedback: feedback.value.trim() }));
}
</script>

<template>
  <div class="ask-user-card sheet-card">
    <div class="ask-question sheet-title">{{ question.question }}</div>
    <div v-if="question.sheet?.description" class="sheet-desc">{{ question.sheet.description }}</div>
    <div class="sheet-fields">
      <div
        v-for="field in fields"
        :key="field.key"
        class="sheet-field"
        :class="{ 'is-changed': isChanged(field) }"
      >
        <label class="sheet-field-label" :for="fieldId(field)">
          <span class="sheet-field-name">{{ field.label }}</span>
          <span v-if="isChanged(field)" class="sheet-field-changed">{{ t("chat.sheet.modified") }}</span>
        </label>
        <div v-if="field.description" class="sheet-field-desc">{{ field.description }}</div>
        <div v-if="field.readonly" class="sheet-field-readonly">{{ field.value }}</div>
        <select
          v-else-if="(field.options?.length ?? 0) > 0"
          :id="fieldId(field)"
          v-model="values[field.key]"
          class="sheet-field-control sheet-field-select"
        >
          <option v-for="option in selectOptions(field)" :key="option" :value="option">{{ option }}</option>
        </select>
        <textarea
          v-else-if="field.multiline"
          :id="fieldId(field)"
          v-model="values[field.key]"
          class="sheet-field-control sheet-field-textarea"
          rows="3"
        ></textarea>
        <input
          v-else
          :id="fieldId(field)"
          v-model="values[field.key]"
          class="sheet-field-control sheet-field-input"
          type="text"
        />
      </div>
    </div>
    <div class="sheet-footer">
      <textarea
        v-model="feedback"
        class="sheet-feedback-input"
        rows="2"
        :placeholder="t('chat.sheet.feedbackPlaceholder')"
        @keydown.enter.ctrl.prevent="submitFeedback"
      ></textarea>
      <div class="sheet-actions">
        <BaseButton
          class="sheet-feedback-btn"
          size="md"
          :disabled="!hasFeedback"
          @click="submitFeedback"
        >{{ t("chat.sheet.requestChanges") }}</BaseButton>
        <BaseButton
          class="sheet-confirm-btn"
          variant="primary"
          size="md"
          @click="submitConfirm"
        >{{ confirmLabel }}</BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sheet-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.sheet-title.ask-question {
  margin-bottom: 0;
}

.sheet-desc {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
  white-space: pre-wrap;
}

.sheet-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-height: 40vh;
  overflow-y: auto;
  padding-right: 2px;
}

.sheet-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.sheet-field-label {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.sheet-field-changed {
  font-size: 11px;
  font-weight: 500;
  color: var(--accent-color);
}

.sheet-field-desc {
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-secondary);
}

.sheet-field-readonly {
  padding: 6px 10px;
  border: 1px dashed var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--bg-color) 70%, transparent);
  color: var(--text-secondary);
  font-size: 13px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.sheet-field-control {
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-color);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  transition: border-color 0.15s;
  box-sizing: border-box;
}

.sheet-field-control:focus {
  border-color: var(--accent-color);
}

.sheet-field.is-changed .sheet-field-control {
  border-color: color-mix(in srgb, var(--accent-color) 60%, var(--border-color) 40%);
}

.sheet-field-textarea,
.sheet-feedback-input {
  resize: vertical;
  line-height: 1.5;
}

.sheet-footer {
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
  padding-top: 10px;
}

.sheet-feedback-input {
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-color);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  transition: border-color 0.15s;
  box-sizing: border-box;
}

.sheet-feedback-input:focus {
  border-color: var(--accent-color);
}

.sheet-feedback-input::placeholder,
.sheet-field-control::placeholder {
  color: var(--text-secondary);
}

.sheet-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
