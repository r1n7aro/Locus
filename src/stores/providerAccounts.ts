// Built-in provider credentials in Settings: API keys, Anthropic sign-in and
// quota, and the Claude Code CLI connection test. Codex lives in
// `codexAccount`, custom providers in `customProviderSettings`.
import { ref } from "vue";
import { defineStore } from "pinia";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  anthropicRateLimits,
  authLogout,
  deleteProviderKey,
  exchangeAuthCode,
  getAuthUrl,
  getProviders,
  importClaudeCodeOAuth as serviceImportClaudeCodeOAuth,
  saveProviderKey,
  testClaudeCodeCli,
} from "../services/auth";
import { normalizeCustomEndpointTestErrorMessage } from "../services/customEndpointTestResult";
import { normalizeAppError } from "../services/errors";
import { filterVisibleProviders } from "../config/providerVisibility";
import { getWarmup } from "../composables/warmupCache";
import { emptyAnthropicQuota, normalizeAnthropicQuota, type AnthropicQuotaState } from "../utils/providerQuota";
import { t } from "../i18n";
import { useCustomProviderSettingsStore } from "./customProviderSettings";
import { useNotificationStore } from "./notification";
import { useSettingsFeedbackStore } from "./settingsFeedback";

export interface ProviderStatus {
  id: string;
  name: string;
  hasKey: boolean;
  keyHint: string;
  /** Auth state for providers managing credentials outside Locus (claude_code):
   *  false = installed but not logged in; absent = unknown / not applicable. */
  loggedIn?: boolean;
}

export type OAuthStep = "idle" | "waiting_code" | "exchanging";
export type ConnectionTestStatus = "idle" | "testing" | "success" | "error";

export const useProviderAccountsStore = defineStore("providerAccounts", () => {
  const providers = ref<ProviderStatus[]>([]);
  const editingProvider = ref<string | null>(null);
  const editKey = ref("");
  const isLoading = ref(false);
  const oauthStep = ref<OAuthStep>("idle");
  const oauthCode = ref("");
  const anthropicQuota = ref<AnthropicQuotaState>(emptyAnthropicQuota());
  const claudeCodeTestStatus = ref<ConnectionTestStatus>("idle");
  const claudeCodeTestResult = ref("");
  let loaded = false;

  function hasAnthropicLogin(): boolean {
    return providers.value.some((provider) => provider.id === "anthropic" && provider.hasKey);
  }

  async function loadProviders(): Promise<void> {
    try {
      providers.value = filterVisibleProviders(await getProviders() as ProviderStatus[]);
    } catch (e) {
      console.error("get_providers failed:", e);
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", err.message, {
        code: err.code,
        operation: "loadProviders",
        skipConsoleLog: true,
      });
    }
  }

  async function loadAnthropicRateLimits(): Promise<void> {
    if (!hasAnthropicLogin()) {
      anthropicQuota.value = emptyAnthropicQuota();
      return;
    }
    anthropicQuota.value = { ...anthropicQuota.value, loading: true, error: null };
    try {
      anthropicQuota.value = normalizeAnthropicQuota(await anthropicRateLimits());
    } catch (e) {
      const err = normalizeAppError(e);
      anthropicQuota.value = { ...anthropicQuota.value, loading: false, error: err.message };
    }
  }

  /** First use in this window: the startup warm-up result, else a fresh load. */
  async function ensureLoaded(): Promise<void> {
    if (loaded) return;
    loaded = true;
    const cached = getWarmup<ProviderStatus[]>("settings:providers");
    if (cached) providers.value = cached;
    else await loadProviders();
    if (hasAnthropicLogin()) void loadAnthropicRateLimits();
  }

  async function testClaudeCode(): Promise<void> {
    claudeCodeTestStatus.value = "testing";
    claudeCodeTestResult.value = "";
    try {
      const reply = await testClaudeCodeCli();
      claudeCodeTestStatus.value = "success";
      claudeCodeTestResult.value = reply;
      // The live test is authoritative — refresh the heuristic status badge.
      await loadProviders();
    } catch (e) {
      claudeCodeTestStatus.value = "error";
      claudeCodeTestResult.value = normalizeCustomEndpointTestErrorMessage(e);
    }
  }

  function startEdit(providerId: string): void {
    const feedback = useSettingsFeedbackStore();
    editingProvider.value = providerId;
    editKey.value = "";
    feedback.clearError();
    feedback.clearSuccess();
  }

  function cancelEdit(): void {
    editingProvider.value = null;
    editKey.value = "";
    useSettingsFeedbackStore().clearError();
  }

  async function saveKey(providerId: string): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    const key = editKey.value.trim();
    if (!key) {
      feedback.setError(t("settings.provider.enterKey"));
      return;
    }
    feedback.clearError();
    isLoading.value = true;
    try {
      await saveProviderKey(providerId, key);
      editingProvider.value = null;
      editKey.value = "";
      await loadProviders();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.provider.saved"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.provider.saveFailed", err.message), {
        code: err.code,
        operation: "saveKey",
      });
    } finally {
      isLoading.value = false;
    }
  }

  async function deleteKey(providerId: string): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    isLoading.value = true;
    try {
      await deleteProviderKey(providerId);
      await loadProviders();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.provider.deleted"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.provider.deleteFailed", err.message), {
        code: err.code,
        operation: "deleteKey",
      });
    } finally {
      isLoading.value = false;
    }
  }

  function handleKeydown(e: KeyboardEvent, providerId: string): void {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void saveKey(providerId);
    } else if (e.key === "Escape") {
      cancelEdit();
    }
  }

  async function startOAuthLogin(): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    feedback.clearError();
    isLoading.value = true;
    try {
      const info = await getAuthUrl();
      await openUrl(info.url);
      oauthStep.value = "waiting_code";
      feedback.flashSuccess(t("settings.anthropic.browserOpened"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.anthropic.authUrlFailed", err.message), {
        code: err.code,
        operation: "oauthLogin",
      });
    } finally {
      isLoading.value = false;
    }
  }

  async function submitOAuthCode(): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    const code = oauthCode.value.trim();
    if (!code) {
      feedback.setError(t("settings.anthropic.pasteCode"));
      return;
    }
    feedback.clearError();
    oauthStep.value = "exchanging";
    isLoading.value = true;
    try {
      await exchangeAuthCode(code);
      oauthStep.value = "idle";
      oauthCode.value = "";
      await loadProviders();
      await loadAnthropicRateLimits();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.anthropic.loginSuccess"), 3000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.anthropic.exchangeFailed", err.message), {
        code: err.code,
        operation: "oauthExchange",
      });
      oauthStep.value = "waiting_code";
    } finally {
      isLoading.value = false;
    }
  }

  function cancelOAuth(): void {
    oauthStep.value = "idle";
    oauthCode.value = "";
    useSettingsFeedbackStore().reset();
  }

  async function oauthLogout(): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    isLoading.value = true;
    try {
      await authLogout();
      await loadProviders();
      anthropicQuota.value = emptyAnthropicQuota();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.anthropic.logoutSuccess"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.anthropic.logoutFailed", err.message), {
        code: err.code,
        operation: "oauthLogout",
      });
    } finally {
      isLoading.value = false;
    }
  }

  /** Imports Claude Code's sign-in, or its proxy endpoint as a custom provider. */
  async function importClaudeCodeOAuth(): Promise<void> {
    const feedback = useSettingsFeedbackStore();
    isLoading.value = true;
    feedback.reset();
    try {
      const result = await serviceImportClaudeCodeOAuth();
      if (result.kind === "custom_endpoint") {
        if (!result.customEndpoint) {
          throw new Error("Claude Code custom endpoint payload is missing");
        }
        await useCustomProviderSettingsStore().saveImportedEndpoint(result.customEndpoint);
        feedback.flashSuccess(t("settings.anthropic.importCustomEndpointSuccess"), 3000);
      } else {
        await loadProviders();
        await loadAnthropicRateLimits();
        feedback.authChanged();
        feedback.flashSuccess(t("settings.anthropic.importSuccess"), 3000);
      }
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.anthropic.importFailed", err.message), {
        code: err.code,
        operation: "importClaudeCodeOAuth",
      });
    } finally {
      isLoading.value = false;
    }
  }

  function handleOAuthKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submitOAuthCode();
    } else if (e.key === "Escape") {
      cancelOAuth();
    }
  }

  /** After "reset all settings": forget local state; the next use reloads. */
  function reset(): void {
    providers.value = [];
    editingProvider.value = null;
    editKey.value = "";
    isLoading.value = false;
    oauthStep.value = "idle";
    oauthCode.value = "";
    anthropicQuota.value = emptyAnthropicQuota();
    claudeCodeTestStatus.value = "idle";
    claudeCodeTestResult.value = "";
    loaded = false;
  }

  return {
    providers,
    editingProvider,
    editKey,
    isLoading,
    oauthStep,
    oauthCode,
    anthropicQuota,
    claudeCodeTestStatus,
    claudeCodeTestResult,
    ensureLoaded,
    loadProviders,
    loadAnthropicRateLimits,
    testClaudeCode,
    startEdit,
    cancelEdit,
    saveKey,
    deleteKey,
    handleKeydown,
    startOAuthLogin,
    submitOAuthCode,
    cancelOAuth,
    oauthLogout,
    importClaudeCodeOAuth,
    handleOAuthKeydown,
    reset,
  };
});
