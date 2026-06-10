import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer load-more flow", () => {
  it("threads folder hydration state from the knowledge view model into the explorer", () => {
    const knowledgeView = read("src/components/KnowledgeView.vue");
    const knowledgeState = read("src/composables/useKnowledgeState.ts");

    expect(knowledgeState).toContain("function hasLoadedDirectoryDocuments(");
    expect(knowledgeState).toContain("hasLoadedDirectoryDocuments,");
    expect(knowledgeView).toContain("hasLoadedDirectoryDocuments,");
    expect(knowledgeView).toContain(
      "function hasLoadedActiveFolderDocuments(path: string): boolean {",
    );
    expect(knowledgeView).toContain(
      ':folder-documents-loaded="hasLoadedActiveFolderDocuments"',
    );
  });

  it("keeps initial folder expansion from auto-loading extra pages in the visible range", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("folderDocumentsLoaded: (path: string) => boolean;");
    expect(explorer).toContain("? props.folderDocumentsLoaded(node.relativePath)");
    expect(explorer).toContain("folderLoaded &&");
    // Folder pages chain only on scroll-driven range changes (stable row
    // count). Structural changes — expanding a folder, a page landing — keep
    // the row count moving and therefore never cascade extra folder loads.
    expect(explorer).toContain(
      "const scrollDriven = rowCount === lastVisibleRangeRowCount;",
    );
    expect(explorer).toContain("if (!scrollDriven) continue;");
    expect(explorer).toContain('emit("loadMoreFolder", entry.path);');
    expect(explorer).toContain("function requestLoadMore(entry: VisibleEntry) {");
    expect(explorer).toContain('@click="requestLoadMore(entry)"');
    expect(explorer).toContain(':disabled="entry.loading"');
  });
});
