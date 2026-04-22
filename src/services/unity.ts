import { ipcInvoke } from "./ipc";
import type { PluginStatus } from "../types";

export interface AssetSearchResult {
  name: string;
  guid: string;
  path: string;
  type: string;
}

export function checkUnityConnection(): Promise<boolean> {
  return ipcInvoke<boolean>("check_unity_connection");
}

export function checkUnityPlugin(): Promise<PluginStatus> {
  return ipcInvoke<PluginStatus>("check_unity_plugin");
}

export function installUnityPlugin(): Promise<string> {
  return ipcInvoke<string>("install_unity_plugin");
}

export function selectUnityAsset(assetPath: string): Promise<void> {
  return ipcInvoke("select_unity_asset", { assetPath });
}

export function searchAssets(query: string): Promise<AssetSearchResult[]> {
  return ipcInvoke<AssetSearchResult[]>("search_assets", { query });
}

export function sendUnityLog(message: string): Promise<void> {
  return ipcInvoke("send_unity_log", { message });
}

export function openFileExternal(filePath: string): Promise<void> {
  return ipcInvoke("open_file_external", { filePath });
}

export function showInFolder(filePath: string): Promise<void> {
  return ipcInvoke("reveal_workspace_file", { filePath });
}

export interface WorkspaceFilePreview {
  displayPath: string;
  exists: boolean;
  kind: "text" | "binary" | "not_found";
  language?: string;
  snippet?: string;
  truncated: boolean;
  isUnityAsset: boolean;
  preferredAction: "editor" | "unity" | "external";
  fileSize?: number;
  snippetStartLine: number;
}

export function previewWorkspaceFile(
  filePath: string,
  line?: number,
): Promise<WorkspaceFilePreview> {
  return ipcInvoke<WorkspaceFilePreview>("preview_workspace_file", { filePath, line });
}
