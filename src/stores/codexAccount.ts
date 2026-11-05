// OpenAI Codex subscription in Settings and onboarding: device sign-in,
// validation, quota and reset credits, plus the `providers.codex.*` settings.
// Lives per window so a sign-in in progress survives page switches.
import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { openUrl } from "@tauri-apps/plugin-opener";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  codexConsumeRateLimitResetCredit,
  codexLogout,
  codexPollLogin,
  codexRateLimits,
  codexRetryAuth,
  codexStartLogin,
  codexStatus as fetchCodexStatus,
  importCodexCli as serviceImportCodexCli,
  type CodexStatus as RemoteCodexStatus,
} from "../services/auth";
import { normalizeAppError } from "../services/errors";
import { normalizeCodexContextWindow } from "../config/codexContext";
import { setSettingValue, settingValue } from "../composables/useSettingsStore";
import { useCopyFeedback } from "../composables/useCopyFeedback";
import { getWarmup } from "../composables/warmupCache";
import { emptyCodexQuota, normalizeCodexQuota, type CodexQuotaState } from "../utils/providerQuota";
import { t } from "../i18n";
import type { CodexModelConfig, CodexTransportMode } from "../types";
import { useModelStore } from "./model";
import { useNotificationStore } from "./notification";
import { useSettingsFeedbackStore } from "./settingsFeedback";

export interface CodexStatusState {
  authenticated: boolean;
  accountId: string | null;
  validationFailed: boolean;
  validationError: string | null;
}

export type CodexLoginStep = "idle" | "opening" | "waiting" | "success";

type CodexOption = Exclude<keyof CodexModelConfig, "extendedContext">;

/** `settings.json` keys of the Codex options. */
export const CODEX_SETTING_KEYS: Record<CodexOption, string> = {
  transport: "providers.codex.transport",
  autoUpdateClientVersion: "providers.codex.autoUpdateClientVersion",
  contextWindow: "providers.codex.contextWindow",
  generateSessionTitles: "providers.codex.generateSessionTitles",
  autoReview: "providers.codex.autoReview",
  useApplyPatch: "providers.codex.useApplyPatch",
  prefixCacheTtlSeconds: "providers.codex.prefixCacheTtlSeconds",
};

const DEFAULT_PREFIX_CACHE_TTL_SECONDS = 30 * 60;
const MAX_PREFIX_CACHE_TTL_SECONDS = 30 * 24 * 60 * 60;

/** Success and failure messages of each option's save. */
const OPTION_MESSAGES: Record<Exclude<CodexOption, "autoUpdateClientVersion">, [string, string]> = {
  transport: ["settings.codex.transportSaved", "settings.codex.transportSaveFailed"],
  contextWindow: ["settings.codex.contextWindowSaved", "settings.codex.contextWindowSaveFailed"],
  generateSessionTitles: ["settings.codex.sessionTitleSaved", "settings.codex.sessionTitleSaveFailed"],
  autoReview: ["settings.codex.autoReviewSaved", "settings.codex.autoReviewSaveFailed"],
  useApplyPatch: ["settings.codex.applyPatchSaved", "settings.codex.applyPatchSaveFailed"],
  prefixCacheTtlSeconds: ["settings.codex.prefixCacheTtlSaved", "settings.codex.prefixCacheTtlSaveFailed"],
};

export function normalizeCodexStatus(status?: RemoteCodexStatus | null): CodexStatusState {
  return {
    authenticated: !!status?.authenticated,
    accountId: status?.accountId ?? null,
    validationFailed: !!status?.validationFailed,
    validationError: status?.validationError ?? null,
  };
}

export function normalizeCodexModelConfig(
  config?: Partial<Record<keyof CodexModelConfig, unknown>> | null,
): CodexModelConfig {
  const prefixCacheTtlSeconds = Number(config?.prefixCacheTtlSeconds);
  return {
    transport: config?.transport === "http" ? "http" : "websocket",
    autoUpdateClientVersion: config?.autoUpdateClientVersion !== false,
    contextWindow: normalizeCodexContextWindow(
      config?.contextWindow,
      config?.extendedContext === true,
    ),
    generateSessionTitles: config?.generateSessionTitles === true,
    autoReview: config?.autoReview === true,
    useApplyPatch: config?.useApplyPatch === true,
    prefixCacheTtlSeconds: config?.prefixCacheTtlSeconds != null && Number.isFinite(prefixCacheTtlSeconds)
      ? Math.min(MAX_PREFIX_CACHE_TTL_SECONDS, Math.max(0, Math.round(prefixCacheTtlSeconds)))
      : DEFAULT_PREFIX_CACHE_TTL_SECONDS,
  };
}

export const useCodexAccountStore = defineStore("codexAccount", () => {
  const step = ref<CodexLoginStep>("idle");
  const status = ref<CodexStatusState>(normalizeCodexStatus());
  const quota = ref<CodexQuotaState>(emptyCodexQuota());
  const refreshing = ref(false);
  const resetCreditBusyId = ref<string | null>(null);
  const retrying = ref(false);
  const importing = ref(false);
  const clientVersionBusy = ref(false);
  const userCode = ref("");
  const url = ref("");
  const deviceAuthId = ref("");
  const interval = ref(5);
  const { copied: codeCopied, copyText, reset: resetCopyState } = useCopyFeedback();
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollInFlight = false;
  let loaded = false;

  /** The Codex options as stored in `settings.json`; defaults until loaded. */
  const modelConfig = computed<CodexModelConfig>(() => {
    const raw: Partial<Record<CodexOption, unknown>> = {};
    for (const [field, key] of Object.entries(CODEX_SETTING_KEYS)) {
      raw[field as CodexOption] = settingValue(key);
    }
    return normalizeCodexModelConfig(raw);
  });

  const signedIn = computed(() => status.value.authenticated && !status.value.validationFailed);

  function stopPolling(): void {
    if (pollTimer) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
    pollInFlight = false;
  }

  function schedulePoll(delayMs = interval.value * 1000): void {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => {
      pollTimer = null;
      void poll();
    }, delayMs);
  }

  function clearQuota(): void {
    quota.value = emptyCodexQuota();
    resetCreditBusyId.value = null;
  }

  async function loadStatus(): Promise<void> {
    try {
      status.value = normalizeCodexStatus(await fetchCodexStatus());
      if (!signedIn.value) clearQuota();
    } catch { /* ignore */ }
  }

  async function loadRateLimits(): Promise<void> {
    if (!signedIn.value) {
      clearQuota();
      return;
    }
    quota.value = { ...quota.value, loading: true, error: null };
    try {
      quota.value = normalizeCodexQuota(await codexRateLimits());
    } catch (e) {
      const err = normalizeAppError(e);
      quota.value = { ...quota.value, loading: false, error: err.message };
    }
  }

  /** First use in this window: the startup warm-up result, else a fresh load. */
  async function ensureLoaded(): Promise<void> {
    if (loaded) return;
    loaded = true;
    const cached = getWarmup<RemoteCodexStatus>("settings:codexStatus");
    if (cached) status.value = normalizeCodexStatus(cached);
    else await loadStatus();
    if (signedIn.value) void loadRateLimits();
  }

  async function refreshSubscription(): Promise<void> {
    if (
      refreshing.value
      || clientVersionBusy.value
      || quota.value.loading
      || resetCreditBusyId.value !== null
      || !signedIn.value
    ) return;

    refreshing.value = true;
    try {
      const [, modelsResult] = await Promise.allSettled([
        loadRateLimits(),
        useModelStore().loadCodexAvailableModels(true),
      ]);
      if (modelsResult.status === "rejected") {
        const err = normalizeAppError(modelsResult.reason);
        useNotificationStore().addNotice("error", err.message, {
          code: err.code,
          operation: "codexModelsRefresh",
        });
      }
    } finally {
      refreshing.value = false;
    }
  }

  async function consumeResetCredit(creditId: string | null): Promise<void> {
    if (resetCreditBusyId.value !== null || !signedIn.value) return;

    const confirmed = await confirm(t("settings.codex.resetCreditConsumeConfirm"), {
      title: t("settings.codex.resetCredits"),
      kind: "warning",
    });
    if (!confirmed) return;

    resetCreditBusyId.value = creditId ?? "__next_available__";
    try {
      const response = await codexConsumeRateLimitResetCredit(creditId);
      await loadRateLimits();
      const notifications = useNotificationStore();
      switch (response.outcome) {
        case "reset":
        case "alreadyRedeemed":
          notifications.addNotice("success", t("settings.codex.resetCreditConsumeSuccess"), {
            operation: "codexRateLimitResetConsume",
          });
          break;
        case "nothingToReset":
          notifications.addNotice("warning", t("settings.codex.resetCreditNothingToReset"), {
            operation: "codexRateLimitResetConsume",
          });
          break;
        case "noCredit":
          notifications.addNotice("warning", t("settings.codex.resetCreditUnavailable"), {
            operation: "codexRateLimitResetConsume",
          });
          break;
      }
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.resetCreditConsumeFailed", err.message), {
        code: err.code,
        operation: "codexRateLimitResetConsume",
      });
    } finally {
      resetCreditBusyId.value = null;
    }
  }

  function publishModelConfig(): void {
    useModelStore().applyCodexModelConfig(modelConfig.value);
  }

  /** Saves the client version switch, then refetches the subscription
   *  catalog the version determines; refreshes wait for both. */
  async function setAutoUpdateClientVersion(enabled: boolean): Promise<void> {
    if (clientVersionBusy.value || refreshing.value) return;
    if (modelConfig.value.autoUpdateClientVersion === enabled) return;
    clientVersionBusy.value = true;
    try {
      await setSettingValue(CODEX_SETTING_KEYS.autoUpdateClientVersion, enabled);
      publishModelConfig();
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.clientVersionSaveFailed", err.message), {
        code: err.code,
        operation: "saveCodexModelConfig",
      });
      clientVersionBusy.value = false;
      return;
    }
    try {
      if (signedIn.value) {
        await useModelStore().loadCodexAvailableModels(true);
      }
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", err.message, {
        code: err.code,
        operation: "codexModelsRefresh",
      });
    } finally {
      clientVersionBusy.value = false;
    }
  }

  async function setOption<K extends keyof typeof OPTION_MESSAGES>(
    option: K,
    value: CodexModelConfig[K],
  ): Promise<void> {
    const next = normalizeCodexModelConfig({ ...modelConfig.value, [option]: value })[option];
    if (modelConfig.value[option] === next) return;
    const [savedKey, failedKey] = OPTION_MESSAGES[option];
    try {
      await setSettingValue(CODEX_SETTING_KEYS[option], next);
      publishModelConfig();
      useSettingsFeedbackStore().flashSuccess(t(savedKey));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t(failedKey, err.message), {
        code: err.code,
        operation: "saveCodexModelConfig",
      });
    }
  }

  const setTransport = (value: CodexTransportMode) => setOption("transport", value);
  const setContextWindow = (value: number) => setOption("contextWindow", value);
  const setSessionTitleGeneration = (value: boolean) => setOption("generateSessionTitles", value);
  const setAutoReview = (value: boolean) => setOption("autoReview", value);
  const setUseApplyPatch = (value: boolean) => setOption("useApplyPatch", value);
  const setPrefixCacheTtlSeconds = (value: number) => setOption("prefixCacheTtlSeconds", value);

  async function poll(): Promise<void> {
    if (pollInFlight || step.value !== "waiting") return;
    pollInFlight = true;
    try {
      const result = await codexPollLogin(deviceAuthId.value, userCode.value);
      if (result.status === "success") {
        stopPolling();
        step.value = "success";
        await loadStatus();
        await loadRateLimits();
        const feedback = useSettingsFeedbackStore();
        feedback.authChanged();
        feedback.flashSuccess(t("settings.codex.loginSuccess"), 3000);
        setTimeout(() => { if (step.value === "success") step.value = "idle"; }, 3000);
      } else if (result.status === "failed") {
        stopPolling();
        step.value = "idle";
        useNotificationStore().addNotice("error", result.message ?? t("settings.codex.authFailed"), {
          operation: "codexLogin",
        });
      } else if (step.value === "waiting") {
        schedulePoll();
      }
    } catch {
      if (step.value === "waiting") {
        schedulePoll();
      }
    } finally {
      pollInFlight = false;
    }
  }

  async function startLogin(): Promise<void> {
    if (step.value === "opening" || step.value === "waiting") return;
    stopPolling();
    resetCopyState();
    useSettingsFeedbackStore().clearError();
    step.value = "opening";
    try {
      const info = await codexStartLogin();
      userCode.value = info.userCode;
      url.value = info.url;
      deviceAuthId.value = info.deviceAuthId;
      interval.value = Math.max(info.interval, 5);
      step.value = "waiting";
      void openUrl(info.url).catch(() => undefined);
      schedulePoll();
    } catch (e) {
      step.value = "idle";
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.loginFailed", err.message), {
        code: err.code,
        operation: "codexLogin",
      });
    }
  }

  function cancelLogin(): void {
    stopPolling();
    resetCopyState();
    step.value = "idle";
  }

  async function logout(): Promise<void> {
    try {
      await codexLogout();
      status.value = normalizeCodexStatus();
      clearQuota();
      const feedback = useSettingsFeedbackStore();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.codex.logoutSuccess"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.logoutFailed", err.message), {
        code: err.code,
        operation: "codexLogout",
      });
    }
  }

  async function importCli(): Promise<void> {
    importing.value = true;
    try {
      status.value = normalizeCodexStatus(await serviceImportCodexCli());
      if (signedIn.value) void loadRateLimits();
      const feedback = useSettingsFeedbackStore();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.codex.importSuccess"), 3000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.importFailed", err.message), {
        code: err.code,
        operation: "importCodexCli",
      });
    } finally {
      importing.value = false;
    }
  }

  async function copyCode(): Promise<void> {
    await copyText(userCode.value);
  }

  async function retryValidation(): Promise<void> {
    if (retrying.value || !status.value.authenticated) return;
    retrying.value = true;
    const feedback = useSettingsFeedbackStore();
    feedback.clearError();
    try {
      status.value = normalizeCodexStatus(await codexRetryAuth());
      await loadRateLimits();
      feedback.authChanged();
      feedback.flashSuccess(t("settings.codex.validationRetrySuccess"));
    } catch (e) {
      await loadStatus();
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.validationRetryFailed", err.message), {
        code: err.code,
        operation: "codexRetryAuth",
      });
    } finally {
      retrying.value = false;
    }
  }

  /** After "reset all settings": forget local state; the next use reloads. */
  function reset(): void {
    stopPolling();
    resetCopyState();
    step.value = "idle";
    retrying.value = false;
    status.value = normalizeCodexStatus();
    clearQuota();
    userCode.value = "";
    url.value = "";
    deviceAuthId.value = "";
    interval.value = 5;
    loaded = false;
  }

  return {
    step,
    status,
    quota,
    refreshing,
    resetCreditBusyId,
    retrying,
    importing,
    clientVersionBusy,
    modelConfig,
    userCode,
    url,
    codeCopied,
    ensureLoaded,
    loadStatus,
    loadRateLimits,
    refreshSubscription,
    consumeResetCredit,
    setAutoUpdateClientVersion,
    setTransport,
    setContextWindow,
    setSessionTitleGeneration,
    setAutoReview,
    setUseApplyPatch,
    setPrefixCacheTtlSeconds,
    startLogin,
    cancelLogin,
    logout,
    importCli,
    copyCode,
    retryValidation,
    reset,
  };
});
