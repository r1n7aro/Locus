import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";
import type { AssetRefAttachment, PluginStatus } from "../types";

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

export interface UnityLaunchResult {
  editorPath: string;
  projectPath: string;
  projectVersion: string;
}

export function launchUnityProject(): Promise<UnityLaunchResult> {
  return ipcInvoke<UnityLaunchResult>("launch_unity_project");
}

export interface SelectUnityAssetOptions {
  focusProjectWindow?: boolean;
}

export function selectUnityAsset(
  assetPath: string,
  options: SelectUnityAssetOptions = {},
): Promise<void> {
  const focusProjectWindow = options.focusProjectWindow ?? getLocusRuntime().kind !== "unity";
  return ipcInvoke("select_unity_asset", { assetPath, focusProjectWindow });
}

export function openUnityAssetInspector(assetPath: string): Promise<void> {
  return ipcInvoke("open_unity_asset_inspector", { assetPath });
}

export function selectUnitySceneObject(
  scenePath: string,
  objectPath: string,
): Promise<void> {
  return ipcInvoke("select_unity_scene_object", { scenePath, objectPath });
}

export function openUnitySceneObjectInspector(
  scenePath: string,
  objectPath: string,
): Promise<void> {
  return ipcInvoke("open_unity_scene_object_inspector", { scenePath, objectPath });
}

export function setUnityEmbedMouseActivationSuppressed(suppressed: boolean): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_set_mouse_activation_suppressed", { suppressed });
}

export function activateUnityEmbedForInput(): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_activate_for_input");
}

export function commitUnityEmbedAssetDrop(): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_commit_asset_drop");
}

export interface UnityEmbedAssetDropPayload {
  refs: AssetRefAttachment[];
}

export interface UnityEmbedAssetDragStatePayload {
  hasRefs: boolean;
  refs: AssetRefAttachment[];
}

export function subscribeUnityEmbedAssetDrop(
  handler: (payload: UnityEmbedAssetDropPayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<UnityEmbedAssetDropPayload>("unity-embed-asset-drop", handler);
}

export function subscribeUnityEmbedAssetDragState(
  handler: (payload: UnityEmbedAssetDragStatePayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<UnityEmbedAssetDragStatePayload>("unity-embed-asset-drag-state", handler);
}

export interface UnityEmbedFocusDebugSnapshot {
  ok: boolean;
  reason: string;
  foregroundHwnd: number;
  foregroundTitle: string;
  overlayHwnd: number;
  overlayTitle: string;
  overlayVisible: boolean;
  overlayForeground: boolean;
  overlayChildWindow: boolean;
  overlayParentHwnd: number;
  overlayNoActivate: boolean;
  activationGuardEnabled: boolean;
  mouseActivateHookInstalled: boolean;
  mouseActivateHookedHwndCount: number;
  mouseActivateBlockCount: number;
  mouseActivationSuppressed: boolean;
  parentHwnd: number;
  parentTitle: string;
  parentVisible: boolean;
  parentForeground: boolean;
}

export function getUnityEmbedFocusDebugSnapshot(): Promise<UnityEmbedFocusDebugSnapshot | null> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(null);
  return runtime.invoke<UnityEmbedFocusDebugSnapshot>("unity_embed_focus_debug_snapshot");
}

export type UnitySceneObjectErrorKind = "sceneNotLoaded" | "objectMissing" | "unknown";

export function classifyUnitySceneObjectError(error: unknown): UnitySceneObjectErrorKind {
  const message = typeof error === "object" && error !== null && "message" in error
    ? String((error as { message?: unknown }).message ?? "")
    : String(error ?? "");

  if (/scene is not loaded/i.test(message)) return "sceneNotLoaded";
  if (/gameobject was not found/i.test(message)) return "objectMissing";
  return "unknown";
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
  previewSuppressed?: "largeFile" | string;
}

export function previewWorkspaceFile(
  filePath: string,
  line?: number,
): Promise<WorkspaceFilePreview> {
  return ipcInvoke<WorkspaceFilePreview>("preview_workspace_file", { filePath, line });
}
