import { describe, expect, it } from "vitest";
import {
  countLocusManagedFiles,
  getLocusManagedTagKind,
  getLocusManagedTagKindForPath,
  isLocusManagedFile,
  isLocusManagedPath,
} from "../composables/locusManagedFiles";

describe("locusManagedFiles", () => {
  it("matches workspace files inside Locus-managed folders", () => {
    expect(isLocusManagedPath("Locus/memory/project-understanding.md")).toBe(true);
    expect(isLocusManagedPath("Library/Locus/knowledge_config.json")).toBe(true);
  });

  it("matches installed Unity plugin files under supported plugin roots", () => {
    expect(isLocusManagedPath("Assets/Locus/Editor/Locus.Editor.asmdef")).toBe(true);
    expect(isLocusManagedPath("Assets/Locus.meta")).toBe(true);
    expect(isLocusManagedPath("Assets/Plugins/Locus/Editor/LocusBridge.cs")).toBe(true);
    expect(isLocusManagedPath("Assets/Plugins/Locus.meta")).toBe(true);
    expect(isLocusManagedPath("Packages/com.farlocus.locus/Editor/LocusBridge.cs")).toBe(true);
  });

  it("does not treat arbitrary external docs as Locus-managed", () => {
    expect(isLocusManagedPath("docs/Design/Combat.md")).toBe(false);
  });

  it("treats renamed files as Locus-managed when either side matches", () => {
    expect(isLocusManagedFile({
      path: "Assets/Notes/design.md",
      oldPath: "Locus/knowledge/design/docs/Design/design.md",
    })).toBe(true);
  });

  it("resolves semantic tags for knowledge files", () => {
    expect(getLocusManagedTagKindForPath("Locus/knowledge/design/system/主要玩法.md")).toBe("design");
    expect(getLocusManagedTagKindForPath("Locus/knowledge/memory/unity-project-understanding/12.md")).toBe("memory");
    expect(getLocusManagedTagKindForPath("Locus/knowledge/memory/unity-project-understanding")).toBe("memory");
    expect(getLocusManagedTagKindForPath("Locus/knowledge/skill/builtin/create-skill.md")).toBe("skill");
    expect(getLocusManagedTagKindForPath("Locus/knowledge/reference/unity/api.md")).toBe("reference");
    expect(getLocusManagedTagKindForPath("Locus/memory/project-understanding.md")).toBe("memory");
  });

  it("falls back to the generic tag for non-knowledge Locus files", () => {
    expect(getLocusManagedTagKindForPath("Assets/Locus/Editor/Locus.Editor.asmdef")).toBe("locus");
  });

  it("uses the semantic tag from renamed knowledge files", () => {
    expect(getLocusManagedTagKind({
      path: "Assets/Notes/design.md",
      oldPath: "Locus/knowledge/design/docs/Design/design.md",
    })).toBe("design");
  });

  it("counts only Locus-managed entries", () => {
    expect(countLocusManagedFiles([
      { path: "Locus/memory/state.json" },
      { path: "Assets/Locus/Editor/LocusBridge.cs" },
      { path: "Packages/com.farlocus.locus/package.json" },
      { path: "Assets/Scripts/Player.cs" },
      { path: "docs/Combat/Combat.md" },
    ])).toBe(3);
  });
});
