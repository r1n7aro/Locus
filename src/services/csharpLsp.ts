import type {
  CodeAnalysisToolsConfig,
  CsharpCompileStatus,
  CsharpLspStatus,
  UnityNativeBrokerStatus,
  UnitySemanticState,
} from "../types";
import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

export function csharpLspGetStatus(): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_get_status", undefined, {
    operation: "csharpLspGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function csharpLspSetEnabled(value: boolean): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>(
    "csharp_lsp_set_enabled",
    { value },
    { operation: "csharpLspSetEnabled", notify: false, throwOnError: true },
  );
}

export function csharpLspRestart(): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_restart", undefined, {
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
): Promise<CodeAnalysisToolsConfig> {
  return ipcInvoke<CodeAnalysisToolsConfig>(
    "code_analysis_tools_set_config",
    { value },
    { operation: "codeAnalysisToolsSetConfig", notify: false, throwOnError: true },
  );
}

export function unitySidecarCompilerGetStatus(): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>("unity_sidecar_compiler_get_status", undefined, {
    operation: "unitySidecarCompilerGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unitySidecarCompilerSetEnabled(value: boolean): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_sidecar_compiler_set_enabled",
    { value },
    { operation: "unitySidecarCompilerSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityHotReloadSetEnabled(value: boolean): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_hot_reload_set_enabled",
    { value },
    { operation: "unityHotReloadSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityRecompileRun(): Promise<string> {
  return ipcInvoke<string>("unity_recompile_run", undefined, {
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

export function unityHotReloadSelfTestRun(): Promise<void> {
  return ipcInvoke<void>("unity_hot_reload_selftest_run", undefined, {
    operation: "unityHotReloadSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityHotReloadSelfTest(
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<HotReloadSelfTestEvent>("unity-hotreload-selftest", handler);
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

export function unityStateProbeGetStatus(): Promise<UnityStateProbeStatus> {
  return ipcInvoke<UnityStateProbeStatus>("get_unity_state_probe_status", undefined, {
    operation: "unityStateProbeGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unityStateProbeSetEnabled(value: boolean): Promise<UnityStateProbeStatus> {
  return ipcInvoke<UnityStateProbeStatus>(
    "set_unity_state_probe_enabled",
    { value },
    { operation: "unityStateProbeSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityStateProbeSelfTestRun(): Promise<void> {
  return ipcInvoke<void>("unity_state_probe_selftest_run", undefined, {
    operation: "unityStateProbeSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function unitySemanticStateGet(): Promise<UnitySemanticState> {
  return ipcInvoke<UnitySemanticState>("get_unity_semantic_state", undefined, {
    operation: "unitySemanticStateGet",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityStateProbeSelfTest(
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<HotReloadSelfTestEvent>("unity-state-probe-selftest", handler);
}

export function unityNativeBridgeGetEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_native_bridge_enabled", undefined, {
    operation: "unityNativeBridgeGetEnabled",
    notify: false,
    throwOnError: true,
  });
}

export function unityNativeBridgeSetEnabled(value: boolean): Promise<boolean> {
  return ipcInvoke<boolean>(
    "set_unity_native_bridge_enabled",
    { value },
    { operation: "unityNativeBridgeSetEnabled", notify: false, throwOnError: true },
  );
}

export function unityNativeBrokerGetStatus(): Promise<UnityNativeBrokerStatus | null> {
  return ipcInvoke<UnityNativeBrokerStatus | null>("get_unity_native_broker_status", undefined, {
    operation: "unityNativeBrokerGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unityNativeBridgeSelfTestRun(): Promise<void> {
  return ipcInvoke<void>("unity_native_bridge_selftest_run", undefined, {
    operation: "unityNativeBridgeSelfTestRun",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeUnityNativeBridgeSelfTest(
  handler: (payload: HotReloadSelfTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<HotReloadSelfTestEvent>(
    "unity-native-bridge-selftest",
    handler,
  );
}

export function subscribeCsharpLspStatus(
  handler: (payload: CsharpLspStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<CsharpLspStatus>("csharp-lsp-status", handler);
}

export function subscribeUnitySidecarCompilerStatus(
  handler: (payload: CsharpCompileStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<CsharpCompileStatus>("csharp-compile-status", handler);
}
