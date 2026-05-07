<script setup lang="ts">
import { computed } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
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

function defaultReasoningParamFormat(apiFormat: ApiFormat): ReasoningParamFormat {
  switch (apiFormat) {
    case "openai_responses": return "openai_responses_reasoning_effort";
    case "anthropic_messages": return "anthropic_thinking";
    default: return "openai_chat_reasoning_effort";
  }
}

function updateEndpointApiFormat(event: Event) {
  if (!endpoint.value) return;
  const apiFormat = (event.target as HTMLSelectElement).value as ApiFormat;
  endpoint.value.apiFormat = apiFormat;
  endpoint.value.reasoningParamFormat = defaultReasoningParamFormat(apiFormat);
}

function toggleReasoningEffort(effort: EffortLevel) {
  if (!endpoint.value) return;
  if (!endpoint.value.supportedReasoningEfforts) endpoint.value.supportedReasoningEfforts = [];
  const idx = endpoint.value.supportedReasoningEfforts.indexOf(effort);
  if (idx >= 0) {
    endpoint.value.supportedReasoningEfforts.splice(idx, 1);
  } else {
    endpoint.value.supportedReasoningEfforts.push(effort);
  }
}

function toggleBetaFlag(flag: string) {
  if (!endpoint.value) return;
  if (!endpoint.value.betaFlags) endpoint.value.betaFlags = [];
  const idx = endpoint.value.betaFlags.indexOf(flag);
  if (idx >= 0) {
    endpoint.value.betaFlags.splice(idx, 1);
  } else {
    endpoint.value.betaFlags.push(flag);
  }
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
      <div class="modal-dialog" role="dialog" aria-modal="true">
        <div class="modal-header">
          <span class="modal-title">{{ modalTitle }}</span>
          <button class="close-btn" type="button" :disabled="saving" @click="emit('close')">
            <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </button>
        </div>

        <div class="modal-body">
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
              class="key-input"
              type="number"
              :disabled="saving"
              min="1024"
              step="1024"
              placeholder="128000"
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
          <div v-if="endpoint.reasoningParamFormat !== 'none'" class="custom-form-row">
            <label class="custom-form-label">
              {{ t("settings.custom.reasoningEfforts") }}
              <span class="custom-form-hint">{{ t("settings.custom.reasoningEffortsHint") }}</span>
            </label>
            <div class="beta-flags-list">
              <label
                v-for="option in customReasoningEffortOptions"
                :key="option.value"
                class="beta-flag-item"
              >
                <input
                  type="checkbox"
                  :disabled="saving"
                  :checked="endpoint.supportedReasoningEfforts?.includes(option.value)"
                  @change="toggleReasoningEffort(option.value)"
                />
                <span class="beta-flag-name">{{ option.label }}</span>
              </label>
            </div>
          </div>
          <div v-if="endpoint.apiFormat === 'anthropic_messages'" class="custom-form-row">
            <label class="custom-form-label">
              {{ t("settings.custom.betaFlags") }}
              <span class="custom-form-hint">{{ t("settings.custom.betaFlagsHint") }}</span>
            </label>
            <div class="beta-flags-list">
              <label class="beta-flag-item">
                <input
                  type="checkbox"
                  :disabled="saving"
                  :checked="endpoint.betaFlags?.includes('context-1m-2025-08-07')"
                  @change="toggleBetaFlag('context-1m-2025-08-07')"
                />
                <span class="beta-flag-name">context-1m-2025-08-07</span>
                <span class="beta-flag-desc">{{ t("settings.custom.betaContext1m") }}</span>
              </label>
              <label class="beta-flag-item">
                <input
                  type="checkbox"
                  :disabled="saving"
                  :checked="endpoint.betaFlags?.includes('interleaved-thinking-2025-05-14')"
                  @change="toggleBetaFlag('interleaved-thinking-2025-05-14')"
                />
                <span class="beta-flag-name">interleaved-thinking-2025-05-14</span>
                <span class="beta-flag-desc">{{ t("settings.custom.betaInterleavedThinking") }}</span>
              </label>
              <label class="beta-flag-item">
                <input
                  type="checkbox"
                  :disabled="saving"
                  :checked="endpoint.betaFlags?.includes('prompt-caching-scope-2026-01-05')"
                  @change="toggleBetaFlag('prompt-caching-scope-2026-01-05')"
                />
                <span class="beta-flag-name">prompt-caching-scope-2026-01-05</span>
                <span class="beta-flag-desc">{{ t("settings.custom.betaPromptCaching") }}</span>
              </label>
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
          <button class="save-btn" type="button" :disabled="saving" @click="emit('save')">
            {{ saving ? '...' : t("settings.custom.save") }}
          </button>
          <button class="test-btn" type="button" @click="emit('test')" :disabled="saving || testStatus === 'testing'">
            {{ testStatus === 'testing' ? '...' : t("settings.custom.test") }}
          </button>
          <button class="cancel-btn" type="button" :disabled="saving" @click="emit('close')">{{ t("settings.custom.cancel") }}</button>
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
  width: 420px;
  max-width: 90%;
  max-height: 80%;
  display: flex;
  flex-direction: column;
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
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

.custom-form-row {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.custom-form-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  display: flex;
  align-items: baseline;
  gap: 6px;
}

.custom-form-hint {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-secondary);
}

.key-input {
  flex: 1;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: var(--font-mono-editor);
  outline: none;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.key-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.model-select {
  width: 100%;
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
}

.model-select:focus {
  border-color: var(--accent-border);
  background-color: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.key-input:disabled,
.model-select:disabled,
.beta-flag-item input[type="checkbox"]:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.beta-flags-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.beta-flag-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  cursor: pointer;
}

.beta-flag-item input[type="checkbox"] {
  margin: 0;
  cursor: pointer;
}

.beta-flag-name {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  color: var(--text-color);
}

.beta-flag-desc {
  font-size: 11px;
  color: var(--text-secondary);
  margin-left: 2px;
}

.save-btn {
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid var(--accent-color);
  background: var(--accent-color);
  color: #fff;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: filter 0.15s ease, opacity 0.15s ease;
  box-shadow: none;
  white-space: nowrap;
}

.save-btn:hover:not(:disabled) {
  filter: brightness(1.06);
}

.save-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.cancel-btn,
.test-btn {
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  box-shadow: none;
  white-space: nowrap;
}

.cancel-btn {
  padding-inline: 10px;
  color: var(--text-secondary);
}

.cancel-btn:hover:not(:disabled),
.test-btn:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--accent-border);
  color: var(--accent-color);
}

.cancel-btn:disabled,
.test-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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
</style>
