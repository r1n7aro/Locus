<script setup lang="ts">
import type { KnowledgeLocalSourceMode } from "../../../types";
import BaseButton from "../../ui/BaseButton.vue";
import BaseSegmented from "../../ui/BaseSegmented.vue";
import type { ReferenceExternalImportLocalWindowModel } from "./referenceExternalImportModels";

defineProps<{
  model: ReferenceExternalImportLocalWindowModel;
}>();

const emit = defineEmits<{
  (e: "pick-file"): void;
  (e: "pick-folder"): void;
  (e: "update:mode", value: KnowledgeLocalSourceMode): void;
  (e: "update:ai-editable", value: boolean): void;
  (e: "update:target-name", value: string): void;
  (e: "sync"): void;
  (e: "delete"): void;
  (e: "cancel"): void;
  (e: "close"): void;
  (e: "start"): void;
}>();
</script>

<template>
  <div class="reference-local-pane">
    <div class="reference-local-summary">{{ model.summary }}</div>

    <div class="reference-local-source">
      <div class="reference-local-label">{{ model.sourceLabel }}</div>
      <div class="reference-local-source-row">
        <div
          class="reference-local-source-path"
          :class="{ 'is-placeholder': !model.sourcePath }"
        >
          {{ model.sourcePath || model.sourcePathPlaceholder }}
        </div>
        <BaseButton size="sm" :disabled="model.pickDisabled" @click="emit('pick-folder')">
          {{ model.pickFolderLabel }}
        </BaseButton>
        <BaseButton size="sm" :disabled="model.pickDisabled" @click="emit('pick-file')">
          {{ model.pickFileLabel }}
        </BaseButton>
      </div>
      <div v-if="model.previewText" class="reference-local-preview">{{ model.previewText }}</div>
      <div v-if="model.sourceMissingText" class="reference-local-warning">
        {{ model.sourceMissingText }}
      </div>
    </div>

    <div class="reference-local-config">
      <div class="reference-local-config-field">
        <div class="reference-local-label">{{ model.modeLabel }}</div>
        <BaseSegmented
          :model-value="model.mode"
          size="sm"
          :options="model.modeOptions"
          :aria-label="model.modeLabel"
          @update:model-value="emit('update:mode', $event as KnowledgeLocalSourceMode)"
        />
        <div class="reference-local-hint">{{ model.modeHint }}</div>
      </div>

      <div class="reference-local-config-field">
        <div class="reference-local-label">{{ model.targetNameLabel }}</div>
        <input
          class="reference-local-input"
          type="text"
          :value="model.targetName"
          :disabled="model.targetNameDisabled"
          @input="emit('update:target-name', ($event.target as HTMLInputElement).value)"
        />
        <label
          v-if="model.aiEditableVisible"
          class="reference-local-checkbox"
          :class="{ 'is-disabled': model.aiEditableDisabled }"
        >
          <input
            type="checkbox"
            :checked="model.aiEditableChecked"
            :disabled="model.aiEditableDisabled"
            @change="emit('update:ai-editable', ($event.target as HTMLInputElement).checked)"
          />
          <span>{{ model.aiEditableLabel }}</span>
        </label>
        <div v-if="model.aiEditableVisible" class="reference-local-hint">
          {{ model.aiEditableHint }}
        </div>
      </div>
    </div>

    <div class="reference-local-hero">
      <div class="reference-local-stage-title">{{ model.stageTitle }}</div>
      <div class="reference-local-stage-value">{{ model.progressLabel }}</div>
    </div>

    <div class="reference-local-track" aria-hidden="true">
      <div class="reference-local-track-fill" :style="{ width: `${model.progressRatio * 100}%` }" />
    </div>

    <div class="reference-local-detail">{{ model.detail }}</div>

    <div class="reference-local-rows">
      <div v-for="row in model.rows" :key="row.label" class="reference-local-row">
        <span>{{ row.label }}</span>
        <span :class="{ mono: row.mono }">{{ row.value }}</span>
      </div>
    </div>

    <div v-if="model.currentPath" class="reference-local-path">
      <div class="reference-local-path-label">{{ model.currentPathLabel }}</div>
      <div class="reference-local-path-value">{{ model.currentPath }}</div>
    </div>

    <div class="reference-local-actions">
      <BaseButton v-if="model.canDelete" variant="danger" @click="emit('delete')">
        {{ model.deleteLabel }}
      </BaseButton>
      <BaseButton
        v-if="model.canSync"
        :disabled="model.syncDisabled"
        @click="emit('sync')"
      >
        {{ model.syncLabel }}
      </BaseButton>
      <BaseButton
        v-if="model.canCancel"
        :disabled="model.cancelDisabled"
        @click="emit('cancel')"
      >
        {{ model.cancelLabel }}
      </BaseButton>
      <BaseButton
        v-else
        variant="primary"
        :disabled="model.primaryDisabled"
        @click="model.primaryClosesWindow ? emit('close') : emit('start')"
      >
        {{ model.primaryLabel }}
      </BaseButton>
    </div>
  </div>
</template>

<style scoped>
.reference-local-pane {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.reference-local-summary,
.reference-local-hint,
.reference-local-detail,
.reference-local-preview {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-local-label,
.reference-local-stage-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-local-source {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.reference-local-source-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.reference-local-source-path {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--input-bg) 88%, transparent);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}

.reference-local-source-path.is-placeholder {
  color: var(--text-secondary);
  font-family: inherit;
}

.reference-local-warning {
  font-size: 12px;
  line-height: 1.6;
  color: var(--status-danger-fg, var(--danger-color, #d9534f));
}

.reference-local-config {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}

.reference-local-config-field {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.reference-local-input {
  width: 100%;
  box-sizing: border-box;
  padding: 7px 10px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.5;
}

.reference-local-input:disabled {
  opacity: 0.6;
}

.reference-local-checkbox {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--text-color);
  cursor: pointer;
  user-select: none;
}

.reference-local-checkbox.is-disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.reference-local-checkbox input {
  margin: 0;
}

.reference-local-hero {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.reference-local-stage-title {
  font-size: 24px;
  line-height: 1.2;
}

.reference-local-stage-value {
  flex-shrink: 0;
  font-size: 28px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.reference-local-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.reference-local-track-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  border-radius: inherit;
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--accent-color) 74%, #ffffff 26%),
    var(--accent-color)
  );
}

.reference-local-rows {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-local-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-local-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.reference-local-row span.mono {
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-local-path {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-local-path-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.reference-local-path-value {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-local-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

@media (max-width: 640px) {
  .reference-local-config {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
