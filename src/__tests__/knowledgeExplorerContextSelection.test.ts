import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer contextual selection", () => {
  it("keeps context-menu selection separate from the primary preview selection", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("const contextMenuPath = computed(() => {");
    expect(explorer).toContain('if (!menu || menu.kind === "root") return null;');
    expect(explorer).toContain("const contextSelectedPath = computed(() => {");
    expect(explorer).toContain("if (props.selectedPath === path) return null;");
    expect(explorer).toContain('"context-selected": contextSelectedPath.value === node.path');
  });

  it("renders a distinct contextual highlight style in the explorer list", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toMatch(/\.workspace-tree-row-shell\.context-selected\)[\s\S]*background:\s*color-mix\(in srgb,\s*var\(--active-bg\)\s*52%,\s*var\(--hover-bg\)\s*48%\);/);
    expect(explorer).toContain("box-shadow: inset 0 0 0 1px");
  });

  it("highlights the active search match through the shared tree row state", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("props.selectedPath === node.path");
    expect(explorer).toContain('"is-open": props.selectedPath === node.path');
    expect(explorer).toMatch(/\.workspace-tree-row-shell\.is-open[\s\S]*background:\s*var\(--active-bg\);/);
    expect(explorer).not.toContain("isSelectedSearchResult");
  });

  it("keeps search-result section labels out of filtered tree rows", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).not.toContain("function searchMeta(result: KnowledgeSearchResult): string {");
    expect(explorer).not.toContain('class="kx-search-meta"');
  });

  it("offers rename, relative-path copy, and file-system reveal actions for single-item context menus", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('emit("renameDocument", draft.relativePath, name, draft.type);');
    expect(explorer).toContain('emit("copyRelativePath", menu.node);');
    expect(explorer).toContain('emit("openInFileSystem", menu.node);');
    expect(explorer).toContain('t("knowledge.explorer.deletePackage")');
    expect(explorer).toContain('t("knowledge.explorer.copyRelativePath")');
    expect(explorer).toContain('t("knowledge.explorer.openInFileSystem")');
  });

  it("adds a reference-only external import entry to root and folder context menus", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('emit("requestExternalImportFolder", parentDir);');
    expect(explorer).toContain("contextMenuType(ctxMenu) === 'reference'");
    expect(explorer).toContain('t("knowledge.explorer.importExternalFolder")');
  });

  it("disables drag semantics and button wrapping while a row is being renamed", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("function isRenamingRow(row: FlatRow): boolean {");
    expect(explorer).toContain("editing: isRenamingRow(row)");
    expect(explorer).toContain("draggable: !isSearchMode.value && canDragNode(node)");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");
    expect(workspaceTree).toContain(':draggable="item.treeRow.draggable && !item.treeRow.editing"');
    expect(workspaceTree).toContain(`:is="item.treeRow.editing ? 'div' : 'button'"`);
    expect(explorer).toContain("@pointerdown.stop");
  });
});
