import { describe, expect, it } from "vitest";
import type { GitFileChange } from "../types";
import {
  buildStagingFolderFileMap,
  buildStagingTreeRows,
  collectStagingFolderPaths,
} from "../components/collab/stagingTree";

function makeFile(path: string, status = "M"): GitFileChange {
  return {
    path,
    status,
    lfs: false,
  };
}

describe("stagingTree", () => {
  it("compresses single-folder chains into one visible folder row", () => {
    const rows = buildStagingTreeRows([
      makeFile("Assets/Locus/Editor/LocusBridge.cs"),
      makeFile("Assets/Locus/Editor/Roslyn/THIRD_PARTY.md"),
      makeFile("README.md", "?"),
    ]);

    expect(rows.map((row) => `${row.kind}:${row.kind === "folder" ? `${row.name}@${row.path}` : row.file.path}:${row.depth}`)).toEqual([
      "folder:Assets/Locus/Editor@Assets/Locus/Editor:0",
      "folder:Roslyn@Assets/Locus/Editor/Roslyn:1",
      "file:Assets/Locus/Editor/Roslyn/THIRD_PARTY.md:2",
      "file:Assets/Locus/Editor/LocusBridge.cs:1",
      "file:README.md:0",
    ]);
  });

  it("hides descendants for collapsed folders while keeping later siblings visible", () => {
    const rows = buildStagingTreeRows(
      [
        makeFile("Assets/Locus/Editor/LocusBridge.cs"),
        makeFile("Assets/Scenes/Main.unity"),
      ],
      new Set(["Assets/Locus"]),
    );

    expect(rows.map((row) => `${row.kind}:${row.kind === "folder" ? `${row.name}@${row.path}` : row.file.path}`)).toEqual([
      "folder:Assets@Assets",
      "folder:Locus/Editor@Assets/Locus/Editor",
      "folder:Scenes@Assets/Scenes",
      "file:Assets/Scenes/Main.unity",
    ]);
  });

  it("preserves branching folders as separate rows", () => {
    const rows = buildStagingTreeRows([
      makeFile("Assets/Locus/Editor/LocusBridge.cs"),
      makeFile("Assets/Scenes/Main.unity"),
    ]);

    expect(rows.map((row) => `${row.kind}:${row.kind === "folder" ? `${row.name}@${row.path}` : row.file.path}:${row.depth}`)).toEqual([
      "folder:Assets@Assets:0",
      "folder:Locus/Editor@Assets/Locus/Editor:1",
      "file:Assets/Locus/Editor/LocusBridge.cs:2",
      "folder:Scenes@Assets/Scenes:1",
      "file:Assets/Scenes/Main.unity:2",
    ]);
  });

  it("places sibling folders before sibling files while preserving incoming order within each group", () => {
    const rows = buildStagingTreeRows([
      makeFile("Locus/View/entity-prefab-table/unity/view.json"),
      makeFile("Locus/View/entity-prefab-table/view.json"),
      makeFile("Locus/View/entity-prefab-table/src/App.vue"),
      makeFile("Locus/View/entity-prefab-table/README.md"),
    ]);

    expect(rows.map((row) => `${row.kind}:${row.kind === "folder" ? `${row.name}@${row.path}` : row.file.path}:${row.depth}`)).toEqual([
      "folder:Locus/View/entity-prefab-table@Locus/View/entity-prefab-table:0",
      "folder:unity@Locus/View/entity-prefab-table/unity:1",
      "file:Locus/View/entity-prefab-table/unity/view.json:2",
      "folder:src@Locus/View/entity-prefab-table/src:1",
      "file:Locus/View/entity-prefab-table/src/App.vue:2",
      "file:Locus/View/entity-prefab-table/view.json:1",
      "file:Locus/View/entity-prefab-table/README.md:1",
    ]);
  });

  it("collects every folder path that appears in the file list", () => {
    const paths = collectStagingFolderPaths([
      makeFile("Assets/Locus/Editor/LocusBridge.cs"),
      makeFile("Assets/Locus/Editor/Roslyn/THIRD_PARTY.md"),
      makeFile("README.md"),
    ]);

    expect([...paths]).toEqual([
      "Assets",
      "Assets/Locus",
      "Assets/Locus/Editor",
      "Assets/Locus/Editor/Roslyn",
    ]);
  });

  it("tracks descendant files for every folder path", () => {
    const map = buildStagingFolderFileMap([
      makeFile("Assets/Locus/Editor/LocusBridge.cs"),
      makeFile("Assets/Locus/Editor/Roslyn/THIRD_PARTY.md"),
      makeFile("Assets/Scenes/Main.unity"),
    ]);

    expect(map.get("Assets")).toEqual([
      "Assets/Locus/Editor/LocusBridge.cs",
      "Assets/Locus/Editor/Roslyn/THIRD_PARTY.md",
      "Assets/Scenes/Main.unity",
    ]);
    expect(map.get("Assets/Locus/Editor")).toEqual([
      "Assets/Locus/Editor/LocusBridge.cs",
      "Assets/Locus/Editor/Roslyn/THIRD_PARTY.md",
    ]);
    expect(map.get("Assets/Locus/Editor/Roslyn")).toEqual([
      "Assets/Locus/Editor/Roslyn/THIRD_PARTY.md",
    ]);
  });

  it("keeps custom file keys for duplicate final paths", () => {
    const rows = buildStagingTreeRows(
      [
        { ...makeFile("Assets/Config.asset"), displayKey: "first-touch" },
        { ...makeFile("Assets/Config.asset"), displayKey: "second-touch" },
      ],
      new Set<string>(),
      (file) => file.displayKey,
    );

    expect(rows.filter((row) => row.kind === "file").map((row) => row.key)).toEqual([
      "file:first-touch",
      "file:second-touch",
    ]);
  });
});
