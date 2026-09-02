import { ref, computed, watch } from "vue";
import { defineStore } from "pinia";
import { confirm } from "@tauri-apps/plugin-dialog";
import * as projectService from "../services/project";
import * as unityService from "../services/unity";
import { extraWorkdirsGet, extraWorkdirsMap } from "../services/extraWorkdirs";
import type { ExtraWorkdirStatus } from "../services/extraWorkdirs";
import { assetDbLightStatus, assetDbScanStart } from "../services/asset";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "./notification";
import { useWorkspaceContextStore } from "./workspaceContext";
import { t } from "../i18n";
import type { UnityLaunchResult } from "../services/unity";
import type {
  AssetDbLightStatus,
  AssetDbScanEvent,
  PluginStatus,
  ScanStats,
  UnityConnectionStatus,
} from "../types";

type PluginNoticeStatus = "missing" | "outdated";
export type UnityLaunchState = "idle" | "starting" | "waitingConnection";

const PLUGIN_STATUS_NOTICE_OPERATION = "unity-plugin-status";
const UNITY_BACKGROUND_HOOK_NOTICE_OPERATION = "unity-background-hook";
const EXTRA_WORKDIRS_MISSING_NOTICE_OPERATION = "extra-workdirs-missing";
const UNITY_LAUNCH_CONNECTION_POLL_MS = 1500;
const UNITY_LAUNCH_WAIT_TIMEOUT_MS = 120_000;

export const useProjectStore = defineStore("project", () => {
  const workspaceContextStore = useWorkspaceContextStore();
  const workingDir = computed(() => workspaceContextStore.focusedRoot);
  const recentDirs = ref<string[]>([]);
  const extraWorkdirs = ref<Record<string, ExtraWorkdirStatus[]>>({});
  const unityConnected = ref(false);
  const unityConnectionStatus = ref<UnityConnectionStatus | null>(null);
  const scanPhase = ref<AssetDbScanEvent | null>(null);
  const lastScanStats = ref<ScanStats | null>(null);
  const pluginToast = ref<"missing" | "outdated" | null>(null);
  const pluginInstalling = ref(false);
  const unityLaunchState = ref<UnityLaunchState>("idle");
  const unityLaunching = computed(() => unityLaunchState.value === "starting");
  let scanInFlight = false;
  let assetStatusRequestSeq = 0;
  let pluginStatusRevision = 0;
  let unityLaunchPollTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  let unityLaunchWaitStartedAt = 0;
  const unityConnectionChecksInFlight = new Map<string, Promise<void>>();
  const pluginStatusChecksInFlight = new Map<string, Promise<void>>();

  function requireWorkspaceRef(): projectService.WorkspaceRef {
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) {
      throw new Error("A focused workspace checkout is required.");
    }
    return {
      checkoutId: workspaceRef.checkoutId,
      expectedGeneration: workspaceRef.expectedGeneration ?? undefined,
    };
  }

  function workspaceRefKey(workspaceRef: projectService.WorkspaceRef): string {
    return `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}`;
  }

  function isFocusedWorkspaceRef(workspaceRef: projectService.WorkspaceRef): boolean {
    const focused = workspaceContextStore.focusedWorkspaceRef;
    return focused?.checkoutId === workspaceRef.checkoutId
      && focused.expectedGeneration === workspaceRef.expectedGeneration;
  }

  const detectedServices = computed(() => (
    workspaceContextStore.focusedRuntime?.detectedServices ?? []
  ));
  const isUnityProject = computed(() => detectedServices.value.includes("unity"));

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

  function applyPluginStatus(status: PluginStatus | null) {
    pluginStatusRevision += 1;
    const notice = status?.status;
    setPluginToast(notice === "missing" || notice === "outdated" ? notice : null);
  }

  function clearUnityOnlyNotices() {
    const notificationStore = useNotificationStore();
    for (const operation of [
      PLUGIN_STATUS_NOTICE_OPERATION,
      UNITY_BACKGROUND_HOOK_NOTICE_OPERATION,
      "ref_graph_scan_start",
      "ref_graph_scan",
    ]) {
      notificationStore.clearByOperation(operation, { includeErrors: true });
    }
  }

  function isScanRunning(phase: AssetDbScanEvent | null): boolean {
    return phase != null
      && phase.phase !== "done"
      && phase.phase !== "reconcileDone"
      && phase.phase !== "error";
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

  function setUnityConnectionStatus(status: UnityConnectionStatus) {
    unityConnectionStatus.value = status;
    setUnityConnected(status.connected);
    const hook = status.backgroundHook;
    const notificationStore = useNotificationStore();
    if (hook?.enabled && hook.state === "failed" && hook.error) {
      notificationStore.addNotice("error", hook.error, {
        operation: UNITY_BACKGROUND_HOOK_NOTICE_OPERATION,
        replaceOperation: true,
        skipConsoleLog: true,
      });
    } else if (hook?.state === "patched" || hook?.state === "disabled") {
      notificationStore.clearByOperation(UNITY_BACKGROUND_HOOK_NOTICE_OPERATION);
    }
  }

  function connectionStatusFromLaunchResult(result: UnityLaunchResult): UnityConnectionStatus {
    const now = Date.now();
    return {
      connected: false,
      editorStatus: "disconnected",
      controlChannelState: "starting",
      editorProcessState: "running",
      editorProcessId: result.processId,
      editorProcessPath: result.editorPath,
      editorProjectPath: result.projectPath,
      launchMode: result.mode,
      headless: result.mode === "headless",
      processCheckedAtMs: now,
      processLastError: null,
      pipeName: unityConnectionStatus.value?.pipeName ?? "",
      latencyMs: null,
      reconnectAttempts: 0,
      lastError: null,
      backgroundHook: unityConnectionStatus.value?.backgroundHook ?? {
        enabled: false,
        supported: false,
        state: "inactive",
        patched: false,
        processId: null,
        editorProcessPath: null,
        symbolCount: 0,
        error: null,
        updatedAtMs: now,
      },
      checkedAtMs: now,
    };
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

  function minimalStatsFromLightStatus(status: AssetDbLightStatus): ScanStats {
    return {
      dirsScanned: 0,
      metaFilesFound: 0,
      yamlAssetsFound: 0,
      nodesAdded: status.nodes,
      edgesAdded: status.edges,
      nodesUpdated: 0,
      nodesDeleted: 0,
      parseFailures: 0,
      elapsedMs: status.lastScanDurationMs ?? 0,
      duplicateGuids: {
        groupCount: 0,
        pathCount: 0,
        assetsOnlyGroups: 0,
        packagesOnlyGroups: 0,
        crossRootGroups: 0,
      },
    };
  }

  function shouldAutoBuildFromLightStatus(status: AssetDbLightStatus): boolean {
    if (!isUnityProject.value) return false;
    if (scanInFlight || isScanRunning(scanPhase.value)) return false;
    if (status.status === "none") return true;
    const phase = status.currentScanPhase;
    return phase?.phase === "error"
      && phase.error.code.startsWith("ref_graph.rescan_required.");
  }

  async function loadRecentDirs() {
    try {
      recentDirs.value = await projectService.listRecentDirs();
      void loadExtraWorkdirs();
    } catch (e) {
      console.error("list_recent_dirs failed:", e);
    }
  }

  async function loadExtraWorkdirs() {
    const pathKeys = new Set([...recentDirs.value, workingDir.value].filter(Boolean).map(
      (path) => path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase(),
    ));
    const workspaceRefs: projectService.WorkspaceRef[] = [];
    for (const checkout of Object.values(workspaceContextStore.checkoutsById)) {
      const pathKey = checkout.root
        .trim()
        .replace(/\\/g, "/")
        .replace(/\/+$/, "")
        .toLowerCase();
      if (!checkout.runtime || !pathKeys.has(pathKey)) continue;
      workspaceRefs.push({
        checkoutId: checkout.checkoutId,
        expectedGeneration: checkout.runtime.workspaceGeneration,
      });
    }
    if (workspaceRefs.length === 0) {
      extraWorkdirs.value = {};
      return;
    }
    try {
      extraWorkdirs.value = await extraWorkdirsMap(workspaceRefs);
    } catch (e) {
      console.error("extra_workdirs_map failed:", e);
    }
  }

  /** Validates the current workspace's attached directories and keeps a
   * warning notice alive while any of them is missing on disk. */
  async function checkCurrentExtraWorkdirs() {
    const dir = workingDir.value;
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!dir || !workspaceRef) return;
    try {
      const statuses = await extraWorkdirsGet(workspaceRef);
      if (
        workingDir.value !== dir
        || workspaceContextStore.focusedWorkspaceRef?.checkoutId !== workspaceRef.checkoutId
        || workspaceContextStore.focusedWorkspaceRef?.expectedGeneration
          !== workspaceRef.expectedGeneration
      ) return;
      const missing = statuses.filter((status) => !status.exists);
      const notificationStore = useNotificationStore();
      if (missing.length > 0) {
        notificationStore.addNotice(
          "warning",
          t("app.dir.extraWorkdirsMissing", missing.map((status) => status.path).join(", ")),
          {
            operation: EXTRA_WORKDIRS_MISSING_NOTICE_OPERATION,
            replaceOperation: true,
          },
        );
      } else {
        notificationStore.clearByOperation(EXTRA_WORKDIRS_MISSING_NOTICE_OPERATION);
      }
    } catch (e) {
      console.error("extra_workdirs_get failed:", e);
    }
  }

  async function handleExtraWorkdirsUpdated(workspacePath: string) {
    await loadExtraWorkdirs();
    if (workspacePath && workspacePath === workingDir.value) {
      await checkCurrentExtraWorkdirs();
    }
  }

  async function removeRecentDir(path: string) {
    recentDirs.value = await projectService.removeRecentDir(path);
  }

  async function openDirInFileExplorer(path: string) {
    await projectService.openDirInFileExplorer(path);
  }

  async function startScan() {
    if (!isUnityProject.value) return;
    if (scanInFlight || isScanRunning(scanPhase.value)) return;
    scanInFlight = true;
    scanPhase.value = { phase: "dirScan" };
    try {
      const result = await assetDbScanStart(requireWorkspaceRef());
      if (!result.started && !result.alreadyRunning) {
        scanInFlight = false;
        scanPhase.value = null;
      }
    } catch (e) {
      const err = normalizeAppError(e);
      scanInFlight = false;
      console.error("ref_graph_scan_start failed:", err);
      scanPhase.value = { phase: "error", error: err };
      useNotificationStore().addNotice("error", err.message, {
        code: err.code,
        operation: "ref_graph_scan_start",
        skipConsoleLog: true,
      });
    }
  }

  async function checkUnityConnection() {
    if (!isUnityProject.value) {
      setUnityConnected(false);
      clearUnityOnlyNotices();
      return;
    }
    const workspaceRef = requireWorkspaceRef();
    const requestKey = workspaceRefKey(workspaceRef);
    const existing = unityConnectionChecksInFlight.get(requestKey);
    if (existing) return existing;
    const request = (async () => {
      try {
        await projectService.startWorkspaceUnityService(workspaceRef);
        const status = await unityService.checkUnityConnectionStatus(workspaceRef);
        if (isFocusedWorkspaceRef(workspaceRef)) {
          setUnityConnected(status.connected);
        }
      } catch {
        if (isFocusedWorkspaceRef(workspaceRef)) {
          setUnityConnected(false);
        }
      }
    })();
    unityConnectionChecksInFlight.set(requestKey, request);
    void request.then(() => {
      if (unityConnectionChecksInFlight.get(requestKey) === request) {
        unityConnectionChecksInFlight.delete(requestKey);
      }
    });
    return request;
  }

  async function checkUnityPlugin() {
    if (!isUnityProject.value) {
      applyPluginStatus(null);
      return;
    }
    const workspaceRef = requireWorkspaceRef();
    const requestKey = workspaceRefKey(workspaceRef);
    const existing = pluginStatusChecksInFlight.get(requestKey);
    if (existing) return existing;
    const requestRevision = pluginStatusRevision;
    const request = (async () => {
      try {
        const status = await unityService.checkUnityPlugin(workspaceRef);
        if (
          requestRevision === pluginStatusRevision
          && isFocusedWorkspaceRef(workspaceRef)
        ) {
          applyPluginStatus(status);
        }
      } catch (error) {
        if (isFocusedWorkspaceRef(workspaceRef)) {
          console.warn("check_unity_plugin failed:", error);
        }
      }
    })();
    pluginStatusChecksInFlight.set(requestKey, request);
    void request.then(() => {
      if (pluginStatusChecksInFlight.get(requestKey) === request) {
        pluginStatusChecksInFlight.delete(requestKey);
      }
    });
    return request;
  }

  async function installPlugin() {
    if (!isUnityProject.value) return;
    if (pluginInstalling.value) return;

    let forceCloseUnity = false;
    try {
      const plan = await unityService.checkUnityPluginInstallPlan(requireWorkspaceRef());
      if (plan.dllUpdateRequired && plan.unityRunning) {
        const confirmed = await confirm(t("app.plugin.closeUnityConfirmMessage"), {
          title: t("app.plugin.closeUnityConfirmTitle"),
          kind: "warning",
          okLabel: t("app.plugin.closeUnityConfirmAction"),
          cancelLabel: t("common.cancel"),
        });
        if (!confirmed) return;
        forceCloseUnity = true;
      }
    } catch (e) {
      console.warn("check_unity_plugin_install_plan failed:", e);
    }

    pluginInstalling.value = true;
    try {
      await unityService.installUnityPlugin(requireWorkspaceRef(), { forceCloseUnity });
    } catch (e) {
      console.error("install_unity_plugin failed:", e);
    } finally {
      pluginInstalling.value = false;
    }
  }

  async function launchUnityProject() {
    if (!isUnityProject.value) return;
    if (unityLaunchState.value !== "idle" || unityConnected.value) return;
    clearUnityLaunchPoll();
    unityLaunchState.value = "starting";
    try {
      const launch = await unityService.launchUnityProject(requireWorkspaceRef());
      setUnityConnectionStatus(connectionStatusFromLaunchResult(launch));
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
    if (!isUnityProject.value) {
      scanPhase.value = null;
      lastScanStats.value = null;
      clearUnityOnlyNotices();
      return;
    }
    const workspaceRef = requireWorkspaceRef();
    const requestSeq = ++assetStatusRequestSeq;
    try {
      const status = await assetDbLightStatus(workspaceRef);
      if (requestSeq !== assetStatusRequestSeq || !isFocusedWorkspaceRef(workspaceRef)) return;
      const currentPhase = status.currentScanPhase ?? null;

      if (currentPhase) {
        scanPhase.value = currentPhase;
      } else if (!isScanRunning(scanPhase.value)) {
        scanPhase.value = null;
      }

      if (status.lastScanStats) {
        lastScanStats.value = status.lastScanStats;
      } else if (status.status === "indexed") {
        lastScanStats.value = minimalStatsFromLightStatus(status);
      } else if (status.status === "none") {
        lastScanStats.value = null;
      }

      if (status.status === "indexed") {
        console.log(
          "[AssetDb] loaded from existing DB:",
          status.nodes,
          "assets,",
          status.edges,
          "edges",
        );
      }

      if (shouldAutoBuildFromLightStatus(status)) {
        void startScan();
      }
    } catch {
      if (requestSeq !== assetStatusRequestSeq || !isFocusedWorkspaceRef(workspaceRef)) return;
      if (!isScanRunning(scanPhase.value)) {
        lastScanStats.value = null;
      }
    }
  }

  function resetWorkspaceState() {
    recentDirs.value = [];
    extraWorkdirs.value = {};
    unityConnected.value = false;
    unityConnectionStatus.value = null;
    scanPhase.value = null;
    lastScanStats.value = null;
    scanInFlight = false;
    assetStatusRequestSeq += 1;
    unityConnectionChecksInFlight.clear();
    pluginStatusChecksInFlight.clear();
    applyPluginStatus(null);
    pluginInstalling.value = false;
    resetUnityLaunchState();
  }

  function resetFocusedWorkspaceState() {
    resetUnityLaunchState();
    unityConnected.value = false;
    unityConnectionStatus.value = null;
    scanPhase.value = null;
    lastScanStats.value = null;
    scanInFlight = false;
    unityConnectionChecksInFlight.clear();
    pluginStatusChecksInFlight.clear();
    applyPluginStatus(null);
    clearUnityOnlyNotices();
    pluginInstalling.value = false;
    if (workingDir.value) void checkCurrentExtraWorkdirs();
  }

  watch(
    () => {
      const scope = workspaceContextStore.focusedWorkspaceRef;
      return scope ? `${scope.checkoutId}:${scope.expectedGeneration ?? ""}` : "";
    },
    (nextScope, previousScope) => {
      if (previousScope !== undefined) resetFocusedWorkspaceState();
      if (nextScope && isUnityProject.value) void checkUnityPlugin();
    },
    { flush: "sync" },
  );

  function handleUnityConnectionStatus(connected: boolean) {
    if (!isUnityProject.value) return;
    setUnityConnected(connected);
  }

  function handleUnityConnectionStatusDetail(status: UnityConnectionStatus) {
    if (!isUnityProject.value) return;
    setUnityConnectionStatus(status);
  }

  function handleScanEvent(event: AssetDbScanEvent) {
    if (!isUnityProject.value) return;
    scanPhase.value = event;
    if (event.phase === "done") {
      scanInFlight = false;
      lastScanStats.value = event.stats;
    } else if (event.phase === "reconcileDone") {
      scanPhase.value = null;
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
    if (!isUnityProject.value) return;
    applyPluginStatus(status);
  }

  return {
    workingDir,
    recentDirs,
    extraWorkdirs,
    unityConnected,
    unityConnectionStatus,
    scanPhase,
    lastScanStats,
    pluginToast,
    pluginInstalling,
    unityLaunchState,
    unityLaunching,
    detectedServices,
    isUnityProject,
    requireWorkspaceRef,
    loadRecentDirs,
    loadExtraWorkdirs,
    checkCurrentExtraWorkdirs,
    handleExtraWorkdirsUpdated,
    removeRecentDir,
    openDirInFileExplorer,
    startScan,
    checkUnityConnection,
    checkUnityPlugin,
    installPlugin,
    launchUnityProject,
    loadAssetDbStatus,
    resetWorkspaceState,
    handleUnityConnectionStatus,
    handleUnityConnectionStatusDetail,
    handleScanEvent,
    handlePluginStatus,
  };
});
