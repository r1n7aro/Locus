import { ipcInvoke } from "./ipc";

let cachedDebugMode: boolean | null = null;
let pendingDebugModeLoad: Promise<boolean> | null = null;
let debugModeCacheVersion = 0;
let cachedFileToolWorkspaceBoundary: boolean | null = null;
let pendingFileToolWorkspaceBoundaryLoad: Promise<boolean> | null = null;
let fileToolWorkspaceBoundaryCacheVersion = 0;

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

export function getCachedDebugMode(): boolean | null {
  return cachedDebugMode;
}

export function getDebugMode(): Promise<boolean> {
  if (cachedDebugMode !== null) {
    return Promise.resolve(cachedDebugMode);
  }

  if (!pendingDebugModeLoad) {
    const cacheVersion = debugModeCacheVersion;
    pendingDebugModeLoad = ipcInvoke<boolean>("get_debug_mode")
      .then((value) => {
        if (cacheVersion === debugModeCacheVersion) {
          cachedDebugMode = value;
        }
        return cachedDebugMode ?? value;
      })
      .finally(() => {
        pendingDebugModeLoad = null;
      });
  }

  return pendingDebugModeLoad;
}

export async function setDebugMode(value: boolean): Promise<void> {
  const previous = cachedDebugMode;
  debugModeCacheVersion += 1;
  cachedDebugMode = value;

  try {
    await ipcInvoke("set_debug_mode", { value });
  } catch (error) {
    debugModeCacheVersion += 1;
    cachedDebugMode = previous;
    throw error;
  }
}

export function getCachedFileToolWorkspaceBoundary(): boolean | null {
  return cachedFileToolWorkspaceBoundary;
}

export function getFileToolWorkspaceBoundary(): Promise<boolean> {
  if (cachedFileToolWorkspaceBoundary !== null) {
    return Promise.resolve(cachedFileToolWorkspaceBoundary);
  }

  if (!pendingFileToolWorkspaceBoundaryLoad) {
    const cacheVersion = fileToolWorkspaceBoundaryCacheVersion;
    pendingFileToolWorkspaceBoundaryLoad = ipcInvoke<boolean>("get_file_tool_workspace_boundary")
      .then((value) => {
        if (cacheVersion === fileToolWorkspaceBoundaryCacheVersion) {
          cachedFileToolWorkspaceBoundary = value;
        }
        return cachedFileToolWorkspaceBoundary ?? value;
      })
      .finally(() => {
        pendingFileToolWorkspaceBoundaryLoad = null;
      });
  }

  return pendingFileToolWorkspaceBoundaryLoad;
}

export async function setFileToolWorkspaceBoundary(value: boolean): Promise<void> {
  const previous = cachedFileToolWorkspaceBoundary;
  fileToolWorkspaceBoundaryCacheVersion += 1;
  cachedFileToolWorkspaceBoundary = value;

  try {
    await ipcInvoke("set_file_tool_workspace_boundary", { value });
  } catch (error) {
    fileToolWorkspaceBoundaryCacheVersion += 1;
    cachedFileToolWorkspaceBoundary = previous;
    throw error;
  }
}
