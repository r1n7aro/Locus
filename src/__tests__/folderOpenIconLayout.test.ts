import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("folder open icons", () => {
  it("uses shared Lucide folder icons with solid closed and outline open states", () => {
    const assetExplorer = read("src/components/asset/AssetExplorer.vue");
    const assetLegacyExplorer = read("src/components/asset/AssetLegacyExplorer.vue");
    const assetDirectoryList = read("src/components/asset/AssetDirectoryList.vue");
    const stagingArea = read("src/components/collab/StagingArea.vue");
    const commitDetail = read("src/components/collab/CommitDetail.vue");
    const sharedIcons = read("src/components/icons/unityAssetIcons.ts");
    const sharedStyles = read("src/styles/asset-icons.css");

    for (const source of [assetExplorer, assetLegacyExplorer, assetDirectoryList, stagingArea, commitDetail]) {
      expect(source).toContain("unityFolderIconNode");
      expect(source).toContain("unityFolderIconClass");
    }

    expect(sharedIcons).toContain("FolderOpen");
    expect(sharedIcons).toContain('open ? "unity-asset-icon--folder-open" : "unity-asset-icon--folder-solid"');
    expect(sharedStyles).toContain(".unity-asset-icon.unity-asset-icon--folder-solid path:first-child");
    expect(assetExplorer).not.toContain("M2.25 4.5A1.25");
    expect(assetLegacyExplorer).not.toContain("M2.25 4.5A1.25");
  });
});
