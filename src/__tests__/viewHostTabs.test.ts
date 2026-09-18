import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
describe("native View editor tabs", () => {
  it("uses the Workbench component tree with no content WebView creation path", () => {
    const editor = readFileSync("src/components/workbench/WorkbenchViewEditor.vue", "utf8");
    const backend = readFileSync("src-tauri/src/view.rs", "utf8");
    const commands = readFileSync("src-tauri/src/lib.rs", "utf8");
    expect(editor).toContain("<ViewRuntimeHost");
    expect(editor).not.toMatch(/viewContentMount|outerPosition|scaleFactor|ResizeObserver/);
    expect(backend).not.toContain("fn build_view_content_window(");
    expect(backend).not.toContain("fn position_view_content_child_window(");
    expect(commands).not.toContain("commands::view_content_mount");
  });
});
