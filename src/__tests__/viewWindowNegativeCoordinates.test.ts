import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
describe("View positioning", () => {
  it("lets the native Workbench layout own position on every monitor", () => {
    const editor = readFileSync("src/components/workbench/WorkbenchViewEditor.vue", "utf8");
    expect(editor).not.toMatch(/getBoundingClientRect|set_position|outerPosition|viewContentMount/);
    expect(editor).toContain("<ViewRuntimeHost");
  });
});
