<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import type { Locale } from "../../i18n";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import { getCachedDebugMode, getDebugMode, setDebugMode } from "../../services/permissions";
import {
  clearAppStorageMigration,
  getAppStorageInfo,
  openAppStorageDirectory,
  scheduleAppStorageMigration,
} from "../../services/storage";
import type { AppStorageInfo } from "../../types";
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
const storageInfo = ref<AppStorageInfo | null>(null);
const storageBusy = ref(false);
const storageInfoLoadFailed = ref(false);
const storageSuccess = ref("");

const languageOptions = [
  { value: "zh", label: "中文" },
  { value: "en", label: "English" },
] as const;

const debugStatusLabel = computed(() => {
  if (!debugReady.value) return t("common.loading");
  return debugEnabled.value
    ? t("settings.general.debugModeOn")
    : t("settings.general.debugModeOff");
});

onMounted(async () => {
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
  await refreshStorageInfo();
});

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
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.general.language") }}</div>
    <p class="section-desc">{{ t("settings.general.languageDesc") }}</p>
    <BaseSegmented
      :model-value="locale"
      :options="[...languageOptions]"
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
</style>
