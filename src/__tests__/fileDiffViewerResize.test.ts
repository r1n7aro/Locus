import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("FileDiffViewer hierarchy resize", () => {
  it("keeps the scene hierarchy column resizable with shared panel behavior", () => {
    const source = read("src/components/diff/FileDiffViewer.vue");

    expect(source).toContain('import { useResizablePanel } from "../../composables/useResizablePanel";');
    expect(source).toContain('storageKey: "locus:diff:scene-hierarchy-width"');
    expect(source).toContain('ref="sceneLayoutRef"');
    expect(source).toContain('class="hierarchy-resize-handle"');
    expect(source).toContain('@mousedown="onHierarchyResizeMouseDown"');
    expect(source).toContain(".semantic-layout.resizing-hierarchy");
    expect(source).toContain("cursor: col-resize;");
  });
});
