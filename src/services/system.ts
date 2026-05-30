import { ipcInvoke } from "./ipc";
import type { ProxyConfig, ProxyStatus, PythonRuntimeState, UnityBackgroundHookStatus } from "../types";

export const APP_CLOSE_REQUESTED_EVENT = "locus-main-window-close-requested";
export type AppCloseBehavior = "exit" | "minimizeToTray";
export type DynamicToolLoadingMode = "metaTool" | "direct";

let pythonRuntimeStateCache: PythonRuntimeState | null = null;
let pythonRuntimeStateRequest: Promise<PythonRuntimeState> | null = null;
let currentPythonRuntimeStateCache: PythonRuntimeState | null = null;
let currentPythonRuntimeStateRequest: Promise<PythonRuntimeState> | null = null;

function normalizeCloseBehavior(value: unknown): AppCloseBehavior {
  return value === "minimizeToTray" ? "minimizeToTray" : "exit";
}

function normalizeDynamicToolLoadingMode(value: unknown): DynamicToolLoadingMode {
  return value === "direct" ? "direct" : "metaTool";
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

export function getUnityBackgroundHookEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_background_hook_enabled");
}

export function setUnityBackgroundHookEnabled(value: boolean): Promise<UnityBackgroundHookStatus> {
  return ipcInvoke<UnityBackgroundHookStatus>("set_unity_background_hook_enabled", { value });
}

export function getUnityBackgroundHookStatus(): Promise<UnityBackgroundHookStatus> {
  return ipcInvoke<UnityBackgroundHookStatus>("get_unity_background_hook_status");
}

export function getViewWindowsAboveMain(): Promise<boolean> {
  return ipcInvoke<boolean>("get_view_windows_above_main");
}

export function setViewWindowsAboveMain(value: boolean): Promise<void> {
  return ipcInvoke<void>("set_view_windows_above_main", { value });
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
