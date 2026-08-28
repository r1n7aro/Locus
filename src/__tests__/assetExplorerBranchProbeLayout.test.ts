import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AssetExplorer branch probe layout", () => {
  it("probes visible folders and toggles through the folder row", () => {
    const source = read("src/components/asset/AssetExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(source).toContain('(e: "probe", path: string): void;');
    expect(source).toContain("if (!folder.hasChildFoldersKnown) return false;");
    expect(source).toContain('if (entry.kind === "row") {');
    expect(source).toContain('emit("probe", entry.node.path);');
    expect(source).toContain("expandable: canToggle");
    expect(source).toContain('emit("select", entry.node.path);');
    expect(source).toContain('emit("toggle", entry.node.path);');
    expect(workspaceTree).toContain("return row.expanded ? FolderOpen : Folder;");
    expect(workspaceTree).not.toContain("workspace-tree-branch");
  });
});
