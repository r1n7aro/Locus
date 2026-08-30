import { describe, expect, it } from "vitest";
import {
  assetWorkspaceReferenceDragData,
  assetWorkspaceReferenceEntry,
} from "../components/asset/assetWorkspaceDrag";
import type { AssetSearchResult } from "../types";

describe("asset workspace drag", () => {
  it("turns code files and folders into workspace-relative file references", () => {
    expect(assetWorkspaceReferenceEntry({
      kind: "file",
      name: "player.ts",
      path: "src/player.ts",
      depth: 2,
    })).toEqual({
      kind: "file",
      path: "src/player.ts",
      isDir: false,
      name: "player.ts",
      typeLabel: undefined,
    });

    expect(assetWorkspaceReferenceEntry({
      kind: "folder",
      name: "Assets",
      path: "Assets",
      depth: 1,
      isRoot: false,
      loaded: false,
      loading: false,
      hasMore: false,
      nextOffset: 0,
      totalCount: 0,
      hasChildFoldersKnown: false,
      hasChildFolders: false,
      branchProbeLoading: false,
      children: [],
    })).toMatchObject({
      kind: "file",
      path: "Assets",
      isDir: true,
      name: "Assets",
    });
  });

  it("preserves search-result directory and type metadata", () => {
    const result: AssetSearchResult = {
      path: "Assets/Characters",
      name: "Characters",
      root: "workspace",
      kind: "folder",
      typeLabel: "Directory",
      isDirectory: true,
      matchScore: 1,
      source: "filesystem",
    };
    expect(assetWorkspaceReferenceEntry(result)).toEqual({
      kind: "file",
      path: "Assets/Characters",
      isDir: true,
      name: "Characters",
      typeLabel: "Directory",
    });
  });

  it("binds the drag payload to the source project and checkout", () => {
    expect(assetWorkspaceReferenceDragData({
      projectId: "project-a",
      workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 9 },
      workspaceRoot: "F:/Game",
    }, {
      kind: "file",
      name: "config.json",
      path: "config.json",
      depth: 1,
    })).toMatchObject({
      version: 1,
      origin: {
        projectId: "project-a",
        workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 9 },
        workspaceRoot: "F:/Game",
      },
      entries: [{
        kind: "file",
        path: "config.json",
        isDir: false,
      }],
    });
  });
});
