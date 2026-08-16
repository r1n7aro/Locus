import { ipcInvoke } from "./ipc";
import type { ProxyConfig, ProxyStatus, PythonRuntimeState, UnityBackgroundHookStatus } from "../types";

export const APP_CLOSE_REQUESTED_EVENT = "locus-main-window-close-requested";
export type AppCloseBehavior = "exit" | "minimizeToTray";
export type DynamicToolLoadingMode = "metaTool" | "direct" | "native";

let pythonRuntimeStateCache: PythonRuntimeState | null = null;
let pythonRuntimeStateRequest: Promise<PythonRuntimeState> | null = null;
let currentPythonRuntimeStateCache: PythonRuntimeState | null = null;
let currentPythonRuntimeStateRequest: Promise<PythonRuntimeState> | null = null;

function normalizeCloseBehavior(value: unknown): AppCloseBehavior {
  return value === "minimizeToTray" ? "minimizeToTray" : "exit";
}

function normalizeDynamicToolLoadingMode(value: unknown): DynamicToolLoadingMode {
  if (value === "direct" || value === "native") return value;
  return "metaTool";
}

export function getSystemLocale(): Promise<string | null> {
  return ipcInvoke<string | null>("get_system_locale");
}

export function sendSystemNotification(title: string, body?: string | null): Promise<void> {
  return ipcInvoke<void>(
    "send_system_notification",
    {
      title,
      body: body ?? null,
    },
    { throwOnError: false },
  );
}

export function playCustomNotificationSound(path: string, volume = 1): Promise<void> {
  return ipcInvoke<void>("play_custom_notification_sound", { path, volume });
}

export function requestAppExit(): Promise<void> {
  return ipcInvoke<void>("request_app_exit");
}

export function getRunningTaskCount(): Promise<number> {
  return ipcInvoke<number>("get_running_task_count");
}

export async function getCloseBehavior(): Promise<AppCloseBehavior> {
  return normalizeCloseBehavior(await ipcInvoke<AppCloseBehavior>("get_close_behavior"));
}

export function setCloseBehavior(value: AppCloseBehavior): Promise<void> {
  return ipcInvoke<void>("set_close_behavior", { value: normalizeCloseBehavior(value) });
}

export async function getDynamicToolLoadingMode(): Promise<DynamicToolLoadingMode> {
  return normalizeDynamicToolLoadingMode(
    await ipcInvoke<DynamicToolLoadingMode>("get_dynamic_tool_loading_mode"),
  );
}

export function setDynamicToolLoadingMode(value: DynamicToolLoadingMode): Promise<void> {
  return ipcInvoke<void>("set_dynamic_tool_loading_mode", {
    value: normalizeDynamicToolLoadingMode(value),
  });
}

export function getAnthropicNativeLazyEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_anthropic_native_lazy_enabled");
}

export function setAnthropicNativeLazyEnabled(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_anthropic_native_lazy_enabled", { value });
}

export function getAsyncTasksEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_async_tasks_enabled");
}

export function setAsyncTasksEnabled(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_async_tasks_enabled", { value });
}

export function getToolFailureLogEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_tool_failure_log_enabled");
}

export function setToolFailureLogEnabled(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_tool_failure_log_enabled", { value });
}

export function getLlmRetryMaxAttempts(): Promise<number> {
  return ipcInvoke<number>("get_llm_retry_max_attempts");
}

export function setLlmRetryMaxAttempts(value: number): Promise<number> {
  return ipcInvoke<number>("set_llm_retry_max_attempts", { value });
}

export function getSubagentMaxDepth(): Promise<number> {
  return ipcInvoke<number>("get_subagent_max_depth");
}

export function setSubagentMaxDepth(value: number): Promise<number> {
  return ipcInvoke<number>("set_subagent_max_depth", { value });
}

export function getSubagentMaxConcurrent(): Promise<number> {
  return ipcInvoke<number>("get_subagent_max_concurrent");
}

export function setSubagentMaxConcurrent(value: number): Promise<number> {
  return ipcInvoke<number>("set_subagent_max_concurrent", { value });
}

export function getUnityBackgroundHookEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_background_hook_enabled");
}

export function setUnityBackgroundHookEnabled(value: boolean): Promise<UnityBackgroundHookStatus> {
  return ipcInvoke<UnityBackgroundHookStatus>("set_unity_background_hook_enabled", { value });
}

export function getUnityBackgroundHookStatus(): Promise<UnityBackgroundHookStatus> {
  return ipcInvoke<UnityBackgroundHookStatus>("get_unity_background_hook_status");
}

export interface ExternalScriptOpenRequest {
  projectPath: string;
  assetPath: string;
  line: number;
  column: number;
}

export function getUnityExternalEditorDefaultEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_external_editor_default_enabled");
}

export function setUnityExternalEditorDefaultEnabled(value: boolean): Promise<boolean> {
  return ipcInvoke<boolean>("set_unity_external_editor_default_enabled", { value });
}

export function takeExternalScriptOpenRequest(): Promise<ExternalScriptOpenRequest | null> {
  return ipcInvoke<ExternalScriptOpenRequest | null>("take_external_script_open_request");
}

export function getViewWindowsAboveMain(): Promise<boolean> {
  return ipcInvoke<boolean>("get_view_windows_above_main");
}

export function setViewWindowsAboveMain(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_view_windows_above_main", { value });
}

export function getViewOpenInExistingWindow(): Promise<boolean> {
  return ipcInvoke<boolean>("get_view_open_in_existing_window");
}

export function setViewOpenInExistingWindow(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_view_open_in_existing_window", { value });
}

export function getProxyStatus(): Promise<ProxyStatus> {
  return ipcInvoke<ProxyStatus>("get_proxy_status");
}

export function saveProxyConfig(config: ProxyConfig): Promise<ProxyStatus> {
  return ipcInvoke<ProxyStatus>("save_proxy_config", { config });
}

export function getPythonRuntimeState(refresh = false, discover = true): Promise<PythonRuntimeState> {
  if (discover && !refresh && pythonRuntimeStateCache) {
    return Promise.resolve(pythonRuntimeStateCache);
  }
  if (!discover && !refresh && currentPythonRuntimeStateCache) {
    return Promise.resolve(currentPythonRuntimeStateCache);
  }
  if (discover && !refresh && pythonRuntimeStateRequest) {
    return pythonRuntimeStateRequest;
  }
  if (!discover && !refresh && currentPythonRuntimeStateRequest) {
    return currentPythonRuntimeStateRequest;
  }

  const request = ipcInvoke<PythonRuntimeState>("get_python_runtime_state", { refresh, discover })
    .then((state) => {
      if (discover) {
        pythonRuntimeStateCache = state;
      }
      currentPythonRuntimeStateCache = state;
      return state;
    })
    .finally(() => {
      if (discover && pythonRuntimeStateRequest === request) {
        pythonRuntimeStateRequest = null;
      }
      if (!discover && currentPythonRuntimeStateRequest === request) {
        currentPythonRuntimeStateRequest = null;
      }
    });

  if (discover) {
    pythonRuntimeStateRequest = request;
  } else {
    currentPythonRuntimeStateRequest = request;
  }
  return request;
}

export function savePythonRuntimeSelection(selectedId: string): Promise<PythonRuntimeState> {
  return ipcInvoke<PythonRuntimeState>("save_python_runtime_selection", { selectedId })
    .then((state) => {
      pythonRuntimeStateCache = state;
      currentPythonRuntimeStateCache = state;
      return state;
    });
}
