import { describe, expect, it } from "vitest";
import { mapWorkspaceMentionResults } from "../components/chat/mentionWorkspaceSearch";
import type { WorkspaceSearchEntry } from "../services/project";

function entry(
  relPath: string,
  isDir: boolean,
  matchScore: number,
): WorkspaceSearchEntry {
  const normalized = relPath.replace(/\\/g, "/");
  const name = normalized.split("/").pop() || normalized;
  return {
    relPath,
    name,
    parentPath: normalized.slice(0, Math.max(0, normalized.lastIndexOf("/"))),
    isDir,
    matchScore,
  };
}

describe("folder mention search", () => {
  it("maps Unity workspace folders into selectable mention results", () => {
    expect(mapWorkspaceMentionResults([
      entry("Assets/Arts/UI/Characters", true, 1200),
      entry("Packages/com.example.characters/Runtime", true, 900),
      entry("ProjectSettings/Presets", true, 800),
    ])).toEqual([
      {
        relPath: "Assets/Arts/UI/Characters",
        name: "Characters",
        parentPath: "Assets/Arts/UI",
        isDir: true,
        matchScore: 1200,
        entryKind: "asset",
      },
      {
        relPath: "Packages/com.example.characters/Runtime",
        name: "Runtime",
        parentPath: "Packages/com.example.characters",
        isDir: true,
        matchScore: 900,
        entryKind: "asset",
      },
      {
        relPath: "ProjectSettings/Presets",
        name: "Presets",
        parentPath: "ProjectSettings",
        isDir: true,
        matchScore: 800,
        entryKind: "asset",
      },
    ]);
  });

  it("keeps generic workspace files and folders searchable", () => {
    expect(mapWorkspaceMentionResults([
      entry("Assets/Characters/Hero.prefab", false, 1000),
      entry("docs/Characters", true, 950),
      entry("agent/Characters", true, 900),
    ])).toMatchObject([
      { relPath: "Assets/Characters/Hero.prefab", isDir: false },
      { relPath: "docs/Characters", isDir: true },
      { relPath: "agent/Characters", isDir: true },
    ]);
  });

  it("normalizes Windows separators before creating the mention result", () => {
    expect(mapWorkspaceMentionResults([
      entry("Assets\\Arts\\Characters", true, 700),
    ])[0]).toMatchObject({
      relPath: "Assets/Arts/Characters",
      parentPath: "Assets/Arts",
      isDir: true,
    });
  });
});
