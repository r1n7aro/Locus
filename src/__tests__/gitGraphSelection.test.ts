import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("git graph selection API", () => {
  it("exposes commit and stash selection methods from GitGraph", () => {
    const selectionTypes = read("src/components/collab/gitGraphSelection.ts");
    const gitGraph = read("src/components/collab/GitGraph.vue");

    expect(selectionTypes).toContain('kind: "commit"; hash: string');
    expect(selectionTypes).toContain('kind: "stash"; hash: string');
    expect(selectionTypes).toContain("interface GitGraphPublicApi");
    expect(gitGraph).toContain("defineExpose<GitGraphPublicApi>");
    expect(gitGraph).toContain("selectHistory");
    expect(gitGraph).toContain("scrollToHistory");
  });

  it("routes sidebar tag clicks through the graph selection API", () => {
    const collabView = read("src/components/CollabView.vue");
    const gitSidebar = read("src/components/collab/GitSidebar.vue");

    expect(collabView).toContain('ref="gitGraphRef"');
    expect(collabView).toContain('@select-tag="onSelectTag"');
    expect(collabView).toContain('selectHistoryInGraph({ kind: "commit", hash: tag.targetHash })');
    expect(gitSidebar).toContain('(e: "selectTag", tag: GitGraphRef): void');
    expect(gitSidebar).toContain("@click=\"emit('selectTag', tag)\"");
  });

});
