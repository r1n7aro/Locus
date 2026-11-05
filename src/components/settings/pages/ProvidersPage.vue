<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import type { ModelOption } from "../../../types";
import { useCodexAccountStore } from "../../../stores/codexAccount";
import { useCustomProviderSettingsStore } from "../../../stores/customProviderSettings";
import { useProviderAccountsStore } from "../../../stores/providerAccounts";
import { useSettingsFeedbackStore } from "../../../stores/settingsFeedback";
import ApiProviders from "../ApiProviders.vue";

defineProps<{
  allModels: ModelOption[];
}>();

const accounts = useProviderAccountsStore();
const codex = useCodexAccountStore();
const custom = useCustomProviderSettingsStore();
const feedback = useSettingsFeedbackStore();

const {
  providers,
  editingProvider,
  editKey,
  isLoading,
  oauthStep,
  oauthCode,
  anthropicQuota,
  claudeCodeTestStatus,
  claudeCodeTestResult,
} = storeToRefs(accounts);
const {
  step: codexStep,
  status: codexStatus,
  quota: codexQuota,
  refreshing: codexRefreshing,
  resetCreditBusyId: codexResetCreditBusyId,
  retrying: codexRetrying,
  importing: codexImporting,
  clientVersionBusy: codexClientVersionBusy,
  modelConfig: codexModelConfig,
  userCode: codexUserCode,
  url: codexUrl,
  codeCopied: codexCodeCopied,
} = storeToRefs(codex);
// The edit dialog is rendered by SettingsView so it covers the whole panel.
const { customProviders, customProviderSaving } = storeToRefs(custom);

onMounted(() => {
  void accounts.ensureLoaded();
  void codex.ensureLoaded();
  void custom.ensureLoaded();
});

function onClaudeConnectionChanged() {
  void accounts.loadProviders();
  feedback.authChanged();
}
</script>

<template>
  <ApiProviders
    :providers="providers"
    :editing-provider="editingProvider"
    :edit-key="editKey"
    :is-loading="isLoading || codexImporting"
    :oauth-step="oauthStep"
    :oauth-code="oauthCode"
    :anthropic-quota="anthropicQuota"
    :codex-step="codexStep"
    :codex-status="codexStatus"
    :codex-quota="codexQuota"
    :codex-refreshing="codexRefreshing || codexClientVersionBusy"
    :codex-reset-credit-busy-id="codexResetCreditBusyId"
    :codex-retrying="codexRetrying"
    :codex-transport="codexModelConfig.transport"
    :codex-auto-update-client-version="codexModelConfig.autoUpdateClientVersion"
    :codex-client-version-busy="codexClientVersionBusy"
    :codex-context-window="codexModelConfig.contextWindow"
    :codex-session-title-generation="codexModelConfig.generateSessionTitles"
    :codex-auto-review="codexModelConfig.autoReview"
    :codex-use-apply-patch="codexModelConfig.useApplyPatch"
    :codex-prefix-cache-ttl-seconds="codexModelConfig.prefixCacheTtlSeconds"
    :codex-user-code="codexUserCode"
    :codex-url="codexUrl"
    :codex-code-copied="codexCodeCopied"
    :all-models="allModels"
    :custom-providers="customProviders"
    :custom-provider-saving="customProviderSaving"
    :claude-code-test-status="claudeCodeTestStatus"
    :claude-code-test-result="claudeCodeTestResult"
    @test-claude-code="accounts.testClaudeCode"
    @claude-connection-changed="onClaudeConnectionChanged"
    @start-edit="accounts.startEdit"
    @cancel-edit="accounts.cancelEdit"
    @save-key="accounts.saveKey"
    @delete-key="accounts.deleteKey"
    @handle-keydown="accounts.handleKeydown"
    @start-o-auth-login="accounts.startOAuthLogin"
    @submit-o-auth-code="accounts.submitOAuthCode"
    @cancel-o-auth="accounts.cancelOAuth"
    @oauth-logout="accounts.oauthLogout"
    @import-claude-code-o-auth="accounts.importClaudeCodeOAuth"
    @handle-o-auth-keydown="accounts.handleOAuthKeydown"
    @refresh-anthropic-quota="accounts.loadAnthropicRateLimits"
    @start-codex-login="codex.startLogin"
    @import-codex-cli="codex.importCli"
    @cancel-codex-login="codex.cancelLogin"
    @codex-logout="codex.logout"
    @retry-codex-validation="codex.retryValidation"
    @refresh-codex-quota="codex.refreshSubscription"
    @consume-codex-reset-credit="codex.consumeResetCredit"
    @copy-code="codex.copyCode"
    @update:codex-transport="codex.setTransport"
    @update:codex-auto-update-client-version="codex.setAutoUpdateClientVersion"
    @update:codex-context-window="codex.setContextWindow"
    @update:codex-session-title-generation="codex.setSessionTitleGeneration"
    @update:codex-auto-review="codex.setAutoReview"
    @update:codex-use-apply-patch="codex.setUseApplyPatch"
    @update:codex-prefix-cache-ttl-seconds="codex.setPrefixCacheTtlSeconds"
    @start-add-provider="custom.startAddCustomProvider"
    @start-edit-provider="custom.startEditCustomProvider"
    @delete-provider="custom.deleteCustomProvider"
    @update:edit-key="editKey = $event"
    @update:oauth-code="oauthCode = $event"
  />
</template>
