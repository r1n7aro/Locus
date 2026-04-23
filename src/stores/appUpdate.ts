import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { locale, t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getAppRuntimeVersion } from "../services/appVersion";
import {
  fetchAppUpdateManifest,
  resolveAppUpdateInfo,
} from "../services/appUpdate";
import type { AppUpdateInfo, AppUpdateManifest, AppUpdateSourceKind } from "../types";
import { useNotificationStore } from "./notification";

const LAST_CHECKED_AT_STORAGE_KEY = "locus-app-update-last-checked-at";

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

export const useAppUpdateStore = defineStore("appUpdate", () => {
  const manifest = ref<AppUpdateManifest | null>(null);
  const currentVersion = ref("");
  const sourceKind = ref<AppUpdateSourceKind | null>(null);
  const sourceBaseUrl = ref("");
  const lastCheckedAt = ref<number | null>(loadLastCheckedAt());
  const lastError = ref<string | null>(null);
  const checking = ref(false);
  const dialogDismissed = ref(false);

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

  async function ensureCurrentVersion(): Promise<string> {
    if (currentVersion.value) {
      return currentVersion.value;
    }

    if (!currentVersionPromise) {
      currentVersionPromise = getAppRuntimeVersion()
        .then((version) => {
          currentVersion.value = version;
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
        const [version, nextManifestResult] = await Promise.all([
          ensureCurrentVersion(),
          fetchAppUpdateManifest({ throwOnError: !silent }),
        ]);

        currentVersion.value = version;
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
    dismissDialog,
  };
});
