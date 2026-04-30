<script setup lang="ts">
import BaseSegmented from "../ui/BaseSegmented.vue";
import { computed } from "vue";
import { t } from "../../i18n";
import type {
  ModelOption,
  CustomEndpoint,
  ApiFormat,
  CodexTransportMode,
} from "../../types";
import type { CodexStatusState, ProviderStatus } from "../../composables/useSettingsState";
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
  codexRetrying: boolean;
  codexTransport: CodexTransportMode;
  codexUserCode: string;
  codexUrl: string;
  codexCodeCopied: boolean;
  allModels: ModelOption[];
  customEndpoints: CustomEndpoint[];
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
  copyCode: [];
  "update:codexTransport": [value: CodexTransportMode];
  startAddEndpoint: [];
  startEditEndpoint: [ep: CustomEndpoint];
  deleteEndpoint: [id: string];
  "update:editKey": [value: string];
  "update:oauthCode": [value: string];
}>();

const anthropicProvider = computed(() => props.providers.find((p) => p.id === "anthropic"));
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
</script>

<template>
  <div class="settings-section" v-if="allModels.length > 0">
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
  <div class="settings-section" v-else>
    <div class="section-label">{{ t("settings.models.available") }}</div>
    <p class="section-desc" style="opacity:0.6;">{{ t("settings.models.noModels") }}</p>
  </div>

  <div class="settings-section" v-if="anthropicProvider">
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

  <div class="settings-section" v-if="providers.find(p => p.id === 'anthropic_sdk')">
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

  <div class="settings-section">
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

      <div v-else-if="codexStep === 'idle'" class="provider-detail">
        <button class="oauth-login-btn" @click="emit('startCodexLogin')" :disabled="isLoading">
          {{ t("settings.codex.loginBtn") }}
        </button>
        <span class="oauth-hint">{{ t("settings.codex.hint") }}</span>
      </div>

      <div v-else-if="codexStep === 'opening'" class="provider-detail">
        <button class="oauth-login-btn" type="button" disabled>
          {{ t("settings.codex.opening") }}
        </button>
        <span class="oauth-hint">{{ t("settings.codex.hint") }}</span>
      </div>

      <div v-else-if="codexStep === 'waiting'" class="edit-form">
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

  <div v-if="thirdPartyProviders.length > 0" class="settings-section">
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

  <div class="settings-section">
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
            <button class="action-btn" @click="emit('startEditEndpoint', ep)">{{ t("settings.custom.edit") }}</button>
            <button class="action-btn danger" @click="emit('deleteEndpoint', ep.id)">{{ t("settings.custom.delete") }}</button>
          </div>
        </div>
      </div>
    </div>
    <p v-else class="section-desc" style="opacity:0.5;">{{ t("settings.custom.noEndpoints") }}</p>

    <button
      class="add-key-btn"
      style="margin-top: 8px;"
      @click="emit('startAddEndpoint')"
    >
      + {{ t("settings.custom.add") }}
    </button>
  </div>
</template>
