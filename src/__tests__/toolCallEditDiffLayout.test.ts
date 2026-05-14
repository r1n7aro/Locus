import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool call edit diff layout", () => {
  it("uses the single-column diff viewer by default for edit previews", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");
    const fileDiffViewer = read("src/components/diff/FileDiffViewer.vue");

    expect(fileDiffViewer).toContain('{ mode: "unified", compact: false, filter: "all", hideBuiltinTabs: false, hideTextDisplayControls: false }');
    expect(toolBlock).not.toContain('mode="side-by-side"');
  });

  it("keeps the edit diff fallback stacked in one column", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");

    expect(toolBlock).toMatch(/\.edit-diff-container\s*\{[\s\S]*display:\s*flex/);
    expect(toolBlock).toMatch(/\.edit-diff-container\s*\{[\s\S]*flex-direction:\s*column/);
    expect(toolBlock).toMatch(/\.edit-diff-old\s*\{[\s\S]*border-bottom:/);
    expect(toolBlock).not.toContain("grid-template-columns: 1fr 1fr;");
  });
});
