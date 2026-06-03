import { ipcInvoke } from "./ipc";
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

export function assetDbOverview(): Promise<AssetDbOverview> {
  return ipcInvoke<AssetDbOverview>("asset_db_overview");
}

export function assetDbLightStatus(): Promise<AssetDbLightStatus> {
  return ipcInvoke<AssetDbLightStatus>("asset_db_light_status");
}

export function assetRiskReport(kind: AssetRiskKind): Promise<string> {
  return ipcInvoke<string>("asset_risk_report", { kind });
}

export function assetDbStatus(): Promise<ScanStats | null> {
  return ipcInvoke<ScanStats | null>("ref_graph_status");
}

export function assetDbScan(): Promise<ScanStats> {
  return ipcInvoke<ScanStats>("ref_graph_scan");
}

export function assetDbScanStart(): Promise<RefGraphScanStartResult> {
  return ipcInvoke<RefGraphScanStartResult>("ref_graph_scan_start");
}

/**
 * `roots` MUST be PascalCase directory names: ["Assets", "Packages", "ProjectSettings"].
 * The response uses camelCase, but the request expects directory names — see
 * `AssetSearchRoot::from_str` in src-tauri/src/commands/asset.rs.
 */
export function searchWorkspaceAssets(
  query: string,
  roots: string[],
  limit?: number,
): Promise<AssetSearchResult[]> {
  const payload = limit === undefined ? { query, roots } : { query, roots, limit };
  return ipcInvoke<AssetSearchResult[]>("search_workspace_assets", payload);
}

export function previewWorkspaceAsset(filePath: string): Promise<AssetPreviewPayload> {
  return ipcInvoke<AssetPreviewPayload>("preview_workspace_asset", { filePath });
}

export interface AssetThumbnailPreview {
  assetPath: string;
  url: string;
  width: number;
  height: number;
  mimeType: string;
}

export function previewWorkspaceAssetThumbnail(filePath: string): Promise<AssetThumbnailPreview> {
  return ipcInvoke<AssetThumbnailPreview>("preview_workspace_asset_thumbnail", { filePath });
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
): Promise<AssetPreviewFrame | null> {
  return ipcInvoke<AssetPreviewFrame | null>("read_workspace_asset_preview_frame_cache", {
    filePath,
  });
}

export function cacheWorkspaceAssetPreviewFrame(
  filePath: string,
  frame: AssetPreviewFrame,
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
  });
}

export function renderWorkspaceAssetPreviewFrame(
  filePath: string,
  request: AssetPreviewFrameRequest,
): Promise<AssetPreviewFrame> {
  return ipcInvoke<AssetPreviewFrame>("render_workspace_asset_preview_frame", {
    filePath,
    ...request,
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
): Promise<SemanticTargetInspector> {
  return ipcInvoke<SemanticTargetInspector>("preview_workspace_asset_target", {
    previewKey,
    targetId,
  });
}
