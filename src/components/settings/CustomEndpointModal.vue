<script setup lang="ts">
import { computed, watch } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  customEndpointTestDetail,
  customEndpointTestHtmlPath,
} from "../../services/customEndpointTestResult";
import type {
  ApiFormat,
  CustomEndpoint,
  EffortLevel,
  ReasoningParamFormat,
} from "../../types";

const endpoint = defineModel<CustomEndpoint | null>("endpoint", { required: true });

const props = defineProps<{
  isAdding: boolean;
  saving?: boolean;
  testStatus: "idle" | "testing" | "success" | "error";
  testResult: string;
}>();

const emit = defineEmits<{
  close: [];
  save: [];
  test: [];
}>();

const customReasoningEffortOptions = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Med" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
] satisfies Array<{ value: EffortLevel; label: string }>;

const customReasoningFormatOptions = [
  { value: "none", label: t("settings.custom.reasoningNone") },
  { value: "openai_chat_reasoning_effort", label: t("settings.custom.reasoningOpenaiChat") },
  { value: "openai_responses_reasoning_effort", label: t("settings.custom.reasoningOpenaiResponses") },
  { value: "anthropic_thinking", label: t("settings.custom.reasoningAnthropic") },
] satisfies Array<{ value: ReasoningParamFormat; label: string }>;

const modalTitle = computed(() => props.isAdding ? t("settings.custom.add") : t("settings.custom.edit"));
const testResultText = computed(() => customEndpointTestDetail(props.testResult));
const testResultHtmlPath = computed(() => customEndpointTestHtmlPath(props.testResult));
const showReplayReasoningContent = computed(() =>
  endpoint.value?.apiFormat === "openai_chat" || endpoint.value?.apiFormat === "anthropic_messages"
);

watch(
  () => [endpoint.value?.id, endpoint.value?.apiFormat] as const,
  () => {
    if (endpoint.value) {
      endpoint.value.supportsToolLazyLoading = false;
    }
  },
  { immediate: true },
);

function defaultReasoningParamFormat(apiFormat: ApiFormat): ReasoningParamFormat {
  switch (apiFormat) {
    case "openai_responses": return "openai_responses_reasoning_effort";
    case "anthropic_messages": return "anthropic_thinking";
    default: return "openai_chat_reasoning_effort";
  }
}

function defaultReplayReasoningContent(apiFormat: ApiFormat): boolean {
  return apiFormat === "openai_chat";
}

function updateEndpointApiFormat(event: Event) {
  if (!endpoint.value) return;
  const apiFormat = (event.target as HTMLSelectElement).value as ApiFormat;
  endpoint.value.apiFormat = apiFormat;
  endpoint.value.reasoningParamFormat = defaultReasoningParamFormat(apiFormat);
  endpoint.value.replayReasoningContent = defaultReplayReasoningContent(apiFormat);
  endpoint.value.supportsToolLazyLoading = false;
}

function setReasoningEffortEnabled(effort: EffortLevel, enabled: boolean) {
  if (!endpoint.value) return;
  if (!endpoint.value.supportedReasoningEfforts) endpoint.value.supportedReasoningEfforts = [];
  const idx = endpoint.value.supportedReasoningEfforts.indexOf(effort);
  if (enabled && idx < 0) {
    endpoint.value.supportedReasoningEfforts.push(effort);
  } else if (!enabled && idx >= 0) {
    endpoint.value.supportedReasoningEfforts.splice(idx, 1);
  }
}

function setBetaFlagEnabled(flag: string, enabled: boolean) {
  if (!endpoint.value) return;
  if (!endpoint.value.betaFlags) endpoint.value.betaFlags = [];
  const idx = endpoint.value.betaFlags.indexOf(flag);
  if (enabled && idx < 0) {
    endpoint.value.betaFlags.push(flag);
  } else if (!enabled && idx >= 0) {
    endpoint.value.betaFlags.splice(idx, 1);
  }
}

function setWebSearchEnabled(enabled: boolean) {
  if (!endpoint.value) return;
  endpoint.value.serverTools = {
    ...(endpoint.value.serverTools ?? { webSearch: false }),
    webSearch: enabled,
  };
}

async function openTestHtml() {
  const path = testResultHtmlPath.value;
  if (!path) return;
  try {
    await openPath(path);
  } catch (e) {
    const err = normalizeAppError(e);
    window.alert(t("settings.custom.openTestHtmlFailed", path, err.message));
  }
}

function handleEndpointKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && !props.saving) emit("close");
}
</script>

<template>
  <Transition name="modal">
    <div v-if="endpoint" class="modal-overlay" @mousedown.self="!saving && emit('close')">
      <div class="modal-dialog custom-endpoint-dialog" role="dialog" aria-modal="true">
        <div class="modal-header">
          <span class="modal-title">{{ modalTitle }}</span>
          <button class="close-btn" type="button" :disabled="saving" @click="emit('close')">
            <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </button>
        </div>

        <div class="modal-body custom-endpoint-body">
          <div class="custom-form-stack">
            <div class="custom-form-row">
              <label class="custom-form-label">{{ t("settings.custom.name") }}</label>
              <input
                v-model="endpoint.name"
                class="key-input"
                type="text"
                :disabled="saving"
                :placeholder="t('settings.custom.namePlaceholder')"
                @keydown="handleEndpointKeydown"
              />
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.apiModel") }}
                <span class="custom-form-hint">{{ t("settings.custom.apiModelHint") }}</span>
              </label>
              <input
                v-model="endpoint.apiModel"
                class="key-input"
                type="text"
                :disabled="saving"
                :placeholder="t('settings.custom.apiModelPlaceholder')"
                @keydown="handleEndpointKeydown"
              />
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.endpoint") }}
                <span class="custom-form-hint">{{ t("settings.custom.endpointHint") }}</span>
              </label>
              <input
                v-model="endpoint.endpoint"
                class="key-input"
                type="text"
                :disabled="saving"
                :placeholder="t('settings.custom.endpointPlaceholder')"
                @keydown="handleEndpointKeydown"
              />
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">{{ t("settings.custom.apiFormat") }}</label>
              <select
                :value="endpoint.apiFormat"
                class="model-select"
                :disabled="saving"
                @change="updateEndpointApiFormat"
              >
                <option value="openai_chat">{{ t("settings.custom.formatOpenaiChat") }}</option>
                <option value="openai_responses">{{ t("settings.custom.formatOpenaiResponses") }}</option>
                <option value="anthropic_messages">{{ t("settings.custom.formatAnthropicMessages") }}</option>
              </select>
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.apiKey") }}
                <span class="custom-form-hint">{{ t("settings.custom.apiKeyOptional") }}</span>
              </label>
              <input
                v-model="endpoint.apiKey"
                class="key-input"
                type="password"
                :disabled="saving"
                :placeholder="t('settings.custom.apiKeyPlaceholder')"
                @keydown="handleEndpointKeydown"
              />
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.contextLength") }}
                <span class="custom-form-hint">{{ t("settings.custom.contextLengthHint") }}</span>
              </label>
              <input
                v-model.number="endpoint.contextLength"
                class="key-input number-input"
                type="number"
                :disabled="saving"
                min="1024"
                step="1024"
                placeholder="256000"
                @keydown="handleEndpointKeydown"
              />
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">{{ t("settings.custom.reasoningFormat") }}</label>
              <select v-model="endpoint.reasoningParamFormat" class="model-select" :disabled="saving">
                <option
                  v-for="option in customReasoningFormatOptions"
                  :key="option.value"
                  :value="option.value"
                >
                  {{ option.label }}
                </option>
              </select>
            </div>
            <div v-if="showReplayReasoningContent" class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.replayReasoningContent") }}
                <span class="custom-form-hint">{{ t("settings.custom.replayReasoningContentHint") }}</span>
              </label>
              <div class="custom-checkbox-control">
                <BaseCheckbox
                  v-model="endpoint.replayReasoningContent"
                  :disabled="saving"
                  :aria-label="t('settings.custom.replayReasoningContent')"
                />
              </div>
            </div>
            <div v-if="endpoint.reasoningParamFormat !== 'none'" class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.reasoningEfforts") }}
                <span class="custom-form-hint">{{ t("settings.custom.reasoningEffortsHint") }}</span>
              </label>
              <div class="custom-effort-options">
                <div
                  v-for="option in customReasoningEffortOptions"
                  :key="option.value"
                  class="custom-option-row compact"
                >
                  <BaseCheckbox
                    :disabled="saving"
                    :model-value="endpoint.supportedReasoningEfforts?.includes(option.value) ?? false"
                    :aria-label="option.label"
                    @update:model-value="setReasoningEffortEnabled(option.value, $event)"
                  />
                  <span class="custom-option-name mono">{{ option.label }}</span>
                </div>
              </div>
            </div>
            <div v-if="endpoint.apiFormat === 'anthropic_messages'" class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.betaFlags") }}
                <span class="custom-form-hint">{{ t("settings.custom.betaFlagsHint") }}</span>
              </label>
              <div class="custom-options-list">
                <div class="custom-option-row">
                  <BaseCheckbox
                    :disabled="saving"
                    :model-value="endpoint.betaFlags?.includes('context-1m-2025-08-07') ?? false"
                    aria-label="context-1m-2025-08-07"
                    @update:model-value="setBetaFlagEnabled('context-1m-2025-08-07', $event)"
                  />
                  <div class="custom-option-copy inline">
                    <span class="custom-option-name mono">context-1m-2025-08-07</span>
                    <span class="custom-option-desc">{{ t("settings.custom.betaContext1m") }}</span>
                  </div>
                </div>
                <div class="custom-option-row">
                  <BaseCheckbox
                    :disabled="saving"
                    :model-value="endpoint.betaFlags?.includes('interleaved-thinking-2025-05-14') ?? false"
                    aria-label="interleaved-thinking-2025-05-14"
                    @update:model-value="setBetaFlagEnabled('interleaved-thinking-2025-05-14', $event)"
                  />
                  <div class="custom-option-copy inline">
                    <span class="custom-option-name mono">interleaved-thinking-2025-05-14</span>
                    <span class="custom-option-desc">{{ t("settings.custom.betaInterleavedThinking") }}</span>
                  </div>
                </div>
                <div class="custom-option-row">
                  <BaseCheckbox
                    :disabled="saving"
                    :model-value="endpoint.betaFlags?.includes('prompt-caching-scope-2026-01-05') ?? false"
                    aria-label="prompt-caching-scope-2026-01-05"
                    @update:model-value="setBetaFlagEnabled('prompt-caching-scope-2026-01-05', $event)"
                  />
                  <div class="custom-option-copy inline">
                    <span class="custom-option-name mono">prompt-caching-scope-2026-01-05</span>
                    <span class="custom-option-desc">{{ t("settings.custom.betaPromptCaching") }}</span>
                  </div>
                </div>
              </div>
            </div>
            <div v-if="endpoint.apiFormat === 'anthropic_messages'" class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.serverTools") }}
                <span class="custom-form-hint">{{ t("settings.custom.serverToolsHint") }}</span>
              </label>
              <div class="custom-options-list">
                <div class="custom-option-row">
                  <BaseCheckbox
                    :disabled="saving"
                    :model-value="endpoint.serverTools?.webSearch ?? false"
                    aria-label="web_search"
                    @update:model-value="setWebSearchEnabled"
                  />
                  <div class="custom-option-copy inline">
                    <span class="custom-option-name mono">web_search</span>
                    <span class="custom-option-desc">{{ t("settings.custom.serverToolWebSearch") }}</span>
                  </div>
                </div>
              </div>
            </div>
            <div class="custom-form-row">
              <label class="custom-form-label">
                {{ t("settings.custom.imageUnderstanding") }}
                <span class="custom-form-hint">{{ t("settings.custom.imageUnderstandingHint") }}</span>
              </label>
              <div class="custom-checkbox-control">
                <BaseCheckbox
                  v-model="endpoint.supportsVision"
                  :disabled="saving"
                  :aria-label="t('settings.custom.imageUnderstanding')"
                />
              </div>
            </div>
          </div>
          <div v-if="testStatus !== 'idle'" class="test-result" :class="testStatus">
            <span v-if="testStatus === 'testing'" class="codex-spinner"></span>
            <span v-if="testStatus === 'testing'">{{ t("settings.custom.testing") }}</span>
            <span v-else-if="testStatus === 'success'" class="test-ok">{{ t("settings.custom.testOk") }}</span>
            <span v-else-if="testStatus === 'error'" class="test-err">{{ t("settings.custom.testFail") }}</span>
            <span v-if="testResultText" class="test-detail">{{ testResultText }}</span>
            <button
              v-if="testResultHtmlPath"
              type="button"
              class="test-result-link"
              @click="openTestHtml"
            >
              {{ t("settings.custom.openInBrowser") }}
            </button>
          </div>
        </div>

        <div class="modal-footer">
          <BaseButton variant="primary" type="button" :disabled="saving" @click="emit('save')">
            {{ saving ? '...' : t("settings.custom.save") }}
          </BaseButton>
          <BaseButton type="button" @click="emit('test')" :disabled="saving || testStatus === 'testing'">
            {{ testStatus === 'testing' ? '...' : t("settings.custom.test") }}
          </BaseButton>
          <BaseButton type="button" :disabled="saving" @click="emit('close')">{{ t("settings.custom.cancel") }}</BaseButton>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.modal-overlay {
  position: absolute;
  inset: 0;
  background: rgba(8, 10, 14, 0.28);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.modal-dialog {
  background: var(--surface-elevated);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  width: 500px;
  max-width: calc(100% - 48px);
  max-height: 84%;
  display: flex;
  flex-direction: column;
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
}

.modal-dialog.custom-endpoint-dialog {
  width: 560px;
  max-width: calc(100% - 48px);
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.modal-title {
  font-size: 14px;
  font-weight: 700;
}

.modal-body {
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow-y: auto;
}

.modal-body.custom-endpoint-body {
  padding: 14px 20px 16px;
  gap: 14px;
}

.custom-form-stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.modal-footer {
  display: flex;
  gap: 8px;
  padding: 12px 20px 16px;
  border-top: 1px solid var(--border-color);
  flex-shrink: 0;
}

.close-btn {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s ease, color 0.15s ease;
  box-shadow: none;
  padding: 0;
}

.close-btn:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.close-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.custom-endpoint-body .custom-form-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.custom-endpoint-body .custom-form-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  flex-wrap: wrap;
  gap: 2px;
  min-width: 0;
  line-height: 1.35;
  padding-top: 0;
}

.custom-endpoint-body .custom-form-hint {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-secondary);
  min-width: 0;
}

.key-input {
  flex: 1;
  width: 100%;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: var(--font-mono-editor);
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.number-input {
  max-width: 180px;
}

.key-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.model-select {
  width: 100%;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease;
  appearance: none;
  -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' fill='%23999' viewBox='0 0 16 16'%3E%3Cpath d='M4.47 5.97a.75.75 0 0 1 1.06 0L8 8.44l2.47-2.47a.75.75 0 1 1 1.06 1.06l-3 3a.75.75 0 0 1-1.06 0l-3-3a.75.75 0 0 1 0-1.06z'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 10px center;
  padding-right: 28px;
  box-sizing: border-box;
}

.model-select:focus {
  border-color: var(--accent-border);
  background-color: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.key-input:disabled,
.model-select:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.custom-options-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.custom-effort-options {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px 18px;
  min-height: 32px;
}

.custom-option-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  min-height: 18px;
}

.custom-option-row.compact {
  align-items: center;
  min-width: 0;
  min-height: 24px;
  gap: 6px;
}

.custom-checkbox-control {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
  min-width: 0;
}

.custom-option-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.custom-option-copy.inline {
  flex-direction: row;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 2px 8px;
}

.custom-option-name {
  font-size: 12px;
  line-height: 18px;
  color: var(--text-color);
}

.custom-option-name.mono {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
}

.custom-option-desc {
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
}

.test-result {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 12px;
  line-height: 1.5;
  flex-wrap: wrap;
}

.test-result.testing {
  background: var(--hover-bg);
  color: var(--text-secondary);
}

.test-result.success {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
}

.test-result.error {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.test-ok,
.test-err {
  font-weight: 600;
}

.test-ok {
  color: var(--status-good-fg);
}

.test-err {
  color: var(--status-danger-fg);
}

.test-detail {
  color: var(--text-secondary);
  word-break: break-all;
  width: 100%;
}

.test-result-link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  font: inherit;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.codex-spinner {
  width: 10px;
  height: 10px;
  margin-top: 3px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.15s ease;
}

.modal-enter-active .modal-dialog,
.modal-leave-active .modal-dialog {
  transition: transform 0.15s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-from .modal-dialog,
.modal-leave-to .modal-dialog {
  transform: scale(0.95) translateY(8px);
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 700px) {
  .modal-dialog.custom-endpoint-dialog {
    width: min(560px, calc(100% - 24px));
    max-width: calc(100% - 24px);
  }

  .number-input {
    max-width: none;
  }
}
</style>
