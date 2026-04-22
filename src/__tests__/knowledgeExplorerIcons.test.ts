import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer row icons", () => {
  it("renders asset-style folder icons while keeping document bullets", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('class="kx-kind-icon folder"');
    expect(explorer).toContain(':class="{ open: entry.row.expanded }"');
    expect(explorer).toContain('<span v-else class="kx-bullet-slot">');
    expect(explorer).toContain('<span class="kx-bullet"></span>');
    expect(explorer).toContain(".kx-kind-icon.folder {");
  });
});
