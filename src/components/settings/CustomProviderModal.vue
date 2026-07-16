<script setup lang="ts">
import { computed, nextTick, ref, watch, type Ref } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import BaseButton from "../ui/BaseButton.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import ProviderCatalogStep from "./ProviderCatalogStep.vue";
import ProviderModelsEditor from "./ProviderModelsEditor.vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  customEndpointTestDetail,
  customEndpointTestHtmlPath,
} from "../../services/customEndpointTestResult";
import {
  catalogKeyPlaceholder,
  catalogProviderToCustomProvider,
  newCustomProvider,
} from "../../services/modelCatalog";
import type {
  ApiFormat,
  CustomProvider,
  CustomProviderModel,
  ModelCatalogProvider,
  ModelCatalogResponse,
} from "../../types";

const provider = defineModel<CustomProvider | null>("provider", { required: true });

const props = defineProps<{
  isAdding: boolean;
  saving?: boolean;
  testStatus: "idle" | "testing" | "success" | "error";
  testResult: string;
  catalog: ModelCatalogResponse | null;
  catalogLoading?: boolean;
  catalogRefreshing?: boolean;
}>();

const emit = defineEmits<{
  close: [];
  save: [];
  test: [modelRowId?: string];
  refreshCatalog: [];
  openCatalog: [];
}>();

/** Add flow: pick a provider first, then configure it. Edit flow: config only. */
const stage = ref<"pick" | "config">("config");
const localError = ref("");
/** Field the current localError points at; drives inline highlighting. */
const invalidField = ref<"name" | "endpoint" | "models" | null>(null);
const lastTestedApiModel = ref("");

const nameInput = ref<HTMLInputElement | null>(null);
const endpointInput = ref<HTMLTextAreaElement | null>(null);
const apiKeyInput = ref<HTMLInputElement | null>(null);

function focusSoon(target: Ref<HTMLInputElement | HTMLTextAreaElement | null>) {
  void nextTick(() => target.value?.focus());
}

/** The endpoint edits in an auto-growing textarea so long URLs stay fully
 *  visible; URLs never contain whitespace, so strip it (incl. pasted
 *  newlines) as it is typed. */
function updateEndpoint(event: Event) {
  if (!provider.value) return;
  const el = event.target as HTMLTextAreaElement;
  const cleaned = el.value.replace(/\s+/g, "");
  if (cleaned !== el.value) el.value = cleaned;
  provider.value.endpoint = cleaned;
  clearInvalid("endpoint");
}

// Distinguish "modal opened" (null → provider) from the internal draft swaps a
// catalog pick performs (provider → new provider object): only opening resets
// the wizard.
watch(
  () => provider.value,
  (current, previous) => {
    if (!current || previous) return;
    stage.value = props.isAdding ? "pick" : "config";
    localError.value = "";
    invalidField.value = null;
    lastTestedApiModel.value = "";
    emit("openCatalog");
    if (!props.isAdding) focusSoon(apiKeyInput);
  },
  { immediate: true },
);

const modalTitle = computed(() =>
  props.isAdding ? t("settings.custom.addProvider") : t("settings.custom.editProvider"),
);
const testResultText = computed(() => customEndpointTestDetail(props.testResult));
const testResultHtmlPath = computed(() => customEndpointTestHtmlPath(props.testResult));

const catalogProviderForDraft = computed<ModelCatalogProvider | null>(() => {
  const catalogId = provider.value?.catalogId;
  return catalogId ? props.catalog?.providers[catalogId] ?? null : null;
});

/** Where the draft came from, shown next to the title as an orientation cue. */
const catalogSourceName = computed(() => {
  if (stage.value !== "config") return "";
  const catalogId = provider.value?.catalogId;
  if (!catalogId) return "";
  return catalogProviderForDraft.value?.name ?? catalogId;
});

/** Editing an existing provider: an empty box means "keep the saved key", so
 *  that hint lives in the placeholder instead of cluttering the label row. */
const keyPlaceholder = computed(() => {
  if (!props.isAdding) return t("settings.custom.apiKeyKeepHint");
  const catalogProvider = catalogProviderForDraft.value;
  const env = catalogProvider ? catalogKeyPlaceholder(catalogProvider) : null;
  return env
    ? t("settings.custom.apiKeyEnvPlaceholder", env)
    : t("settings.custom.apiKeyPlaceholder");
});

function clearInvalid(field: "name" | "endpoint") {
  if (invalidField.value !== field) return;
  invalidField.value = null;
  localError.value = "";
}

function pickCatalogProvider(providerId: string, catalogProvider: ModelCatalogProvider) {
  const draft = catalogProviderToCustomProvider(providerId, catalogProvider);
  provider.value = draft;
  localError.value = "";
  invalidField.value = null;
  stage.value = "config";
  // Gateway-only providers ship without an endpoint: put the cursor on the
  // missing URL; otherwise the API key is the only blank the user must fill.
  focusSoon(draft.endpoint.trim() ? apiKeyInput : endpointInput);
}

function pickManualProvider() {
  provider.value = newCustomProvider();
  localError.value = "";
  invalidField.value = null;
  stage.value = "config";
  focusSoon(nameInput);
}

function backToPick() {
  localError.value = "";
  invalidField.value = null;
  stage.value = "pick";
}

const apiFormatOptions = computed(() => [
  { value: "openai_chat", label: t("settings.custom.formatOpenaiChat") },
  { value: "openai_responses", label: t("settings.custom.formatOpenaiResponses") },
  { value: "anthropic_messages", label: t("settings.custom.formatAnthropicMessages") },
]);

function updateProviderApiFormat(value: string) {
  if (!provider.value) return;
  provider.value.apiFormat = value as ApiFormat;
  for (const model of provider.value.models) {
    model.reasoningParamFormat = null; // re-derive from the new format on save
  }
}

function validate(): boolean {
  const draft = provider.value;
  if (!draft) return false;
  if (!draft.name.trim()) {
    localError.value = t("settings.custom.nameRequired");
    invalidField.value = "name";
    focusSoon(nameInput);
    return false;
  }
  if (!draft.endpoint.trim()) {
    localError.value = t("settings.custom.endpointRequired");
    invalidField.value = "endpoint";
    focusSoon(endpointInput);
    return false;
  }
  if (!draft.models.some((model) => model.apiModel.trim())) {
    localError.value = t("settings.custom.apiModelRequired");
    invalidField.value = "models";
    return false;
  }
  localError.value = "";
  invalidField.value = null;
  return true;
}

function handleSave() {
  if (!validate()) return;
  emit("save");
}

function handleFooterTest() {
  const model = provider.value?.models.find((m) => m.apiModel.trim());
  lastTestedApiModel.value = model?.apiModel ?? "";
  localError.value = "";
  invalidField.value = null;
  emit("test", undefined);
}

function handleModelTest(model: CustomProviderModel) {
  if (!model.id) return; // the editor assigns row ids before emitting
  lastTestedApiModel.value = model.apiModel;
  localError.value = "";
  invalidField.value = null;
  emit("test", model.id);
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

function handleDialogKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && !props.saving) emit("close");
}
</script>

<template>
  <Transition name="modal">
    <div v-if="provider" class="provider-modal-overlay" @mousedown.self="!saving && emit('close')">
      <div
        class="custom-provider-dialog"
        role="dialog"
        aria-modal="true"
        @keydown="handleDialogKeydown"
      >
        <div class="provider-modal-header">
          <div class="provider-modal-header-lead">
            <button
              v-if="isAdding && stage === 'config'"
              class="header-back"
              type="button"
              :disabled="saving"
              :title="t('settings.custom.backToProviders')"
              :aria-label="t('settings.custom.backToProviders')"
              @click="backToPick"
            >
              <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
                <path d="M10.03 12.53a.75.75 0 0 1-1.06 0l-4-4a.75.75 0 0 1 0-1.06l4-4a.75.75 0 1 1 1.06 1.06L6.56 8l3.47 3.47a.75.75 0 0 1 0 1.06z"/>
              </svg>
            </button>
            <span class="provider-modal-title">{{ modalTitle }}</span>
            <span v-if="catalogSourceName" class="header-source">{{ catalogSourceName }}</span>
          </div>
          <button class="close-btn" type="button" :disabled="saving" @click="emit('close')">
            <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
              <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </button>
        </div>

        <div v-if="stage === 'pick'" class="pick-body">
          <ProviderCatalogStep
            :catalog="catalog"
            :loading="catalogLoading"
            :refreshing="catalogRefreshing"
            :disabled="saving"
            @pick-catalog="pickCatalogProvider"
            @pick-manual="pickManualProvider"
            @refresh="emit('refreshCatalog')"
          />
        </div>

        <div v-else class="config-body">
          <aside class="config-side">
            <div class="config-field">
              <label class="config-label">{{ t("settings.custom.apiKey") }}</label>
              <input
                ref="apiKeyInput"
                v-model="provider.apiKey"
                class="config-input mono-input"
                type="password"
                :disabled="saving"
                :placeholder="keyPlaceholder"
              />
            </div>
            <div class="config-field">
              <label class="config-label">{{ t("settings.custom.name") }}</label>
              <input
                ref="nameInput"
                v-model="provider.name"
                class="config-input"
                :class="{ invalid: invalidField === 'name' }"
                type="text"
                :disabled="saving"
                :placeholder="t('settings.custom.namePlaceholder')"
                @input="clearInvalid('name')"
              />
            </div>
            <div class="config-field">
              <label class="config-label">{{ t("settings.custom.endpoint") }}</label>
              <textarea
                ref="endpointInput"
                :value="provider.endpoint"
                class="config-input mono-input endpoint-input"
                :class="{ invalid: invalidField === 'endpoint' }"
                rows="1"
                spellcheck="false"
                :disabled="saving"
                :placeholder="t('settings.custom.endpointPlaceholder')"
                @input="updateEndpoint"
                @keydown.enter.prevent
              ></textarea>
            </div>
            <div class="config-field">
              <label class="config-label">{{ t("settings.custom.apiFormat") }}</label>
              <BaseDropdown
                size="md"
                menu-align="start"
                :model-value="provider.apiFormat"
                :options="apiFormatOptions"
                :aria-label="t('settings.custom.apiFormat')"
                :disabled="saving"
                @update:model-value="updateProviderApiFormat"
              />
            </div>
          </aside>

          <section class="config-main">
            <ProviderModelsEditor
              :key="provider.id"
              :provider="provider"
              :catalog-provider-id="provider.catalogId ?? null"
              :catalog-provider="catalogProviderForDraft"
              :catalog="catalog"
              :catalog-loading="catalogLoading"
              :catalog-refreshing="catalogRefreshing"
              :saving="saving"
              :testing="testStatus === 'testing'"
              :highlight-missing="invalidField === 'models'"
              @test="handleModelTest"
              @refresh-catalog="emit('refreshCatalog')"
            />
          </section>
        </div>

        <div v-if="stage === 'config' && (localError || testStatus !== 'idle')" class="provider-modal-status">
          <div v-if="localError" class="local-error">{{ localError }}</div>
          <div v-else class="endpoint-test-result" :class="testStatus">
            <span v-if="testStatus === 'testing'" class="endpoint-test-spinner"></span>
            <span v-if="testStatus === 'testing'">{{ t("settings.custom.testing") }}</span>
            <span v-else-if="testStatus === 'success'" class="endpoint-test-ok">{{ t("settings.custom.testOk") }}</span>
            <span v-else-if="testStatus === 'error'" class="endpoint-test-err">{{ t("settings.custom.testFail") }}</span>
            <span v-if="lastTestedApiModel" class="endpoint-test-target mono">{{ lastTestedApiModel }}</span>
            <span v-if="testResultText" class="endpoint-test-detail">{{ testResultText }}</span>
            <button
              v-if="testResultHtmlPath"
              type="button"
              class="endpoint-test-link"
              @click="openTestHtml"
            >
              {{ t("settings.custom.openInBrowser") }}
            </button>
          </div>
        </div>

        <div class="provider-modal-footer">
          <template v-if="stage === 'config'">
            <BaseButton variant="primary" type="button" :disabled="saving" @click="handleSave">
              {{ saving ? '...' : t("settings.custom.save") }}
            </BaseButton>
            <BaseButton
              type="button"
              :disabled="saving || testStatus === 'testing'"
              @click="handleFooterTest"
            >
              {{ testStatus === 'testing' ? '...' : t("settings.custom.test") }}
            </BaseButton>
          </template>
          <BaseButton type="button" :disabled="saving" @click="emit('close')">
            {{ t("settings.custom.cancel") }}
          </BaseButton>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.provider-modal-overlay {
  position: absolute;
  inset: 0;
  background: rgba(8, 10, 14, 0.28);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.custom-provider-dialog {
  background: var(--surface-elevated);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  width: 1080px;
  max-width: calc(100% - 48px);
  height: min(720px, 92%);
  display: flex;
  flex-direction: column;
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
}

.provider-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 20px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.provider-modal-header-lead {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.header-back {
  width: 26px;
  height: 26px;
  margin-left: -6px;
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
  flex-shrink: 0;
}

.header-back:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.header-back:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.provider-modal-title {
  font-size: 14px;
  font-weight: 700;
  white-space: nowrap;
}

.header-source {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--hover-bg);
  border-radius: 10px;
  padding: 3px 8px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
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
  flex-shrink: 0;
}

.close-btn:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.close-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}



.pick-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  overflow: hidden;
  padding: 14px 20px 16px;
}

/* Config stage: connection sidebar + models pane, each with its own scroll,
 * so growing model cards can never squeeze the connection fields out. */
.config-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: row;
  align-items: stretch;
  overflow: hidden;
}

.config-side {
  width: 320px;
  flex-shrink: 0;
  overflow-y: auto;
  padding: 14px 16px 16px;
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.config-main {
  flex: 1;
  min-width: 0;
  display: flex;
  padding: 14px 18px 6px;
  overflow: hidden;
}

.config-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.config-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.config-input {
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
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.mono-input {
  font-family: var(--font-mono-editor);
}

.config-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.config-input.invalid {
  border-color: var(--status-danger-border);
  background: color-mix(in srgb, var(--input-bg) 85%, var(--status-danger-bg) 15%);
}

/* Auto-grows with content so the full URL is always visible. */
.endpoint-input {
  field-sizing: content;
  resize: none;
  overflow: hidden;
  word-break: break-all;
  line-height: 1.5;
  font-size: 12px;
}

.config-input:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.provider-modal-status {
  flex-shrink: 0;
  padding: 8px 20px 0;
}

.local-error {
  padding: 8px 10px;
  border-radius: 6px;
  border: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.5;
}

.endpoint-test-result {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 12px;
  line-height: 1.5;
  flex-wrap: wrap;
  max-height: 120px;
  overflow-y: auto;
}

.endpoint-test-result.testing {
  background: var(--hover-bg);
  color: var(--text-secondary);
}

.endpoint-test-result.success {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
}

.endpoint-test-result.error {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.endpoint-test-ok,
.endpoint-test-err {
  font-weight: 600;
}

.endpoint-test-ok {
  color: var(--status-good-fg);
}

.endpoint-test-err {
  color: var(--status-danger-fg);
}

.endpoint-test-target {
  font-size: 11px;
  opacity: 0.85;
}

.endpoint-test-detail {
  color: var(--text-secondary);
  word-break: break-all;
  width: 100%;
}

.endpoint-test-link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  font: inherit;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.endpoint-test-spinner {
  width: 10px;
  height: 10px;
  align-self: center;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: provider-modal-spin 0.8s linear infinite;
  flex-shrink: 0;
}

.provider-modal-footer {
  display: flex;
  gap: 8px;
  padding: 12px 20px 16px;
  border-top: 1px solid var(--border-color);
  flex-shrink: 0;
  margin-top: 8px;
}

.mono {
  font-family: var(--font-mono-identifier);
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.15s ease;
}

.modal-enter-active .custom-provider-dialog,
.modal-leave-active .custom-provider-dialog {
  transition: transform 0.15s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-from .custom-provider-dialog,
.modal-leave-to .custom-provider-dialog {
  transform: scale(0.95) translateY(8px);
}

@keyframes provider-modal-spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 860px) {
  .custom-provider-dialog {
    width: min(900px, calc(100% - 24px));
    max-width: calc(100% - 24px);
  }

  .config-body {
    flex-direction: column;
    overflow-y: auto;
  }

  .config-side {
    width: auto;
    flex-shrink: 0;
    overflow: visible;
    border-right: none;
    border-bottom: 1px solid var(--border-color);
  }

  .config-main {
    overflow: visible;
    flex: none;
  }
}
</style>
