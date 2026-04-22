import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("commit detail tree view", () => {
  it("reuses the collab tree toggle for commit and stash file lists", () => {
    const component = read("src/components/collab/CommitDetail.vue");

    expect(component).toContain('readStoredStagingViewMode');
    expect(component).toContain('persistStagingViewMode');
    expect(component).toContain('buildStagingTreeRows');
    expect(component).toContain('class="view-toggle-btn"');
    expect(component).toContain(`v-if="fileViewMode === 'tree'"`);
    expect(component).toContain(`@click="toggleTreeFolder(row.chainPaths, row.expanded)"`);
  });
});
