import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("CollabSearchWindow performance", () => {
  it("keeps large result sets virtualized during column resizing", () => {
    const source = read("src/components/CollabSearchWindow.vue");

    expect(source).toContain("const RESULT_ROW_HEIGHT = 38");
    expect(source).toContain("const RESULT_ROW_BUFFER = 12");
    expect(source).toContain("const virtualResultRows = computed(() => {");
    expect(source).toContain('class="collab-search-result-spacer"');
    expect(source).toContain('v-for="{ result, top } in virtualResultRows.rows"');
    expect(source).toContain(':style="{ transform: `translateY(${top}px)` }"');
    expect(source).toMatch(/\.collab-search-result\s*\{[\s\S]*position:\s*absolute;[\s\S]*height:\s*var\(--collab-search-result-row-height\);/);
  });

  it("writes column drag changes through raf and persists only committed widths", () => {
    const source = read("src/components/CollabSearchWindow.vue");

    expect(source).toContain("let pendingColumnResizeFrame = 0");
    expect(source).toContain("function scheduleResultGridSizing(widths: Record<ResultColumnKey, number>)");
    expect(source).toContain("pendingColumnResizeFrame = requestAnimationFrame(flushScheduledResultGridSizing)");
    expect(source).toContain("function commitResultColumnWidths(widths: Record<ResultColumnKey, number>)");
    expect(source).toContain("persistResultColumnWidths();");
    expect(source).toContain("scheduleResultGridSizing(latestWidths);");
    expect(source).toContain("commitResultColumnWidths(latestWidths);");

    const moveHandler = source.match(/columnResizeMoveHandler = \(nextEvent: MouseEvent\) => \{[\s\S]*?\n  \};/);
    expect(moveHandler?.[0]).not.toContain("resultColumnWidths.value");
    expect(moveHandler?.[0]).not.toContain("persistResultColumnWidths()");
  });
});
