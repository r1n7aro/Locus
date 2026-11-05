import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { locale, t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getAppRuntimeReleaseChannel, getAppRuntimeVersion } from "../services/appVersion";
import { getBundledRemoteApp } from "../services/remoteAccess";
import {
  fetchAppUpdateManifest,
  normalizeAppUpdateChannel,
  resolveAppUpdateInfo,
} from "../services/appUpdate";
import type { AppUpdateChannel, AppUpdateInfo, AppUpdateManifest, AppUpdateSourceKind } from "../types";
import { useNotificationStore } from "./notification";
import { saveSetting, settingValue } from "../composables/useSettingsStore";

const LAST_CHECKED_AT_STORAGE_KEY = "locus-app-update-last-checked-at";
/** `auto` (the default) follows the channel of the installed build. */
const UPDATE_CHANNEL_SETTING_KEY = "updates.channel";

function loadLastCheckedAt(): number | null {
  try {
    const raw = localStorage.getItem(LAST_CHECKED_AT_STORAGE_KEY);
    if (!raw) return null;
    const parsed = Number.parseInt(raw, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

function persistLastCheckedAt(value: number | null) {
  try {
    if (value == null) {
      localStorage.removeItem(LAST_CHECKED_AT_STORAGE_KEY);
      return;
    }
    localStorage.setItem(LAST_CHECKED_AT_STORAGE_KEY, String(value));
  } catch {
    /* ignore */
  }
}

function readPreferredUpdateChannel(): AppUpdateChannel | null {
  const value = settingValue(UPDATE_CHANNEL_SETTING_KEY);
  return value === "stable" || value === "experimental" ? normalizeAppUpdateChannel(value) : null;
}

export const useAppUpdateStore = defineStore("appUpdate", () => {
  const manifest = ref<AppUpdateManifest | null>(null);
  const currentVersion = ref("");
  const currentChannel = ref<AppUpdateChannel>("stable");
  /** The running installer flavor bundles the Locus Remote app; updates keep it. */
  const bundlesRemoteApp = ref(false);
  const preferredUpdateChannel = computed<AppUpdateChannel | null>({
    get: readPreferredUpdateChannel,
    set: (value) => saveSetting(UPDATE_CHANNEL_SETTING_KEY, value ?? "auto"),
  });
  const sourceKind = ref<AppUpdateSourceKind | null>(null);
  const sourceBaseUrl = ref("");
  const lastCheckedAt = ref<number | null>(loadLastCheckedAt());
  const lastError = ref<string | null>(null);
  const checking = ref(false);
  const dialogDismissed = ref(false);
  const currentIsExperimental = computed(() => currentChannel.value === "experimental");
  const updateChannel = computed<AppUpdateChannel>(() =>
    preferredUpdateChannel.value ?? currentChannel.value,
  );

  const updateInfo = computed<AppUpdateInfo | null>(() => {
    if (!manifest.value || !currentVersion.value) {
      return null;
    }

    return resolveAppUpdateInfo(
      manifest.value,
      currentVersion.value,
      locale.value,
      sourceBaseUrl.value || undefined,
      sourceKind.value ?? "remote",
      currentChannel.value,
      { remoteApp: bundlesRemoteApp.value },
    );
  });

  const hasUpdate = computed(() => Boolean(updateInfo.value));
  const sourceLabel = computed(() => {
    if (!sourceBaseUrl.value) {
      return t("settings.about.versionSourceUnknown");
    }

    try {
      const { host } = new URL(sourceBaseUrl.value);
      return sourceKind.value === "local"
        ? t("settings.about.versionSourceLocal", host)
        : t("settings.about.versionSourceRemote", host);
    } catch {
      return sourceKind.value === "local"
        ? t("settings.about.versionSourceLocal", sourceBaseUrl.value)
        : t("settings.about.versionSourceRemote", sourceBaseUrl.value);
    }
  });

  let currentVersionPromise: Promise<string> | null = null;
  let activeCheckPromise: Promise<AppUpdateInfo | null> | null = null;

  function setLastCheckedAt(value: number) {
    lastCheckedAt.value = value;
    persistLastCheckedAt(value);
  }

  function setUpdateChannel(value: AppUpdateChannel) {
    const nextChannel = normalizeAppUpdateChannel(value);
    preferredUpdateChannel.value = nextChannel;
    manifest.value = null;
    sourceKind.value = null;
    sourceBaseUrl.value = "";
    lastError.value = null;
    dialogDismissed.value = false;
  }

  async function ensureCurrentVersion(): Promise<string> {
    if (currentVersion.value) {
      return currentVersion.value;
    }

    if (!currentVersionPromise) {
      currentVersionPromise = Promise.all([
        getAppRuntimeVersion(),
        getAppRuntimeReleaseChannel(),
        getBundledRemoteApp().catch(() => null),
      ])
        .then(([version, channel, remoteApp]) => {
          currentVersion.value = version;
          currentChannel.value = channel;
          bundlesRemoteApp.value = remoteApp !== null;
          return version;
        })
        .finally(() => {
          currentVersionPromise = null;
        });
    }

    return currentVersionPromise;
  }

  async function checkForUpdates(options?: { silent?: boolean }): Promise<AppUpdateInfo | null> {
    if (activeCheckPromise) {
      return activeCheckPromise;
    }

    const silent = options?.silent ?? false;
    const checkedAt = Date.now();
    const notificationStore = useNotificationStore();

    checking.value = true;
    activeCheckPromise = (async () => {
      try {
        const version = await ensureCurrentVersion();
        currentVersion.value = version;
        const nextManifestResult = await fetchAppUpdateManifest({
          throwOnError: !silent,
          channel: updateChannel.value,
        });
        if (!nextManifestResult) {
          throw new Error("Missing update manifest");
        }

        manifest.value = nextManifestResult.manifest;
        sourceKind.value = nextManifestResult.sourceKind;
        sourceBaseUrl.value = nextManifestResult.sourceBaseUrl;
        lastError.value = null;
        setLastCheckedAt(checkedAt);
        dialogDismissed.value = false;

        const nextInfo = updateInfo.value;
        if (!silent && !nextInfo) {
          notificationStore.addNotice("success", t("app.update.upToDateNotice"), {
            operation: "appUpdateCheck",
            replaceOperation: true,
          });
        }

        return nextInfo;
      } catch (error) {
        const normalized = normalizeAppError(error);
        lastError.value = normalized.message;
        setLastCheckedAt(checkedAt);

        if (!silent) {
          notificationStore.addNotice(
            "error",
            t("app.update.checkFailed", normalized.message),
            {
              code: normalized.code,
              operation: "appUpdateCheck",
              replaceOperation: true,
              skipConsoleLog: true,
            },
          );
        }

        return null;
      } finally {
        checking.value = false;
        activeCheckPromise = null;
      }
    })();

    return activeCheckPromise;
  }

  function dismissDialog() {
    dialogDismissed.value = true;
  }

  return {
    manifest,
    currentVersion,
    currentChannel,
    currentIsExperimental,
    preferredUpdateChannel,
    updateChannel,
    sourceKind,
    sourceBaseUrl,
    sourceLabel,
    lastCheckedAt,
    lastError,
    checking,
    dialogDismissed,
    updateInfo,
    hasUpdate,
    ensureCurrentVersion,
    checkForUpdates,
    setUpdateChannel,
    dismissDialog,
  };
});
