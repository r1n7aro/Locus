import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeView state persistence", () => {
  it("keeps the current knowledge selection when switching tabs", () => {
    const view = read("src/components/KnowledgeView.vue");

    expect(view).not.toMatch(/watch\(\s*\(\)\s*=>\s*uiStore\.activeTab[\s\S]*clearSelection\(\)/);
  });

  it("refreshes retrieval settings when the workspace changes on the retrieval page", () => {
    const view = read("src/components/KnowledgeView.vue");

    expect(view).toContain('specialPage.value !== "retrieval"');
    expect(view).toContain("void refreshRetrievalState();");
    expect(view).toContain("normalizeWorkspaceKey(workingDir)");
  });
});
