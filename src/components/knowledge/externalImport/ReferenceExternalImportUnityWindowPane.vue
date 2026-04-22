<script setup lang="ts">
import { t } from "../../../i18n";
import type { UnityReferenceImportLocale } from "../../../types";
import BaseButton from "../../ui/BaseButton.vue";
import BaseDropdown from "../../ui/BaseDropdown.vue";
import type { ReferenceExternalImportUnityWindowModel } from "./referenceExternalImportModels";

defineProps<{
  model: ReferenceExternalImportUnityWindowModel;
}>();

const emit = defineEmits<{
  (e: "update:locale", value: UnityReferenceImportLocale): void;
  (e: "open-existing"): void;
  (e: "delete"): void;
  (e: "cancel"): void;
  (e: "close"): void;
  (e: "start"): void;
}>();
</script>

<template>
  <div class="reference-unity-pane">
    <div class="reference-unity-summary">{{ model.summary }}</div>

    <div class="reference-unity-config">
      <div class="reference-unity-config-copy">
        <div class="reference-unity-config-label">
          {{ t("knowledge.referenceImport.window.language") }}
        </div>
        <div class="reference-unity-config-hint">
          {{ t("knowledge.referenceImport.window.languageHint") }}
        </div>
      </div>
      <BaseDropdown
        :model-value="model.locale"
        class="reference-unity-locale"
        size="md"
        :disabled="model.localeDisabled"
        :options="model.localeOptions"
        :aria-label="t('knowledge.referenceImport.window.language')"
        @update:model-value="emit('update:locale', $event as UnityReferenceImportLocale)"
      />
    </div>

    <div v-if="model.foreignBindingText" class="reference-unity-note">
      <span>{{ model.foreignBindingText }}</span>
      <button
        v-if="model.canOpenExisting"
        type="button"
        class="reference-unity-link"
        @click="emit('open-existing')"
      >
        {{ model.openExistingLabel }}
      </button>
    </div>

    <div class="reference-unity-hero">
      <div class="reference-unity-hero-copy">
        <div class="reference-unity-stage-title">{{ model.stageTitle }}</div>
        <div class="reference-unity-stage-caption">{{ model.stageCaption }}</div>
      </div>
      <div class="reference-unity-stage-value">{{ model.progressLabel }}</div>
    </div>

    <div class="reference-unity-track" aria-hidden="true">
      <div class="reference-unity-track-fill" :style="{ width: `${model.progressRatio * 100}%` }" />
    </div>

    <div class="reference-unity-stage-list">
      <div
        v-for="item in model.stageItems"
        :key="item.key"
        class="reference-unity-stage-row"
        :class="{
          'is-complete': item.complete,
          'is-current': item.current,
          'is-error': item.error,
        }"
      >
        <div class="reference-unity-stage-head">
          <span class="reference-unity-stage-dot"></span>
          <span class="reference-unity-stage-name">{{ item.label }}</span>
        </div>
        <div class="reference-unity-stage-status">{{ item.statusText }}</div>
        <div class="reference-unity-stage-track" aria-hidden="true">
          <div
            class="reference-unity-stage-track-fill"
            :style="{ width: `${Math.round(item.progress * 100)}%` }"
          />
        </div>
      </div>
    </div>

    <div class="reference-unity-rows">
      <div
        v-for="row in model.rows"
        :key="row.label"
        class="reference-unity-row"
      >
        <span>{{ row.label }}</span>
        <span :class="{ mono: row.mono }">{{ row.value }}</span>
      </div>
    </div>

    <div class="reference-unity-detail">
      {{ model.detail }}
    </div>

    <div v-if="model.currentPath" class="reference-unity-path">
      <div class="reference-unity-path-label">{{ model.currentPathLabel }}</div>
      <div class="reference-unity-path-value">{{ model.currentPath }}</div>
    </div>

    <div class="reference-unity-actions">
      <BaseButton
        v-if="model.canDelete"
        variant="danger"
        @click="emit('delete')"
      >
        {{ model.deleteLabel }}
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
.reference-unity-pane {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.reference-unity-summary,
.reference-unity-config-hint,
.reference-unity-stage-caption,
.reference-unity-detail,
.reference-unity-note {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-unity-config {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
}

.reference-unity-config-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.reference-unity-config-label,
.reference-unity-stage-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-unity-locale {
  width: 180px;
  flex-shrink: 0;
}

.reference-unity-note {
  display: flex;
  align-items: center;
  gap: 8px;
}

.reference-unity-link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  font: inherit;
  cursor: pointer;
}

.reference-unity-link:hover {
  text-decoration: underline;
}

.reference-unity-hero {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.reference-unity-hero-copy {
  min-width: 0;
}

.reference-unity-stage-title {
  font-size: 24px;
  line-height: 1.2;
}

.reference-unity-stage-caption {
  margin-top: 4px;
}

.reference-unity-stage-value {
  flex-shrink: 0;
  font-size: 28px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.reference-unity-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.reference-unity-track-fill {
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

.reference-unity-stage-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.reference-unity-stage-row {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--input-bg) 16%);
  color: var(--text-secondary);
}

.reference-unity-stage-row.is-complete,
.reference-unity-stage-row.is-current {
  color: var(--text-color);
}

.reference-unity-stage-row.is-current {
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--accent-soft) 30%);
}

.reference-unity-stage-row.is-error {
  color: var(--status-danger-fg, var(--text-color));
  border-color: color-mix(in srgb, var(--danger-color, #d9534f) 28%, var(--border-color));
}

.reference-unity-stage-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.reference-unity-stage-dot {
  width: 7px;
  height: 7px;
  flex-shrink: 0;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 60%, transparent);
}

.reference-unity-stage-row.is-complete .reference-unity-stage-dot,
.reference-unity-stage-row.is-current .reference-unity-stage-dot {
  background: color-mix(in srgb, var(--accent-color) 76%, white 24%);
}

.reference-unity-stage-row.is-error .reference-unity-stage-dot {
  background: var(--danger-color, #d9534f);
}

.reference-unity-stage-name,
.reference-unity-stage-status {
  font-size: 11px;
  line-height: 1.4;
}

.reference-unity-stage-track {
  position: relative;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 82%, var(--border-color) 18%);
  overflow: hidden;
}

.reference-unity-stage-track-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent-color) 78%, white 22%);
}

.reference-unity-rows {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-unity-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-unity-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.reference-unity-row span.mono {
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-unity-path {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-unity-path-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.reference-unity-path-value {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-unity-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

@media (max-width: 960px) {
  .reference-unity-config {
    flex-direction: column;
    align-items: stretch;
  }

  .reference-unity-locale {
    width: 100%;
  }
}

@media (max-width: 640px) {
  .reference-unity-stage-list {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
