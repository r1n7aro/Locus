import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer tree UX", () => {
  it("indents leaf rows with a chevron-width spacer so hierarchy stays visible", () => {
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    // Every non-chevron row (documents included) renders the spacer; without
    // it a child document's name lands on the same x as its parent folder.
    expect(workspaceTree).toContain('v-else class="workspace-tree-branch-spacer"');
    expect(workspaceTree).toContain("paddingLeft: rowIndent(item.treeRow, baseIndent, indentSize)");
  });

  it("locks row heights to the virtualizer's row-height", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(explorer).toContain(':row-height="30"');
    // Shell, create row, and load row must all be exactly 30px tall, otherwise
    // FileTreeList's fixed-height spacer math drifts on long lists.
    expect(workspaceTree).toMatch(/\.workspace-tree-row\s*\{[^}]*min-height:\s*30px/s);
    expect(explorer).toMatch(/\.kx-create-row\s*\{[^}]*height:\s*30px/s);
    expect(explorer).toMatch(/\.kx-load-row\s*\{[^}]*height:\s*30px/s);
    expect(explorer).not.toContain(".kx-row-shell {");
  });

  it("distinguishes the opened row from multi-select marks", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('"is-open": props.selectedPath === node.path');
    expect(explorer).toContain('"is-marked": selectedPaths.value.has(node.path)');
    expect(explorer).toMatch(
      /\.workspace-tree-row-shell\.is-open[\s\S]*box-shadow:\s*inset 2px 0 0 var\(--accent-color\)/,
    );
    expect(explorer).toMatch(/\.workspace-tree-row-shell\.is-marked/);
  });

  it("uses folder rows for expansion and keeps configuration in the context menu", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(explorer).toContain("function activateNode(row: FlatRow) {");
    expect(explorer).toMatch(/if \(row\.node\.kind === "folder"\) \{\s*toggleExpansion\(row\);/);
    expect(explorer).toContain('@click="openSelectedFolderConfig"');
    expect(explorer).toContain('t("knowledge.explorer.folderConfig")');
    expect(workspaceTree).toContain('@click.stop="toggleBranch(item, $event)"');
    expect(workspaceTree).toContain("if (event.detail >= 2) return;");
    // Rapid repeated row clicks keep the expanded state stable.
    expect(explorer).toContain("if (event.detail >= 2) return;");
    expect(explorer).not.toContain("onRowDoubleClick");
    expect(explorer).not.toContain("@double-click");
    expect(explorer).toContain('@click="startRenameSelection"');
  });

  it("provides keyboard navigation with tree ARIA semantics", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(explorer).toContain('role="tree"');
    expect(workspaceTree).toContain('role="treeitem"');
    expect(workspaceTree).toContain(':aria-level="item.treeRow.depth + 1"');
    expect(explorer).toContain(":aria-activedescendant=\"focusedRowDomId\"");
    expect(explorer).toContain('@keydown="onTreeKeydown"');
    expect(explorer).toContain("resolveKnowledgeTreeKeyboardAction({");
    expect(explorer).toContain("function applyKeyboardAction(action: KnowledgeTreeKeyboardAction) {");
    // Roving focus: rows stay out of the tab order.
    expect(workspaceTree).toContain('tabindex="-1"');
  });

  it("reveals the selection via FileTreeList scrolling", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const treeList = read("src/components/explorer/FileTreeList.vue");

    expect(treeList).toContain("function scrollToIndex(index: number");
    expect(treeList).toContain("defineExpose({ scrollToIndex });");
    expect(explorer).toContain("function revealVisiblePath(");
    expect(explorer).toContain("treeListRef.value?.scrollToIndex(index, options);");
    expect(explorer).toContain("() => props.selectedPath");
  });

  it("keeps a useful virtual viewport while the knowledge tab is hidden", () => {
    const treeList = read("src/components/explorer/FileTreeList.vue");

    expect(treeList).toContain("new ResizeObserver(updateViewportMetrics)");
    expect(treeList).toContain("window.innerHeight");
    expect(treeList).toContain("if (nextViewportHeight > 0 || viewportHeight.value === 0)");
    expect(treeList).not.toContain("createAnimationFrameResizeObserver");
  });

  it("uses the shared tree without the legacy toolbar", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('import WorkspaceTree, {');
    expect(explorer).toContain('<WorkspaceTree');
    expect(explorer).not.toContain('class="kx-toolbar"');
    expect(explorer).not.toContain("openToolbarCreate");
    expect(explorer).toContain('class="kx-empty-actions"');
    expect(explorer).toContain("openEmptyStateCreate('document')");
  });

  it("marks managed rows and disables their destructive menu items", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("t('knowledge.explorer.pluginManaged')");
    expect(explorer).toContain('class="kx-lock"');
    expect(explorer).toContain("function isPackageContentNode(");
    expect(explorer).toContain("function deleteBlocked(");
    expect(explorer).toContain("function renameBlocked(");
    expect(explorer).toContain('t("knowledge.explorer.pluginManagedHint")');
    expect(explorer).toContain('t("knowledge.explorer.packageManagedHint")');
    expect(explorer).toContain(':disabled="deleteBlocked(ctxMenu)"');
  });

  it("supports multi-node drags with pruning and batched moves", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const view = read("src/components/KnowledgeView.vue");
    const state = read("src/composables/useKnowledgeState.ts");

    expect(explorer).toContain("pruneKnowledgeDragNodes(");
    expect(explorer).toContain('emit("moveNodes", movable, targetDir, row.node.type);');
    expect(explorer).toContain("function scheduleDragExpand(row: FlatRow) {");
    expect(explorer).toContain("DRAG_EXPAND_DELAY_MS");
    expect(view).toContain('@move-nodes="handleMoveNodes"');
    expect(state).toContain("async function moveExplorerNodes(");
  });

  it("keeps reveal expansion in the knowledge state without toolbar wiring", () => {
    const view = read("src/components/KnowledgeView.vue");
    const state = read("src/composables/useKnowledgeState.ts");

    expect(state).toContain("function expandAncestors(path: string) {");
    expect(view).toContain("if (selectedPath.value) expandAncestors(selectedPath.value);");
    expect(view).not.toContain('@collapse-all=');
    expect(view).not.toContain('@expand-to-selection=');
  });

  it("filters search results inside the existing tree hierarchy", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const view = read("src/components/KnowledgeView.vue");
    const state = read("src/composables/useKnowledgeState.ts");

    expect(state).toContain("export function buildSearchExplorerTree(");
    expect(state).toContain("searchQuery.value.trim() ? searchExplorerTree.value");
    expect(state).toContain(": explorerTree.value");
    expect(explorer).toContain("!searchCollapsedPaths.value.has(node.path)");
    expect(explorer).toContain('emit("selectSearchResult", searchResult)');
    expect(explorer).not.toContain('class="kx-search-row"');
    expect(explorer).not.toMatch(/v-for="result in searchResults"/);
    expect(explorer).toContain('t("knowledge.search.revealInTree")');
    expect(view).toContain("async function handleRevealSearchResult(");
    expect(view).toContain("copySearchResultRelativePath");
  });

  it("commits inline create on outside click like rename", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("if (inlineCreate.value.name.trim()) submitInlineCreate();");
  });

  it("degrades secondary badges at narrow sidebar widths via container query", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("container-type: inline-size;");
    expect(explorer).toContain("@container (max-width: 259px)");
    expect(explorer).toMatch(/\.kx-row-side \.kx-flag\.flag-command,/);
  });

  it("omits folder counts and the removed toolbar legend", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const labels = read("src/components/knowledge/knowledgeMetaLabels.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    // Folder/package rows must not render a descendant-document count badge —
    // the number added noise without a decision the user could take from it.
    expect(explorer).not.toContain("kx-count");
    expect(explorer).not.toContain("descendantDocumentCount");

    expect(explorer).not.toContain('t("knowledge.explorer.legend")');
    expect(explorer).not.toContain("kx-legend-menu");
    expect(labels).toContain("export function buildKnowledgeLegendEntries(");
    for (const key of [
      "knowledge.explorer.pluginManaged",
      "knowledge.explorer.pluginManagedHint",
      "knowledge.explorer.packageManaged",
      "knowledge.explorer.packageManagedHint",
      "knowledge.search.openResult",
      "knowledge.search.revealInTree",
      "knowledge.legend.autoDesc",
      "knowledge.legend.searchOnDesc",
      "knowledge.legend.externalDesc",
      "knowledge.legend.commandDesc",
    ]) {
      expect(zh).toContain(`"${key}"`);
      expect(en).toContain(`"${key}"`);
    }
  });
});
