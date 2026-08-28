import type {
  CodeAnalysisToolsConfig,
  CsharpCompileStatus,
  CsharpLspStatus,
  UnityNativeBrokerStatus,
  UnitySemanticState,
} from "../types";
import { ipcInvoke } from "./ipc";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
  type WorkspaceRef,
} from "./project";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

export function csharpLspGetStatus(workspaceRef: WorkspaceRef): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_get_status", { workspaceRef }, {
    operation: "csharpLspGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function csharpLspSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>(
    "csharp_lsp_set_enabled",
    { value, workspaceRef },
    { operation: "csharpLspSetEnabled", notify: false, throwOnError: true },
  );
}

export function csharpLspRestart(workspaceRef: WorkspaceRef): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_restart", { workspaceRef }, {
    operation: "csharpLspRestart",
    notify: false,
    throwOnError: true,
  });
}

export function codeAnalysisToolsGetConfig(): Promise<CodeAnalysisToolsConfig> {
  return ipcInvoke<CodeAnalysisToolsConfig>("code_analysis_tools_get_config", undefined, {
    operation: "codeAnalysisToolsGetConfig",
    notify: false,
    throwOnError: true,
  });
}

export function codeAnalysisToolsSetConfig(
  value: CodeAnalysisToolsConfig,
  workspaceRef: WorkspaceRef,
): Promise<CodeAnalysisToolsConfig> {
  return ipcInvoke<CodeAnalysisToolsConfig>(
    "code_analysis_tools_set_config",
    { value, workspaceRef },
    { operation: "codeAnalysisToolsSetConfig", notify: false, throwOnError: true },
  );
}

export function unitySidecarCompilerGetStatus(
  workspaceRef: WorkspaceRef,
): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>("unity_sidecar_compiler_get_status", { workspaceRef }, {
    operation: "unitySidecarCompilerGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unitySidecarCompilerSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_sidecar_compiler_set_enabled",
    { value, workspaceRef },
    { operation: "unitySidecarCompilerSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityNonPublicAccessSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_non_public_access_set_enabled",
    { value, workspaceRef },
    { operation: "unityNonPublicAccessSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityHotReloadSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_hot_reload_set_enabled",
    { value, workspaceRef },
    { operation: "unityHotReloadSetEnabled", notify: false, throwOnError: true },
  );
}

export interface HotReloadPreflight {
  connected: boolean;
  /** "debug" | "release" when readable; null when the editor is unreachable. */
  codeOptimization: string | null;
  /** Whether entering Play Mode reloads the domain (true = Unity default,
   * false = DisableDomainReload); null when unreadable / older plugin. */
  domainReloadOnPlay: boolean | null;
}

/** Enable-time check: the connected editor's Code Optimization, for the
 * Debug-mode gate the hot-reload toggles run before turning the feature on. */
export function unityHotReloadPreflight(workspaceRef: WorkspaceRef): Promise<HotReloadPreflight> {
  return ipcInvoke<HotReloadPreflight>("unity_hot_reload_preflight", { workspaceRef }, {
    operation: "unityHotReloadPreflight",
    notify: false,
    throwOnError: true,
  });
}

export interface CodeOptimizationResult {
  codeOptimization: string;
}

/** Switch the connected editor's Code Optimization to Debug (the auto-fix the
 * user confirms in the enable-time prompt). Triggers a Unity recompile. */
export function unityHotReloadSetCodeOptimizationDebug(
  workspaceRef: WorkspaceRef,
): Promise<CodeOptimizationResult> {
  return ipcInvoke<CodeOptimizationResult>(
    "unity_hot_reload_set_code_optimization_debug",
    { workspaceRef },
    {
      operation: "unityHotReloadSetCodeOptimizationDebug",
      notify: false,
      throwOnError: true,
    },
  );
}

/** Switch the connected editor's Code Optimization to an explicit level
 * ("debug" | "release"), from the hot-reload popover dropdown. Triggers a
 * Unity recompile. */
export function unityHotReloadSetCodeOptimization(
  level: "debug" | "release",
  workspaceRef: WorkspaceRef,
): Promise<CodeOptimizationResult> {
  return ipcInvoke<CodeOptimizationResult>(
    "unity_hot_reload_set_code_optimization",
    { level, workspaceRef },
    {
      operation: "unityHotReloadSetCodeOptimization",
      notify: false,
      throwOnError: true,
    },
  );
}

export interface PlayModeReloadResult {
  domainReloadOnPlay: boolean;
}

/** Set whether entering Play Mode reloads the domain (EditorSettings
 * enterPlayModeOptions / DisableDomainReload), from the hot-reload popover
 * toggle. Unlike the Code Optimization switch this does NOT trigger a Unity
 * recompile. */
export function unityHotReloadSetPlayModeReload(
  domainReload: boolean,
  workspaceRef: WorkspaceRef,
): Promise<PlayModeReloadResult> {
  return ipcInvoke<PlayModeReloadResult>(
    "unity_hot_reload_set_play_mode_reload",
    { domainReload, workspaceRef },
    {
      operation: "unityHotReloadSetPlayModeReload",
      notify: false,
      throwOnError: true,
    },
  );
}

export function unityRecompileRun(workspaceRef: WorkspaceRef): Promise<string> {
  return ipcInvoke<string>("unity_recompile_run", { workspaceRef }, {
    operation: "unityRecompileRun",
    notify: false,
    throwOnError: true,
  });
}

export interface HotReloadSelfTestEvent {
  running: boolean;
  finished: boolean;
  line?: string | null;
  passed: number;
  failed: number;
}

export function unityHotReloadSelfTestRun(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("unity_hot_reload_selftest_run", { workspaceRef }, {
    operation: "unityHotReloadSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityHotReloadSelfTest(
  workspaceRef: WorkspaceRef,
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<HotReloadSelfTestEvent>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "unity-hotreload-selftest") return;
      if (event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      handler(event.payload);
    },
  );
}

export type UnityStateProbeTier =
  | "disabled"
  | "inactive"
  | "passive"
  | "stack"
  | "cpu_only"
  | "inference"
  | "unsupported";

export interface UnityStateProbeStatus {
  checkoutId: string;
  workspaceGeneration: number;
  serviceInstanceId?: string | null;
  serviceGeneration?: number | null;
  enabled: boolean;
  supported: boolean;
  tier: UnityStateProbeTier;
  processId?: number | null;
  reloadSymbols: number;
  totalSymbols: number;
  lastPhase?: string | null;
  error?: string | null;
  updatedAtMs: number;
}

export function unityStateProbeGetStatus(workspaceRef: WorkspaceRef): Promise<UnityStateProbeStatus> {
  return ipcInvoke<UnityStateProbeStatus>("get_unity_state_probe_status", { workspaceRef }, {
    operation: "unityStateProbeGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unityStateProbeSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<UnityStateProbeStatus> {
  return ipcInvoke<UnityStateProbeStatus>(
    "set_unity_state_probe_enabled",
    { value, workspaceRef },
    { operation: "unityStateProbeSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityStateProbeSelfTestRun(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("unity_state_probe_selftest_run", { workspaceRef }, {
    operation: "unityStateProbeSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function unitySemanticStateGet(workspaceRef: WorkspaceRef): Promise<UnitySemanticState> {
  return ipcInvoke<UnitySemanticState>("get_unity_semantic_state", { workspaceRef }, {
    operation: "unitySemanticStateGet",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityStateProbeSelfTest(
  workspaceRef: WorkspaceRef,
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<HotReloadSelfTestEvent>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "unity-state-probe-selftest") return;
      if (event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      handler(event.payload);
    },
  );
}

export function unityNativeBridgeGetEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_native_bridge_enabled", undefined, {
    operation: "unityNativeBridgeGetEnabled",
    notify: false,
    throwOnError: true,
  });
}

export function unityNativeBridgeSetEnabled(
  value: boolean,
  workspaceRef: WorkspaceRef,
): Promise<boolean> {
  return ipcInvoke<boolean>(
    "set_unity_native_bridge_enabled",
    { value, workspaceRef },
    { operation: "unityNativeBridgeSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityNativeBrokerGetStatus(
  workspaceRef: WorkspaceRef,
): Promise<UnityNativeBrokerStatus | null> {
  return ipcInvoke<UnityNativeBrokerStatus | null>("get_unity_native_broker_status", { workspaceRef }, {
    operation: "unityNativeBrokerGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unityNativeBridgeSelfTestRun(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("unity_native_bridge_selftest_run", { workspaceRef }, {
    operation: "unityNativeBridgeSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityNativeBridgeSelfTest(
  workspaceRef: WorkspaceRef,
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<HotReloadSelfTestEvent>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "unity-native-bridge-selftest") return;
      if (event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      handler(event.payload);
    },
  );
}

export function subscribeCsharpLspStatus(
  workspaceRef: WorkspaceRef,
  handler: (payload: CsharpLspStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<CsharpLspStatus>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "csharp-lsp-status") return;
      if (event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      handler(event.payload);
    },
  );
}

export function subscribeUnitySidecarCompilerStatus(
  workspaceRef: WorkspaceRef,
  handler: (payload: CsharpCompileStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<CsharpCompileStatus>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "csharp-compile-status") return;
      if (event.checkoutId !== workspaceRef.checkoutId) return;
      if (
        workspaceRef.expectedGeneration != null
        && event.workspaceGeneration !== workspaceRef.expectedGeneration
      ) return;
      handler(event.payload);
    },
  );
}
