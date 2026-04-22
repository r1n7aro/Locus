import { ref, computed } from "vue";
import { defineStore } from "pinia";
import * as projectService from "../services/project";
import * as unityService from "../services/unity";
import { assetDbScan, assetDbStatus } from "../services/asset";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "./notification";
import type { AssetDbScanEvent, ScanStats, PluginStatus } from "../types";

export const useProjectStore = defineStore("project", () => {
  const workingDir = ref("");
  const recentDirs = ref<string[]>([]);
  const unityConnected = ref(false);
  const scanPhase = ref<AssetDbScanEvent | null>(null);
  const lastScanStats = ref<ScanStats | null>(null);
  const pluginToast = ref<"missing" | "outdated" | null>(null);
  const pluginInstalling = ref(false);

  const isUnityProject = computed(() => workingDir.value.length > 0);

  async function loadWorkingDir() {
    try {
      workingDir.value = await projectService.getWorkingDir();
    } catch (e) {
      console.error("get_working_dir failed:", e);
    }
  }

  async function setWorkingDir(path: string): Promise<string> {
    const result = await projectService.setWorkingDir(path);
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
    }
  }

  async function checkUnityConnection() {
    try {
      unityConnected.value = await unityService.checkUnityConnection();
    } catch {
      unityConnected.value = false;
    }
  }

  async function checkUnityPlugin() {
    try {
      const ps = await unityService.checkUnityPlugin();
      pluginToast.value = (ps.status === "missing" || ps.status === "outdated") ? ps.status : null;
    } catch {
      pluginToast.value = null;
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

  async function loadAssetDbStatus() {
    try {
      const stats = await assetDbStatus();
      if (stats) {
        lastScanStats.value = stats;
        console.log("[AssetDb] loaded from existing DB:", stats.nodesAdded, "assets,", stats.edgesAdded, "edges");
      } else {
        lastScanStats.value = null;
      }
    } catch { /* ignore */ }
  }

  function resetWorkspaceState() {
    workingDir.value = "";
    recentDirs.value = [];
    unityConnected.value = false;
    scanPhase.value = null;
    lastScanStats.value = null;
    pluginToast.value = null;
    pluginInstalling.value = false;
  }

  function handleScanEvent(event: AssetDbScanEvent) {
    scanPhase.value = event;
    if (event.phase === "done") {
      lastScanStats.value = event.stats;
    } else if (event.phase === "error") {
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
      pluginToast.value = s;
    } else {
      pluginToast.value = null;
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
    isUnityProject,
    loadWorkingDir,
    setWorkingDir,
    loadRecentDirs,
    startScan,
    checkUnityConnection,
    checkUnityPlugin,
    installPlugin,
    loadAssetDbStatus,
    resetWorkspaceState,
    handleScanEvent,
    handlePluginStatus,
  };
});
