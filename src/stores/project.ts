import { ref, computed } from "vue";
import { defineStore } from "pinia";
import * as projectService from "../services/project";
import * as unityService from "../services/unity";
import { assetDbOverview, assetDbScan } from "../services/asset";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "./notification";
import { t } from "../i18n";
import type { AssetDbOverview, AssetDbScanEvent, ScanStats, PluginStatus } from "../types";

type PluginNoticeStatus = "missing" | "outdated";
export type UnityLaunchState = "idle" | "starting" | "waitingConnection";

const PLUGIN_STATUS_NOTICE_OPERATION = "unity-plugin-status";
const UNITY_LAUNCH_CONNECTION_POLL_MS = 1500;
const UNITY_LAUNCH_WAIT_TIMEOUT_MS = 120_000;

export const useProjectStore = defineStore("project", () => {
  const workingDir = ref("");
  const recentDirs = ref<string[]>([]);
  const unityConnected = ref(false);
  const scanPhase = ref<AssetDbScanEvent | null>(null);
  const lastScanStats = ref<ScanStats | null>(null);
  const pluginToast = ref<"missing" | "outdated" | null>(null);
  const pluginInstalling = ref(false);
  const unityLaunchState = ref<UnityLaunchState>("idle");
  const unityLaunching = computed(() => unityLaunchState.value === "starting");
  let scanInFlight = false;
  let unityLaunchPollTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  let unityLaunchWaitStartedAt = 0;

  const isUnityProject = computed(() => workingDir.value.length > 0);

  function pluginStatusLabel(status: PluginNoticeStatus): string {
    return status === "missing" ? t("app.plugin.notInstalled") : t("app.plugin.needUpdate");
  }

  function setPluginToast(status: PluginNoticeStatus | null) {
    pluginToast.value = status;
    const notificationStore = useNotificationStore();
    if (status) {
      notificationStore.addNotice("error", pluginStatusLabel(status), {
        operation: PLUGIN_STATUS_NOTICE_OPERATION,
        replaceOperation: true,
        skipConsoleLog: true,
      });
    } else {
      notificationStore.clearByOperation(PLUGIN_STATUS_NOTICE_OPERATION);
    }
  }

  function isScanRunning(phase: AssetDbScanEvent | null): boolean {
    return phase != null && phase.phase !== "done" && phase.phase !== "error";
  }

  function clearUnityLaunchPoll() {
    if (unityLaunchPollTimer) {
      globalThis.clearTimeout(unityLaunchPollTimer);
      unityLaunchPollTimer = null;
    }
    unityLaunchWaitStartedAt = 0;
  }

  function resetUnityLaunchState() {
    clearUnityLaunchPoll();
    unityLaunchState.value = "idle";
  }

  function setUnityConnected(connected: boolean) {
    unityConnected.value = connected;
    if (connected) {
      resetUnityLaunchState();
    }
  }

  function scheduleUnityLaunchConnectionCheck(delayMs = UNITY_LAUNCH_CONNECTION_POLL_MS) {
    if (unityLaunchPollTimer) {
      globalThis.clearTimeout(unityLaunchPollTimer);
    }
    unityLaunchPollTimer = globalThis.setTimeout(() => {
      unityLaunchPollTimer = null;
      void checkUnityConnectionAfterLaunch();
    }, delayMs);
  }

  async function checkUnityConnectionAfterLaunch() {
    await checkUnityConnection();
    if (unityConnected.value || unityLaunchState.value !== "waitingConnection") return;

    if (
      unityLaunchWaitStartedAt > 0
      && Date.now() - unityLaunchWaitStartedAt >= UNITY_LAUNCH_WAIT_TIMEOUT_MS
    ) {
      resetUnityLaunchState();
      return;
    }

    scheduleUnityLaunchConnectionCheck();
  }

  function minimalStatsFromOverview(overview: AssetDbOverview): ScanStats {
    return {
      dirsScanned: 0,
      metaFilesFound: 0,
      yamlAssetsFound: 0,
      nodesAdded: overview.nodes,
      edgesAdded: overview.edges,
      nodesUpdated: 0,
      nodesDeleted: 0,
      parseFailures:
        overview.assetRisks.find((entry) => entry.kind === "parseFailures")?.count ?? 0,
      elapsedMs: overview.lastScanDurationMs ?? 0,
      duplicateGuids: overview.duplicateGuids,
    };
  }

  function shouldAutoBuildFromOverview(overview: AssetDbOverview): boolean {
    if (!workingDir.value.trim()) return false;
    if (scanInFlight || isScanRunning(scanPhase.value)) return false;
    if (overview.status === "none") return true;
    const phase = overview.currentScanPhase;
    return phase?.phase === "error"
      && phase.error.code.startsWith("ref_graph.rescan_required.");
  }

  async function loadWorkingDir() {
    try {
      workingDir.value = await projectService.getWorkingDir();
    } catch (e) {
      console.error("get_working_dir failed:", e);
    }
  }

  async function setWorkingDir(path: string): Promise<string> {
    const result = await projectService.setWorkingDir(path);
    resetUnityLaunchState();
    workingDir.value = result;
    scanPhase.value = null;
    lastScanStats.value = null;
    return result;
  }

  async function loadRecentDirs() {
    try {
      recentDirs.value = await projectService.listRecentDirs();
    } catch (e) {
      console.error("list_recent_dirs failed:", e);
    }
  }

  async function startScan() {
    if (scanInFlight || isScanRunning(scanPhase.value)) return;
    scanInFlight = true;
    scanPhase.value = { phase: "dirScan" };
    try {
      const stats = await assetDbScan();
      lastScanStats.value = stats;
    } catch (e) {
      const err = normalizeAppError(e);
      console.error("ref_graph_scan failed:", err);
      scanPhase.value = { phase: "error", error: err };
      useNotificationStore().addNotice("error", err.message, {
        code: err.code,
        operation: "ref_graph_scan",
        skipConsoleLog: true,
      });
    } finally {
      scanInFlight = false;
    }
  }

  async function checkUnityConnection() {
    try {
      setUnityConnected(await unityService.checkUnityConnection());
    } catch {
      setUnityConnected(false);
    }
  }

  async function checkUnityPlugin() {
    try {
      const ps = await unityService.checkUnityPlugin();
      setPluginToast((ps.status === "missing" || ps.status === "outdated") ? ps.status : null);
    } catch {
      setPluginToast(null);
    }
  }

  async function installPlugin() {
    pluginInstalling.value = true;
    try {
      await unityService.installUnityPlugin();
    } catch (e) {
      console.error("install_unity_plugin failed:", e);
    } finally {
      pluginInstalling.value = false;
    }
  }

  async function launchUnityProject() {
    if (unityLaunchState.value !== "idle" || unityConnected.value) return;
    clearUnityLaunchPoll();
    unityLaunchState.value = "starting";
    try {
      await unityService.launchUnityProject();
      if (unityConnected.value) {
        resetUnityLaunchState();
        return;
      }
      unityLaunchState.value = "waitingConnection";
      unityLaunchWaitStartedAt = Date.now();
      scheduleUnityLaunchConnectionCheck();
    } catch (e) {
      resetUnityLaunchState();
      const err = normalizeAppError(e);
      console.error("launch_unity_project failed:", err);
      useNotificationStore().addNotice("error", t("app.unityLaunchFailed", err.message), {
        code: err.code,
        operation: "launch_unity_project",
        skipConsoleLog: true,
      });
    }
  }

  async function loadAssetDbStatus() {
    try {
      const overview = await assetDbOverview();
      const currentPhase = overview.currentScanPhase ?? null;

      if (currentPhase) {
        scanPhase.value = currentPhase;
      } else if (!isScanRunning(scanPhase.value)) {
        scanPhase.value = null;
      }

      if (overview.lastScanStats) {
        lastScanStats.value = overview.lastScanStats;
      } else if (overview.status === "indexed") {
        lastScanStats.value = minimalStatsFromOverview(overview);
      } else if (overview.status === "none") {
        lastScanStats.value = null;
      }

      if (overview.status === "indexed") {
        console.log(
          "[AssetDb] loaded from existing DB:",
          overview.nodes,
          "assets,",
          overview.edges,
          "edges",
        );
      }

      if (shouldAutoBuildFromOverview(overview)) {
        void startScan();
      }
    } catch {
      if (!isScanRunning(scanPhase.value)) {
        lastScanStats.value = null;
      }
    }
  }

  function resetWorkspaceState() {
    workingDir.value = "";
    recentDirs.value = [];
    unityConnected.value = false;
    scanPhase.value = null;
    lastScanStats.value = null;
    scanInFlight = false;
    setPluginToast(null);
    pluginInstalling.value = false;
    resetUnityLaunchState();
  }

  function handleUnityConnectionStatus(connected: boolean) {
    setUnityConnected(connected);
  }

  function handleScanEvent(event: AssetDbScanEvent) {
    scanPhase.value = event;
    if (event.phase === "done") {
      scanInFlight = false;
      lastScanStats.value = event.stats;
    } else if (event.phase === "error") {
      scanInFlight = false;
      console.error("[AssetDb] scan error:", event.error);
      useNotificationStore().addNotice("error", event.error.message, {
        code: event.error.code,
        operation: "ref_graph_scan",
        skipConsoleLog: true,
      });
    }
  }

  function handlePluginStatus(status: PluginStatus) {
    const s = status.status;
    if (s === "missing" || s === "outdated") {
      setPluginToast(s);
    } else {
      setPluginToast(null);
    }
  }

  return {
    workingDir,
    recentDirs,
    unityConnected,
    scanPhase,
    lastScanStats,
    pluginToast,
    pluginInstalling,
    unityLaunchState,
    unityLaunching,
    isUnityProject,
    loadWorkingDir,
    setWorkingDir,
    loadRecentDirs,
    startScan,
    checkUnityConnection,
    checkUnityPlugin,
    installPlugin,
    launchUnityProject,
    loadAssetDbStatus,
    resetWorkspaceState,
    handleUnityConnectionStatus,
    handleScanEvent,
    handlePluginStatus,
  };
});
