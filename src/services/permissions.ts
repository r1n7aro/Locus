import { ipcInvoke } from "./ipc";

export function getToolPermissionMode(): Promise<string> {
  return ipcInvoke<string>("get_tool_permission_mode");
}

export function saveToolPermissionMode(mode: string): Promise<void> {
  return ipcInvoke("save_tool_permission_mode", { value: mode });
}

export function getToolPermissions(): Promise<Record<string, string>> {
  return ipcInvoke<Record<string, string>>("get_tool_permissions");
}

export function saveToolPermissions(value: Record<string, string>): Promise<void> {
  return ipcInvoke("save_tool_permissions", { value });
}

export function getDebugMode(): Promise<boolean> {
  return ipcInvoke<boolean>("get_debug_mode");
}

export function setDebugMode(value: boolean): Promise<void> {
  return ipcInvoke("set_debug_mode", { value });
}
