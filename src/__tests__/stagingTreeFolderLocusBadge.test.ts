import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("staging tree folder Locus badges", () => {
  it("renders Locus tags on folder rows in the file changes tree", () => {
    const component = read("src/components/collab/StagingArea.vue");

    expect(component).toContain("getLocusManagedTagKindForPath");
    expect(component).toContain("function folderLocusBadgeLabel(path: string)");
    expect(component).toContain('v-if="folderLocusBadgeLabel(row.path)" class="locus-badge"');
  });

  it("keeps commit detail tree folder rows aligned with staging rows", () => {
    const component = read("src/components/collab/CommitDetail.vue");

    expect(component).toContain("getLocusManagedTagKindForPath");
    expect(component).toContain("function folderLocusBadgeLabel(path: string)");
    expect(component).toContain('v-if="folderLocusBadgeLabel(row.path)" class="locus-badge ui-select-none"');
  });
});
