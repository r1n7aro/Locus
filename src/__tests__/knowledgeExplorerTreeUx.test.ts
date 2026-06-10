import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer tree UX", () => {
  it("indents leaf rows with a chevron-width spacer so hierarchy stays visible", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    // Every non-chevron row (documents included) renders the spacer; without
    // it a child document's name lands on the same x as its parent folder.
    expect(explorer).toMatch(
      /v-else\s*\n\s*class="kx-branch-spacer"/,
    );
    expect(explorer).not.toMatch(
      /v-else-if="entry\.row\.node\.kind !== 'document'"\s*\n\s*class="kx-branch-spacer"/,
    );
  });

  it("locks row heights to the virtualizer's row-height", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain(':row-height="30"');
    // Shell, create row, and load row must all be exactly 30px tall, otherwise
    // FileTreeList's fixed-height spacer math drifts on long lists.
    expect(explorer).toMatch(/\.kx-row-shell\s*\{[^}]*height:\s*30px/s);
    expect(explorer).toMatch(/\.kx-create-row\s*\{[^}]*height:\s*30px/s);
    expect(explorer).toMatch(/\.kx-load-row\s*\{[^}]*height:\s*30px/s);
    expect(explorer).not.toMatch(/\.kx-row\s*\{[^}]*min-height:\s*26px/s);
  });

  it("distinguishes the opened row from multi-select marks", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("'is-open': selectedPath === entry.row.node.path");
    expect(explorer).toContain("'is-marked': selectedPaths.has(entry.row.node.path)");
    expect(explorer).toMatch(
      /\.kx-row-shell\.is-open[\s\S]*box-shadow:\s*inset 2px 0 0 var\(--accent-color\)/,
    );
    expect(explorer).toMatch(/\.kx-row-shell\.is-marked/);
  });

  it("unifies click semantics: folders select + toggle, open packages toggle", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("function activateNode(row: FlatRow) {");
    expect(explorer).toContain('emit("selectFolderConfig", row.node.relativePath);');
    expect(explorer).toContain("if (props.selectedPath === row.node.path) {");
    // The second click of a double-click must not re-toggle.
    expect(explorer).toContain("if (event.detail >= 2) return;");
    expect(explorer).toContain("function onRowDoubleClick(row: FlatRow) {");
  });

  it("provides keyboard navigation with tree ARIA semantics", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('role="tree"');
    expect(explorer).toContain('role="treeitem"');
    expect(explorer).toContain(':aria-level="entry.row.node.depth"');
    expect(explorer).toContain(":aria-activedescendant=\"focusedRowDomId\"");
    expect(explorer).toContain('@keydown="onTreeKeydown"');
    expect(explorer).toContain("resolveKnowledgeTreeKeyboardAction({");
    expect(explorer).toContain("function applyKeyboardAction(action: KnowledgeTreeKeyboardAction) {");
    // Roving focus: rows stay out of the tab order.
    expect(explorer).toContain('tabindex="-1"');
  });

  it("reveals the selection via FileTreeList scrolling", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const treeList = read("src/components/explorer/FileTreeList.vue");

    expect(treeList).toContain("function scrollToIndex(index: number");
    expect(treeList).toContain("defineExpose({ scrollToIndex });");
    expect(explorer).toContain("function revealVisiblePath(");
    expect(explorer).toContain("function requestRevealSelection() {");
    expect(explorer).toContain('emit("expandToSelection");');
  });

  it("ships a tree toolbar plus empty-state actions", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('class="kx-toolbar"');
    expect(explorer).toContain("openToolbarCreate('document')");
    expect(explorer).toContain("openToolbarCreate('folder')");
    expect(explorer).toContain("function toolbarImport() {");
    expect(explorer).toContain("emit('collapseAll')");
    expect(explorer).toContain("t('knowledge.explorer.revealSelection')");
    expect(explorer).toContain('class="kx-empty-actions"');
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
    expect(explorer).toContain('emit("moveNodes", movable, targetDir);');
    expect(explorer).toContain("function scheduleDragExpand(row: FlatRow) {");
    expect(explorer).toContain("DRAG_EXPAND_DELAY_MS");
    expect(view).toContain('@move-nodes="handleMoveNodes"');
    expect(state).toContain("async function moveExplorerNodes(");
  });

  it("wires collapse-all and reveal through the knowledge state", () => {
    const view = read("src/components/KnowledgeView.vue");
    const state = read("src/composables/useKnowledgeState.ts");

    expect(state).toContain("function collapseAllForType(type: KnowledgeDocumentType) {");
    expect(view).toContain('@collapse-all="handleCollapseAll"');
    expect(view).toContain('@expand-to-selection="handleExpandToSelection"');
  });

  it("enriches search results with snippets, reveal, and a context menu", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const view = read("src/components/KnowledgeView.vue");

    expect(explorer).toContain("buildKnowledgeSnippetSegments(");
    expect(explorer).toContain('class="kx-search-snippet"');
    expect(explorer).toContain('class="kx-search-mark"');
    expect(explorer).toContain('class="kx-search-reveal"');
    expect(explorer).toContain("openSearchContextMenu($event, result)");
    expect(explorer).toContain('t("knowledge.search.revealInTree")');
    // Search rows host an inner reveal button, so they cannot be <button>.
    expect(explorer).toMatch(/<div\s*\n\s*v-for="result in searchResults"/);
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

  it("explains badges via the legend and omits folder document counts", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const labels = read("src/components/knowledge/knowledgeMetaLabels.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    // Folder/package rows must not render a descendant-document count badge —
    // the number added noise without a decision the user could take from it.
    expect(explorer).not.toContain("kx-count");
    expect(explorer).not.toContain("descendantDocumentCount");

    expect(explorer).toContain('t("knowledge.explorer.legend")');
    expect(labels).toContain("export function buildKnowledgeLegendEntries(");
    for (const key of [
      "knowledge.explorer.collapseAll",
      "knowledge.explorer.revealSelection",
      "knowledge.explorer.legend",
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
