
<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ApiFormat, CodexModelConfig, CustomEndpoint, GitProbeResult, ModelDefaults, PluginStatus, AssetDbScanEvent, ScanStats } from "../types";
import { t, setLocale, locale, type Locale } from "../i18n";
import { useTheme, type ThemePreference } from "../composables/useTheme";
import { useSettingsState } from "../composables/useSettingsState";
import { normalizeAppError } from "../services/errors";
import {
  customEndpointTestDetail as formatCustomEndpointTestDetail,
  customEndpointTestHtmlPath as extractCustomEndpointTestHtmlPath,
} from "../services/customEndpointTestResult";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";
import { useUiStore } from "../stores/ui";
import { setWorkingDir, getWorkingDir } from "../services/project";
import { checkUnityPlugin, installUnityPlugin } from "../services/unity";
import { gitCheckUserConfig, gitInitUnity, gitProbe, gitSetUserConfig } from "../services/git";
import { assetDbScan } from "../services/asset";
import BaseDropdown from "./ui/BaseDropdown.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import GitMissingHelp from "./git/GitMissingHelp.vue";
import {
  canInitOnboardingGit,
  resolveOnboardingGitInitTargetPath,
  resolveOnboardingVcsStepState,
} from "./onboarding/onboardingVcs";

const emit = defineEmits<{ completed: [] }>();
const uiStore = useUiStore();
const authStore = useAuthStore();
const modelStore = useModelStore();

const step = ref(0); // 0=welcome, 1=auth, 2=project, 3=plugin, 4=vcs, 5=scan
const TOTAL_STEPS = 6;
const direction = ref(1);
const titlebarStepText = computed(() => t("onboarding.step", step.value + 1, TOTAL_STEPS));

function goNext() {
  direction.value = 1;
  step.value++;
}
function goBack() {
  direction.value = -1;
  step.value--;
}
function skipAll() {
  emit("completed");
}

function selectLanguage(lang: Locale) {
  setLocale(lang);
}

const { preference: themePreference, setThemePreference } = useTheme();
const languageOptions = [
  { value: "zh", label: "中文" },
  { value: "en", label: "English" },
] as const;
const themeOptionDefs: { value: ThemePreference; labelKey: string }[] = [
  { value: "dark",   labelKey: "settings.display.themeDark" },
  { value: "light",  labelKey: "settings.display.themeLight" },
  { value: "system", labelKey: "settings.display.themeSystem" },
];
const themeOptions = computed(() =>
  themeOptionDefs.map((opt) => ({
    value: opt.value,
    label: t(opt.labelKey),
  })),
);

function handleLanguageChange(value: string) {
  selectLanguage(value as Locale);
}

function handleThemeChange(value: string) {
  setThemePreference(value as ThemePreference);
}

function emitSettingsState(event: "authChanged"): void;
function emitSettingsState(event: "modelDefaultsChanged", defaults: ModelDefaults): void;
function emitSettingsState(event: "codexTransportChanged", config: CodexModelConfig): void;
function emitSettingsState(event: "customEndpointsChanged", endpoints: CustomEndpoint[]): void;
function emitSettingsState(event: "resetOnboarding"): void;
function emitSettingsState(
  event: "authChanged" | "modelDefaultsChanged" | "codexTransportChanged" | "customEndpointsChanged" | "resetOnboarding",
  payload?: ModelDefaults | CodexModelConfig | CustomEndpoint[],
) {
  if (event === "authChanged") {
    void handleSettingsAuthChanged();
  } else if (event === "modelDefaultsChanged") {
    modelStore.applyModelDefaults(payload as ModelDefaults);
  } else if (event === "codexTransportChanged") {
    modelStore.applyCodexModelConfig(payload as CodexModelConfig);
  } else if (event === "customEndpointsChanged") {
    modelStore.applyCustomEndpoints(payload as CustomEndpoint[]);
  }
}

const {
  errorMsg: settingsErrorMsg,
  successMsg: settingsSuccessMsg,
  codexStep: settingsCodexStep,
  codexStatus: settingsCodexStatus,
  codexUserCode: settingsCodexUserCode,
  codexUrl: settingsCodexUrl,
  codexCodeCopied: settingsCodexCodeCopied,
  cancelCodexLogin: settingsCancelCodexLogin,
  codexLogout: settingsCodexLogout,
  copyCode: settingsCopyCode,
  requestCodexLogin: settingsRequestCodexLogin,
  customEndpoints: settingsCustomEndpoints,
  editingEndpoint: settingsEditingEndpoint,
  testStatus: settingsEndpointTestStatus,
  testResult: settingsEndpointTestResult,
  startAddEndpoint: settingsStartAddEndpoint,
  startEditEndpoint: settingsStartEditEndpoint,
  cancelEditEndpoint: settingsCancelEditEndpoint,
  saveEndpoint: settingsSaveEndpoint,
  deleteEndpoint: settingsDeleteEndpoint,
  testEndpoint: settingsTestEndpoint,
} = useSettingsState(emitSettingsState);

const authExpanded = ref<"custom" | "codex" | null>("custom");
const customEndpointConfigured = computed(() => settingsCustomEndpoints.value.length > 0);
const codexConfigured = computed(() =>
  settingsCodexStatus.value.authenticated || settingsCodexStep.value === "success",
);
const customEndpointReady = computed(() => {
  const ep = settingsEditingEndpoint.value;
  return !!ep?.name.trim() && !!ep.apiModel.trim() && !!ep.endpoint.trim();
});
const customEndpointTestReady = computed(() => {
  const ep = settingsEditingEndpoint.value;
  return !!ep && !!ep.apiModel.trim() && !!ep.endpoint.trim();
});
const customEndpointTestDetail = computed(() =>
  formatCustomEndpointTestDetail(settingsEndpointTestResult.value),
);
const customEndpointTestHtmlPath = computed(() =>
  extractCustomEndpointTestHtmlPath(settingsEndpointTestResult.value),
);
const customApiFormatOptions = computed(() => [
  { value: "openai_chat" as ApiFormat, label: t("settings.custom.formatOpenaiChat") },
  { value: "openai_responses" as ApiFormat, label: t("settings.custom.formatOpenaiResponses") },
  { value: "anthropic_messages" as ApiFormat, label: t("settings.custom.formatAnthropicMessages") },
]);

async function handleSettingsAuthChanged() {
  await authStore.loadProviderStatus();
  await modelStore.loadCodexAvailableModels();
  modelStore.resolveSelectedModel(true);
}

function defaultReasoningParamFormat(apiFormat: ApiFormat): CustomEndpoint["reasoningParamFormat"] {
  switch (apiFormat) {
    case "openai_responses": return "openai_responses_reasoning_effort";
    case "anthropic_messages": return "anthropic_thinking";
    default: return "openai_chat_reasoning_effort";
  }
}

function toggleAuthProvider(provider: "custom" | "codex") {
  authExpanded.value = authExpanded.value === provider ? null : provider;
  if (provider === "custom" && authExpanded.value === "custom" && settingsCustomEndpoints.value.length === 0 && !settingsEditingEndpoint.value) {
    settingsStartAddEndpoint();
  }
}

function updateInlineEndpointApiFormat(value: string) {
  if (!settingsEditingEndpoint.value) return;
  const apiFormat = value as ApiFormat;
  settingsEditingEndpoint.value.apiFormat = apiFormat;
  settingsEditingEndpoint.value.reasoningParamFormat = defaultReasoningParamFormat(apiFormat);
}

function handleInlineEndpointKeydown(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    void settingsSaveEndpoint();
  } else if (e.key === "Escape") {
    settingsCancelEditEndpoint();
  }
}

async function openCustomEndpointTestHtml() {
  if (!customEndpointTestHtmlPath.value) return;
  try {
    await openPath(customEndpointTestHtmlPath.value);
  } catch (e) {
    const err = normalizeAppError(e);
    settingsEndpointTestResult.value = t(
      "settings.custom.openTestHtmlFailed",
      customEndpointTestHtmlPath.value,
      err.message,
    );
  }
}

const projectPath = ref("");
const projectError = ref("");
const projectValid = ref(false);
const projectOpening = ref(false);

async function waitForProjectOpeningPaint() {
  await nextTick();
  await new Promise<void>((resolve) => {
    if (typeof requestAnimationFrame !== "function") {
      globalThis.setTimeout(resolve, 0);
      return;
    }
    requestAnimationFrame(() => globalThis.setTimeout(resolve, 0));
  });
}

async function browseProject() {
  if (projectOpening.value) return;
  try {
    const selected = await open({ directory: true, multiple: false });
    if (selected && typeof selected === "string") {
      projectError.value = "";
      projectPath.value = selected;
      projectValid.value = false;
      projectOpening.value = true;
      await waitForProjectOpeningPaint();
      try {
        projectPath.value = await setWorkingDir(selected);
        projectValid.value = true;
      } catch (e) {
        projectError.value = normalizeAppError(e).message;
        projectValid.value = false;
      } finally {
        projectOpening.value = false;
      }
    }
  } catch { /* cancelled */ }
}

const pluginStatus = ref<PluginStatus | null>(null);
const pluginInstalling = ref(false);
const pluginError = ref("");
let unlistenPlugin: UnlistenFn | null = null;

async function checkPlugin() {
  pluginError.value = "";
  try {
    pluginStatus.value = await checkUnityPlugin();
  } catch {
    pluginStatus.value = null;
  }
}

async function installPlugin() {
  pluginInstalling.value = true;
  pluginError.value = "";
  try {
    await installUnityPlugin();
    pluginStatus.value = { status: "upToDate" };
  } catch (e) {
    pluginError.value = normalizeAppError(e).message;
  } finally {
    pluginInstalling.value = false;
  }
}

// ── Step 4: VCS ──
const gitProbeState = ref<GitProbeResult | null>(null);
const gitInitLoading = ref(false);
const gitError = ref("");

const gitConfigName = ref("");
const gitConfigEmail = ref("");
const gitConfigLoadedName = ref("");
const gitConfigLoadedEmail = ref("");
const gitConfigSaving = ref(false);
const gitConfigError = ref("");
const gitUserMissing = ref(false);

const gitInitAvailable = computed(() => !!gitProbeState.value?.available);
const gitInitTargetPath = computed(() => resolveOnboardingGitInitTargetPath(projectPath.value, projectValid.value));
const gitVcsStepState = computed(() => resolveOnboardingVcsStepState(gitInitTargetPath.value, gitProbeState.value));
const gitCanInit = computed(() => canInitOnboardingGit(gitInitTargetPath.value, gitProbeState.value));
const gitConfigComplete = computed(() => !!gitConfigName.value.trim() && !!gitConfigEmail.value.trim());
const gitConfigDirty = computed(() => (
  gitConfigName.value.trim() !== gitConfigLoadedName.value
  || gitConfigEmail.value.trim() !== gitConfigLoadedEmail.value
));
const gitHelpText = computed(() => {
  const probe = gitProbeState.value;
  if (!probe) return "";
  if (!probe.available) {
    return probe.envOverride
      ? t("git.detect.invalidOverride", probe.envOverride)
      : t("git.detect.missing");
  }
  if (!probe.inPath && probe.path) {
    return t("git.detect.foundOutsidePath", probe.path);
  }
  return "";
});

function syncGitConfigForm(cfg: { name: string; email: string }) {
  const name = cfg.name.trim();
  const email = cfg.email.trim();
  gitConfigName.value = name;
  gitConfigEmail.value = email;
  gitConfigLoadedName.value = name;
  gitConfigLoadedEmail.value = email;
  gitUserMissing.value = !name || !email;
}

function resetGitConfigForm() {
  gitConfigName.value = "";
  gitConfigEmail.value = "";
  gitConfigLoadedName.value = "";
  gitConfigLoadedEmail.value = "";
  gitConfigError.value = "";
  gitUserMissing.value = false;
}

async function checkGit() {
  gitError.value = "";
  gitConfigError.value = "";
  try {
    gitProbeState.value = await gitProbe();
    if (gitProbeState.value?.available) {
      try {
        syncGitConfigForm(await gitCheckUserConfig());
      } catch { /* ignore — non-fatal */ }
    } else {
      resetGitConfigForm();
    }
  } catch (e) {
    resetGitConfigForm();
    gitProbeState.value = {
      available: false,
      inPath: false,
      isRepo: false,
    };
    gitError.value = normalizeAppError(e).message;
  }
}

async function persistGitConfig(): Promise<boolean> {
  const name = gitConfigName.value.trim();
  const email = gitConfigEmail.value.trim();
  if (!name || !email) {
    gitUserMissing.value = true;
    gitConfigError.value = t("git.config.required");
    return false;
  }

  gitConfigSaving.value = true;
  gitConfigError.value = "";
  try {
    await gitSetUserConfig(name, email);
    syncGitConfigForm({ name, email });
    return true;
  } catch (e) {
    gitConfigError.value = normalizeAppError(e).message;
    return false;
  } finally {
    gitConfigSaving.value = false;
  }
}

async function saveGitConfig() {
  gitError.value = "";
  await persistGitConfig();
}

async function checkGitConfigAndInit() {
  gitError.value = "";
  if (!gitInitTargetPath.value) {
    gitError.value = t("onboarding.vcs.needProject");
    return;
  }
  if (!gitInitAvailable.value) {
    gitError.value = gitHelpText.value || t("git.detect.missing");
    return;
  }
  try {
    if (!gitConfigComplete.value) {
      gitUserMissing.value = true;
      gitConfigError.value = t("git.config.required");
      return;
    }
    if (gitUserMissing.value || gitConfigDirty.value) {
      const saved = await persistGitConfig();
      if (!saved) return;
    }
    await doInitGit();
  } catch (e) {
    gitError.value = normalizeAppError(e).message;
  }
}

async function doInitGit() {
  gitInitLoading.value = true;
  gitError.value = "";
  try {
    await gitInitUnity();
    await checkGit();
  } catch (e) {
    gitError.value = normalizeAppError(e).message;
  } finally {
    gitInitLoading.value = false;
  }
}

const scanPhase = ref<AssetDbScanEvent | null>(null);
const scanDone = ref(false);
const scanStats = ref<ScanStats | null>(null);
let unlistenScan: UnlistenFn | null = null;

async function startScan() {
  scanPhase.value = { phase: "dirScan" };
  scanDone.value = false;
  try {
    unlistenScan = await listen<AssetDbScanEvent>("ref-graph-scan", (e) => {
      scanPhase.value = e.payload;
      if (e.payload.phase === "done") {
        scanStats.value = e.payload.stats;
        scanDone.value = true;
      }
    });
    await assetDbScan();
  } catch (e) {
    scanPhase.value = { phase: "error", error: normalizeAppError(e) };
  }
}

watch(step, async (s) => {
  if (s === 1 && authExpanded.value === "custom" && settingsCustomEndpoints.value.length === 0 && !settingsEditingEndpoint.value) {
    settingsStartAddEndpoint();
  }
  if (s === 3) {
    await checkPlugin();
    if (!unlistenPlugin) {
      unlistenPlugin = await listen<PluginStatus>("unity-plugin-status", (e) => {
        pluginStatus.value = e.payload;
      });
    }
  }
  if (s === 4) await checkGit();
});

watch(
  () => [step.value, authExpanded.value, settingsCustomEndpoints.value.length] as const,
  ([s, expanded, endpointCount]) => {
    if (s === 1 && expanded === "custom" && endpointCount === 0 && !settingsEditingEndpoint.value) {
      settingsStartAddEndpoint();
    }
  },
  { immediate: true },
);

function scanProgressText(): string {
  if (!scanPhase.value) return "";
  switch (scanPhase.value.phase) {
    case "dirScan": return t("chat.assetDb.scanning.dirScan");
    case "metaParse": return t("chat.assetDb.scanning.metaParse", scanPhase.value.completed, scanPhase.value.total);
    case "yamlParse": return t("chat.assetDb.scanning.yamlParse", scanPhase.value.completed, scanPhase.value.total);
    case "dbWrite": return t("chat.assetDb.scanning.dbWrite");
    case "done": return t("onboarding.scan.complete", scanPhase.value.stats.nodesAdded, scanPhase.value.stats.edgesAdded);
    case "error": return t("chat.assetDb.scanning.error", scanPhase.value.error.message);
    default: return "";
  }
}

onMounted(async () => {
  try {
    const dir = await getWorkingDir();
    if (dir && dir.trim() !== "") {
      projectPath.value = dir;
      projectValid.value = true;
    }
  } catch { /* ignore */ }
});

onUnmounted(() => {
  unlistenScan?.();
  unlistenPlugin?.();
});
</script>

<template>
  <div class="onboarding-container">
    <div class="onboarding-titlebar">
      <div class="onboarding-titlebar-left">
        <div class="onboarding-titlebar-logo" aria-hidden="true">
          <svg viewBox="0 0 32 32" width="18" height="18" fill="none">
            <rect width="32" height="32" rx="8" fill="var(--accent-color)"/>
            <text x="16" y="22" text-anchor="middle" font-size="16" font-weight="bold" fill="var(--bg-color)">L</text>
          </svg>
        </div>
        <span class="onboarding-titlebar-brand">Locus</span>
        <span class="onboarding-titlebar-caption">{{ titlebarStepText }}</span>
      </div>

      <div class="onboarding-titlebar-actions">
        <button v-if="step > 0" class="skip-all-btn" @click.stop="skipAll">{{ t("onboarding.skip") }}</button>
        <div class="window-controls">
          <button class="win-ctrl-btn" @click="uiStore.winMinimize" :title="t('app.win.minimize')">
            <svg viewBox="0 0 12 12" width="12" height="12"><rect x="1" y="5.5" width="10" height="1" fill="currentColor"/></svg>
          </button>
          <button class="win-ctrl-btn" @click="uiStore.winToggleMaximize" :title="t('app.win.maximize')">
            <svg v-if="!uiStore.isMaximized" viewBox="0 0 12 12" width="12" height="12"><rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
            <svg v-else viewBox="0 0 12 12" width="12" height="12"><rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1"/><rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1"/></svg>
          </button>
          <button class="win-ctrl-btn win-close" @click="uiStore.winClose" :title="t('app.win.close')">
            <svg viewBox="0 0 12 12" width="12" height="12"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
          </button>
        </div>
      </div>
    </div>

    <div class="onboarding-body">
      <div class="step-indicator" v-if="step > 0">
        <div
          v-for="i in TOTAL_STEPS"
          :key="i"
          class="step-dot"
          :class="{ active: i - 1 === step, done: i - 1 < step }"
        />
      </div>

    <div v-if="step === 0" class="step-card welcome-card">
      <div class="welcome-logo">
        <svg viewBox="0 0 32 32" width="48" height="48" fill="none">
          <rect width="32" height="32" rx="8" fill="var(--accent-color)"/>
          <text x="16" y="22" text-anchor="middle" font-size="16" font-weight="bold" fill="var(--bg-color)">L</text>
        </svg>
      </div>
      <h1 class="welcome-title">{{ t("onboarding.welcome.title") }}</h1>
      <p class="welcome-subtitle">{{ t("onboarding.welcome.subtitle") }}</p>

      <div class="welcome-preferences">
        <section class="welcome-section">
          <p class="welcome-section-label">{{ t("onboarding.welcome.selectLang") }}</p>
          <BaseSegmented
            class="welcome-segmented"
            :model-value="locale"
            :options="[...languageOptions]"
            @update:model-value="handleLanguageChange"
          />
        </section>

        <section class="welcome-section">
          <p class="welcome-section-label">{{ t("settings.display.themeTitle") }}</p>
          <BaseSegmented
            class="welcome-segmented"
            :model-value="themePreference"
            :options="themeOptions"
            @update:model-value="handleThemeChange"
          />
        </section>
      </div>

      <div class="step-actions welcome-actions">
        <button class="ob-btn primary welcome-next-btn" @click="goNext">{{ t("onboarding.next") }}</button>
      </div>
    </div>

    <div v-else-if="step === 1" class="step-card">
      <h2 class="step-title">{{ t("onboarding.auth.title") }}</h2>
      <div class="step-desc auth-desc-lines">
        <p>{{ t("onboarding.auth.desc1") }}</p>
        <p>{{ t("onboarding.auth.desc2") }}</p>
        <p>{{ t("onboarding.auth.desc3") }}</p>
      </div>

      <div class="provider-list">
        <div class="provider-card" :class="{ expanded: authExpanded === 'custom' }">
          <div class="provider-header" @click="toggleAuthProvider('custom')">
            <div class="provider-left">
              <span class="provider-name">{{ t("onboarding.auth.customEndpoint") }}</span>
              <span class="provider-desc-text">{{ t("onboarding.auth.customEndpointDesc") }}</span>
            </div>
            <span class="provider-badge" :class="{ configured: customEndpointConfigured }">
              {{ customEndpointConfigured ? t("onboarding.auth.configured") : t("onboarding.auth.notConfigured") }}
            </span>
          </div>
          <div v-if="authExpanded === 'custom'" class="provider-body custom-endpoint-body" @click.stop>
            <div v-if="settingsSuccessMsg" class="msg success">{{ settingsSuccessMsg }}</div>
            <div v-if="settingsErrorMsg" class="msg error">{{ settingsErrorMsg }}</div>

            <div
              v-if="settingsCustomEndpoints.length > 0 && !settingsEditingEndpoint"
              class="custom-endpoints-inline-list"
            >
              <div
                v-for="ep in settingsCustomEndpoints"
                :key="ep.id"
                class="custom-endpoint-summary"
              >
                <div class="custom-endpoint-summary-main">
                  <span class="custom-endpoint-summary-name">{{ ep.name }}</span>
                  <span class="custom-endpoint-summary-meta">{{ ep.apiModel }}</span>
                </div>
                <div class="custom-endpoint-summary-actions">
                  <button class="ob-btn secondary small" type="button" @click="settingsStartEditEndpoint(ep)">
                    {{ t("settings.custom.edit") }}
                  </button>
                  <button class="ob-btn secondary small" type="button" @click="settingsDeleteEndpoint(ep.id)">
                    {{ t("settings.custom.delete") }}
                  </button>
                </div>
              </div>
              <button class="ob-btn secondary" type="button" @click="settingsStartAddEndpoint">
                {{ t("settings.custom.add") }}
              </button>
            </div>

            <div v-if="settingsEditingEndpoint" class="custom-endpoint-fields">
              <label class="custom-endpoint-field">
                <span class="custom-endpoint-label">{{ t("settings.custom.name") }}</span>
                <input
                  v-model="settingsEditingEndpoint.name"
                  class="ob-input"
                  type="text"
                  :placeholder="t('settings.custom.namePlaceholder')"
                  @keydown="handleInlineEndpointKeydown"
                />
              </label>
              <label class="custom-endpoint-field">
                <span class="custom-endpoint-label">{{ t("settings.custom.apiModel") }}</span>
                <input
                  v-model="settingsEditingEndpoint.apiModel"
                  class="ob-input"
                  type="text"
                  :placeholder="t('settings.custom.apiModelPlaceholder')"
                  @keydown="handleInlineEndpointKeydown"
                />
              </label>
              <label class="custom-endpoint-field">
                <span class="custom-endpoint-label">{{ t("settings.custom.endpoint") }}</span>
                <input
                  v-model="settingsEditingEndpoint.endpoint"
                  class="ob-input"
                  type="text"
                  :placeholder="t('settings.custom.endpointPlaceholder')"
                  @keydown="handleInlineEndpointKeydown"
                />
              </label>
              <label class="custom-endpoint-field">
                <span class="custom-endpoint-label">{{ t("settings.custom.apiFormat") }}</span>
                <BaseDropdown
                  class="custom-endpoint-format-dropdown"
                  :model-value="settingsEditingEndpoint.apiFormat"
                  :options="customApiFormatOptions"
                  :aria-label="t('settings.custom.apiFormat')"
                  menu-align="start"
                  size="md"
                  @update:model-value="updateInlineEndpointApiFormat"
                />
              </label>
              <label class="custom-endpoint-field">
                <span class="custom-endpoint-label custom-endpoint-label-with-hint">
                  <span>{{ t("settings.custom.apiKey") }}</span>
                  <span class="custom-endpoint-hint">{{ t("settings.custom.apiKeyOptional") }}</span>
                </span>
                <input
                  v-model="settingsEditingEndpoint.apiKey"
                  class="ob-input"
                  type="password"
                  :placeholder="t('settings.custom.apiKeyPlaceholder')"
                  @keydown="handleInlineEndpointKeydown"
                />
              </label>
            </div>

            <div
              v-if="settingsEndpointTestStatus !== 'idle'"
              class="custom-endpoint-test-result"
              :class="settingsEndpointTestStatus"
            >
              <span v-if="settingsEndpointTestStatus === 'testing'" class="custom-endpoint-spinner"></span>
              <span v-if="settingsEndpointTestStatus === 'testing'">{{ t("settings.custom.testing") }}</span>
              <span v-else-if="settingsEndpointTestStatus === 'success'" class="custom-endpoint-test-heading">
                {{ t("settings.custom.testOk") }}
              </span>
              <span v-else-if="settingsEndpointTestStatus === 'error'" class="custom-endpoint-test-heading">
                {{ t("settings.custom.testFail") }}
              </span>
              <span v-if="customEndpointTestDetail" class="custom-endpoint-test-detail">
                {{ customEndpointTestDetail }}
              </span>
              <button
                v-if="customEndpointTestHtmlPath"
                class="custom-endpoint-test-link"
                type="button"
                @click="openCustomEndpointTestHtml"
              >
                {{ t("settings.custom.openInBrowser") }}
              </button>
            </div>

            <div v-if="settingsEditingEndpoint" class="custom-endpoint-actions">
              <button
                class="ob-btn secondary"
                type="button"
                :disabled="settingsEndpointTestStatus === 'testing' || !customEndpointTestReady"
                @click="settingsTestEndpoint"
              >
                {{ settingsEndpointTestStatus === "testing" ? "..." : t("settings.custom.test") }}
              </button>
              <button
                v-if="customEndpointConfigured"
                class="ob-btn secondary"
                type="button"
                @click="settingsCancelEditEndpoint"
              >
                {{ t("settings.custom.cancel") }}
              </button>
              <button
                class="ob-btn primary"
                type="button"
                :disabled="!customEndpointReady"
                @click="settingsSaveEndpoint"
              >
                {{ t("settings.custom.save") }}
              </button>
            </div>
          </div>
        </div>

        <div class="provider-card" :class="{ expanded: authExpanded === 'codex' }">
          <div class="provider-header" @click="toggleAuthProvider('codex')">
            <div class="provider-left">
              <span class="provider-name">{{ t("onboarding.auth.codex") }}</span>
              <span class="provider-desc-text">{{ t("onboarding.auth.codexDesc") }}</span>
            </div>
            <span class="provider-badge" :class="{ configured: codexConfigured }">
              {{ codexConfigured ? t("onboarding.auth.configured") : t("onboarding.auth.notConfigured") }}
            </span>
          </div>
          <div v-if="authExpanded === 'codex'" class="provider-body" @click.stop>
            <template v-if="settingsCodexStatus.authenticated && settingsCodexStep === 'idle'">
              <div class="status-row ok">{{ t("settings.codex.loggedIn") }}</div>
              <button class="ob-btn secondary" type="button" @click="settingsCodexLogout">
                {{ t("settings.codex.logout") }}
              </button>
            </template>
            <template v-else-if="settingsCodexStep === 'idle'">
              <button class="ob-btn primary" type="button" @click="settingsRequestCodexLogin">
                {{ t("settings.codex.loginBtn") }}
              </button>
              <span class="hint-text">{{ t("settings.codex.hint") }}</span>
            </template>
            <template v-else-if="settingsCodexStep === 'opening'">
              <button class="ob-btn primary" type="button" disabled>{{ t("settings.codex.opening") }}</button>
              <span class="hint-text">{{ t("settings.codex.hint") }}</span>
            </template>
            <template v-else-if="settingsCodexStep === 'waiting'">
              <p class="instruction-text">{{ t("settings.codex.instruction") }}</p>
              <a v-if="settingsCodexUrl" :href="settingsCodexUrl" target="_blank" class="codex-url">
                {{ settingsCodexUrl }}
              </a>
              <button
                class="codex-code-row"
                :class="{ copied: settingsCodexCodeCopied }"
                type="button"
                :title="settingsCodexCodeCopied ? t('common.copied') : t('common.clickToCopy')"
                @click="settingsCopyCode"
              >
                <code class="codex-code">{{ settingsCodexUserCode }}</code>
                <span class="codex-copy-indicator">
                  {{ settingsCodexCodeCopied ? t("common.copied") : t("common.clickToCopy") }}
                </span>
              </button>
              <div class="status-row loading">{{ t("settings.codex.waiting") }}</div>
              <button class="ob-btn secondary" type="button" @click="settingsCancelCodexLogin">
                {{ t("settings.codex.cancel") }}
              </button>
            </template>
            <template v-else-if="settingsCodexStep === 'success'">
              <div class="status-row ok">{{ t("settings.codex.loginSuccess") }}</div>
            </template>
          </div>
        </div>
      </div>

      <p class="skip-hint">{{ t("onboarding.auth.skipHint") }}</p>

      <div class="step-actions">
        <button class="ob-btn secondary" @click="goBack">{{ t("onboarding.back") }}</button>
        <button class="ob-btn primary" @click="goNext">{{ t("onboarding.next") }}</button>
      </div>
    </div>

    <div v-else-if="step === 2" class="step-card">
      <h2 class="step-title">{{ t("onboarding.project.title") }}</h2>
      <p class="step-desc">{{ t("onboarding.project.desc") }}</p>

      <div class="project-area">
        <button
          class="browse-btn"
          :disabled="projectOpening"
          :aria-busy="projectOpening"
          @click="browseProject"
        >
          <span v-if="projectOpening" class="project-opening-spinner" aria-hidden="true"></span>
          <svg v-else viewBox="0 0 16 16" fill="currentColor" width="20" height="20">
            <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
          </svg>
          {{ projectOpening ? t("onboarding.project.opening") : t("onboarding.project.browse") }}
        </button>
        <div
          v-if="projectPath"
          class="project-selected"
          :class="{ valid: projectValid && !projectOpening, invalid: !projectValid && !projectOpening, loading: projectOpening }"
        >
          <span v-if="projectOpening" class="project-opening-spinner" aria-hidden="true"></span>
          <svg v-else-if="projectValid" class="status-icon ok" viewBox="0 0 16 16" fill="currentColor" width="16" height="16">
            <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.75.75 0 0 1 1.06-1.06L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0z"/>
          </svg>
          <span class="project-path">{{ projectPath }}</span>
        </div>
        <div v-if="projectError" class="msg error">{{ projectError }}</div>
      </div>

      <p class="skip-hint">{{ t("onboarding.project.skipHint") }}</p>

      <div class="step-actions">
        <button class="ob-btn secondary" :disabled="projectOpening" @click="goBack">{{ t("onboarding.back") }}</button>
        <button class="ob-btn primary" :disabled="projectOpening" @click="goNext">{{ t("onboarding.next") }}</button>
      </div>
    </div>

    <div v-else-if="step === 3" class="step-card">
      <h2 class="step-title">{{ t("onboarding.plugin.title") }}</h2>
      <p class="step-desc">{{ t("onboarding.plugin.desc") }}</p>

      <div class="status-area">
        <template v-if="!projectValid">
          <div class="status-row warn">{{ t("onboarding.plugin.needProject") }}</div>
        </template>
        <template v-else-if="pluginStatus === null">
          <div class="status-row loading">{{ t("common.loading") }}</div>
        </template>
        <template v-else-if="pluginStatus.status === 'upToDate'">
          <div class="status-row ok">
            <svg class="status-icon" viewBox="0 0 16 16" fill="currentColor" width="16" height="16">
              <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.75.75 0 0 1 1.06-1.06L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0z"/>
            </svg>
            {{ t("onboarding.plugin.installed") }}
          </div>
        </template>
        <template v-else>
          <div class="status-row warn">
            {{ pluginStatus.status === 'missing' ? t("onboarding.plugin.missing") : t("onboarding.plugin.outdated") }}
          </div>
          <button
            class="ob-btn primary"
            :disabled="pluginInstalling"
            @click="installPlugin"
          >
            {{ pluginInstalling ? t("onboarding.plugin.installing") : pluginStatus.status === 'missing' ? t("onboarding.plugin.install") : t("onboarding.plugin.update") }}
          </button>
        </template>
        <div v-if="pluginError" class="msg error">{{ pluginError }}</div>
      </div>

      <div class="step-actions">
        <button class="ob-btn secondary" @click="goBack">{{ t("onboarding.back") }}</button>
        <button class="ob-btn primary" @click="goNext">{{ t("onboarding.next") }}</button>
      </div>
    </div>

    <div v-else-if="step === 4" class="step-card">
      <h2 class="step-title">{{ t("onboarding.vcs.title") }}</h2>
      <div class="step-desc">
        <p>{{ t("onboarding.vcs.desc1") }}</p>
        <p>{{ t("onboarding.vcs.desc2") }}</p>
      </div>

      <div class="status-area">
        <template v-if="gitVcsStepState === 'loading'">
          <div class="status-row loading">{{ t("common.loading") }}</div>
        </template>
        <template v-else-if="gitVcsStepState === 'detected'">
          <div class="status-row ok">
            <svg class="status-icon" viewBox="0 0 16 16" fill="currentColor" width="16" height="16">
              <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.75.75 0 0 1 1.06-1.06L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0z"/>
            </svg>
            {{ t("onboarding.vcs.detected") }}
          </div>
        </template>
        <template v-else>
          <div class="status-row warn">
            {{
              gitVcsStepState === "needProject"
                ? t("onboarding.vcs.needProject")
                : gitVcsStepState === "notRepo"
                  ? t("onboarding.vcs.notRepo", gitInitTargetPath)
                  : t("onboarding.vcs.gitMissing")
            }}
          </div>
        </template>

        <div v-if="gitProbeState && gitProbeState.available" class="git-config-panel">
          <div class="git-config-panel-header">
            <span class="git-config-panel-title">{{ t("git.config.title") }}</span>
            <p class="git-config-panel-desc">{{ t("git.config.desc") }}</p>
          </div>
          <div v-if="gitUserMissing" class="status-row warn compact">{{ t("onboarding.vcs.userMissing") }}</div>
          <p
            v-else-if="gitConfigLoadedName && gitConfigLoadedEmail"
            class="hint-text git-config-summary"
          >
            {{ t("onboarding.vcs.userConfigured", gitConfigLoadedName, gitConfigLoadedEmail) }}
          </p>
          <div class="git-config-fields">
            <div class="git-config-field">
              <label class="git-config-label">{{ t("git.config.name") }}</label>
              <input
                v-model="gitConfigName"
                class="git-config-input"
                :placeholder="t('git.config.namePlaceholder')"
                @keydown.enter="saveGitConfig"
              />
            </div>
            <div class="git-config-field">
              <label class="git-config-label">{{ t("git.config.email") }}</label>
              <input
                v-model="gitConfigEmail"
                type="email"
                class="git-config-input"
                :placeholder="t('git.config.emailPlaceholder')"
                @keydown.enter="saveGitConfig"
              />
            </div>
          </div>
          <div class="git-config-actions">
            <button
              class="ob-btn secondary"
              :disabled="gitConfigSaving || !gitConfigComplete || !gitConfigDirty"
              @click="saveGitConfig"
            >
              {{ gitConfigSaving ? t("git.config.saving") : t("common.save") }}
            </button>
          </div>
          <div v-if="gitConfigError" class="msg error">{{ t("git.config.saveFailed", gitConfigError) }}</div>
        </div>

        <template v-if="gitCanInit">
          <button
            class="ob-btn primary"
            :disabled="gitInitLoading"
            @click="checkGitConfigAndInit"
          >
            {{ gitInitLoading ? t("onboarding.vcs.initializing") : t("onboarding.vcs.init") }}
          </button>
        </template>
        <div v-if="gitHelpText" class="hint-text">{{ gitHelpText }}</div>
        <div v-if="gitError" class="msg error">{{ gitError }}</div>
        <GitMissingHelp
          v-if="gitProbeState && !gitProbeState.available"
          :probe="gitProbeState"
          @resolved="checkGit"
        />
      </div>

      <p class="skip-hint">{{ t("onboarding.vcs.skipHint") }}</p>

      <div class="step-actions">
        <button class="ob-btn secondary" @click="goBack">{{ t("onboarding.back") }}</button>
        <button class="ob-btn primary" @click="goNext">{{ t("onboarding.next") }}</button>
      </div>
    </div>

    <div v-else-if="step === 5" class="step-card">
      <h2 class="step-title">{{ t("onboarding.scan.title") }}</h2>
      <p class="step-desc">{{ t("onboarding.scan.desc") }}</p>

      <div class="status-area">
        <template v-if="!scanPhase">
          <button class="ob-btn primary" @click="startScan" :disabled="!projectValid">
            {{ t("onboarding.scan.start") }}
          </button>
          <p v-if="!projectValid" class="skip-hint">{{ t("onboarding.scan.needProject") }}</p>
        </template>
        <template v-else>
          <div class="scan-progress">
            <div class="scan-phase-text">{{ scanProgressText() }}</div>
            <div v-if="!scanDone && scanPhase.phase !== 'error'" class="scan-bar">
              <div class="scan-bar-inner" />
            </div>
          </div>
        </template>
      </div>

      <div class="step-actions">
        <button class="ob-btn secondary" @click="goBack">{{ t("onboarding.back") }}</button>
        <button class="ob-btn primary finish" @click="emit('completed')">
          {{ t("onboarding.finish") }}
        </button>
      </div>
    </div>
    </div>

  </div>

</template>

<style scoped>
.onboarding-container {
  position: fixed;
  inset: 0;
  z-index: 9999;
  background: var(--bg-color);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.onboarding-titlebar {
  display: flex;
  align-items: stretch;
  justify-content: space-between;
  gap: 12px;
  padding-left: 16px;
  height: 40px;
  flex-shrink: 0;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
  -webkit-app-region: drag;
}

.onboarding-titlebar-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  height: 100%;
}

.onboarding-titlebar-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex-shrink: 0;
}

.onboarding-titlebar-brand {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-color);
}

.onboarding-titlebar-caption {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.onboarding-titlebar-actions {
  display: flex;
  align-items: center;
  align-self: stretch;
  gap: 0;
  height: 100%;
  min-width: 0;
  -webkit-app-region: no-drag;
}

.onboarding-body {
  flex: 1;
  min-height: 0;
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px;
  overflow-y: auto;
  box-sizing: border-box;
}

.skip-all-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
  padding: 0 14px;
  border-radius: 6px;
  transition: color 0.15s, background 0.15s;
  height: 100%;
  font-family: inherit;
  line-height: 1;
  -webkit-app-region: no-drag;
}
.skip-all-btn:hover {
  color: var(--text-color);
  background: var(--hover-bg);
}

.window-controls {
  display: flex;
  align-items: center;
  height: 100%;
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}

.win-ctrl-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 48px;
  min-width: 48px;
  height: 100%;
  padding: 0;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s, color 0.1s;
  -webkit-app-region: no-drag;
}

.win-ctrl-btn svg {
  pointer-events: none;
}

.win-ctrl-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.win-ctrl-btn.win-close:hover {
  background: #e81123;
  color: #fff;
}

.step-indicator {
  display: flex;
  gap: 8px;
  margin-bottom: 24px;
}
.step-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--border-color);
  transition: background 0.2s, transform 0.2s;
}
.step-dot.active {
  background: var(--accent-color);
  transform: scale(1.25);
}
.step-dot.done {
  background: var(--accent-color);
  opacity: 0.5;
}

.step-card {
  width: 100%;
  max-width: 600px;
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 16px;
  padding: 32px;
  animation: stepIn 0.25s ease;
}
@keyframes stepIn {
  from { opacity: 0; transform: translateY(12px); }
  to { opacity: 1; transform: translateY(0); }
}

.step-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--text-color);
  margin: 0 0 8px;
}
.step-desc {
  font-size: 14px;
  color: var(--text-secondary);
  margin: 0 0 20px;
  line-height: 1.5;
}
.auth-desc-lines p {
  margin: 0;
  line-height: 1.8;
}

.welcome-card {
  max-width: 520px;
  min-height: 460px;
  padding: 44px 32px 22px;
  display: flex;
  flex-direction: column;
  gap: 0;
  text-align: center;
}
.welcome-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0 auto 18px;
}
.welcome-title {
  font-size: 28px;
  font-weight: 700;
  color: var(--text-color);
  margin: 0 0 8px;
  line-height: 1.15;
}
.welcome-subtitle {
  font-size: 14px;
  color: var(--text-secondary);
  margin: 0 0 32px;
  line-height: 1.6;
}
.welcome-preferences {
  display: flex;
  flex-direction: column;
  gap: 24px;
  align-items: center;
}
.welcome-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
  align-items: center;
}
.welcome-section-label {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
  line-height: 1.4;
}
.welcome-segmented {
  width: auto;
  max-width: 100%;
}
.welcome-segmented :deep(.base-segmented-item) {
  justify-content: center;
  white-space: nowrap;
}
.step-actions.welcome-actions {
  justify-content: flex-end;
  margin-top: auto;
  padding-top: 16px;
  border-top: 1px solid var(--border-color);
}
.welcome-next-btn {
  min-width: 120px;
}

.provider-card {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  margin-bottom: 10px;
  overflow: hidden;
  transition: border-color 0.15s;
}
.provider-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.provider-list .provider-card {
  margin-bottom: 0;
}
.provider-card.expanded {
  border-color: var(--accent-color);
}
.provider-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  transition: background 0.15s;
  gap: 14px;
  cursor: pointer;
}
.provider-header:hover {
  background: var(--hover-bg);
}
.provider-left {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.provider-name {
  font-size: 14px;
  font-weight: 500;
  color: var(--text-color);
}
.provider-desc-text {
  font-size: 12px;
  color: var(--text-secondary);
}
.provider-badge {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  border: 1px solid transparent;
  white-space: nowrap;
}
.provider-badge.configured {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  border-color: var(--status-good-border);
}

.provider-body {
  padding: 0 16px 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.custom-endpoints-inline-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.custom-endpoint-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 9px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--input-bg);
}

.custom-endpoint-summary-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.custom-endpoint-summary-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.custom-endpoint-summary-meta {
  font-size: 12px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.custom-endpoint-summary-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.custom-endpoint-fields {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 10px;
  align-items: start;
}

.custom-endpoint-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-height: 0;
  min-width: 0;
}

.custom-endpoint-field:nth-child(3),
.custom-endpoint-field:nth-child(5) {
  grid-column: 1 / -1;
}

.custom-endpoint-label {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.custom-endpoint-label-with-hint {
  align-items: flex-start;
  justify-content: space-between;
}

.custom-endpoint-label-with-hint > span:first-child {
  flex-shrink: 0;
}

.custom-endpoint-hint {
  flex-shrink: 0;
  font-weight: 400;
  color: var(--text-secondary);
  opacity: 0.8;
  line-height: 1.35;
  text-align: right;
  white-space: nowrap;
}

.custom-endpoint-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.custom-endpoint-test-result {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 12px;
  line-height: 1.5;
  flex-wrap: wrap;
}

.custom-endpoint-test-result.testing {
  background: var(--hover-bg);
  color: var(--text-secondary);
}

.custom-endpoint-test-result.success {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
}

.custom-endpoint-test-result.error {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.custom-endpoint-test-heading {
  font-weight: 600;
}

.custom-endpoint-test-detail {
  min-width: 0;
  word-break: break-word;
}

.custom-endpoint-test-link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  font: inherit;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.custom-endpoint-spinner {
  width: 10px;
  height: 10px;
  margin-top: 3px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

.ob-input {
  flex: 1;
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  transition: border-color 0.15s;
}

.ob-input:focus {
  border-color: var(--accent-color);
}

.custom-endpoint-format-dropdown {
  width: 100%;
}

.custom-endpoint-format-dropdown :deep(.base-dropdown-menu) {
  width: 100%;
  min-width: 100%;
  max-width: 100%;
}

.ob-btn {
  padding: 8px 16px;
  border-radius: 8px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.15s;
  white-space: nowrap;
}
.ob-btn.primary {
  background: var(--accent-color);
  color: var(--bg-color);
  border-color: var(--accent-color);
}
.ob-btn.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}
.ob-btn.primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.ob-btn.secondary {
  background: transparent;
  color: var(--text-secondary);
  border-color: var(--border-color);
}
.ob-btn.secondary:hover {
  color: var(--text-color);
  background: var(--hover-bg);
}
.ob-btn.small {
  padding: 4px 10px;
  font-size: 12px;
}
.ob-btn.finish {
  min-width: 120px;
}

/* ── Step actions ── */
.step-actions {
  display: flex;
  justify-content: space-between;
  margin-top: 24px;
  gap: 12px;
}

@media (max-width: 760px) {
  .welcome-card {
    min-height: 0;
    padding: 24px 20px 20px;
  }

  .welcome-preferences {
    gap: 20px;
  }

  .step-actions.welcome-actions {
    justify-content: stretch;
  }

  .welcome-next-btn {
    width: 100%;
  }

  .provider-header {
    align-items: flex-start;
  }

  .custom-endpoint-fields {
    grid-template-columns: 1fr;
  }

  .custom-endpoint-field:nth-child(3),
  .custom-endpoint-field:nth-child(5) {
    grid-column: auto;
  }

  .custom-endpoint-actions {
    justify-content: stretch;
  }

  .custom-endpoint-actions .ob-btn {
    flex: 1;
  }
}

.status-area {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: 4px;
}
.status-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-radius: 8px;
  font-size: 13px;
}
.status-row.ok {
  background: color-mix(in srgb, #22c55e 10%, transparent);
  color: #22c55e;
}
.status-row.warn {
  background: color-mix(in srgb, #eab308 10%, transparent);
  color: #eab308;
}
.status-row.loading {
  background: var(--hover-bg);
  color: var(--text-secondary);
}
.status-icon {
  flex-shrink: 0;
}
.status-icon.ok {
  color: #22c55e;
}

.msg {
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
}
.msg.error {
  background: color-mix(in srgb, #ef4444 10%, transparent);
  color: #ef4444;
}
.msg.success {
  background: color-mix(in srgb, #22c55e 10%, transparent);
  color: #22c55e;
}

.skip-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin: 12px 0 0;
  text-align: center;
}
.hint-text {
  font-size: 12px;
  color: var(--text-secondary);
}
.instruction-text {
  font-size: 13px;
  color: var(--text-secondary);
  line-height: 1.5;
  margin: 0;
}

.project-area {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.browse-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  width: 100%;
  padding: 24px;
  border: 2px dashed var(--border-color);
  border-radius: 12px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 15px;
  cursor: pointer;
  transition: all 0.15s;
}
.browse-btn:hover {
  border-color: var(--accent-color);
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 5%, transparent);
}
.browse-btn:disabled,
.browse-btn:disabled:hover {
  border-color: var(--border-color);
  color: var(--text-secondary);
  background: var(--hover-bg);
  cursor: wait;
}
.project-selected {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-radius: 8px;
  font-size: 13px;
  word-break: break-all;
}
.project-selected.valid {
  background: color-mix(in srgb, #22c55e 10%, transparent);
  color: #22c55e;
}
.project-selected.invalid {
  background: color-mix(in srgb, #ef4444 10%, transparent);
  color: #ef4444;
}
.project-selected.loading {
  background: var(--hover-bg);
  color: var(--text-secondary);
}
.project-path {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}
.project-opening-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

.codex-url {
  font-size: 11px;
  color: var(--accent-color);
  text-decoration: underline;
  word-break: break-all;
}

.codex-code-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--input-bg);
  color: inherit;
  cursor: pointer;
  text-align: left;
  box-shadow: none;
  transition: border-color 0.15s, background 0.15s;
}

.codex-code-row:hover {
  background: var(--hover-bg);
  border-color: var(--accent-color);
}

.codex-code-row:focus-visible {
  outline: none;
  border-color: var(--accent-color);
}

.codex-code-row.copied {
  border-color: var(--status-good-border);
  background: var(--status-good-bg);
}

.codex-code {
  font-family: var(--font-mono-display);
  font-size: 20px;
  font-weight: 600;
  color: var(--accent-color);
  letter-spacing: 2px;
}

.codex-copy-indicator {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--text-secondary);
}

.codex-code-row.copied .codex-copy-indicator {
  color: var(--status-good-fg);
}

.scan-progress {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.scan-phase-text {
  font-size: 13px;
  color: var(--text-secondary);
}
.scan-bar {
  height: 4px;
  border-radius: 2px;
  background: var(--border-color);
  overflow: hidden;
}
.scan-bar-inner {
  height: 100%;
  width: 40%;
  background: var(--accent-color);
  border-radius: 2px;
  animation: scanSlide 1.5s ease-in-out infinite;
}
@keyframes scanSlide {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(350%); }
}
@keyframes spin {
  to { transform: rotate(360deg); }
}

/* ── Git Config Panel ── */
.git-config-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--sidebar-bg) 88%, var(--hover-bg));
}
.git-config-panel-header {
  display: flex;
  flex-direction: column;
}
.git-config-panel-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}
.git-config-panel-desc {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
}
.git-config-summary {
  margin: 0;
}
.git-config-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.git-config-field {
  display: flex;
  flex-direction: column;
}
.git-config-label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 4px;
}
.git-config-input {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-primary);
  font-size: 14px;
  box-sizing: border-box;
  outline: none;
  transition: border-color 0.15s;
}
.git-config-input:focus {
  border-color: var(--accent-color);
}
.git-config-actions {
  display: flex;
  justify-content: flex-start;
  gap: 8px;
}
.status-row.compact {
  padding: 8px 12px;
}

</style>
