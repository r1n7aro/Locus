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
