import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AssetExplorer branch probe layout", () => {
  it("probes visible folders before rendering expand toggles", () => {
    const source = read("src/components/asset/AssetExplorer.vue");

    expect(source).toContain('(e: "probe", path: string): void;');
    expect(source).toContain("if (!folder.hasChildFoldersKnown) return false;");
    expect(source).toContain('if (entry.kind === "row") {');
    expect(source).toContain('emit("probe", entry.node.path);');
    expect(source).toContain('v-if="entry.canToggle"');
  });
});
