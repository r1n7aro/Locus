import { ipcInvoke } from "./ipc";
import type { ProxyConfig, ProxyStatus, PythonRuntimeState } from "../types";

export const APP_CLOSE_REQUESTED_EVENT = "locus-main-window-close-requested";

let pythonRuntimeStateCache: PythonRuntimeState | null = null;
let pythonRuntimeStateRequest: Promise<PythonRuntimeState> | null = null;

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

export function requestAppExit(): Promise<void> {
  return ipcInvoke<void>("request_app_exit");
}

export function getProxyStatus(): Promise<ProxyStatus> {
  return ipcInvoke<ProxyStatus>("get_proxy_status");
}

export function saveProxyConfig(config: ProxyConfig): Promise<ProxyStatus> {
  return ipcInvoke<ProxyStatus>("save_proxy_config", { config });
}

export function getPythonRuntimeState(refresh = false): Promise<PythonRuntimeState> {
  if (!refresh && pythonRuntimeStateCache) {
    return Promise.resolve(pythonRuntimeStateCache);
  }
  if (!refresh && pythonRuntimeStateRequest) {
    return pythonRuntimeStateRequest;
  }

  const request = ipcInvoke<PythonRuntimeState>("get_python_runtime_state", { refresh })
    .then((state) => {
      pythonRuntimeStateCache = state;
      return state;
    })
    .finally(() => {
      if (pythonRuntimeStateRequest === request) {
        pythonRuntimeStateRequest = null;
      }
    });

  pythonRuntimeStateRequest = request;
  return request;
}

export function savePythonRuntimeSelection(selectedId: string): Promise<PythonRuntimeState> {
  return ipcInvoke<PythonRuntimeState>("save_python_runtime_selection", { selectedId })
    .then((state) => {
      pythonRuntimeStateCache = state;
      return state;
    });
}
