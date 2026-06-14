import { ref, computed, onMounted, onUnmounted } from "vue";
import { clearWarmup, getWarmup, setWarmup } from "./warmupCache";
import { resetAllConfig } from "../services/project";
import {
  getProviders,
  testClaudeCodeCli,
  saveProviderKey,
  deleteProviderKey,
  getAuthUrl,
  exchangeAuthCode,
  authLogout,
  codexStatus as fetchCodexStatus,
  codexRateLimits as fetchCodexRateLimits,
  codexStartLogin,
  codexPollLogin,
  codexLogout as serviceCodexLogout,
  codexRetryAuth as serviceCodexRetryAuth,
} from "../services/auth";
import type {
  CodexRateLimitSnapshot,
  CodexRateLimitWindow as RemoteCodexRateLimitWindow,
  CodexRateLimitsResponse,
  CodexStatus as RemoteCodexStatus,
} from "../services/auth";
import {
  getModelDefaults,
  saveModelDefaults as serviceSaveModelDefaults,
  getCodexModelConfig,
  saveCodexModelConfig as serviceSaveCodexModelConfig,
  getCustomEndpoints,
  saveCustomEndpoints,
  testCustomEndpoint,
} from "../services/model";
import {
  getDynamicToolLoadingMode,
  setDynamicToolLoadingMode as serviceSetDynamicToolLoadingMode,
  type DynamicToolLoadingMode,
} from "../services/system";
import {
  getFileToolWorkspaceBoundary,
  getToolPermissions,
  setFileToolWorkspaceBoundary,
  saveToolPermissions as serviceSaveToolPermissions,
} from "../services/permissions";
import {
  customEndpointTestStatusForReply,
  normalizeCustomEndpointTestErrorMessage,
} from "../services/customEndpointTestResult";
import { openUrl } from "@tauri-apps/plugin-opener";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "../stores/notification";
import type {
  ModelDefaults,
  CustomEndpoint,
  EffortLevel,
  ApiFormat,
  ReasoningParamFormat,
  CodexTransportMode,
  CodexModelConfig,
} from "../types";
import { t } from "../i18n";
import { filterVisibleProviders } from "../config/providerVisibility";
import { useCopyFeedback } from "./useCopyFeedback";
import { setThemePreference } from "./useTheme";

const DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH = 256_000;

export interface ProviderStatus {
  id: string;
  name: string;
  hasKey: boolean;
  keyHint: string;
  /** Auth state for providers managing credentials outside Locus (claude_code):
   *  false = installed but not logged in; absent = unknown / not applicable. */
  loggedIn?: boolean;
}

export interface CodexStatusState {
  authenticated: boolean;
  accountId: string | null;
  validationFailed: boolean;
  validationError: string | null;
}

export interface CodexQuotaWindowState {
  id: string;
  limitId: string;
  limitName: string | null;
  windowType: "primary" | "secondary";
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
}

export interface CodexQuotaCreditsState {
  unlimited: boolean;
  balance: string | null;
}

export interface CodexQuotaState {
  loading: boolean;
  loaded: boolean;
  error: string | null;
  fetchedAtMs: number | null;
  windows: CodexQuotaWindowState[];
  credits: CodexQuotaCreditsState | null;
  planType: string | null;
}

type SettingsEmit = {
  (e: "authChanged"): void;
  (e: "modelDefaultsChanged", defaults: ModelDefaults): void;
  (e: "codexTransportChanged", config: CodexModelConfig): void;
  (e: "customEndpointsChanged", endpoints: CustomEndpoint[]): void;
  (e: "resetOnboarding"): void;
};

export function useSettingsState(emit: SettingsEmit) {
  function emptyCodexQuota(): CodexQuotaState {
    return {
      loading: false,
      loaded: false,
      error: null,
      fetchedAtMs: null,
      windows: [],
      credits: null,
      planType: null,
    };
  }

  function normalizeCodexStatus(status?: RemoteCodexStatus | null): CodexStatusState {
    return {
      authenticated: !!status?.authenticated,
      accountId: status?.accountId ?? null,
      validationFailed: !!status?.validationFailed,
      validationError: status?.validationError ?? null,
    };
  }

  function normalizeCodexModelConfig(
    config?: Partial<CodexModelConfig> | null,
  ): CodexModelConfig {
    return {
      transport: config?.transport === "http" ? "http" : "websocket",
    };
  }

  function clampPercent(value: unknown, fallback: number): number {
    const numeric = typeof value === "number" ? value : Number(value);
    if (!Number.isFinite(numeric)) return fallback;
    return Math.max(0, Math.min(100, numeric));
  }

  function normalizeRateLimitWindow(
    window: RemoteCodexRateLimitWindow | null | undefined,
    fallbackRemaining: number,
  ): Pick<CodexQuotaWindowState, "usedPercent" | "remainingPercent" | "windowMinutes" | "resetsAt"> | null {
    if (!window) return null;
    const usedPercent = clampPercent(window.usedPercent, 100 - fallbackRemaining);
    const remainingPercent = clampPercent(window.remainingPercent, 100 - usedPercent);
    const windowMinutes = typeof window.windowMinutes === "number" && Number.isFinite(window.windowMinutes)
      ? window.windowMinutes
      : null;
    const resetsAt = typeof window.resetsAt === "number" && Number.isFinite(window.resetsAt)
      ? window.resetsAt
      : null;

    return { usedPercent, remainingPercent, windowMinutes, resetsAt };
  }

  function appendQuotaWindows(
    result: CodexQuotaWindowState[],
    limitId: string,
    snapshot: CodexRateLimitSnapshot,
  ) {
    const limitName = snapshot.limitName ?? null;
    const primary = normalizeRateLimitWindow(snapshot.primary, 100);
    if (primary) {
      result.push({
        id: `${limitId}:primary`,
        limitId,
        limitName,
        windowType: "primary",
        ...primary,
      });
    }

    const secondary = normalizeRateLimitWindow(snapshot.secondary, 100);
    if (secondary) {
      result.push({
        id: `${limitId}:secondary`,
        limitId,
        limitName,
        windowType: "secondary",
        ...secondary,
      });
    }
  }

  function normalizeCodexQuota(response?: CodexRateLimitsResponse | null): CodexQuotaState {
    if (!response?.rateLimits) return emptyCodexQuota();

    const windows: CodexQuotaWindowState[] = [];
    const primaryLimitId = response.rateLimits.limitId ?? "codex";
    appendQuotaWindows(windows, primaryLimitId, response.rateLimits);

    const credits = response.rateLimits.credits?.hasCredits
      ? {
          unlimited: !!response.rateLimits.credits.unlimited,
          balance: response.rateLimits.credits.balance ?? null,
        }
      : null;

    return {
      loading: false,
      loaded: true,
      error: null,
      fetchedAtMs: response.fetchedAtMs,
      windows,
      credits,
      planType: response.rateLimits.planType ?? null,
    };
  }

  // ── General ──────────────────────────────────────────────────────────
  const resetConfirm = ref(false);

  async function handleResetOnboarding() {
    const emptyDefaults: ModelDefaults = { mainModel: "", planModel: "", subagentModels: {} };
    try {
      localStorage.removeItem("locus-onboarding-completed");
      localStorage.removeItem("locus-locale");
      localStorage.removeItem("locus-theme-preference");
      localStorage.removeItem("locus-unity-embed-theme-preference");
      localStorage.removeItem("locus-knowledge-access-mode");
      localStorage.removeItem("locus:sessionPanelWidth");
      localStorage.removeItem("locus:unity:sessionPanelWidth");
      localStorage.removeItem("locus:unity:sessionPanelCollapsed");
      localStorage.removeItem("locus:chatSidebarWidth");
      localStorage.removeItem("locus:chatSidebarHeight");
      localStorage.removeItem("locus:unity:chatSidebarWidth");
      localStorage.removeItem("locus:unity:chatSidebarHeight");
      localStorage.removeItem("locus:collabLeftColWidth");
      localStorage.removeItem("locus:collabTerminalHeight");
    } catch { /* ignore */ }
    setThemePreference("main", "dark");
    setThemePreference("unityEmbed", "dark");
    try {
      await resetAllConfig();
    } catch (e) {
      console.error("reset_all_config failed:", e);
    }
    clearWarmup();
    resetConfirm.value = false;
    activeCategory.value = "general";
    providers.value = [];
    editingProvider.value = null;
    editKey.value = "";
    errorMsg.value = "";
    successMsg.value = "";
    isLoading.value = false;
    oauthStep.value = "idle";
    oauthCode.value = "";
    stopCodexPolling();
    resetCodexCopyState();
    codexStep.value = "idle";
    codexRetrying.value = false;
    codexStatus.value = normalizeCodexStatus();
    codexQuota.value = emptyCodexQuota();
    codexModelConfig.value = normalizeCodexModelConfig();
    codexUserCode.value = "";
    codexUrl.value = "";
    codexDeviceAuthId.value = "";
    codexInterval.value = 5;
    showDisclaimer.value = false;
    disclaimerTarget.value = null;
    modelDefaults.value = emptyDefaults;
    modelSaveMsg.value = "";
    toolPermissions.value = {};
    permSaveMsg.value = "";
    dynamicToolLoadingMode.value = "metaTool";
    dynamicToolLoadingBusy.value = false;
    customEndpoints.value = [];
    editingEndpoint.value = null;
    isAddingEndpoint.value = false;
    customEndpointSaving.value = false;
    testStatus.value = "idle";
    testResult.value = "";
    emit("authChanged");
    emit("modelDefaultsChanged", emptyDefaults);
    emit("customEndpointsChanged", []);
    emit("resetOnboarding");
  }

  // ── Navigation ───────────────────────────────────────────────────────
  const activeCategory = ref<"api" | "models" | "permissions" | "codeAnalysis" | "hotReload" | "unityConnection" | "testing" | "proxy" | "general" | "display" | "notifications" | "shortcuts" | "knowledge" | "archived" | "console" | "about">("general");

  // ── Provider / API key state ─────────────────────────────────────────
  const providers = ref<ProviderStatus[]>([]);
  const editingProvider = ref<string | null>(null);
  const editKey = ref("");
  const errorMsg = ref("");
  const successMsg = ref("");
  const isLoading = ref(false);
  const claudeCodeTestStatus = ref<"idle" | "testing" | "success" | "error">("idle");
  const claudeCodeTestResult = ref("");

  async function testClaudeCode() {
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

  async function loadProviders() {
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

  function startEdit(providerId: string) {
    editingProvider.value = providerId;
    editKey.value = "";
    errorMsg.value = "";
    successMsg.value = "";
  }

  function cancelEdit() {
    editingProvider.value = null;
    editKey.value = "";
    errorMsg.value = "";
  }

  async function saveKey(providerId: string) {
    const key = editKey.value.trim();
    if (!key) {
      errorMsg.value = t("settings.provider.enterKey");
      return;
    }

    errorMsg.value = "";
    isLoading.value = true;

    try {
      await saveProviderKey(providerId, key);
      successMsg.value = t("settings.provider.saved");
      editingProvider.value = null;
      editKey.value = "";
      await loadProviders();
      emit("authChanged");
      setTimeout(() => { successMsg.value = ""; }, 2000);
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

  async function deleteKey(providerId: string) {
    isLoading.value = true;
    try {
      await deleteProviderKey(providerId);
      await loadProviders();
      emit("authChanged");
      successMsg.value = t("settings.provider.deleted");
      setTimeout(() => { successMsg.value = ""; }, 2000);
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

  function handleKeydown(e: KeyboardEvent, providerId: string) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      saveKey(providerId);
    } else if (e.key === "Escape") {
      cancelEdit();
    }
  }

  // ── OAuth ────────────────────────────────────────────────────────────
  const oauthStep = ref<"idle" | "waiting_code" | "exchanging">("idle");
  const oauthCode = ref("");

  async function startOAuthLogin() {
    errorMsg.value = "";
    isLoading.value = true;
    try {
      const info = await getAuthUrl();
      await openUrl(info.url);
      oauthStep.value = "waiting_code";
      successMsg.value = t("settings.anthropic.browserOpened");
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

  async function submitOAuthCode() {
    const code = oauthCode.value.trim();
    if (!code) {
      errorMsg.value = t("settings.anthropic.pasteCode");
      return;
    }
    errorMsg.value = "";
    oauthStep.value = "exchanging";
    isLoading.value = true;
    try {
      await exchangeAuthCode(code);
      successMsg.value = t("settings.anthropic.loginSuccess");
      oauthStep.value = "idle";
      oauthCode.value = "";
      await loadProviders();
      emit("authChanged");
      setTimeout(() => { successMsg.value = ""; }, 3000);
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

  function cancelOAuth() {
    oauthStep.value = "idle";
    oauthCode.value = "";
    errorMsg.value = "";
    successMsg.value = "";
  }

  async function oauthLogout() {
    isLoading.value = true;
    try {
      await authLogout();
      await loadProviders();
      emit("authChanged");
      successMsg.value = t("settings.anthropic.logoutSuccess");
      setTimeout(() => { successMsg.value = ""; }, 2000);
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

  function handleOAuthKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submitOAuthCode();
    } else if (e.key === "Escape") {
      cancelOAuth();
    }
  }

  // ── Dynamic tool loading ────────────────────────────────────────────
  const dynamicToolLoadingMode = ref<DynamicToolLoadingMode>("metaTool");
  const dynamicToolLoadingBusy = ref(false);

  function normalizeDynamicToolLoadingMode(value: string): DynamicToolLoadingMode {
    return value === "direct" ? "direct" : "metaTool";
  }

  async function loadDynamicToolLoadingMode() {
    try {
      dynamicToolLoadingMode.value = await getDynamicToolLoadingMode();
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.dynamicToolLoading.loadFailed", err.message), {
        code: err.code,
        operation: "loadDynamicToolLoadingMode",
        skipConsoleLog: true,
      });
    }
  }

  async function setDynamicToolLoadingMode(value: string) {
    const next = normalizeDynamicToolLoadingMode(value);
    if (dynamicToolLoadingMode.value === next) return;
    const previous = dynamicToolLoadingMode.value;
    dynamicToolLoadingMode.value = next;
    dynamicToolLoadingBusy.value = true;
    try {
      await serviceSetDynamicToolLoadingMode(next);
      successMsg.value = t("settings.dynamicToolLoading.saved");
      setTimeout(() => { successMsg.value = ""; }, 2000);
    } catch (e) {
      dynamicToolLoadingMode.value = previous;
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.dynamicToolLoading.saveFailed", err.message), {
        code: err.code,
        operation: "saveDynamicToolLoadingMode",
      });
    } finally {
      dynamicToolLoadingBusy.value = false;
    }
  }

  // ── Codex (device auth) ──────────────────────────────────────────────
  type CodexStep = "idle" | "opening" | "waiting" | "success";
  const codexStep = ref<CodexStep>("idle");
  const codexStatus = ref<CodexStatusState>(normalizeCodexStatus());
  const codexQuota = ref<CodexQuotaState>(emptyCodexQuota());
  const codexRetrying = ref(false);
  const codexModelConfig = ref<CodexModelConfig>(normalizeCodexModelConfig());
  const codexUserCode = ref("");
  const codexUrl = ref("");
  const codexDeviceAuthId = ref("");
  const codexInterval = ref(5);
  const { copied: codexCodeCopied, copyText: copyCodexText, reset: resetCodexCopyState } = useCopyFeedback();
  let codexTimer: ReturnType<typeof setTimeout> | null = null;
  let codexPollInFlight = false;

  function stopCodexPolling() {
    if (codexTimer) {
      clearTimeout(codexTimer);
      codexTimer = null;
    }
    codexPollInFlight = false;
  }

  function scheduleCodexPoll(delayMs = codexInterval.value * 1000) {
    if (codexTimer) clearTimeout(codexTimer);
    codexTimer = setTimeout(() => {
      codexTimer = null;
      void pollCodex();
    }, delayMs);
  }

  async function loadCodexStatus() {
    try {
      codexStatus.value = normalizeCodexStatus(await fetchCodexStatus());
      if (!codexStatus.value.authenticated || codexStatus.value.validationFailed) {
        codexQuota.value = emptyCodexQuota();
      }
    } catch { /* ignore */ }
  }

  async function loadCodexRateLimits() {
    if (!codexStatus.value.authenticated || codexStatus.value.validationFailed) {
      codexQuota.value = emptyCodexQuota();
      return;
    }

    codexQuota.value = {
      ...codexQuota.value,
      loading: true,
      error: null,
    };

    try {
      codexQuota.value = normalizeCodexQuota(await fetchCodexRateLimits());
    } catch (e) {
      const err = normalizeAppError(e);
      codexQuota.value = {
        ...codexQuota.value,
        loading: false,
        error: err.message,
      };
    }
  }

  async function loadCodexModelConfig() {
    try {
      codexModelConfig.value = normalizeCodexModelConfig(await getCodexModelConfig());
    } catch { /* ignore */ }
  }

  async function setCodexTransportMode(transport: CodexTransportMode) {
    const next = normalizeCodexModelConfig({ transport });
    if (codexModelConfig.value.transport === next.transport) return;
    codexModelConfig.value = next;
    try {
      await serviceSaveCodexModelConfig(next);
      emit("codexTransportChanged", next);
      successMsg.value = t("settings.codex.transportSaved");
      setTimeout(() => { successMsg.value = ""; }, 2000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.transportSaveFailed", err.message), {
        code: err.code,
        operation: "saveCodexModelConfig",
      });
      await loadCodexModelConfig();
    }
  }

  async function pollCodex() {
    if (codexPollInFlight || codexStep.value !== "waiting") return;
    codexPollInFlight = true;
    try {
      const result = await codexPollLogin(codexDeviceAuthId.value, codexUserCode.value);
      if (result.status === "success") {
        stopCodexPolling();
        codexStep.value = "success";
        await loadCodexStatus();
        await loadCodexRateLimits();
        emit("authChanged");
        successMsg.value = t("settings.codex.loginSuccess");
        setTimeout(() => { successMsg.value = ""; codexStep.value = "idle"; }, 3000);
      } else if (result.status === "failed") {
        stopCodexPolling();
        codexStep.value = "idle";
        useNotificationStore().addNotice("error", result.message ?? t("settings.codex.authFailed"), {
          operation: "codexLogin",
        });
      } else if (codexStep.value === "waiting") {
        scheduleCodexPoll();
      }
    } catch {
      if (codexStep.value === "waiting") {
        scheduleCodexPoll();
      }
    } finally {
      codexPollInFlight = false;
    }
  }

  async function startCodexLogin() {
    if (codexStep.value === "opening" || codexStep.value === "waiting") return;
    stopCodexPolling();
    resetCodexCopyState();
    errorMsg.value = "";
    codexStep.value = "opening";
    try {
      const info = await codexStartLogin();
      codexUserCode.value = info.userCode;
      codexUrl.value = info.url;
      codexDeviceAuthId.value = info.deviceAuthId;
      codexInterval.value = Math.max(info.interval, 5);
      codexStep.value = "waiting";
      void openUrl(info.url).catch(() => undefined);
      scheduleCodexPoll();
    } catch (e) {
      codexStep.value = "idle";
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.loginFailed", err.message), {
        code: err.code,
        operation: "codexLogin",
      });
    }
  }

  function cancelCodexLogin() {
    stopCodexPolling();
    resetCodexCopyState();
    codexStep.value = "idle";
  }

  async function codexLogout() {
    try {
      await serviceCodexLogout();
      codexStatus.value = normalizeCodexStatus();
      codexQuota.value = emptyCodexQuota();
      emit("authChanged");
      successMsg.value = t("settings.codex.logoutSuccess");
      setTimeout(() => { successMsg.value = ""; }, 2000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.logoutFailed", err.message), {
        code: err.code,
        operation: "codexLogout",
      });
    }
  }

  async function copyCode() {
    await copyCodexText(codexUserCode.value);
  }

  async function retryCodexValidation() {
    if (codexRetrying.value || !codexStatus.value.authenticated) return;
    codexRetrying.value = true;
    errorMsg.value = "";
    try {
      codexStatus.value = normalizeCodexStatus(await serviceCodexRetryAuth());
      await loadCodexRateLimits();
      emit("authChanged");
      successMsg.value = t("settings.codex.validationRetrySuccess");
      setTimeout(() => { successMsg.value = ""; }, 2000);
    } catch (e) {
      await loadCodexStatus();
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.codex.validationRetryFailed", err.message), {
        code: err.code,
        operation: "codexRetryAuth",
      });
    } finally {
      codexRetrying.value = false;
    }
  }

  // ── Subscription disclaimer ─────────────────────────────────────────
  const showDisclaimer = ref(false);
  const disclaimerTarget = ref<"anthropic" | null>(null);

  function requestOAuthLogin() {
    disclaimerTarget.value = "anthropic";
    oauthStep.value = "idle";
    oauthCode.value = "";
    errorMsg.value = "";
    successMsg.value = "";
    showDisclaimer.value = true;
  }

  function requestCodexLogin() {
    void startCodexLogin();
  }

  function confirmDisclaimer() {
    showDisclaimer.value = false;
    disclaimerTarget.value = null;
  }

  function cancelDisclaimer() {
    showDisclaimer.value = false;
    disclaimerTarget.value = null;
  }

  // ── Model defaults ──────────────────────────────────────────────────
  const modelDefaults = ref<ModelDefaults>({ mainModel: "", planModel: "", subagentModels: {} });
  const modelSaveMsg = ref("");

  async function loadModelDefaults() {
    try {
      modelDefaults.value = await getModelDefaults();
    } catch { /* use empty defaults */ }
  }

  async function saveModelDefaults() {
    try {
      await serviceSaveModelDefaults(modelDefaults.value);
      emit("modelDefaultsChanged", modelDefaults.value);
      modelSaveMsg.value = t("settings.models.saved");
      setTimeout(() => { modelSaveMsg.value = ""; }, 2000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.models.saveFailed", err.message), {
        code: err.code,
        operation: "saveModelDefaults",
      });
    }
  }

  // ── Tool permissions ─────────────────────────────────────────────────
  const permSaveMsg = ref("");
  const fileToolWorkspaceBoundary = ref(false);
  const fileToolWorkspaceBoundaryReady = ref(false);
  const fileToolWorkspaceBoundaryBusy = ref(false);
  let permSaveTimer: ReturnType<typeof setTimeout> | null = null;

  const toolList = computed(() => [
    { name: "read",               label: "read",               desc: t("tool.desc.read"),               defaultMode: "auto" as const },
    { name: "grep",               label: "grep",               desc: t("tool.desc.grep"),               defaultMode: "auto" as const },
    { name: "list",               label: "list",               desc: t("tool.desc.list"),               defaultMode: "auto" as const },
    { name: "task",               label: "task",               desc: t("tool.desc.task"),               defaultMode: "ask"  as const },
    { name: "todowrite",          label: "todowrite",          desc: t("tool.desc.todowrite"),          defaultMode: "auto" as const },
    { name: "ask_user_question",  label: "ask_user_question",  desc: t("tool.desc.ask_user_question"),  defaultMode: "auto" as const },
    { name: "sheet",              label: "sheet",              desc: t("tool.desc.sheet"),              defaultMode: "auto" as const },
    { name: "graph_view",         label: "graph_view",         desc: t("tool.desc.graph_view"),         defaultMode: "auto" as const },
    { name: "write",              label: "write",              desc: t("tool.desc.write"),              defaultMode: "ask"  as const },
    { name: "edit",               label: "edit",               desc: t("tool.desc.edit"),               defaultMode: "ask"  as const },
    { name: "bash",               label: "bash",               desc: t("tool.desc.bash"),               defaultMode: "ask"  as const },
    { name: "web_fetch",          label: "web_fetch",          desc: t("tool.desc.web_fetch"),          defaultMode: "ask"  as const },
    { name: "unity_execute",      label: "unity_execute",      desc: t("tool.desc.unity_execute"),      defaultMode: "ask"  as const },
    { name: "unity_run_states",   label: "unity_run_states",   desc: t("tool.desc.unity_run_states"),   defaultMode: "ask"  as const },
    { name: "unity_recompile",    label: "unity_recompile",    desc: t("tool.desc.unity_recompile"),    defaultMode: "auto" as const },
    { name: "unity_hot_reload",   label: "unity_hot_reload",   desc: t("tool.desc.unity_hot_reload"),   defaultMode: "auto" as const },
    { name: "unity_ref_search",   label: "unity_ref_search",   desc: t("tool.desc.unity_ref_search"),   defaultMode: "auto" as const },
    { name: "code_find_references", label: "code_find_references", desc: t("tool.desc.code_find_references"), defaultMode: "auto" as const },
    { name: "code_goto_definition", label: "code_goto_definition", desc: t("tool.desc.code_goto_definition"), defaultMode: "auto" as const },
    { name: "code_symbol_search",   label: "code_symbol_search",   desc: t("tool.desc.code_symbol_search"),   defaultMode: "auto" as const },
    { name: "code_diagnostics",     label: "code_diagnostics",     desc: t("tool.desc.code_diagnostics"),     defaultMode: "auto" as const },
    { name: "code_hover",           label: "code_hover",           desc: t("tool.desc.code_hover"),           defaultMode: "auto" as const },
    { name: "unity_code_usages",    label: "unity_code_usages",    desc: t("tool.desc.unity_code_usages"),    defaultMode: "auto" as const },
    { name: "unity_asset_search", label: "unity_asset_search", desc: t("tool.desc.unity_asset_search"), defaultMode: "auto" as const },
    { name: "unity_yaml_list",    label: "unity_yaml_list",    desc: t("tool.desc.unity_yaml_list"),    defaultMode: "auto" as const },
    { name: "unity_yaml_search",  label: "unity_yaml_search",  desc: t("tool.desc.unity_yaml_search"),  defaultMode: "auto" as const },
    { name: "unity_yaml_read",    label: "unity_yaml_read",    desc: t("tool.desc.unity_yaml_read"),    defaultMode: "auto" as const },
    { name: "knowledge_list",     label: "knowledge_list",     desc: t("tool.desc.knowledge_list"),     defaultMode: "auto" as const },
    { name: "knowledge_query",    label: "knowledge_query",    desc: t("tool.desc.knowledge_query"),    defaultMode: "auto" as const },
    { name: "knowledge_read",     label: "knowledge_read",     desc: t("tool.desc.knowledge_read"),     defaultMode: "auto" as const },
    { name: "knowledge_create",   label: "knowledge_create",   desc: t("tool.desc.knowledge_create"),   defaultMode: "auto" as const },
    { name: "knowledge_delete",   label: "knowledge_delete",   desc: t("tool.desc.knowledge_delete"),   defaultMode: "auto" as const },
    { name: "knowledge_move",     label: "knowledge_move",     desc: t("tool.desc.knowledge_move"),     defaultMode: "auto" as const },
    { name: "knowledge_edit",     label: "knowledge_edit",     desc: t("tool.desc.knowledge_edit"),     defaultMode: "auto" as const },
    { name: "skill_create",       label: "skill_create",       desc: t("tool.desc.skill_create"),       defaultMode: "auto" as const },
    { name: "skill_reload",       label: "skill_reload",       desc: t("tool.desc.skill_reload"),       defaultMode: "auto" as const },
    { name: "skill_list",         label: "skill_list",         desc: t("tool.desc.skill_list"),         defaultMode: "auto" as const },
  ]);

  const approvalBehaviorList = computed(() => [
    {
      name: "behavior.unity_editor_status_change",
      label: t("settings.perms.behavior.unityEditorStatusChange"),
      desc: t("settings.perms.behavior.unityEditorStatusChangeDesc"),
      defaultMode: "auto" as const,
    },
    {
      name: "behavior.knowledge_governance",
      label: t("settings.perms.behavior.knowledgeGovernance"),
      desc: t("settings.perms.behavior.knowledgeGovernanceDesc"),
      defaultMode: "auto" as const,
    },
  ]);

  const permissionList = computed(() => [
    ...toolList.value,
    ...approvalBehaviorList.value,
  ]);

  const toolPermissions = ref<Record<string, "auto" | "ask">>({});

  function getToolMode(name: string): "auto" | "ask" {
    return toolPermissions.value[name] ?? (permissionList.value.find(item => item.name === name)?.defaultMode ?? "ask");
  }

  async function loadToolPermissions() {
    try {
      const perms = await getToolPermissions();
      const normalized: Record<string, "auto" | "ask"> = {};
      for (const [k, v] of Object.entries(perms)) {
        normalized[k] = v === "ask" ? "ask" : "auto";
      }
      toolPermissions.value = normalized;
    } catch { /* use defaults */ }
  }

  async function loadFileToolWorkspaceBoundary() {
    try {
      fileToolWorkspaceBoundary.value = await getFileToolWorkspaceBoundary();
    } catch {
      fileToolWorkspaceBoundary.value = false;
    } finally {
      fileToolWorkspaceBoundaryReady.value = true;
    }
  }

  async function setFileToolWorkspaceBoundaryEnabled(value: boolean) {
    if (!fileToolWorkspaceBoundaryReady.value || fileToolWorkspaceBoundaryBusy.value) return;
    const previous = fileToolWorkspaceBoundary.value;
    if (previous === value) return;
    fileToolWorkspaceBoundary.value = value;
    fileToolWorkspaceBoundaryBusy.value = true;
    try {
      await setFileToolWorkspaceBoundary(value);
      permSaveMsg.value = t("settings.perms.saved");
      if (permSaveTimer) clearTimeout(permSaveTimer);
      permSaveTimer = setTimeout(() => {
        permSaveMsg.value = "";
        permSaveTimer = null;
      }, 2000);
    } catch (e) {
      fileToolWorkspaceBoundary.value = previous;
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.perms.fileBoundarySaveFailed", err.message), {
        code: err.code,
        operation: "setFileToolWorkspaceBoundary",
      });
    } finally {
      fileToolWorkspaceBoundaryBusy.value = false;
    }
  }

  async function setToolPermission(name: string, mode: "auto" | "ask") {
    const previousPermissions = toolPermissions.value;
    const previousMode = getToolMode(name);
    if (previousMode === mode) return;
    toolPermissions.value = { ...toolPermissions.value, [name]: mode };
    try {
      await saveToolPermissions();
    } catch {
      toolPermissions.value = previousPermissions;
    }
  }

  async function toggleToolPermission(name: string) {
    const current = getToolMode(name);
    await setToolPermission(name, current === "auto" ? "ask" : "auto");
  }

  async function saveToolPermissions() {
    try {
      const fullMap: Record<string, string> = {};
      for (const item of permissionList.value) {
        fullMap[item.name] = getToolMode(item.name);
      }
      await serviceSaveToolPermissions(fullMap);
      setWarmup("settings:toolPermissions", fullMap);
      permSaveMsg.value = t("settings.perms.saved");
      if (permSaveTimer) clearTimeout(permSaveTimer);
      permSaveTimer = setTimeout(() => {
        permSaveMsg.value = "";
        permSaveTimer = null;
      }, 2000);
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.perms.saveFailed", err.message), {
        code: err.code,
        operation: "saveToolPermissions",
      });
      throw e;
    }
  }

  // ── Custom endpoints ─────────────────────────────────────────────────
  const customEndpoints = ref<CustomEndpoint[]>([]);
  const editingEndpoint = ref<CustomEndpoint | null>(null);
  const isAddingEndpoint = ref(false);
  const customEndpointSaving = ref(false);
  const testStatus = ref<"idle" | "testing" | "success" | "error">("idle");
  const testResult = ref("");
  const defaultReasoningEfforts: EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];
  const legacyDefaultReasoningEfforts: EffortLevel[] = ["low", "medium", "high", "max"];
  const reasoningEffortSet = new Set<EffortLevel>(["none", "low", "medium", "high", "xhigh", "max"]);
  let customEndpointMutationQueue: Promise<void> = Promise.resolve();
  let pendingCustomEndpointMutations = 0;

  function defaultReasoningParamFormat(apiFormat: ApiFormat): ReasoningParamFormat {
    switch (apiFormat) {
      case "openai_responses": return "openai_responses_reasoning_effort";
      case "anthropic_messages": return "anthropic_thinking";
      default: return "openai_chat_reasoning_effort";
    }
  }

  function normalizeReasoningEfforts(values?: EffortLevel[] | null): EffortLevel[] {
    const normalized = Array.isArray(values)
      ? values.filter((value): value is EffortLevel => reasoningEffortSet.has(value))
      : [];
    if (isSameEffortList(normalized, legacyDefaultReasoningEfforts)) {
      return [...defaultReasoningEfforts];
    }
    return normalized.length > 0 ? normalized : [...defaultReasoningEfforts];
  }

  function isSameEffortList(a: EffortLevel[], b: EffortLevel[]): boolean {
    return a.length === b.length && a.every((value, index) => value === b[index]);
  }

  function defaultReplayReasoningContent(ep: CustomEndpoint): boolean {
    return ep.apiFormat === "openai_chat";
  }

  function normalizeServerTools(value?: Partial<CustomEndpoint["serverTools"]> | null): CustomEndpoint["serverTools"] {
    return {
      webSearch: value?.webSearch === true,
    };
  }

  function normalizeCustomEndpoint(ep: CustomEndpoint): CustomEndpoint {
    return {
      ...ep,
      contextLength: Number.isFinite(ep.contextLength) && ep.contextLength > 0
        ? ep.contextLength
        : DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH,
      betaFlags: [...(ep.betaFlags ?? [])],
      supportedReasoningEfforts: normalizeReasoningEfforts(ep.supportedReasoningEfforts),
      reasoningParamFormat: ep.reasoningParamFormat ?? defaultReasoningParamFormat(ep.apiFormat),
      replayReasoningContent: typeof ep.replayReasoningContent === "boolean"
        ? ep.replayReasoningContent
        : defaultReplayReasoningContent(ep),
      serverTools: normalizeServerTools(ep.serverTools),
      supportsToolLazyLoading: false,
      supportsVision: ep.supportsVision !== false,
    };
  }

  function applyLoadedCustomEndpoints(endpoints: CustomEndpoint[]): CustomEndpoint[] {
    const normalized = endpoints.map(normalizeCustomEndpoint);
    customEndpoints.value = normalized;
    setWarmup("settings:customEndpoints", normalized);
    return normalized;
  }

  async function reloadCustomEndpointsAfterMutation() {
    const latest = applyLoadedCustomEndpoints(await getCustomEndpoints());
    emit("customEndpointsChanged", latest);
    return latest;
  }

  function enqueueCustomEndpointMutation(task: () => Promise<void>): Promise<void> {
    pendingCustomEndpointMutations += 1;
    customEndpointSaving.value = true;
    const run = customEndpointMutationQueue
      .catch(() => undefined)
      .then(task)
      .finally(() => {
        pendingCustomEndpointMutations = Math.max(0, pendingCustomEndpointMutations - 1);
        if (pendingCustomEndpointMutations === 0) {
          customEndpointSaving.value = false;
        }
      });
    customEndpointMutationQueue = run;
    return run;
  }

  function newEmptyEndpoint(): CustomEndpoint {
    const apiFormat: ApiFormat = "openai_chat";
    return {
      id: crypto.randomUUID(),
      name: "",
      apiModel: "",
      endpoint: "",
      apiFormat,
      apiKey: "",
      contextLength: DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH,
      betaFlags: [],
      supportedReasoningEfforts: [...defaultReasoningEfforts],
      reasoningParamFormat: defaultReasoningParamFormat(apiFormat),
      replayReasoningContent: true,
      serverTools: normalizeServerTools(),
      supportsToolLazyLoading: false,
      supportsVision: true,
    };
  }

  async function loadCustomEndpoints() {
    try {
      applyLoadedCustomEndpoints(await getCustomEndpoints());
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.custom.loadFailed", err.message), {
        code: err.code,
        operation: "loadCustomEndpoints",
      });
    }
  }

  function startAddEndpoint() {
    editingEndpoint.value = newEmptyEndpoint();
    isAddingEndpoint.value = true;
    testStatus.value = "idle";
    testResult.value = "";
  }

  function startEditEndpoint(ep: CustomEndpoint) {
    editingEndpoint.value = normalizeCustomEndpoint(ep);
    isAddingEndpoint.value = false;
    testStatus.value = "idle";
    testResult.value = "";
  }

  function cancelEditEndpoint() {
    editingEndpoint.value = null;
    isAddingEndpoint.value = false;
  }

  async function saveEndpoint() {
    if (!editingEndpoint.value) return;
    const ep = normalizeCustomEndpoint(editingEndpoint.value);
    if (!ep.name.trim()) { errorMsg.value = t("settings.custom.nameRequired"); return; }
    if (!ep.apiModel.trim()) { errorMsg.value = t("settings.custom.apiModelRequired"); return; }
    if (!ep.endpoint.trim()) { errorMsg.value = t("settings.custom.endpointRequired"); return; }
    errorMsg.value = "";

    await enqueueCustomEndpointMutation(async () => {
      const list = [...customEndpoints.value];
      const idx = list.findIndex(e => e.id === ep.id);
      if (idx >= 0) {
        list[idx] = ep;
      } else {
        list.push(ep);
      }

      try {
        await saveCustomEndpoints(list);
        await reloadCustomEndpointsAfterMutation();
        editingEndpoint.value = null;
        isAddingEndpoint.value = false;
        successMsg.value = t("settings.custom.saved");
        setTimeout(() => { successMsg.value = ""; }, 2000);
      } catch (e) {
        const err = normalizeAppError(e);
        useNotificationStore().addNotice("error", t("settings.custom.saveFailed", err.message), {
          code: err.code,
          operation: "saveEndpoint",
        });
      }
    });
  }

  async function deleteEndpoint(id: string) {
    await enqueueCustomEndpointMutation(async () => {
      const list = customEndpoints.value.filter(e => e.id !== id);
      try {
        await saveCustomEndpoints(list);
        await reloadCustomEndpointsAfterMutation();
        if (editingEndpoint.value?.id === id) {
          editingEndpoint.value = null;
          isAddingEndpoint.value = false;
        }
        successMsg.value = t("settings.custom.deleted");
        setTimeout(() => { successMsg.value = ""; }, 2000);
      } catch (e) {
        const err = normalizeAppError(e);
        useNotificationStore().addNotice("error", t("settings.custom.saveFailed", err.message), {
          code: err.code,
          operation: "deleteEndpoint",
        });
      }
    });
  }

  async function testEndpoint() {
    if (!editingEndpoint.value) return;
    const ep = normalizeCustomEndpoint(editingEndpoint.value);
    if (!ep.apiModel.trim() || !ep.endpoint.trim()) {
      testStatus.value = "error";
      testResult.value = t("settings.custom.testMissingFields");
      return;
    }
    testStatus.value = "testing";
    testResult.value = "";
    try {
      const reply = await testCustomEndpoint(ep);
      testStatus.value = customEndpointTestStatusForReply(reply);
      testResult.value = reply;
    } catch (e) {
      testStatus.value = "error";
      testResult.value = normalizeCustomEndpointTestErrorMessage(e);
    }
  }

  function handleEndpointKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") cancelEditEndpoint();
  }

  // ── Init ─────────────────────────────────────────────────────────────
  onMounted(async () => {
    // Use background warmup cache if available
    const cachedProviders = getWarmup<ProviderStatus[]>("settings:providers");
    const cachedCodex = getWarmup<RemoteCodexStatus>("settings:codexStatus");
    const cachedDefaults = getWarmup<ModelDefaults>("settings:modelDefaults");
    const cachedPerms = getWarmup<Record<string, string>>("settings:toolPermissions");
    const cachedEndpoints = getWarmup<CustomEndpoint[]>("settings:customEndpoints");

    if (cachedProviders) providers.value = cachedProviders;
    else await loadProviders();
    await loadDynamicToolLoadingMode();

    if (cachedCodex) codexStatus.value = normalizeCodexStatus(cachedCodex);
    else await loadCodexStatus();
    if (codexStatus.value.authenticated && !codexStatus.value.validationFailed) {
      void loadCodexRateLimits();
    }
    await loadCodexModelConfig();

    if (cachedDefaults) modelDefaults.value = cachedDefaults;
    else await loadModelDefaults();

    if (cachedPerms) {
      const normalized: Record<string, "auto" | "ask"> = {};
      for (const [k, v] of Object.entries(cachedPerms)) {
        normalized[k] = v === "ask" ? "ask" : "auto";
      }
      toolPermissions.value = normalized;
    } else {
      await loadToolPermissions();
    }
    await loadFileToolWorkspaceBoundary();

    if (cachedEndpoints) customEndpoints.value = cachedEndpoints.map(normalizeCustomEndpoint);
    else await loadCustomEndpoints();
  });

  onUnmounted(() => {
    stopCodexPolling();
  });

  // ── Public API ───────────────────────────────────────────────────────
  return {
    // general
    resetConfirm,
    handleResetOnboarding,
    activeCategory,

    // providers
    providers,
    editingProvider,
    editKey,
    errorMsg,
    successMsg,
    isLoading,
    loadProviders,
    claudeCodeTestStatus,
    claudeCodeTestResult,
    testClaudeCode,
    startEdit,
    cancelEdit,
    saveKey,
    deleteKey,
    handleKeydown,

    // dynamic tool loading
    dynamicToolLoadingMode,
    dynamicToolLoadingBusy,
    loadDynamicToolLoadingMode,
    setDynamicToolLoadingMode,

    // oauth
    oauthStep,
    oauthCode,
    startOAuthLogin,
    submitOAuthCode,
    cancelOAuth,
    oauthLogout,
    handleOAuthKeydown,

    // codex
    codexStep,
    codexStatus,
    codexQuota,
    codexRetrying,
    codexModelConfig,
    codexUserCode,
    codexUrl,
    codexCodeCopied,
    codexDeviceAuthId,
    codexInterval,
    loadCodexStatus,
    loadCodexRateLimits,
    loadCodexModelConfig,
    startCodexLogin,
    pollCodex,
    cancelCodexLogin,
    codexLogout,
    retryCodexValidation,
    copyCode,
    setCodexTransportMode,

    // subscription disclaimer
    showDisclaimer,
    disclaimerTarget,
    requestOAuthLogin,
    requestCodexLogin,
    confirmDisclaimer,
    cancelDisclaimer,

    // model defaults
    modelDefaults,
    modelSaveMsg,
    loadModelDefaults,
    saveModelDefaults,

    // tool permissions
    permSaveMsg,
    toolList,
    approvalBehaviorList,
    toolPermissions,
    fileToolWorkspaceBoundary,
    fileToolWorkspaceBoundaryReady,
    fileToolWorkspaceBoundaryBusy,
    loadToolPermissions,
    loadFileToolWorkspaceBoundary,
    setToolPermission,
    setFileToolWorkspaceBoundaryEnabled,
    toggleToolPermission,
    saveToolPermissions,
    getToolMode,

    // custom endpoints
    customEndpoints,
    editingEndpoint,
    isAddingEndpoint,
    customEndpointSaving,
    testStatus,
    testResult,
    newEmptyEndpoint,
    loadCustomEndpoints,
    startAddEndpoint,
    startEditEndpoint,
    cancelEditEndpoint,
    saveEndpoint,
    deleteEndpoint,
    testEndpoint,
    handleEndpointKeydown,
  };
}
