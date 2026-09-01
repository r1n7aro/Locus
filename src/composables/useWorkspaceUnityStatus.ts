import { confirm } from "@tauri-apps/plugin-dialog";
import {
  computed,
  onScopeDispose,
  ref,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import type { RuntimeUnsubscribe } from "../services/locusRuntime";
import type { WorkspaceRef } from "../services/project";
import {
  checkUnityConnectionStatus,
  checkUnityPlugin,
  checkUnityPluginInstallPlan,
  installUnityPlugin,
  launchUnityProject,
  subscribeWorkspaceUnityStatus,
  type UnityLaunchResult,
} from "../services/unity";
import type {
  AppErrorPayload,
  PluginStatus,
  UnityConnectionStatus,
} from "../types";

export type WorkspaceUnityLaunchState = "idle" | "starting" | "waitingConnection";
export type WorkspaceUnityPluginNotice = "missing" | "outdated" | null;

interface WorkspaceUnityStatusOptions {
  workspaceRef: MaybeRefOrGetter<WorkspaceRef | null>;
  enabled?: MaybeRefOrGetter<boolean>;
  onError?: (error: AppErrorPayload, operation: string) => void;
}

const CONNECTION_POLL_MS = 1500;
const CONNECTION_TIMEOUT_MS = 120_000;

function workspaceRefKey(workspaceRef: WorkspaceRef | null): string {
  if (!workspaceRef) return "";
  return `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}`;
}

function pluginNotice(status: PluginStatus): WorkspaceUnityPluginNotice {
  return status.status === "missing" || status.status === "outdated" ? status.status : null;
}

function launchedConnectionStatus(
  result: UnityLaunchResult,
  previous: UnityConnectionStatus | null,
): UnityConnectionStatus {
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
    pipeName: previous?.pipeName ?? "",
    latencyMs: null,
    reconnectAttempts: 0,
    lastError: null,
    backgroundHook: previous?.backgroundHook ?? {
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

export function useWorkspaceUnityStatus(options: WorkspaceUnityStatusOptions) {
  const connected = ref(false);
  const connectionStatus = ref<UnityConnectionStatus | null>(null);
  const pluginStatus = ref<WorkspaceUnityPluginNotice>(null);
  const pluginInstalling = ref(false);
  const launchState = ref<WorkspaceUnityLaunchState>("idle");
  const launching = computed(() => launchState.value === "starting");
  const enabled = computed(() => options.enabled == null || toValue(options.enabled));
  const currentWorkspaceRef = computed(() => toValue(options.workspaceRef));
  const bindingKey = computed(() => (
    enabled.value ? workspaceRefKey(currentWorkspaceRef.value) : ""
  ));

  let bindingVersion = 0;
  let connectionRevision = 0;
  let pluginRevision = 0;
  let unsubscribe: RuntimeUnsubscribe | null = null;
  let launchPollTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  let launchWaitStartedAt = 0;

  function isCurrentBinding(workspaceRef: WorkspaceRef, version: number): boolean {
    return version === bindingVersion
      && enabled.value
      && workspaceRefKey(currentWorkspaceRef.value) === workspaceRefKey(workspaceRef);
  }

  function clearLaunchPoll() {
    if (launchPollTimer) globalThis.clearTimeout(launchPollTimer);
    launchPollTimer = null;
    launchWaitStartedAt = 0;
  }

  function resetLaunchState() {
    clearLaunchPoll();
    launchState.value = "idle";
  }

  function setConnected(value: boolean) {
    connectionRevision += 1;
    connected.value = value;
    if (value) resetLaunchState();
  }

  function setConnectionStatus(status: UnityConnectionStatus) {
    connectionStatus.value = status;
    setConnected(status.connected);
  }

  function setPluginStatus(status: PluginStatus) {
    pluginRevision += 1;
    pluginStatus.value = pluginNotice(status);
  }

  function clearState() {
    connectionRevision += 1;
    pluginRevision += 1;
    connected.value = false;
    connectionStatus.value = null;
    pluginStatus.value = null;
    pluginInstalling.value = false;
    resetLaunchState();
  }

  async function refresh(workspaceRef = currentWorkspaceRef.value): Promise<void> {
    if (!workspaceRef || !enabled.value) return;
    const scopedRef = { ...workspaceRef };
    const version = bindingVersion;
    const connectionRequestRevision = connectionRevision;
    const pluginRequestRevision = pluginRevision;
    const [connectionResult, pluginResult] = await Promise.allSettled([
      checkUnityConnectionStatus(scopedRef),
      checkUnityPlugin(scopedRef),
    ]);
    if (!isCurrentBinding(scopedRef, version)) return;
    if (connectionResult.status === "fulfilled" && connectionRequestRevision === connectionRevision) {
      setConnected(connectionResult.value.connected);
    }
    if (pluginResult.status === "fulfilled" && pluginRequestRevision === pluginRevision) {
      setPluginStatus(pluginResult.value);
    }
  }

  async function bindWorkspace(): Promise<void> {
    const version = ++bindingVersion;
    unsubscribe?.();
    unsubscribe = null;
    clearState();

    const workspaceRef = currentWorkspaceRef.value;
    if (!workspaceRef || !enabled.value) return;
    const scopedRef = { ...workspaceRef };
    try {
      const release = await subscribeWorkspaceUnityStatus(scopedRef, {
        onConnection(value) {
          if (isCurrentBinding(scopedRef, version)) setConnected(value);
        },
        onConnectionDetail(status) {
          if (isCurrentBinding(scopedRef, version)) setConnectionStatus(status);
        },
        onPluginStatus(status) {
          if (isCurrentBinding(scopedRef, version)) setPluginStatus(status);
        },
      });
      if (!isCurrentBinding(scopedRef, version)) {
        release();
        return;
      }
      unsubscribe = release;
    } catch (error) {
      if (isCurrentBinding(scopedRef, version)) {
        console.warn("[Unity] failed to subscribe scoped status:", normalizeAppError(error));
      }
    }
    try {
      await refresh(scopedRef);
    } catch (error) {
      if (isCurrentBinding(scopedRef, version)) {
        console.warn("[Unity] failed to load scoped status:", normalizeAppError(error));
      }
    }
  }

  function scheduleConnectionPoll(workspaceRef: WorkspaceRef, version: number) {
    clearLaunchPoll();
    launchWaitStartedAt = Date.now();
    const poll = async () => {
      if (!isCurrentBinding(workspaceRef, version) || launchState.value !== "waitingConnection") return;
      try {
        const status = await checkUnityConnectionStatus(workspaceRef);
        if (!isCurrentBinding(workspaceRef, version)) return;
        setConnected(status.connected);
      } catch {
        // The routed connection event remains authoritative while polling recovers.
      }
      if (connected.value || launchState.value !== "waitingConnection") return;
      if (Date.now() - launchWaitStartedAt >= CONNECTION_TIMEOUT_MS) {
        resetLaunchState();
        return;
      }
      launchPollTimer = globalThis.setTimeout(poll, CONNECTION_POLL_MS);
    };
    launchPollTimer = globalThis.setTimeout(poll, CONNECTION_POLL_MS);
  }

  async function launch(): Promise<boolean> {
    const workspaceRef = currentWorkspaceRef.value;
    if (!workspaceRef || !enabled.value || connected.value || launchState.value !== "idle") return false;
    const scopedRef = { ...workspaceRef };
    const version = bindingVersion;
    launchState.value = "starting";
    try {
      const result = await launchUnityProject(scopedRef);
      if (!isCurrentBinding(scopedRef, version)) return false;
      if (connected.value) {
        resetLaunchState();
        return true;
      }
      connectionStatus.value = launchedConnectionStatus(result, connectionStatus.value);
      launchState.value = "waitingConnection";
      scheduleConnectionPoll(scopedRef, version);
      return true;
    } catch (error) {
      if (!isCurrentBinding(scopedRef, version)) return false;
      resetLaunchState();
      const normalized = normalizeAppError(error);
      options.onError?.(normalized, "launch_unity_project");
      return false;
    }
  }

  async function installPlugin(): Promise<boolean> {
    const workspaceRef = currentWorkspaceRef.value;
    if (!workspaceRef || !enabled.value || pluginInstalling.value) return false;
    const scopedRef = { ...workspaceRef };
    const version = bindingVersion;
    let forceCloseUnity = false;
    try {
      const plan = await checkUnityPluginInstallPlan(scopedRef);
      if (!isCurrentBinding(scopedRef, version)) return false;
      if (plan.dllUpdateRequired && plan.unityRunning) {
        const approved = await confirm(t("app.plugin.closeUnityConfirmMessage"), {
          title: t("app.plugin.closeUnityConfirmTitle"),
          kind: "warning",
          okLabel: t("app.plugin.closeUnityConfirmAction"),
          cancelLabel: t("common.cancel"),
        });
        if (!approved || !isCurrentBinding(scopedRef, version)) return false;
        forceCloseUnity = true;
      }
    } catch (error) {
      if (!isCurrentBinding(scopedRef, version)) return false;
      console.warn("[Unity] failed to check scoped plugin install plan:", normalizeAppError(error));
    }

    pluginInstalling.value = true;
    try {
      await installUnityPlugin(scopedRef, { forceCloseUnity });
      if (!isCurrentBinding(scopedRef, version)) return false;
      setPluginStatus({ status: "upToDate" });
      return true;
    } catch (error) {
      if (!isCurrentBinding(scopedRef, version)) return false;
      const normalized = normalizeAppError(error);
      options.onError?.(normalized, "install_unity_plugin");
      return false;
    } finally {
      if (isCurrentBinding(scopedRef, version)) pluginInstalling.value = false;
    }
  }

  watch(bindingKey, () => {
    void bindWorkspace();
  }, { immediate: true });

  onScopeDispose(() => {
    bindingVersion += 1;
    unsubscribe?.();
    unsubscribe = null;
    clearLaunchPoll();
  });

  return {
    connected,
    connectionStatus,
    pluginStatus,
    pluginInstalling,
    launchState,
    launching,
    refresh,
    launch,
    installPlugin,
  };
}
