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
    expect(explorer).toContain("if (entry.path) continue;");
    expect(explorer).toContain("function requestLoadMore(entry: VisibleEntry) {");
    expect(explorer).toContain('emit("loadMoreFolder", entry.path);');
    expect(explorer).toContain('@click="requestLoadMore(entry)"');
    expect(explorer).toContain(':disabled="entry.loading"');
  });
});
