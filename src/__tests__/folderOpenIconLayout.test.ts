import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

const closedFolderPath = 'd="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"';
const openFolderPath = 'd="M2.5 4.5a1.25 1.25 0 0 1 1.25-1.25h1.9c.28 0 .55.11.74.31l.98.98c.2.2.46.31.74.31h4.14a1.25 1.25 0 0 1 1.25 1.25v5.1a1.25 1.25 0 0 1-1.25 1.25h-8.5A1.25 1.25 0 0 1 2.5 11.2V4.5Z"';

describe("folder open icons", () => {
  it("keeps the original closed folder icon and uses the flat outline icon when expanded", () => {
    const knowledgeExplorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const assetExplorer = read("src/components/asset/AssetExplorer.vue");
    const assetLegacyExplorer = read("src/components/asset/AssetLegacyExplorer.vue");

    for (const source of [knowledgeExplorer, assetExplorer, assetLegacyExplorer]) {
      expect(source).toContain(closedFolderPath);
      expect(source).toContain(openFolderPath);
      expect(source).toContain('fill="currentColor"');
      expect(source).toContain('stroke-linecap="round"');
    }
  });
});
