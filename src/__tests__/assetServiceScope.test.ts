import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  assetDbLightStatus,
  assetDbOverview,
  assetDbScan,
  assetDbScanStart,
  assetDbStatus,
  assetRiskReport,
  cacheWorkspaceAssetPreviewFrame,
  previewWorkspaceAsset,
  previewWorkspaceAssetTarget,
  previewWorkspaceAssetThumbnail,
  readWorkspaceAssetPreviewFrameCache,
  renderWorkspaceAssetPreviewFrame,
  searchWorkspaceAssets,
  searchWorkspaceSceneObjects,
} from "../services/asset";
import { resolveRefGraphGuid, resolveRefGraphPath } from "../services/refGraph";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef = {
  checkoutId: "checkout-b",
  expectedGeneration: 7,
};

describe("asset service workspace scope", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
  });

  it("forwards WorkspaceRef through overview, scan, search, and preview IPC", async () => {
    await assetDbOverview(workspaceRef);
    await assetDbScanStart(workspaceRef);
    await searchWorkspaceAssets("player", ["Assets"], 25, workspaceRef);
    await previewWorkspaceAsset("Assets/Player.prefab", 12, workspaceRef);
    await previewWorkspaceAssetTarget("preview-1", "go:1", workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "asset_db_overview", { workspaceRef });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "ref_graph_scan_start", { workspaceRef });
    expect(mockedInvoke).toHaveBeenNthCalledWith(3, "search_workspace_assets", {
      query: "player",
      roots: ["Assets"],
      limit: 25,
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(4, "preview_workspace_asset", {
      filePath: "Assets/Player.prefab",
      focusLine: 12,
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(5, "preview_workspace_asset_target", {
      previewKey: "preview-1",
      targetId: "go:1",
      workspaceRef,
    });
  });

  it("forwards WorkspaceRef through ref graph resolution IPC", async () => {
    await resolveRefGraphGuid("Assets/Same.asset", workspaceRef);
    await resolveRefGraphPath("0123456789abcdef0123456789abcdef", workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "ref_graph_resolve_guid", {
      path: "Assets/Same.asset",
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "ref_graph_resolve_path", {
      guidHex: "0123456789abcdef0123456789abcdef",
      workspaceRef,
    });
  });

  it("forwards WorkspaceRef through status, risk, scene, thumbnail, and frame IPC", async () => {
    await assetDbLightStatus(workspaceRef);
    await assetDbStatus(workspaceRef);
    await assetDbScan(workspaceRef);
    await assetRiskReport("brokenReferences", workspaceRef);
    await searchWorkspaceSceneObjects("Assets/Main.unity", "Player", 20, workspaceRef);
    await previewWorkspaceAssetThumbnail("Assets/Player.prefab", workspaceRef);
    await readWorkspaceAssetPreviewFrameCache("Assets/Player.prefab", workspaceRef);
    const frame = {
      assetPath: "Assets/Player.prefab",
      url: "data:image/png;base64,AA==",
      width: 320,
      height: 180,
      mimeType: "image/png",
    };
    await cacheWorkspaceAssetPreviewFrame("Assets/Player.prefab", frame, workspaceRef);
    await renderWorkspaceAssetPreviewFrame("Assets/Player.prefab", {
      width: 320,
      height: 180,
      yaw: 25,
      pitch: -12,
      distance: 1.15,
      panX: 0,
      panY: 0,
      panZ: 0,
    }, workspaceRef);

    for (const call of mockedInvoke.mock.calls) {
      expect(call[1]).toMatchObject({ workspaceRef });
    }
  });
});
