import { describe, expect, it } from "vitest";
import { partitionMetaPaths, withMetaCompanionPaths } from "../composables/useHideMeta";

describe("withMetaCompanionPaths", () => {
  it("adds hidden .meta companions for visible primary files", () => {
    expect(withMetaCompanionPaths(
      ["Assets/Plugins/Locus"],
      [
        "Assets/Plugins/Locus",
        "Assets/Plugins/Locus.meta",
        "Assets/Plugins/Locus/Editor.meta",
      ],
      true,
    )).toEqual([
      "Assets/Plugins/Locus",
      "Assets/Plugins/Locus.meta",
    ]);
  });

  it("preserves the original selection when companion expansion is disabled", () => {
    expect(withMetaCompanionPaths(
      ["Assets/Plugins/Locus", "Assets/Plugins/Locus.meta"],
      [
        "Assets/Plugins/Locus",
        "Assets/Plugins/Locus.meta",
      ],
      false,
    )).toEqual([
      "Assets/Plugins/Locus",
      "Assets/Plugins/Locus.meta",
    ]);
  });
});

describe("partitionMetaPaths", () => {
  it("separates hideable meta files from orphan meta files", () => {
    const result = partitionMetaPaths([
      "Assets/Plugins/Locus",
      "Assets/Plugins/Locus.meta",
      "Assets/Plugins/Locus/Editor.meta",
      "Assets/Scenes/Main.unity.meta",
    ]);

    expect([...result.hideableMetaPaths]).toEqual([
      "Assets/Plugins/Locus.meta",
    ]);
    expect([...result.orphanMetaPaths]).toEqual([
      "Assets/Plugins/Locus/Editor.meta",
      "Assets/Scenes/Main.unity.meta",
    ]);
  });

  it("suppresses orphan tags when the primary path still exists in the workspace", () => {
    const result = partitionMetaPaths([
      {
        path: "Assets/Art/UI.meta",
        primaryExistsInWorkspace: true,
        primaryIsDirectoryInWorkspace: true,
      },
      {
        path: "Assets/Scenes/Main.unity.meta",
        primaryExistsInWorkspace: true,
        primaryIsDirectoryInWorkspace: false,
      },
    ]);

    expect([...result.hideableMetaPaths]).toEqual([]);
    expect([...result.orphanMetaPaths]).toEqual([]);
  });

  it("keeps workspace-backed meta visible until a same-list primary file exists", () => {
    const result = partitionMetaPaths([
      "Assets/Art/HUD.prefab",
      {
        path: "Assets/Art/HUD.prefab.meta",
        primaryExistsInWorkspace: true,
        primaryIsDirectoryInWorkspace: false,
      },
      {
        path: "Assets/Art/UI.meta",
        primaryExistsInWorkspace: true,
        primaryIsDirectoryInWorkspace: true,
      },
    ]);

    expect([...result.hideableMetaPaths]).toEqual([
      "Assets/Art/HUD.prefab.meta",
    ]);
    expect([...result.orphanMetaPaths]).toEqual([]);
  });

  it("suppresses orphan tags for folder meta files when the same list still contains descendants", () => {
    const result = partitionMetaPaths([
      "Assets/World.meta",
      "Assets/World/Map.prefab",
      "Assets/World/Map.prefab.meta",
    ]);

    expect([...result.hideableMetaPaths]).toEqual([
      "Assets/World/Map.prefab.meta",
    ]);
    expect([...result.orphanMetaPaths]).toEqual([]);
  });

  it("suppresses orphan tags for renamed meta files", () => {
    const result = partitionMetaPaths([
      {
        path: "Assets/NewFolder.meta",
        oldPath: "Assets/OldFolder.meta",
      },
    ]);

    expect([...result.hideableMetaPaths]).toEqual([]);
    expect([...result.orphanMetaPaths]).toEqual([]);
  });

  it("ignores legacy Locus knowledge meta files in orphan detection", () => {
    const result = partitionMetaPaths([
      "Locus/knowledge/design/combat.meta",
      "Assets/Scenes/Main.unity.meta",
    ]);

    expect([...result.hideableMetaPaths]).toEqual([]);
    expect([...result.orphanMetaPaths]).toEqual([
      "Assets/Scenes/Main.unity.meta",
    ]);
  });
});
