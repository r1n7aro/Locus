import { describe, expect, it } from "vitest";
import { workbenchComposerFileAttachment } from "../components/workbench/workbenchComposerDrop";

describe("workbench composer file drops", () => {
  it("keeps Unity project files as asset references", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "F:\\Game\\Assets\\Prefabs\\Player.prefab",
      workspaceRoot: "F:\\Game",
      relativePath: "Prefabs/Player.prefab",
      name: "Player.prefab",
    })).toEqual({
      assetRef: {
        path: "Assets/Prefabs/Player.prefab",
        kind: "asset",
        name: "Player.prefab",
        typeLabel: undefined,
        source: "manual",
      },
    });
  });

  it("maps project knowledge files to knowledge references", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "F:\\Game\\Locus\\knowledge\\design\\combat.md",
      workspaceRoot: "F:\\Game",
      knowledgeSource: true,
      name: "combat.md",
    })).toEqual({
      assetRef: {
        path: "design/combat.md",
        kind: "knowledge",
        name: "combat.md",
        typeLabel: undefined,
        source: "manual",
      },
    });
  });

  it("preserves files outside the project as local file attachments", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "E:\\References\\brief.pdf",
      workspaceRoot: "F:\\Game",
      name: "brief.pdf",
      typeLabel: "PDF",
    })).toEqual({
      localFile: {
        path: "E:/References/brief.pdf",
        isDir: false,
        name: "brief.pdf",
        typeLabel: "PDF",
        source: "local",
      },
    });
  });
});
