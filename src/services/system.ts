import { ipcInvoke } from "./ipc";
import type { PythonRuntimeState } from "../types";

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

export function getPythonRuntimeState(): Promise<PythonRuntimeState> {
  return ipcInvoke<PythonRuntimeState>("get_python_runtime_state");
}

export function savePythonRuntimeSelection(selectedId: string): Promise<PythonRuntimeState> {
  return ipcInvoke<PythonRuntimeState>("save_python_runtime_selection", { selectedId });
}
