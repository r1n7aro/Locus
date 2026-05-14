
<script setup lang="ts">
import { watch } from "vue";
import type {
  ModelOption,
  ModelDefaults,
  AgentInfo,
  CustomEndpoint,
  CodexModelConfig,
} from "../types";
import { t, locale, setLocale } from "../i18n";
import { useSettingsState } from "../composables/useSettingsState";
import GeneralSettings from "./settings/GeneralSettings.vue";
import DisplaySettings from "./settings/DisplaySettings.vue";
import ShortcutSettings from "./settings/ShortcutSettings.vue";
import ConsoleSettings from "./settings/ConsoleSettings.vue";
import AboutSettings from "./settings/AboutSettings.vue";
import ApiProviders from "./settings/ApiProviders.vue";
import CustomEndpointModal from "./settings/CustomEndpointModal.vue";
import ModelDefaultsPanel from "./settings/ModelDefaults.vue";
import ToolPermissions from "./settings/ToolPermissions.vue";
import ArchivedSessionsSettings from "./settings/ArchivedSessionsSettings.vue";
import SubscriptionDisclaimerModal from "./SubscriptionDisclaimerModal.vue";
import { useUiStore } from "../stores/ui";
import { useChatStore } from "../stores/chat";

defineProps<{
  allModels: ModelOption[];
  agents: AgentInfo[];
  subagents: AgentInfo[];
}>();

const emit = defineEmits<{
  close: [];
  authChanged: [];
  modelDefaultsChanged: [defaults: ModelDefaults];
  codexTransportChanged: [config: CodexModelConfig];
  customEndpointsChanged: [endpoints: CustomEndpoint[]];
  resetOnboarding: [];
}>();

const {
  resetConfirm, handleResetOnboarding, activeCategory,
  providers, editingProvider, editKey, errorMsg, successMsg, isLoading,
  startEdit, cancelEdit, saveKey, deleteKey, handleKeydown,
  oauthStep, oauthCode, submitOAuthCode, cancelOAuth, oauthLogout, handleOAuthKeydown,
  codexStep, codexStatus, codexQuota, codexRetrying, codexModelConfig, codexUserCode, codexUrl, codexCodeCopied, cancelCodexLogin, codexLogout, retryCodexValidation, copyCode, setCodexTransportMode, loadCodexRateLimits,
  showDisclaimer, requestOAuthLogin, requestCodexLogin, cancelDisclaimer,
  modelDefaults, modelSaveMsg, saveModelDefaults,
  permSaveMsg, toolList, approvalBehaviorList, toolPermissions, setToolPermission,
  customEndpoints, editingEndpoint, isAddingEndpoint, customEndpointSaving, testStatus, testResult,
  startAddEndpoint, startEditEndpoint, cancelEditEndpoint, saveEndpoint, deleteEndpoint, testEndpoint,
} = useSettingsState(emit);

const uiStore = useUiStore();
const chatStore = useChatStore();

watch(
  () => uiStore.settingsCategoryHint,
  (category) => {
    if (!category) return;
    activeCategory.value = category;
    uiStore.clearSettingsCategoryHint();
  },
  { immediate: true },
);

</script>

<template>
  <div class="settings-panel">
    <div class="settings-sidebar">
      <div class="sidebar-header">
        <span class="sidebar-title">{{ t("settings.title") }}</span>
        <button class="close-btn" @click="emit('close')" :title="t('settings.close')">
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
          </svg>
        </button>
      </div>
      <div class="sidebar-nav">
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'general' }"
          @click="activeCategory = 'general'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zm0 1.5a6.5 6.5 0 0 1 4.936 10.752l-.221-.164c-.976-.725-2.622-1.338-4.715-1.338s-3.739.613-4.715 1.338l-.221.164A6.5 6.5 0 0 1 8 1.5zM4 6a4 4 0 1 1 8 0 4 4 0 0 1-8 0z"/>
          </svg>
          <span>{{ t("settings.tab.general") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'display' }"
          @click="activeCategory = 'display'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h9A1.5 1.5 0 0 1 14 3.5v6a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 9.5v-6zM3.5 3a.5.5 0 0 0-.5.5v6a.5.5 0 0 0 .5.5h9a.5.5 0 0 0 .5-.5v-6a.5.5 0 0 0-.5-.5h-9zM5 13a.75.75 0 0 0 0 1.5h6a.75.75 0 0 0 0-1.5H5z"/>
          </svg>
          <span>{{ t("settings.tab.display") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'shortcuts' }"
          @click="activeCategory = 'shortcuts'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M2 4.25A2.25 2.25 0 0 1 4.25 2h7.5A2.25 2.25 0 0 1 14 4.25v7.5A2.25 2.25 0 0 1 11.75 14h-7.5A2.25 2.25 0 0 1 2 11.75v-7.5zm2.25-1.25A1.25 1.25 0 0 0 3 4.25V6h10V4.25A1.25 1.25 0 0 0 11.75 3h-7.5zM13 7H3v4.75A1.25 1.25 0 0 0 4.25 13h7.5A1.25 1.25 0 0 0 13 11.75V7zM4.25 8.25h1.5a.75.75 0 0 1 0 1.5h-1.5a.75.75 0 0 1 0-1.5zm0 2.5h2.5a.75.75 0 0 1 0 1.5h-2.5a.75.75 0 0 1 0-1.5zm4-2.5h1.5a.75.75 0 0 1 0 1.5h-1.5a.75.75 0 0 1 0-1.5zm0 2.5h3.5a.75.75 0 0 1 0 1.5h-3.5a.75.75 0 0 1 0-1.5z"/>
          </svg>
          <span>{{ t("settings.tab.shortcuts") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'api' }"
          @click="activeCategory = 'api'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M8 0a4 4 0 0 0-4 4v1H3a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-1V4a4 4 0 0 0-4-4zm2.5 5V4a2.5 2.5 0 1 0-5 0v1h5z"/>
          </svg>
          <span>{{ t("settings.tab.api") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'models' }"
          @click="activeCategory = 'models'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M1 2.5A1.5 1.5 0 0 1 2.5 1h3A1.5 1.5 0 0 1 7 2.5v3A1.5 1.5 0 0 1 5.5 7h-3A1.5 1.5 0 0 1 1 5.5v-3zm8 0A1.5 1.5 0 0 1 10.5 1h3A1.5 1.5 0 0 1 15 2.5v3A1.5 1.5 0 0 1 13.5 7h-3A1.5 1.5 0 0 1 9 5.5v-3zm-8 8A1.5 1.5 0 0 1 2.5 9h3A1.5 1.5 0 0 1 7 10.5v3A1.5 1.5 0 0 1 5.5 15h-3A1.5 1.5 0 0 1 1 13.5v-3zm8 0A1.5 1.5 0 0 1 10.5 9h3a1.5 1.5 0 0 1 1.5 1.5v3a1.5 1.5 0 0 1-1.5 1.5h-3A1.5 1.5 0 0 1 9 13.5v-3z"/>
          </svg>
          <span>{{ t("settings.tab.models") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'permissions' }"
          @click="activeCategory = 'permissions'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M8 1a3.5 3.5 0 0 0-3.5 3.5v1H3.25A1.25 1.25 0 0 0 2 6.75v7A1.25 1.25 0 0 0 3.25 15h9.5A1.25 1.25 0 0 0 14 13.75v-7A1.25 1.25 0 0 0 12.75 5.5H11.5v-1A3.5 3.5 0 0 0 8 1zm-2 4.5v-1a2 2 0 1 1 4 0v1H6z"/>
          </svg>
          <span>{{ t("settings.tab.permissions") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'archived' }"
          @click="activeCategory = 'archived'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M2.5 2h11A1.5 1.5 0 0 1 15 3.5v2A1.5 1.5 0 0 1 13.5 7H13v5.5A1.5 1.5 0 0 1 11.5 14h-7A1.5 1.5 0 0 1 3 12.5V7h-.5A1.5 1.5 0 0 1 1 5.5v-2A1.5 1.5 0 0 1 2.5 2zm0 1a.5.5 0 0 0-.5.5v2a.5.5 0 0 0 .5.5h11a.5.5 0 0 0 .5-.5v-2a.5.5 0 0 0-.5-.5h-11zM4 7v5.5a.5.5 0 0 0 .5.5h7a.5.5 0 0 0 .5-.5V7H4zm2 2h4v1H6V9z"/>
          </svg>
          <span>{{ t("settings.tab.archived") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'console' }"
          @click="activeCategory = 'console'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M2.75 2A1.75 1.75 0 0 0 1 3.75v8.5C1 13.216 1.784 14 2.75 14h10.5A1.75 1.75 0 0 0 15 12.25v-8.5A1.75 1.75 0 0 0 13.25 2H2.75zm0 1h10.5c.414 0 .75.336.75.75v1.643H2V3.75c0-.414.336-.75.75-.75zM2 6.393h12v5.857a.75.75 0 0 1-.75.75H2.75a.75.75 0 0 1-.75-.75V6.393zm2.22 1.327a.75.75 0 0 0-1.06 1.06L4.379 10 3.16 11.22a.75.75 0 0 0 1.06 1.06L6.5 10 4.22 7.72zM7.75 11a.75.75 0 0 0 0 1.5h4a.75.75 0 0 0 0-1.5h-4z"/>
          </svg>
          <span>{{ t("settings.tab.console") }}</span>
        </button>
        <button
          class="sidebar-item"
          :class="{ active: activeCategory === 'about' }"
          @click="activeCategory = 'about'"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
            <path d="M8 1.25a6.75 6.75 0 1 0 0 13.5 6.75 6.75 0 0 0 0-13.5zM8 2.5a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11zm0 2a.875.875 0 1 0 0 1.75A.875.875 0 0 0 8 4.5zm-.75 3.25a.75.75 0 0 0 0 1.5h.25v2a.75.75 0 0 0 1.5 0v-2a.75.75 0 0 0-.75-.75h-1z"/>
          </svg>
          <span>{{ t("settings.tab.about") }}</span>
        </button>
      </div>
    </div>

    <div class="settings-content">
      <Transition name="fade">
        <div v-if="successMsg || modelSaveMsg" class="success-msg">{{ successMsg || modelSaveMsg }}</div>
      </Transition>

      <template v-if="activeCategory === 'api'">
        <ApiProviders
          :providers="providers"
          :editing-provider="editingProvider"
          :edit-key="editKey"
          :error-msg="errorMsg"
          :success-msg="successMsg"
          :is-loading="isLoading"
          :oauth-step="oauthStep"
          :oauth-code="oauthCode"
          :codex-step="codexStep"
          :codex-status="codexStatus"
          :codex-quota="codexQuota"
          :codex-retrying="codexRetrying"
          :codex-transport="codexModelConfig.transport"
          :codex-user-code="codexUserCode"
          :codex-url="codexUrl"
          :codex-code-copied="codexCodeCopied"
          :all-models="allModels"
          :custom-endpoints="customEndpoints"
          :custom-endpoint-saving="customEndpointSaving"
          @start-edit="startEdit"
          @cancel-edit="cancelEdit"
          @save-key="saveKey"
          @delete-key="deleteKey"
          @handle-keydown="handleKeydown"
          @start-o-auth-login="requestOAuthLogin"
          @submit-o-auth-code="submitOAuthCode"
          @cancel-o-auth="cancelOAuth"
          @oauth-logout="oauthLogout"
          @handle-o-auth-keydown="handleOAuthKeydown"
          @start-codex-login="requestCodexLogin"
          @cancel-codex-login="cancelCodexLogin"
          @codex-logout="codexLogout"
          @retry-codex-validation="retryCodexValidation"
          @refresh-codex-quota="loadCodexRateLimits"
          @copy-code="copyCode"
          @update:codex-transport="setCodexTransportMode"
          @start-add-endpoint="startAddEndpoint"
          @start-edit-endpoint="startEditEndpoint"
          @delete-endpoint="deleteEndpoint"
          @update:edit-key="editKey = $event"
          @update:oauth-code="oauthCode = $event"
        />
      </template>

      <template v-if="activeCategory === 'models'">
        <ModelDefaultsPanel
          :model-defaults="modelDefaults"
          :all-models="allModels"
          :agents="agents"
          :subagents="subagents"
          :model-save-msg="modelSaveMsg"
          @update:model-defaults="modelDefaults = $event"
          @save="saveModelDefaults"
        />
      </template>

      <template v-if="activeCategory === 'permissions'">
        <ToolPermissions
          :tool-permission-mode="chatStore.toolPermissionMode"
          :tool-list="toolList"
          :behavior-list="approvalBehaviorList"
          :tool-permissions="toolPermissions"
          :perm-save-msg="permSaveMsg"
          @set-global-permission-mode="chatStore.setToolPermissionMode"
          @set-permission="setToolPermission"
        />
      </template>

      <template v-if="activeCategory === 'display'">
        <DisplaySettings />
      </template>

      <template v-if="activeCategory === 'shortcuts'">
        <ShortcutSettings />
      </template>

      <template v-if="activeCategory === 'archived'">
        <ArchivedSessionsSettings />
      </template>

      <template v-if="activeCategory === 'console'">
        <ConsoleSettings />
      </template>

      <template v-if="activeCategory === 'about'">
        <AboutSettings />
      </template>

      <template v-if="activeCategory === 'general'">
        <GeneralSettings
          :locale="locale"
          :reset-confirm="resetConfirm"
          @set-locale="setLocale"
          @start-reset="resetConfirm = true"
          @confirm-reset="handleResetOnboarding"
          @cancel-reset="resetConfirm = false"
        />
      </template>

      <div v-if="errorMsg" class="error-msg">{{ errorMsg }}</div>
    </div><!-- end settings-content -->

    <CustomEndpointModal
      v-model:endpoint="editingEndpoint"
      :is-adding="isAddingEndpoint"
      :saving="customEndpointSaving"
      :test-status="testStatus"
      :test-result="testResult"
      @close="cancelEditEndpoint"
      @save="saveEndpoint"
      @test="testEndpoint"
    />

    <SubscriptionDisclaimerModal
      :open="showDisclaimer"
      @cancel="cancelDisclaimer"
    />
  </div>
</template>

<style scoped>
.settings-panel {
  flex: 1;
  display: flex;
  flex-direction: row;
  background: var(--bg-color);
  height: 100%;
  overflow: hidden;
}

:deep(.settings-sidebar) {
  width: 160px;
  min-width: 140px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  flex-shrink: 0;
  overflow: hidden;
}

:deep(.sidebar-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px 10px;
  flex-shrink: 0;
}

:deep(.sidebar-title) {
  font-size: 14px;
  font-weight: 650;
  letter-spacing: -0.2px;
}

:deep(.sidebar-nav) {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 8px 8px;
}

:deep(.sidebar-item) {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  min-height: 34px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 13px;
  line-height: 1.3;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
  box-shadow: none;
  text-align: left;
  white-space: nowrap;
}

:deep(.sidebar-item:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
}

:deep(.sidebar-item.active) {
  background: color-mix(in srgb, var(--panel-bg) 40%, var(--active-bg) 60%);
  color: var(--text-color);
}

:deep(.sidebar-item svg) {
  flex-shrink: 0;
  opacity: 0.6;
}

:deep(.sidebar-item.active svg) {
  opacity: 1;
}

:deep(.settings-content) {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  min-width: 0;
  background: var(--panel-bg);
}

:deep(.close-btn) {
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
  transition: all 0.15s;
  box-shadow: none;
  padding: 0;
}

:deep(.close-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
}

:deep(.settings-section) {
  padding: 18px 28px;
}

:deep(.section-label) {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 12px;
}

:deep(.provider-card) {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 14px 16px;
  margin-bottom: 10px;
  transition: border-color 0.15s ease, background 0.15s ease;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

:deep(.provider-card:hover) {
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--hover-bg) 12%);
}

:deep(.provider-header) {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

:deep(.provider-info) {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

:deep(.provider-name) {
  font-size: 14px;
  font-weight: 600;
}

:deep(.provider-desc) {
  font-size: 12px;
  color: var(--text-secondary);
}

:deep(.provider-status) {
  font-size: 11px;
  font-weight: 500;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  border: 1px solid transparent;
  flex-shrink: 0;
  white-space: nowrap;
}

:deep(.provider-status.active) {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  border-color: var(--status-good-border);
}

:deep(.provider-status.error) {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
}

:deep(.provider-detail) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
}

:deep(.key-hint) {
  font-size: 12px;
  color: var(--text-secondary);
}

:deep(.key-hint.mono) {
  font-family: var(--font-mono-identifier);
}

:deep(.provider-actions) {
  display: flex;
  gap: 6px;
}

:deep(.codex-detail) {
  align-items: flex-start;
}

:deep(.codex-status-copy) {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

:deep(.codex-transport-detail) {
  align-items: center;
}

:deep(.codex-transport-copy) {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

:deep(.codex-transport-label) {
  color: var(--text-color);
}

:deep(.codex-validation-label) {
  font-size: 11px;
  color: var(--status-danger-fg);
  line-height: 1.4;
}

:deep(.codex-validation-error) {
  line-height: 1.5;
  color: var(--text-secondary);
}

:deep(.action-btn) {
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  box-shadow: none;
}

:deep(.action-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

:deep(.action-btn.danger:hover) {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
}

:deep(.add-key-btn) {
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

:deep(.add-key-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

:deep(.get-key-link) {
  font-size: 11px;
  color: var(--text-secondary);
  text-decoration: underline;
  text-underline-offset: 2px;
  transition: color 0.15s;
}

:deep(.get-key-link:hover) {
  color: var(--text-color);
}

:deep(.edit-form) {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.edit-row) {
  display: flex;
  gap: 6px;
}

:deep(.key-input) {
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

:deep(.key-input:focus) {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

:deep(.save-btn) {
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

:deep(.save-btn:hover:not(:disabled)) {
  filter: brightness(1.06);
}

:deep(.save-btn:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}

:deep(.cancel-btn) {
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

:deep(.cancel-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

:deep(.oauth-login-btn) {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
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

:deep(.oauth-login-btn:hover:not(:disabled)) {
  filter: brightness(1.06);
}

:deep(.oauth-login-btn:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}

:deep(.oauth-hint) {
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.oauth-instruction) {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
}

:deep(.error-msg) {
  margin: 0 28px 16px;
  padding: 10px 14px;
  border-radius: 8px;
  border: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 13px;
  line-height: 1.5;
}

:deep(.success-msg) {
  margin: 8px 28px 0;
  padding: 8px 14px;
  border-radius: 8px;
  border: 1px solid var(--status-good-border);
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  font-size: 13px;
  font-weight: 500;
}

:deep(.fade-enter-active),
:deep(.fade-leave-active) {
  transition: opacity 0.2s;
}

:deep(.fade-enter-from),
:deep(.fade-leave-to) {
  opacity: 0;
}

:deep(.codex-code-row) {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

:deep(.codex-url) {
  font-size: 11px;
  color: var(--accent-color);
  text-decoration: underline;
  word-break: break-all;
}

:deep(.codex-code-wrap) {
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

:deep(.codex-code-wrap:hover) {
  background: var(--hover-bg);
  border-color: var(--border-strong, var(--accent-color));
}

:deep(.codex-code-wrap:focus-visible) {
  outline: none;
  border-color: var(--accent-color);
}

:deep(.codex-code-wrap.copied) {
  border-color: var(--status-good-border);
  background: var(--status-good-bg);
}

:deep(.codex-code) {
  flex: 1;
  font-family: var(--font-mono-display);
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 3px;
  color: var(--accent-color);
}

:deep(.codex-copy-indicator) {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  transition: color 0.15s;
}

:deep(.codex-code-wrap.copied .codex-copy-indicator) {
  color: var(--status-good-fg);
}

:deep(.codex-poll-row) {
  display: flex;
  align-items: center;
  gap: 6px;
}

:deep(.codex-spinner) {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

@keyframes spin { to { transform: rotate(360deg); } }

:deep(.section-desc) {
  font-size: 12px;
  color: var(--text-secondary);
  margin: -4px 0 14px;
  line-height: 1.5;
}

:deep(.model-default-card) {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 14px 16px;
  margin-bottom: 10px;
  transition: border-color 0.15s ease, background 0.15s ease;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

:deep(.model-default-card:hover) {
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--hover-bg) 12%);
}

:deep(.model-default-header) {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 10px;
}

:deep(.model-default-label) {
  font-size: 14px;
  font-weight: 600;
}

:deep(.model-default-hint) {
  font-size: 12px;
  color: var(--text-secondary);
}

:deep(.model-select) {
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

:deep(.model-select:focus) {
  border-color: var(--accent-border);
  background-color: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

:deep(.model-select optgroup) {
  font-weight: 600;
  font-style: normal;
}

:deep(.model-select option) {
  font-weight: 400;
}

:deep(.model-select.inline) {
  width: 180px;
  flex-shrink: 0;
}

:deep(.model-default-card.compact) {
  padding: 10px 14px;
  margin-bottom: 6px;
}

:deep(.model-default-row) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

:deep(.model-default-agent) {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

:deep(.model-default-agent .model-default-label) {
  font-size: 13px;
}

:deep(.model-default-agent .model-default-hint) {
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

:deep(.tool-perm-list) {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 8px;
}

:deep(.tool-perm-row) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  border-radius: 6px;
  transition: background 0.12s;
}

:deep(.tool-perm-row:hover) {
  background: var(--hover-bg, rgba(128, 128, 128, 0.08));
}

:deep(.tool-perm-info) {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

:deep(.tool-perm-name) {
  font-size: 12.5px;
  font-weight: 600;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
}

:deep(.tool-perm-desc) {
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.tool-perm-toggle) {
  flex-shrink: 0;
  padding: 3px 12px;
  border-radius: 4px;
  font-size: 11px;
  font-weight: 600;
  border: 1px solid var(--border-color);
  cursor: pointer;
  transition: all 0.15s;
  min-width: 50px;
  text-align: center;
}

:deep(.tool-perm-toggle.auto) {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  border-color: var(--status-good-border);
}

:deep(.tool-perm-toggle.ask) {
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
  border-color: var(--status-warn-border);
}

:deep(.tool-perm-toggle:hover) {
  filter: brightness(1.03);
}

:deep(.lang-switcher) {
  display: flex;
  gap: 6px;
}

:deep(.lang-btn) {
  padding: 8px 18px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  box-shadow: none;
}

:deep(.lang-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-strong);
}

:deep(.lang-btn.active) {
  background: var(--accent-soft);
  color: var(--accent-color);
  border-color: var(--accent-border);
}

:deep(.reset-onboarding-btn) {
  padding: 7px 16px;
  border-radius: 6px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  border: 1px solid var(--status-danger-border);
  background: transparent;
  color: var(--status-danger-fg);
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}
:deep(.reset-onboarding-btn:hover) {
  background: var(--status-danger-bg);
  border-color: var(--status-danger-fg);
}
:deep(.reset-confirm-row) {
  display: flex;
  align-items: center;
  gap: 10px;
}
:deep(.reset-confirm-text) {
  font-size: 13px;
  color: var(--status-danger-fg);
}

:deep(.available-models-grid) {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

:deep(.available-models-group) {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

:deep(.available-models-provider) {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
}

:deep(.available-models-list) {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

:deep(.available-model-tag) {
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

:deep(.custom-endpoints-list) {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 8px;
}

:deep(.custom-form-row) {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

:deep(.custom-form-label) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  display: flex;
  align-items: baseline;
  gap: 6px;
}

:deep(.custom-form-hint) {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-secondary);
}

:deep(.beta-flags-list) {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

:deep(.beta-flag-item) {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  cursor: pointer;
}

:deep(.beta-flag-item input[type="checkbox"]) {
  margin: 0;
  cursor: pointer;
}

:deep(.beta-flag-name) {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  color: var(--text-color);
}

:deep(.beta-flag-desc) {
  font-size: 11px;
  color: var(--text-secondary);
  margin-left: 2px;
}

:deep(.modal-overlay) {
  position: absolute;
  inset: 0;
  background: rgba(8, 10, 14, 0.28);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

:deep(.modal-dialog) {
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

:deep(.modal-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

:deep(.modal-title) {
  font-size: 14px;
  font-weight: 700;
}

:deep(.modal-body) {
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow-y: auto;
}

:deep(.modal-footer) {
  display: flex;
  gap: 8px;
  padding: 12px 20px 16px;
  border-top: 1px solid var(--border-color);
  flex-shrink: 0;
}

:deep(.modal-enter-active),
:deep(.modal-leave-active) {
  transition: opacity 0.15s ease;
}

:deep(.modal-enter-active .modal-dialog),
:deep(.modal-leave-active .modal-dialog) {
  transition: transform 0.15s ease;
}

:deep(.modal-enter-from),
:deep(.modal-leave-to) {
  opacity: 0;
}

:deep(.modal-enter-from .modal-dialog) {
  transform: scale(0.95) translateY(8px);
}

:deep(.modal-leave-to .modal-dialog) {
  transform: scale(0.95) translateY(8px);
}

:deep(.test-btn) {
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

:deep(.test-btn:hover:not(:disabled)) {
  background: var(--hover-bg);
  border-color: var(--accent-border);
  color: var(--accent-color);
}

:deep(.test-btn:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}

:deep(.test-result) {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 12px;
  line-height: 1.5;
  flex-wrap: wrap;
}

:deep(.test-result.testing) {
  background: var(--hover-bg);
  color: var(--text-secondary);
}

:deep(.test-result.success) {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
}

:deep(.test-result.error) {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

:deep(.test-ok) {
  color: #22c55e;
  font-weight: 600;
}

:deep(.test-err) {
  color: #dc3232;
  font-weight: 600;
}

:deep(.test-detail) {
  color: var(--text-secondary);
  word-break: break-all;
  width: 100%;
}

</style>
