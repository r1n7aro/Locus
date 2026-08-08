import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer row icons", () => {
  it("uses the local LucideIcon wrapper for folders, packages, and documents", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(explorer).toContain(
      'import LucideIcon from "../icons/LucideIcon.vue"',
    );
    expect(explorer).toContain("unityAssetIconClassForPath");
    expect(explorer).toContain("unityAssetIconNodeForPath");
    expect(explorer).toContain('<template #icon="{ item }">');
    expect(explorer).toContain(':icon="entry.row.expanded ? FolderOpen : Folder"');
    expect(explorer).toContain(':icon="Package"');
    expect(explorer).toContain(':class="documentIconClass(entry.row.node)"');
    expect(explorer).toContain(':icon="documentIconNode(entry.row.node)"');
    expect(workspaceTree).toContain('class="workspace-tree-icon"');
    expect(workspaceTree).toContain('class="workspace-tree-row-shell"');
  });

  it("uses a spacer instead of a chevron for collapsed branches without child rows", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(explorer).toContain("entry.row.directChildCount > 0");
    expect(workspaceTree).toContain('v-else class="workspace-tree-branch-spacer"');
    expect(workspaceTree).not.toContain("empty: row.directChildCount === 0");
  });
});
