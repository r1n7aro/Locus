<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import type { Locale } from "../../i18n";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import { getCachedDebugMode, getDebugMode, setDebugMode } from "../../services/permissions";
import { gitRuntimeState, gitSaveRuntimeSelection } from "../../services/git";
import {
  getCloseBehavior,
  getPythonRuntimeState,
  getUnityBackgroundHookEnabled,
  getUnityBackgroundHookStatus,
  savePythonRuntimeSelection,
  setCloseBehavior,
  setUnityBackgroundHookEnabled,
  type AppCloseBehavior,
} from "../../services/system";
import {
  clearAppTempDir,
  clearAppStorageMigration,
  getAppTempInfo,
  getAppStorageInfo,
  openAppStorageDirectory,
  openAppTempDirectory,
  scheduleAppStorageMigration,
} from "../../services/storage";
import type {
  AppStorageInfo,
  AppTempInfo,
  GitRuntimeInfo,
  GitRuntimeState,
  PythonRuntimeInfo,
  PythonRuntimeState,
  UnityBackgroundHookStatus,
} from "../../types";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";

defineProps<{
  locale: string;
  resetConfirm: boolean;
}>();

const emit = defineEmits<{
  setLocale: [locale: Locale];
  startReset: [];
  confirmReset: [];
  cancelReset: [];
}>();

const notificationStore = useNotificationStore();
const initialDebugMode = getCachedDebugMode();
const debugEnabled = ref(initialDebugMode ?? false);
const debugReady = ref(initialDebugMode !== null);
const debugBusy = ref(false);
const closeBehavior = ref<AppCloseBehavior>("exit");
const closeBehaviorReady = ref(false);
const closeBehaviorBusy = ref(false);
const unityBackgroundHookEnabled = ref(true);
const unityBackgroundHookReady = ref(false);
const unityBackgroundHookBusy = ref(false);
const unityBackgroundHookStatus = ref<UnityBackgroundHookStatus | null>(null);
const storageInfo = ref<AppStorageInfo | null>(null);
const storageBusy = ref(false);
const storageInfoLoadFailed = ref(false);
const storageSuccess = ref("");
const tempInfo = ref<AppTempInfo | null>(null);
const tempBusy = ref(false);
const tempInfoLoadFailed = ref(false);
const tempSuccess = ref("");
const gitState = ref<GitRuntimeState | null>(null);
const gitBusy = ref(false);
const pythonState = ref<PythonRuntimeState | null>(null);
const pythonBusy = ref(false);
const pythonRuntimeDiscovered = ref(false);
let gitLoadToken = 0;
let pythonLoadToken = 0;

const languageOptions = computed(() => [
  { value: "zh", label: t("language.zh") },
  { value: "en", label: t("language.en") },
]);

const closeBehaviorOptions = computed(() => [
  {
    value: "exit",
    label: t("settings.general.closeBehaviorExit"),
    disabled: !closeBehaviorReady.value || closeBehaviorBusy.value,
  },
  {
    value: "minimizeToTray",
    label: t("settings.general.closeBehaviorTray"),
    disabled: !closeBehaviorReady.value || closeBehaviorBusy.value,
  },
]);

const debugStatusLabel = computed(() => {
  if (!debugReady.value) return t("common.loading");
  return debugEnabled.value
    ? t("settings.general.debugModeOn")
    : t("settings.general.debugModeOff");
});

const unityBackgroundHookStatusLabel = computed(() => {
  if (!unityBackgroundHookReady.value) return t("common.loading");
  const status = unityBackgroundHookStatus.value;
  if (!unityBackgroundHookEnabled.value || status?.state === "disabled") {
    return t("settings.general.unityBackgroundHookOff");
  }
  if (status?.state === "patched") return t("settings.general.unityBackgroundHookPatched");
  if (status?.state === "failed") return t("settings.general.unityBackgroundHookFailed");
  if (status?.state === "unsupported") return t("settings.general.unityBackgroundHookUnsupported");
  return t("settings.general.unityBackgroundHookOn");
});

const pythonOptions = computed(() => {
  const options = (pythonState.value?.runtimes ?? []).map((runtime) => ({
    value: runtime.id,
    label: pythonRuntimeLabel(runtime),
    hint: runtime.available
      ? pythonRuntimeHint(runtime)
      : t("settings.general.pythonUnavailable"),
    disabled: !runtime.available,
  }));
  if (pythonBusy.value && !pythonRuntimeDiscovered.value) {
    return [
      {
        value: "__python_runtime_searching",
        label: t("settings.general.pythonSearching"),
        disabled: true,
      },
      ...options,
    ];
  }
  return options;
});

const selectedPythonId = computed(() => pythonState.value?.selectedId ?? "");
const selectedPythonLabel = computed(() => {
  const runtime = pythonState.value?.effective;
  if (!runtime) return pythonBusy.value ? t("common.loading") : t("settings.general.pythonNone");
  return pythonRuntimeLabel(runtime);
});

const effectivePythonPath = computed(() => pythonState.value?.effective?.path ?? "");
const hasAvailablePythonOption = computed(() => pythonOptions.value.some((option) => !option.disabled));
const gitOptions = computed(() =>
  (gitState.value?.runtimes ?? []).map((runtime) => ({
    value: runtime.id,
    label: gitRuntimeDisplayLabel(runtime),
    hint: runtime.available
      ? gitRuntimeHint(runtime)
      : t("settings.general.gitUnavailable"),
    disabled: !runtime.available,
  })),
);
const selectedGitId = computed(() => gitState.value?.selectedId ?? "");
const gitRuntimeLabel = computed(() => {
  const runtime = gitState.value?.effective;
  if (!runtime) return gitBusy.value ? t("common.loading") : t("settings.general.gitNone");
  return gitRuntimeDisplayLabel(runtime);
});
const gitRuntimePath = computed(() => gitState.value?.effective?.path ?? "");
const hasAvailableGitOption = computed(() => gitOptions.value.some((option) => !option.disabled));

onMounted(() => {
  void refreshDebugMode();
  void refreshUnityBackgroundHook();
  void refreshCloseBehavior();
  void refreshStorageInfo();
  void refreshTempInfo();
  void refreshGitRuntimeState(false);
  void refreshPythonRuntimeState(false, false);
});

async function refreshDebugMode() {
  try {
    debugEnabled.value = await getDebugMode();
    debugReady.value = true;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadDebugMode",
    });
    debugReady.value = true;
  }
}

async function toggleDebug() {
  if (!debugReady.value || debugBusy.value) return;
  debugBusy.value = true;
  const next = !debugEnabled.value;
  try {
    await setDebugMode(next);
    debugEnabled.value = next;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleDebugMode",
    });
  } finally {
    debugBusy.value = false;
  }
}

async function refreshUnityBackgroundHook() {
  try {
    const [enabled, status] = await Promise.all([
      getUnityBackgroundHookEnabled(),
      getUnityBackgroundHookStatus(),
    ]);
    unityBackgroundHookEnabled.value = enabled;
    unityBackgroundHookStatus.value = status;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadUnityBackgroundHook",
    });
  } finally {
    unityBackgroundHookReady.value = true;
  }
}

async function toggleUnityBackgroundHook() {
  if (!unityBackgroundHookReady.value || unityBackgroundHookBusy.value) return;
  unityBackgroundHookBusy.value = true;
  const next = !unityBackgroundHookEnabled.value;
  unityBackgroundHookEnabled.value = next;
  try {
    const status = await setUnityBackgroundHookEnabled(next);
    unityBackgroundHookStatus.value = status;
    if (status.state === "failed" && status.error) {
      notificationStore.addNotice("error", status.error, {
        operation: "unity-background-hook",
        replaceOperation: true,
      });
    }
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unity-background-hook",
      replaceOperation: true,
    });
    await refreshUnityBackgroundHook();
  } finally {
    unityBackgroundHookBusy.value = false;
  }
}

async function refreshCloseBehavior() {
  try {
    closeBehavior.value = await getCloseBehavior();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadCloseBehavior",
    });
  } finally {
    closeBehaviorReady.value = true;
  }
}

async function selectCloseBehavior(value: string) {
  if (!closeBehaviorReady.value || closeBehaviorBusy.value) return;
  const next = value === "minimizeToTray" ? "minimizeToTray" : "exit";
  if (next === closeBehavior.value) return;
  const previous = closeBehavior.value;
  closeBehavior.value = next;
  closeBehaviorBusy.value = true;
  try {
    await setCloseBehavior(next);
  } catch (e) {
    closeBehavior.value = previous;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "saveCloseBehavior",
    });
  } finally {
    closeBehaviorBusy.value = false;
  }
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision = value >= 100 || unitIndex === 0 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

async function refreshStorageInfo() {
  storageBusy.value = true;
  try {
    storageInfo.value = await getAppStorageInfo();
    storageInfoLoadFailed.value = false;
  } catch (e) {
    storageInfoLoadFailed.value = true;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadStorageInfo",
    });
  } finally {
    storageBusy.value = false;
  }
}

async function refreshTempInfo() {
  tempBusy.value = true;
  try {
    tempInfo.value = await getAppTempInfo();
    tempInfoLoadFailed.value = false;
  } catch (e) {
    tempInfoLoadFailed.value = true;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadTempInfo",
    });
  } finally {
    tempBusy.value = false;
  }
}

async function clearTempDirectory() {
  if (tempBusy.value) return;
  const confirmed = await confirm(
    t("settings.general.tempClearConfirm"),
    {
      title: t("settings.general.tempFiles"),
      kind: "warning",
    },
  );
  if (!confirmed) return;

  tempBusy.value = true;
  tempSuccess.value = "";
  try {
    tempInfo.value = await clearAppTempDir();
    tempInfoLoadFailed.value = false;
    tempSuccess.value = t("settings.general.tempClearDone");
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "clearTempDir",
    });
  } finally {
    tempBusy.value = false;
  }
}

async function openStorageDirectory(path: string | null | undefined) {
  if (!path) return;
  try {
    await openAppStorageDirectory();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openStorageDirectory",
    });
  }
}

async function openTempDirectory(path: string | null | undefined) {
  if (!path) return;
  try {
    await openAppTempDirectory();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openTempDirectory",
    });
  }
}

async function chooseStorageDirectory() {
  if (storageBusy.value) return;
  const current = storageInfo.value;
  const selected = await open({
    directory: true,
    multiple: false,
    defaultPath: current?.pendingTargetPath || current?.activePath || undefined,
  });
  if (typeof selected !== "string" || !selected.trim()) return;

  const confirmed = await confirm(
    t("settings.general.storageSwitchConfirm", selected),
    {
      title: t("settings.general.storage"),
      kind: "warning",
    },
  );
  if (!confirmed) return;

  storageBusy.value = true;
  storageSuccess.value = "";
  try {
    storageInfo.value = await scheduleAppStorageMigration(selected);
    storageSuccess.value = t("settings.general.storagePendingRestart");
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "scheduleStorageMigration",
    });
  } finally {
    storageBusy.value = false;
  }
}

async function restoreDefaultStorageDirectory() {
  if (storageBusy.value || !storageInfo.value) return;
  const confirmed = await confirm(
    t("settings.general.storageRestoreConfirm", storageInfo.value.defaultPath),
    {
      title: t("settings.general.storage"),
      kind: "warning",
    },
  );
  if (!confirmed) return;

  storageBusy.value = true;
  storageSuccess.value = "";
  try {
    storageInfo.value = await scheduleAppStorageMigration(storageInfo.value.defaultPath);
    storageSuccess.value = t("settings.general.storagePendingRestart");
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "restoreStorageDirectory",
    });
  } finally {
    storageBusy.value = false;
  }
}

async function cancelStorageMigration() {
  if (storageBusy.value) return;
  const confirmed = await confirm(
    t("settings.general.storageCancelConfirm"),
    {
      title: t("settings.general.storage"),
      kind: "warning",
    },
  );
  if (!confirmed) return;

  storageBusy.value = true;
  storageSuccess.value = "";
  try {
    storageInfo.value = await clearAppStorageMigration();
    storageSuccess.value = t("settings.general.storagePendingCleared");
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "clearStorageMigration",
    });
  } finally {
    storageBusy.value = false;
  }
}

function gitSourceLabel(source: GitRuntimeInfo["source"]): string {
  switch (source) {
    case "envOverride":
      return t("settings.general.gitManual");
    case "managed":
      return t("settings.general.gitManaged");
    case "path":
      return t("settings.general.gitPath");
    case "commonLocation":
      return t("settings.general.gitSystem");
    default:
      return t("settings.general.gitSystem");
  }
}

async function refreshGitRuntimeState(refresh = false) {
  const token = ++gitLoadToken;
  gitBusy.value = true;
  try {
    const nextState = await gitRuntimeState(refresh);
    if (token === gitLoadToken) {
      gitState.value = nextState;
    }
  } catch (e) {
    if (token !== gitLoadToken) return;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadGitRuntime",
    });
  } finally {
    if (token === gitLoadToken) {
      gitBusy.value = false;
    }
  }
}

function gitRuntimeDisplayLabel(runtime: GitRuntimeInfo): string {
  const source = gitSourceLabel(runtime.source);
  return runtime.version ? `${source} ${runtime.version}` : source;
}

function gitRuntimeHint(runtime: GitRuntimeInfo): string {
  return runtime.path;
}

async function selectGitRuntime(selectedId: string) {
  if (gitBusy.value || !selectedId) return;
  const token = ++gitLoadToken;
  gitBusy.value = true;
  try {
    const nextState = await gitSaveRuntimeSelection(selectedId);
    if (token === gitLoadToken) {
      gitState.value = nextState;
    }
  } catch (e) {
    if (token !== gitLoadToken) return;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "saveGitRuntime",
    });
    await refreshGitRuntimeState(true);
  } finally {
    if (token === gitLoadToken) {
      gitBusy.value = false;
    }
  }
}

function pythonRuntimeLabel(runtime: PythonRuntimeInfo): string {
  const source = runtime.source === "managed"
    ? t("settings.general.pythonManaged")
    : t("settings.general.pythonSystem");
  return runtime.version ? `${source} ${runtime.version}` : source;
}

function pythonRuntimeHint(runtime: PythonRuntimeInfo): string {
  return runtime.path;
}

async function refreshPythonRuntimeState(refresh = false, discover = true) {
  const token = ++pythonLoadToken;
  pythonBusy.value = true;
  try {
    const nextState = await getPythonRuntimeState(refresh, discover);
    if (token === pythonLoadToken) {
      pythonState.value = nextState;
      if (discover) {
        pythonRuntimeDiscovered.value = true;
      }
    }
  } catch (e) {
    if (token !== pythonLoadToken) return;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadPythonRuntime",
    });
  } finally {
    if (token === pythonLoadToken) {
      pythonBusy.value = false;
    }
  }
}

function openPythonRuntimeOptions() {
  if (pythonBusy.value || pythonRuntimeDiscovered.value) return;
  void refreshPythonRuntimeState(false, true);
}

async function selectPythonRuntime(selectedId: string) {
  if (pythonBusy.value || !selectedId) return;
  const token = ++pythonLoadToken;
  pythonBusy.value = true;
  try {
    const nextState = await savePythonRuntimeSelection(selectedId);
    if (token === pythonLoadToken) {
      pythonState.value = nextState;
      pythonRuntimeDiscovered.value = true;
    }
  } catch (e) {
    if (token !== pythonLoadToken) return;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "savePythonRuntime",
    });
    await refreshPythonRuntimeState(true, true);
  } finally {
    if (token === pythonLoadToken) {
      pythonBusy.value = false;
    }
  }
}
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.language") }}</div>
    <p class="section-desc">{{ t("settings.general.languageDesc") }}</p>
    <BaseSegmented
      :model-value="locale"
      :options="languageOptions"
      @update:model-value="emit('setLocale', $event as Locale)"
    />
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.debugMode") }}</div>
    <p class="section-desc">{{ t("settings.general.debugModeDesc") }}</p>
    <label class="debug-toggle" :aria-busy="!debugReady">
      <BaseSwitch
        v-if="debugReady"
        :model-value="debugEnabled"
        :disabled="debugBusy"
        :aria-label="t('settings.general.debugMode')"
        @update:model-value="toggleDebug"
      />
      <span v-else class="debug-toggle-placeholder" aria-hidden="true" />
      <span class="debug-toggle-label">{{ debugStatusLabel }}</span>
    </label>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.unityBackgroundHook") }}</div>
    <p class="section-desc">{{ t("settings.general.unityBackgroundHookDesc") }}</p>
    <label class="debug-toggle" :aria-busy="!unityBackgroundHookReady">
      <BaseSwitch
        v-if="unityBackgroundHookReady"
        :model-value="unityBackgroundHookEnabled"
        :disabled="unityBackgroundHookBusy"
        :aria-label="t('settings.general.unityBackgroundHook')"
        @update:model-value="toggleUnityBackgroundHook"
      />
      <span v-else class="debug-toggle-placeholder" aria-hidden="true" />
      <span class="debug-toggle-label">{{ unityBackgroundHookStatusLabel }}</span>
    </label>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.closeBehavior") }}</div>
    <p class="section-desc">{{ t("settings.general.closeBehaviorDesc") }}</p>
    <BaseSegmented
      class="close-behavior-segmented"
      :model-value="closeBehavior"
      :options="closeBehaviorOptions"
      :aria-label="t('settings.general.closeBehavior')"
      size="sm"
      @update:model-value="selectCloseBehavior"
    />
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.storage") }}</div>
    <p class="section-desc">{{ t("settings.general.storageDesc") }}</p>
    <div class="storage-block">
      <div class="storage-row">
        <span class="storage-label">{{ t("settings.general.storageCurrentPath") }}</span>
        <code class="storage-path" :title="storageInfo?.activePath || ''">
          {{ storageInfo?.activePath || (storageBusy ? t("common.loading") : "—") }}
        </code>
        <button
          class="action-btn storage-btn"
          :disabled="storageBusy || !storageInfo"
          @click="openStorageDirectory(storageInfo?.activePath)"
        >
          {{ t("settings.general.storageOpen") }}
        </button>
      </div>
      <div class="storage-row">
        <span class="storage-label">{{ t("settings.general.storageSize") }}</span>
        <span class="storage-text">{{ storageInfo ? formatBytes(storageInfo.activeSizeBytes) : "—" }}</span>
      </div>
      <div class="storage-row" v-if="storageInfo?.usesCustomPath">
        <span class="storage-label">{{ t("settings.general.storageDefaultPath") }}</span>
        <code class="storage-path" :title="storageInfo.defaultPath">{{ storageInfo.defaultPath }}</code>
      </div>
      <div v-if="storageInfoLoadFailed && !storageInfo && !storageBusy" class="storage-status">
        <span class="storage-status-text">{{ t("settings.general.storageUnavailable") }}</span>
        <button class="action-btn storage-btn" @click="refreshStorageInfo">
          {{ t("common.refresh") }}
        </button>
      </div>
      <div v-if="storageInfo?.pendingTargetPath" class="storage-pending">
        <div class="storage-pending-title">{{ t("settings.general.storagePendingTitle") }}</div>
        <code class="storage-path" :title="storageInfo.pendingTargetPath">{{ storageInfo.pendingTargetPath }}</code>
        <div class="storage-hint">{{ t("settings.general.storagePendingDesc") }}</div>
      </div>
      <div class="storage-actions">
        <button class="action-btn storage-btn" :disabled="storageBusy" @click="chooseStorageDirectory">
          {{ t("settings.general.storageChange") }}
        </button>
        <button
          v-if="storageInfo?.usesCustomPath"
          class="action-btn storage-btn"
          :disabled="storageBusy"
          @click="restoreDefaultStorageDirectory"
        >
          {{ t("settings.general.storageRestoreDefault") }}
        </button>
        <button
          v-if="storageInfo?.pendingTargetPath"
          class="action-btn storage-btn"
          :disabled="storageBusy"
          @click="cancelStorageMigration"
        >
          {{ t("settings.general.storageCancelPending") }}
        </button>
      </div>
    </div>
    <div v-if="storageSuccess" class="storage-success">{{ storageSuccess }}</div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.tempFiles") }}</div>
    <p class="section-desc">{{ t("settings.general.tempFilesDesc") }}</p>
    <div class="storage-block">
      <div class="storage-row">
        <span class="storage-label">{{ t("settings.general.storageCurrentPath") }}</span>
        <code class="storage-path" :title="tempInfo?.path || ''">
          {{ tempInfo?.path || (tempBusy ? t("common.loading") : "—") }}
        </code>
        <button
          class="action-btn storage-btn"
          :disabled="tempBusy || !tempInfo"
          @click="openTempDirectory(tempInfo?.path)"
        >
          {{ t("settings.general.storageOpen") }}
        </button>
      </div>
      <div class="storage-row">
        <span class="storage-label">{{ t("settings.general.storageSize") }}</span>
        <span class="storage-text">{{ tempInfo ? formatBytes(tempInfo.sizeBytes) : "—" }}</span>
      </div>
      <div v-if="tempInfoLoadFailed && !tempInfo && !tempBusy" class="storage-status">
        <span class="storage-status-text">{{ t("settings.general.tempUnavailable") }}</span>
        <button class="action-btn storage-btn" @click="refreshTempInfo">
          {{ t("common.refresh") }}
        </button>
      </div>
      <div class="storage-actions">
        <button class="action-btn storage-btn" :disabled="tempBusy" @click="refreshTempInfo">
          {{ t("common.refresh") }}
        </button>
        <button
          class="action-btn storage-btn danger"
          :disabled="tempBusy || !tempInfo || tempInfo.sizeBytes <= 0"
          @click="clearTempDirectory"
        >
          {{ t("settings.general.tempClear") }}
        </button>
      </div>
    </div>
    <div v-if="tempSuccess" class="storage-success">{{ tempSuccess }}</div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.gitRuntime") }}</div>
    <p class="section-desc">{{ t("settings.general.gitRuntimeDesc") }}</p>
    <div class="runtime-block">
      <div class="runtime-row">
        <span class="runtime-label">{{ t("settings.general.gitSelected") }}</span>
        <BaseDropdown
          class="runtime-dropdown"
          :model-value="selectedGitId"
          :selected-label="gitRuntimeLabel"
          :options="gitOptions"
          menu-align="start"
          :placeholder="t('settings.general.gitNone')"
          :aria-label="t('settings.general.gitRuntime')"
          :disabled="gitBusy || !hasAvailableGitOption"
          @update:model-value="selectGitRuntime"
        />
        <button class="action-btn runtime-btn" :disabled="gitBusy" @click="refreshGitRuntimeState(true)">
          {{ t("common.refresh") }}
        </button>
      </div>
      <div class="runtime-row">
        <span class="runtime-label">{{ t("settings.general.gitPathLabel") }}</span>
        <code class="runtime-path" :title="gitRuntimePath">
          {{ gitRuntimePath || (gitBusy ? t("common.loading") : "—") }}
        </code>
      </div>
      <div v-if="gitState?.missingSelected" class="runtime-hint">
        {{ t("settings.general.gitMissingSelected") }}
      </div>
      <div v-else-if="!gitState?.effective && !gitBusy" class="runtime-hint">
        {{ t("settings.general.gitNoRuntime") }}
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.pythonRuntime") }}</div>
    <p class="section-desc">{{ t("settings.general.pythonRuntimeDesc") }}</p>
    <div class="runtime-block">
      <div class="runtime-row">
        <span class="runtime-label">{{ t("settings.general.pythonSelected") }}</span>
        <BaseDropdown
          class="runtime-dropdown"
          :model-value="selectedPythonId"
          :selected-label="selectedPythonLabel"
          :options="pythonOptions"
          menu-align="start"
          :placeholder="t('settings.general.pythonNone')"
          :aria-label="t('settings.general.pythonRuntime')"
          :disabled="pythonBusy || (pythonRuntimeDiscovered && !hasAvailablePythonOption)"
          @open="openPythonRuntimeOptions"
          @update:model-value="selectPythonRuntime"
        />
        <button class="action-btn python-btn" :disabled="pythonBusy" @click="refreshPythonRuntimeState(true, true)">
          {{ t("common.refresh") }}
        </button>
      </div>
      <div class="runtime-row">
        <span class="runtime-label">{{ t("settings.general.pythonPath") }}</span>
        <code class="runtime-path" :title="effectivePythonPath">
          {{ effectivePythonPath || (pythonBusy ? t("common.loading") : "—") }}
        </code>
      </div>
      <div v-if="pythonState?.missingSelected" class="runtime-hint">
        {{ t("settings.general.pythonMissingSelected") }}
      </div>
      <div v-else-if="!pythonState?.effective && !pythonBusy" class="runtime-hint">
        {{ t("settings.general.pythonNoRuntime") }}
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.resetOnboarding") }}</div>
    <p class="section-desc">{{ t("settings.general.resetOnboardingDesc") }}</p>
    <div v-if="!resetConfirm">
      <button class="reset-onboarding-btn" @click="emit('startReset')">
        {{ t("settings.general.resetOnboardingBtn") }}
      </button>
    </div>
    <div v-else class="reset-confirm-row">
      <span class="reset-confirm-text">{{ t("settings.general.resetOnboardingConfirm") }}</span>
      <button class="reset-onboarding-btn" @click="emit('confirmReset')">
        {{ t("common.confirm") }}
      </button>
      <button class="cancel-btn" @click="emit('cancelReset')">{{ t("common.cancel") }}</button>
    </div>
  </div>
</template>

<style scoped>
.debug-toggle {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: var(--text-color);
  user-select: none;
}
.debug-toggle-placeholder {
  flex-shrink: 0;
  width: 34px;
  height: 18px;
  border: 1px solid color-mix(in srgb, var(--border-strong) 82%, var(--text-secondary) 18%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--hover-bg) 24%);
  opacity: 0.55;
}
.debug-toggle-label {
  font-size: 13px;
}
.close-behavior-segmented {
  width: fit-content;
  max-width: 100%;
}
.storage-block {
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-width: 760px;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}
.storage-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}
.storage-label {
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 72px;
}
.storage-path {
  display: inline-block;
  max-width: min(860px, 100%);
  padding: 4px 8px;
  border-radius: 6px;
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.storage-text {
  font-size: 12px;
  color: var(--text-secondary);
}
.storage-pending {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-left: 10px;
  border-left: 1px solid var(--border-strong);
}
.storage-status {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg) 75%, transparent);
}
.storage-status-text {
  font-size: 12px;
  color: var(--text-secondary);
}
.storage-pending-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}
.storage-hint {
  font-size: 11px;
  color: var(--text-secondary);
}
.storage-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 6px;
}
.storage-btn {
  font-size: 11px;
}
.storage-btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.storage-success {
  display: inline-flex;
  margin-top: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--status-good-border);
  background: var(--status-good-bg);
  color: var(--status-good-fg);
  font-size: 12px;
}
.runtime-block {
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-width: 760px;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}
.runtime-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}
.runtime-label {
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 72px;
}
.runtime-dropdown {
  width: min(420px, 100%);
}
.runtime-path {
  display: inline-block;
  max-width: min(860px, 100%);
  padding: 4px 8px;
  border-radius: 6px;
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.runtime-btn,
.python-btn {
  font-size: 11px;
}
.runtime-btn:disabled,
.python-btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.runtime-hint {
  font-size: 11px;
  color: var(--text-secondary);
}
</style>
