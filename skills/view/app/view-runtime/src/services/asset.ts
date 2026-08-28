import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
import type {
  AssetDbLightStatus,
  AssetDbOverview,
  AssetRiskKind,
  AssetSearchResult,
  AssetPreviewPayload,
  RefGraphScanStartResult,
  ScanStats,
  SemanticTargetInspector,
  WatcherTuning,
} from "../types";

export function assetDbOverview(workspaceRef: WorkspaceRef): Promise<AssetDbOverview> {
  return ipcInvoke<AssetDbOverview>("asset_db_overview", { workspaceRef });
}

export function assetDbLightStatus(workspaceRef: WorkspaceRef): Promise<AssetDbLightStatus> {
  return ipcInvoke<AssetDbLightStatus>("asset_db_light_status", { workspaceRef });
}

export function assetRiskReport(
  kind: AssetRiskKind,
  workspaceRef: WorkspaceRef,
): Promise<string> {
  return ipcInvoke<string>("asset_risk_report", { kind, workspaceRef });
}

export function assetDbStatus(workspaceRef: WorkspaceRef): Promise<ScanStats | null> {
  return ipcInvoke<ScanStats | null>("ref_graph_status", { workspaceRef });
}

export function assetDbScan(workspaceRef: WorkspaceRef): Promise<ScanStats> {
  return ipcInvoke<ScanStats>("ref_graph_scan", { workspaceRef });
}

export function assetDbScanStart(
  workspaceRef: WorkspaceRef,
): Promise<RefGraphScanStartResult> {
  return ipcInvoke<RefGraphScanStartResult>("ref_graph_scan_start", { workspaceRef });
}

/**
 * `roots` MUST be PascalCase directory names: ["Assets", "Packages", "ProjectSettings"].
 * The response uses camelCase, but the request expects directory names — see
 * `AssetSearchRoot::from_str` in src-tauri/src/commands/asset.rs.
 */
export function searchWorkspaceAssets(
  query: string,
  roots: string[],
  limit: number | undefined,
  workspaceRef: WorkspaceRef,
): Promise<AssetSearchResult[]> {
  const payload = limit === undefined
    ? { query, roots, workspaceRef }
    : { query, roots, limit, workspaceRef };
  return ipcInvoke<AssetSearchResult[]>("search_workspace_assets", payload);
}

export interface WorkspaceSceneObjectSearchResult {
  scenePath: string;
  objectPath: string;
  name: string;
  matchScore: number;
}

export function searchWorkspaceSceneObjects(
  scenePath: string,
  query: string,
  limit = 160,
  workspaceRef: WorkspaceRef,
): Promise<WorkspaceSceneObjectSearchResult[]> {
  return ipcInvoke<WorkspaceSceneObjectSearchResult[]>("search_workspace_scene_objects", {
    scenePath,
    query,
    limit,
    workspaceRef,
  });
}

export function previewWorkspaceAsset(
  filePath: string,
  focusLine: number | undefined,
  workspaceRef: WorkspaceRef,
): Promise<AssetPreviewPayload> {
  const payload = focusLine == null
    ? { filePath, workspaceRef }
    : { filePath, focusLine, workspaceRef };
  return ipcInvoke<AssetPreviewPayload>("preview_workspace_asset", payload);
}

export interface AssetThumbnailPreview {
  assetPath: string;
  url: string;
  width: number;
  height: number;
  mimeType: string;
}

export function previewWorkspaceAssetThumbnail(
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<AssetThumbnailPreview> {
  return ipcInvoke<AssetThumbnailPreview>("preview_workspace_asset_thumbnail", {
    filePath,
    workspaceRef,
  });
}

export interface AssetPreviewFrame {
  assetPath: string;
  url: string;
  width: number;
  height: number;
  mimeType: string;
  yaw?: number;
  pitch?: number;
  distance?: number;
  panX?: number;
  panY?: number;
  panZ?: number;
}

export interface AssetPreviewFrameRequest {
  width: number;
  height: number;
  yaw: number;
  pitch: number;
  distance: number;
  panX: number;
  panY: number;
  panZ: number;
}

export function readWorkspaceAssetPreviewFrameCache(
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<AssetPreviewFrame | null> {
  return ipcInvoke<AssetPreviewFrame | null>("read_workspace_asset_preview_frame_cache", {
    filePath,
    workspaceRef,
  });
}

export function cacheWorkspaceAssetPreviewFrame(
  filePath: string,
  frame: AssetPreviewFrame,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("cache_workspace_asset_preview_frame", {
    filePath,
    url: frame.url,
    width: frame.width,
    height: frame.height,
    mimeType: frame.mimeType,
    yaw: frame.yaw ?? 25,
    pitch: frame.pitch ?? -12,
    distance: frame.distance ?? 1.15,
    panX: frame.panX ?? 0,
    panY: frame.panY ?? 0,
    panZ: frame.panZ ?? 0,
    workspaceRef,
  });
}

export function renderWorkspaceAssetPreviewFrame(
  filePath: string,
  request: AssetPreviewFrameRequest,
  workspaceRef: WorkspaceRef,
): Promise<AssetPreviewFrame> {
  return ipcInvoke<AssetPreviewFrame>("render_workspace_asset_preview_frame", {
    filePath,
    ...request,
    workspaceRef,
  });
}

export function getWatcherTuning(): Promise<WatcherTuning> {
  return ipcInvoke<WatcherTuning>("get_watcher_tuning");
}

export function setWatcherTuning(
  debounceMs: number,
  workerCount: number,
): Promise<WatcherTuning> {
  return ipcInvoke<WatcherTuning>("set_watcher_tuning", { debounceMs, workerCount });
}

export function previewWorkspaceAssetTarget(
  previewKey: string,
  targetId: string,
  workspaceRef: WorkspaceRef,
): Promise<SemanticTargetInspector> {
  return ipcInvoke<SemanticTargetInspector>("preview_workspace_asset_target", {
    previewKey,
    targetId,
    workspaceRef,
  });
}
