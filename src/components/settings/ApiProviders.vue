<script setup lang="ts">
import BaseSegmented from "../ui/BaseSegmented.vue";
import { computed } from "vue";
import { locale, t } from "../../i18n";
import type {
  ModelOption,
  CustomEndpoint,
  ApiFormat,
  CodexTransportMode,
} from "../../types";
import type { CodexQuotaState, CodexQuotaWindowState, CodexStatusState, ProviderStatus } from "../../composables/useSettingsState";
import { visibleProviderOrder } from "../../config/providerVisibility";

interface ModelGroup {
  provider: string;
  label: string;
  models: ModelOption[];
}

const props = defineProps<{
  providers: ProviderStatus[];
  editingProvider: string | null;
  editKey: string;
  errorMsg: string;
  successMsg: string;
  isLoading: boolean;
  oauthStep: "idle" | "waiting_code" | "exchanging";
  oauthCode: string;
  codexStep: "idle" | "opening" | "waiting" | "success";
  codexStatus: CodexStatusState;
  codexQuota: CodexQuotaState;
  codexRetrying: boolean;
  codexTransport: CodexTransportMode;
  codexUserCode: string;
  codexUrl: string;
  codexCodeCopied: boolean;
  allModels: ModelOption[];
  customEndpoints: CustomEndpoint[];
  customEndpointSaving?: boolean;
  mode?: "full" | "onboarding";
  onboardingFocus?: "custom" | "codex" | null;
}>();

const emit = defineEmits<{
  startEdit: [providerId: string];
  cancelEdit: [];
  saveKey: [providerId: string];
  deleteKey: [providerId: string];
  handleKeydown: [e: KeyboardEvent, providerId: string];
  startOAuthLogin: [];
  submitOAuthCode: [];
  cancelOAuth: [];
  oauthLogout: [];
  handleOAuthKeydown: [e: KeyboardEvent];
  startCodexLogin: [];
  cancelCodexLogin: [];
  codexLogout: [];
  retryCodexValidation: [];
  refreshCodexQuota: [];
  copyCode: [];
  "update:codexTransport": [value: CodexTransportMode];
  startAddEndpoint: [];
  startEditEndpoint: [ep: CustomEndpoint];
  deleteEndpoint: [id: string];
  "update:editKey": [value: string];
  "update:oauthCode": [value: string];
}>();

const anthropicProvider = computed(() => props.providers.find((p) => p.id === "anthropic"));
const isOnboardingMode = computed(() => props.mode === "onboarding");
const thirdPartyProviders = computed(() =>
  props.providers.filter(
    (p) => p.id !== "anthropic" && p.id !== "anthropic_sdk" && p.id !== "openrouter",
  ),
);

function providerMeta(id: string): { desc: string; url: string; placeholder: string } {
  switch (id) {
    case "openrouter":
      return {
        desc: t("settings.provider.openrouter.desc"),
        url: "https://openrouter.ai/keys",
        placeholder: "sk-or-...",
      };
    case "anthropic":
      return {
        desc: t("settings.provider.anthropic.desc"),
        url: "",
        placeholder: "",
      };
    default:
      return { desc: "", url: "", placeholder: "sk-..." };
  }
}

function providerLabel(provider: string): string {
  const labels: Record<string, string> = {
    openrouter: "OpenRouter",
    anthropic: t("model.provider.anthropic"),
    anthropic_sdk: t("model.provider.anthropic_sdk"),
    openai_codex: t("model.provider.openai"),
    custom: t("model.provider.custom"),
  };
  return labels[provider] || provider;
}

function groupedAllModels(): ModelGroup[] {
  const map = new Map<string, ModelOption[]>();
  for (const m of props.allModels) {
    const list = map.get(m.provider) || [];
    list.push(m);
    map.set(m.provider, list);
  }
  const groups: ModelGroup[] = [];
  for (const provider of visibleProviderOrder) {
    const models = map.get(provider);
    if (models && models.length > 0) {
      groups.push({ provider, label: providerLabel(provider), models });
    }
  }
  return groups;
}

function formatLabel(fmt: ApiFormat): string {
  switch (fmt) {
    case "openai_chat": return t("settings.custom.formatOpenaiChat");
    case "openai_responses": return t("settings.custom.formatOpenaiResponses");
    case "anthropic_messages": return t("settings.custom.formatAnthropicMessages");
    default: return fmt;
  }
}

const codexTransportOptions = [
  {
    value: "http",
    label: t("settings.codex.transportHttp"),
    hint: t("settings.codex.transportHttpHint"),
  },
  {
    value: "websocket",
    label: t("settings.codex.transportWebsocket"),
    hint: t("settings.codex.transportWebsocketHint"),
  },
] satisfies Array<{ value: CodexTransportMode; label: string; hint: string }>;

function updateCodexTransport(value: string) {
  emit("update:codexTransport", value === "websocket" ? "websocket" : "http");
}

function focusSectionClass(section: "custom" | "codex") {
  return {
    "focus-section": isOnboardingMode.value && props.onboardingFocus === section,
  };
}

function formatQuotaPercent(value: number): string {
  return Math.round(Math.max(0, Math.min(100, value))).toString();
}

function formatQuotaWindowLabel(window: CodexQuotaWindowState): string {
  const minutes = window.windowMinutes;
  let label: string;
  if (!minutes || minutes <= 0) {
    label = window.windowType === "primary"
      ? t("settings.codex.quotaWindowPrimary")
      : t("settings.codex.quotaWindowSecondary");
  } else if (minutes % 10080 === 0) {
    label = t("settings.codex.quotaWindowWeeks", minutes / 10080);
  } else if (minutes % 1440 === 0) {
    label = t("settings.codex.quotaWindowDays", minutes / 1440);
  } else if (minutes % 60 === 0) {
    label = t("settings.codex.quotaWindowHours", minutes / 60);
  } else {
    label = t("settings.codex.quotaWindowMinutes", minutes);
  }

  if (window.limitId !== "codex") {
    return `${window.limitName || window.limitId} ${label}`;
  }
  return label;
}

function formatQuotaReset(resetsAt: number | null): string {
  if (!resetsAt) return "";
  const date = new Date(resetsAt * 1000);
  if (Number.isNaN(date.getTime())) return "";
  const dateLocale = locale.value === "zh" ? "zh-CN" : "en-US";
  const dateLabel = date.toLocaleDateString(dateLocale, {
    month: "2-digit",
    day: "2-digit",
  });
  const timeLabel = date.toLocaleTimeString(dateLocale, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${dateLabel} ${timeLabel}`;
}

function quotaBarStyle(window: CodexQuotaWindowState) {
  return {
    width: `${formatQuotaPercent(window.remainingPercent)}%`,
  };
}

function quotaCreditsLabel() {
  const credits = props.codexQuota.credits;
  if (!credits) return "";
  if (credits.unlimited) return t("settings.codex.quotaCreditsUnlimited");
  if (credits.balance) return t("settings.codex.quotaCredits", credits.balance);
  return "";
}
</script>

<template>
  <div class="settings-api-providers" :class="{ 'is-onboarding': isOnboardingMode }">
  <div class="settings-section" v-if="!isOnboardingMode && allModels.length > 0">
    <div class="section-label">{{ t("settings.models.available") }}</div>
    <div class="available-models-grid">
      <div
        v-for="group in groupedAllModels()"
        :key="group.provider"
        class="available-models-group"
      >
        <div class="available-models-provider">{{ group.label }}</div>
        <div class="available-models-list">
          <span
            v-for="m in group.models"
            :key="m.id"
            class="available-model-tag"
          >{{ m.name }}</span>
        </div>
      </div>
    </div>
  </div>
  <div class="settings-section" v-else-if="!isOnboardingMode">
    <div class="section-label">{{ t("settings.models.available") }}</div>
    <p class="section-desc" style="opacity:0.6;">{{ t("settings.models.noModels") }}</p>
  </div>

  <div class="settings-section" v-if="!isOnboardingMode && anthropicProvider">
    <div class="section-label">{{ t("settings.anthropic.title") }}</div>
    <div class="provider-card">
      <div class="provider-header">
        <div class="provider-info">
          <span class="provider-name">Anthropic (OAuth)</span>
          <span class="provider-desc">{{ providerMeta('anthropic').desc }}</span>
        </div>
        <span class="provider-status">
          {{ t("settings.anthropic.disabledStatus") }}
        </span>
      </div>

      <div class="provider-detail">
        <span class="key-hint">
          {{
            anthropicProvider?.hasKey
              ? (anthropicProvider.keyHint || t("settings.anthropic.loggedIn"))
              : t("settings.anthropic.disabledHint")
          }}
        </span>
        <div class="provider-actions">
          <button class="action-btn" @click="emit('startOAuthLogin')" :disabled="isLoading">
            {{ t("settings.anthropic.detailsBtn") }}
          </button>
          <button v-if="anthropicProvider?.hasKey" class="action-btn danger" @click="emit('oauthLogout')" :disabled="isLoading">
            {{ t("settings.anthropic.logout") }}
          </button>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section" v-if="!isOnboardingMode && providers.find(p => p.id === 'anthropic_sdk')">
    <div class="section-label">{{ t("settings.anthropicSdk.title") }}</div>
    <div class="provider-card">
      <div class="provider-header">
        <div class="provider-info">
          <span class="provider-name">{{ providers.find(p => p.id === 'anthropic_sdk')?.name || 'Anthropic Agent SDK' }}</span>
          <span class="provider-desc">{{ t("settings.provider.anthropic_sdk.desc") }}</span>
        </div>
        <span
          class="provider-status"
          :class="{ active: providers.find(p => p.id === 'anthropic_sdk')?.hasKey }"
        >
          {{ providers.find(p => p.id === 'anthropic_sdk')?.hasKey ? t("settings.anthropicSdk.installed") : t("settings.anthropicSdk.notInstalled") }}
        </span>
      </div>

      <div class="provider-detail">
        <span
          class="key-hint"
          :class="{ mono: providers.find(p => p.id === 'anthropic_sdk')?.hasKey }"
        >
          {{ providers.find(p => p.id === 'anthropic_sdk')?.hasKey
            ? (providers.find(p => p.id === 'anthropic_sdk')?.keyHint || t("settings.anthropicSdk.installed"))
            : t("settings.anthropicSdk.installHint") }}
        </span>
      </div>

      <div class="provider-detail" style="padding-top: 0;">
        <span class="oauth-hint">{{ t("settings.anthropicSdk.hint") }}</span>
      </div>
    </div>
  </div>

  <div class="settings-section" :class="focusSectionClass('codex')">
    <div class="section-label">{{ t("settings.codex.title") }}</div>
    <div class="provider-card">
      <div class="provider-header">
        <div class="provider-info">
          <span class="provider-name">OpenAI Codex</span>
          <span class="provider-desc">{{ t("settings.codex.desc") }}</span>
        </div>
        <span
          class="provider-status"
          :class="{
            active: codexStatus.authenticated && !codexStatus.validationFailed,
            error: codexStatus.validationFailed,
          }"
        >
          {{
            codexStatus.validationFailed
              ? t("settings.codex.validationFailed")
              : codexStatus.authenticated
                ? t("settings.codex.loggedIn")
                : t("settings.codex.notLoggedIn")
          }}
        </span>
      </div>

      <div v-if="codexStatus.authenticated" class="provider-detail codex-detail">
        <div class="codex-status-copy">
          <span class="key-hint">{{ codexStatus.accountId ?? t("settings.codex.authenticated") }}</span>
          <span v-if="codexStatus.validationFailed" class="codex-validation-label">
            {{ t("settings.codex.validationFailedHint") }}
          </span>
          <span v-if="codexStatus.validationError" class="oauth-hint codex-validation-error">
            {{ codexStatus.validationError }}
          </span>
        </div>
        <div class="provider-actions">
          <button
            v-if="codexStatus.validationFailed"
            class="action-btn"
            :disabled="codexRetrying"
            @click="emit('retryCodexValidation')"
          >
            {{ codexRetrying ? t("settings.codex.retrying") : t("settings.codex.retryValidation") }}
          </button>
          <button class="action-btn danger" @click="emit('codexLogout')">{{ t("settings.codex.logout") }}</button>
        </div>
      </div>

      <div
        v-if="codexStatus.authenticated && !codexStatus.validationFailed"
        class="provider-detail codex-quota-detail"
      >
        <div class="codex-quota-copy">
          <span class="key-hint codex-quota-label">{{ t("settings.codex.quotaLabel") }}</span>
          <div v-if="codexQuota.windows.length > 0" class="codex-quota-list">
            <div
              v-for="window in codexQuota.windows"
              :key="window.id"
              class="codex-quota-row"
            >
              <span class="codex-quota-name">{{ formatQuotaWindowLabel(window) }}</span>
              <span class="codex-quota-track" aria-hidden="true">
                <span class="codex-quota-fill" :style="quotaBarStyle(window)"></span>
              </span>
              <span class="codex-quota-percent">
                {{ t("settings.codex.quotaPercent", formatQuotaPercent(window.remainingPercent)) }}
              </span>
              <span v-if="formatQuotaReset(window.resetsAt)" class="codex-quota-reset">
                {{ formatQuotaReset(window.resetsAt) }}
              </span>
            </div>
            <span v-if="quotaCreditsLabel()" class="oauth-hint">{{ quotaCreditsLabel() }}</span>
            <span v-if="codexQuota.error" class="oauth-hint codex-validation-error">
              {{ codexQuota.error }}
            </span>
          </div>
          <span v-else-if="codexQuota.loading" class="oauth-hint">{{ t("settings.codex.quotaLoading") }}</span>
          <span v-else-if="codexQuota.error" class="oauth-hint codex-validation-error">
            {{ codexQuota.error }}
          </span>
          <span v-else class="oauth-hint">{{ t("settings.codex.quotaUnavailable") }}</span>
        </div>
        <div class="provider-actions">
          <button
            class="action-btn"
            type="button"
            :disabled="codexQuota.loading"
            @click="emit('refreshCodexQuota')"
          >
            {{ codexQuota.loading ? t("settings.codex.quotaRefreshing") : t("settings.codex.refreshQuota") }}
          </button>
        </div>
      </div>

      <div v-if="!codexStatus.authenticated && codexStep === 'idle'" class="provider-detail">
        <button class="oauth-login-btn" @click="emit('startCodexLogin')" :disabled="isLoading">
          {{ t("settings.codex.loginBtn") }}
        </button>
        <span class="oauth-hint">{{ t("settings.codex.hint") }}</span>
      </div>

      <div v-else-if="!codexStatus.authenticated && codexStep === 'opening'" class="provider-detail">
        <button class="oauth-login-btn" type="button" disabled>
          {{ t("settings.codex.opening") }}
        </button>
        <span class="oauth-hint">{{ t("settings.codex.hint") }}</span>
      </div>

      <div v-else-if="!codexStatus.authenticated && codexStep === 'waiting'" class="edit-form">
        <div class="oauth-instruction">{{ t("settings.codex.instruction") }}</div>
        <div class="codex-code-row">
          <a :href="codexUrl" target="_blank" class="codex-url">{{ codexUrl }}</a>
          <button
            class="codex-code-wrap"
            :class="{ copied: codexCodeCopied }"
            type="button"
            :title="codexCodeCopied ? t('common.copied') : t('common.clickToCopy')"
            @click="emit('copyCode')"
          >
            <span class="codex-code">{{ codexUserCode }}</span>
            <span class="codex-copy-indicator">
              {{ codexCodeCopied ? t("common.copied") : t("common.clickToCopy") }}
            </span>
          </button>
        </div>
        <div class="codex-poll-row">
          <span class="codex-spinner"></span>
          <span class="oauth-hint">{{ t("settings.codex.waiting") }}</span>
          <button class="cancel-btn" style="margin-left:auto" @click="emit('cancelCodexLogin')">{{ t("settings.codex.cancel") }}</button>
        </div>
      </div>

      <div v-if="codexStep !== 'waiting'" class="provider-detail codex-transport-detail">
        <div class="codex-transport-copy">
          <span class="key-hint codex-transport-label">{{ t("settings.codex.transportLabel") }}</span>
          <span class="oauth-hint">{{ t("settings.codex.transportDesc") }}</span>
        </div>
        <BaseSegmented
          size="sm"
          :model-value="codexTransport"
          :options="codexTransportOptions"
          @update:model-value="updateCodexTransport"
        />
      </div>
    </div>
  </div>

  <div v-if="!isOnboardingMode && thirdPartyProviders.length > 0" class="settings-section">
    <div class="section-label">{{ t("settings.provider.title") }}</div>

    <div
      v-for="provider in thirdPartyProviders"
      :key="provider.id"
      class="provider-card"
    >
      <div class="provider-header">
        <div class="provider-info">
          <span class="provider-name">{{ provider.name }}</span>
          <span class="provider-desc">{{ providerMeta(provider.id).desc }}</span>
        </div>
        <span
          class="provider-status"
          :class="{ active: provider.hasKey }"
        >
          {{ provider.hasKey ? t("settings.provider.configured") : t("settings.provider.notConfigured") }}
        </span>
      </div>

      <template>
        <div v-if="provider.hasKey && editingProvider !== provider.id" class="provider-detail">
          <span class="key-hint mono">{{ provider.keyHint }}</span>
          <div class="provider-actions">
            <button class="action-btn" @click="emit('startEdit', provider.id)">{{ t("settings.provider.edit") }}</button>
            <button class="action-btn danger" @click="emit('deleteKey', provider.id)">{{ t("settings.provider.delete") }}</button>
          </div>
        </div>

        <div v-if="!provider.hasKey && editingProvider !== provider.id" class="provider-detail">
          <button class="add-key-btn" @click="emit('startEdit', provider.id)">
            {{ t("settings.provider.addKey") }}
          </button>
          <a
            v-if="providerMeta(provider.id).url"
            :href="providerMeta(provider.id).url"
            target="_blank"
            class="get-key-link"
          >{{ t("settings.provider.getKey") }}</a>
        </div>

        <div v-if="editingProvider === provider.id" class="edit-form">
          <div class="edit-row">
            <input
              :value="editKey"
              @input="emit('update:editKey', ($event.target as HTMLInputElement).value)"
              class="key-input"
              type="password"
              :placeholder="providerMeta(provider.id).placeholder"
              autofocus
              @keydown="(e) => emit('handleKeydown', e, provider.id)"
            />
            <button
              class="save-btn"
              :disabled="isLoading || !editKey.trim()"
              @click="emit('saveKey', provider.id)"
            >
              {{ isLoading ? '...' : t("settings.provider.save") }}
            </button>
            <button class="cancel-btn" @click="emit('cancelEdit')">{{ t("settings.provider.cancel") }}</button>
          </div>
          <a
            v-if="providerMeta(provider.id).url"
            :href="providerMeta(provider.id).url"
            target="_blank"
            class="get-key-link"
          >{{ t("settings.provider.goGetKey", provider.name) }}</a>
        </div>
      </template>
    </div>
  </div>

  <div class="settings-section" :class="focusSectionClass('custom')">
    <div class="section-label">{{ t("settings.custom.title") }}</div>
    <p class="section-desc">{{ t("settings.custom.desc") }}</p>

    <div v-if="customEndpoints.length > 0" class="custom-endpoints-list">
      <div
        v-for="ep in customEndpoints"
        :key="ep.id"
        class="provider-card"
      >
        <div class="provider-header">
          <div class="provider-info">
            <span class="provider-name">{{ ep.name }}</span>
            <span class="provider-desc">{{ ep.apiModel }} · {{ formatLabel(ep.apiFormat) }}</span>
          </div>
          <span class="provider-status active">{{ ep.endpoint }}</span>
        </div>
        <div class="provider-detail">
          <span class="key-hint mono">{{ ep.apiKey ? ep.apiKey.slice(0, 8) + '...' : '(no key)' }}</span>
          <div class="provider-actions">
            <button
              class="action-btn"
              type="button"
              :disabled="customEndpointSaving"
              @click="emit('startEditEndpoint', ep)"
            >
              {{ t("settings.custom.edit") }}
            </button>
            <button
              class="action-btn danger"
              type="button"
              :disabled="customEndpointSaving"
              @click="emit('deleteEndpoint', ep.id)"
            >
              {{ t("settings.custom.delete") }}
            </button>
          </div>
        </div>
      </div>
    </div>
    <p v-else class="section-desc" style="opacity:0.5;">{{ t("settings.custom.noEndpoints") }}</p>

    <button
      class="add-key-btn"
      style="margin-top: 8px;"
      type="button"
      :disabled="customEndpointSaving"
      @click="emit('startAddEndpoint')"
    >
      + {{ t("settings.custom.add") }}
    </button>
  </div>
  </div>
</template>

<style scoped>
.settings-api-providers {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.settings-section {
  padding: 18px 28px;
}

.settings-api-providers.is-onboarding .settings-section {
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-color);
}

.settings-api-providers.is-onboarding .settings-section:last-child {
  border-bottom: none;
}

.settings-api-providers.is-onboarding .focus-section {
  order: -1;
}

.section-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 12px;
}

.section-desc {
  font-size: 12px;
  color: var(--text-secondary);
  margin: -4px 0 14px;
  line-height: 1.5;
}

.provider-card {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 14px 16px;
  margin-bottom: 10px;
  transition: border-color 0.15s ease, background 0.15s ease;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

.provider-card:hover {
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--hover-bg) 12%);
}

.provider-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.provider-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.provider-name {
  font-size: 14px;
  font-weight: 600;
}

.provider-desc {
  font-size: 12px;
  color: var(--text-secondary);
}

.provider-status {
  font-size: 11px;
  font-weight: 500;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  border: 1px solid transparent;
  flex-shrink: 0;
  white-space: nowrap;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.provider-status.active {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  border-color: var(--status-good-border);
}

.provider-status.error {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
}

.provider-detail {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
}

.key-hint {
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 0;
  word-break: break-word;
}

.key-hint.mono {
  font-family: var(--font-mono-identifier);
}

.provider-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.codex-detail {
  align-items: flex-start;
}

.codex-status-copy,
.codex-transport-copy,
.codex-quota-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.codex-transport-detail {
  align-items: center;
}

.codex-transport-label {
  color: var(--text-color);
}

.codex-quota-detail {
  align-items: flex-start;
}

.codex-quota-label {
  color: var(--text-color);
}

.codex-quota-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: min(420px, 100%);
}

.codex-quota-row {
  display: grid;
  grid-template-columns: minmax(72px, 128px) minmax(96px, 1fr) 40px minmax(86px, max-content);
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary);
}

.codex-quota-name,
.codex-quota-percent,
.codex-quota-reset {
  white-space: nowrap;
}

.codex-quota-name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.codex-quota-reset {
  text-align: right;
}

.codex-quota-track {
  position: relative;
  height: 4px;
  min-width: 72px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--hover-bg);
}

.codex-quota-fill {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
  background: var(--accent-color);
}

.codex-validation-label {
  font-size: 11px;
  color: var(--status-danger-fg);
  line-height: 1.4;
}

.codex-validation-error {
  line-height: 1.5;
  color: var(--text-secondary);
}

.action-btn,
.cancel-btn,
.test-btn {
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  box-shadow: none;
  white-space: nowrap;
}

.action-btn {
  padding: 4px 10px;
}

.action-btn:hover,
.cancel-btn:hover,
.test-btn:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

.action-btn.danger:hover {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
}

.add-key-btn {
  padding: 6px 12px;
  border-radius: 6px;
  border: 1px dashed var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  box-shadow: none;
}

.add-key-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

.get-key-link,
.codex-url {
  font-size: 11px;
  color: var(--text-secondary);
  text-decoration: underline;
  text-underline-offset: 2px;
  transition: color 0.15s;
}

.codex-url {
  color: var(--accent-color);
  word-break: break-all;
}

.get-key-link:hover {
  color: var(--text-color);
}

.edit-form {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.edit-row {
  display: flex;
  gap: 6px;
}

.key-input {
  flex: 1;
  min-width: 0;
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

.save-btn,
.oauth-login-btn {
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

.oauth-login-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
}

.save-btn:hover:not(:disabled),
.oauth-login-btn:hover:not(:disabled) {
  filter: brightness(1.06);
}

.save-btn:disabled,
.oauth-login-btn:disabled,
.test-btn:disabled,
.action-btn:disabled,
.add-key-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.oauth-hint {
  font-size: 11px;
  color: var(--text-secondary);
}

.oauth-instruction {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.codex-code-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.codex-code-wrap {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 8px 10px;
  border-radius: 6px;
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  color: inherit;
  cursor: pointer;
  text-align: left;
  box-shadow: none;
  transition: border-color 0.15s, background 0.15s;
}

.codex-code-wrap:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

.codex-code-wrap:focus-visible {
  outline: none;
  border-color: var(--accent-color);
}

.codex-code-wrap.copied {
  border-color: var(--status-good-border);
  background: var(--status-good-bg);
}

.codex-code {
  flex: 1;
  font-family: var(--font-mono-display);
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 3px;
  color: var(--accent-color);
}

.codex-copy-indicator {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  transition: color 0.15s;
}

.codex-code-wrap.copied .codex-copy-indicator {
  color: var(--status-good-fg);
}

.codex-poll-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.codex-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

.available-models-grid,
.custom-endpoints-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.available-models-grid {
  gap: 12px;
}

.available-models-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.available-models-provider {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
}

.available-models-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.available-model-tag {
  display: inline-block;
  padding: 3px 10px;
  font-size: 12px;
  font-weight: 500;
  border-radius: var(--radius-badge);
  background: color-mix(in srgb, var(--panel-bg) 60%, var(--hover-bg) 40%);
  color: var(--text-secondary);
  border: 1px solid var(--border-color);
  white-space: nowrap;
}

@media (max-width: 680px) {
  .settings-section {
    padding: 14px 18px;
  }

  .provider-header,
  .provider-detail,
  .edit-row {
    flex-direction: column;
    align-items: stretch;
  }

  .provider-status,
  .provider-actions,
  .save-btn,
  .cancel-btn {
    align-self: flex-start;
  }
}

@keyframes spin {
  to { transform: rotate(360deg); }
}
</style>
