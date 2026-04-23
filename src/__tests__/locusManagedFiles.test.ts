import { describe, expect, it } from "vitest";
import {
  countLocusManagedFiles,
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
