import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const templateSource = readFileSync(
  resolve(process.cwd(), "src-tauri/src/view/templates/serialized_table.rs"),
  "utf8",
);

describe("serialized table View layout", () => {
  it("uses measured column widths and horizontal scrolling instead of compressing headers", () => {
    expect(templateSource).toContain("const TABLE_MIN_DATA_COLUMN_WIDTH = 88;");
    expect(templateSource).toContain("const TABLE_MAX_DATA_COLUMN_WIDTH = 220;");
    expect(templateSource).toContain("const assetColumnWidth = computed(() =>");
    expect(templateSource).toContain("const statusColumnWidth = computed(() =>");
    expect(templateSource).toContain("columnWidthFromOverride(tableColumnKey(column), autoColumnWidth(column), TABLE_MIN_DATA_COLUMN_WIDTH)");
    expect(templateSource).toContain("width: `max(100%, ${tablePixelWidth.value}px)`");
    expect(templateSource).toContain("<colgroup>");
    expect(templateSource).toContain(":style=\"{ width: columnWidths[index] + 'px' }\"");
    expect(templateSource).toContain("class=\"column-label\"");
    expect(templateSource).toContain("scrollbar-gutter: stable both-edges;");
    expect(templateSource).toContain("overscroll-behavior: contain;");
    expect(templateSource).toContain("touch-action: pan-x pan-y;");
    expect(templateSource).toContain(".value-cell .unity-property-tree");
  });

  it("supports resizable column separators with persisted widths", () => {
    expect(templateSource).toContain("const columnWidthOverrides = ref<Record<string, number>>({});");
    expect(templateSource).toContain("function startColumnResize(event: PointerEvent");
    expect(templateSource).toContain("function updateColumnResize(event: PointerEvent)");
    expect(templateSource).toContain("function persistColumnWidths()");
    expect(templateSource).toContain("columnWidths: columnWidthOverrides.value");
    expect(templateSource).toContain("class=\"column-resize-handle\"");
    expect(templateSource).toContain('v-for="(column, index) in columns"');
    expect(templateSource).toContain("@pointerdown.prevent.stop=\"startColumnResize");
    expect(templateSource).toContain("@dblclick.prevent.stop=\"resetColumnWidth");
    expect(templateSource).toContain("border-right: 1px solid");
    expect(templateSource).toContain("body.serialized-table-resizing");
    expect(templateSource).toContain("stopColumnResize(false);");
  });

  it("logs table layout failures when rendered columns overlap or lose scrollability", () => {
    expect(templateSource).toContain("function checkTableLayout()");
    expect(templateSource).toContain("header ${index + 1} overlaps previous column");
    expect(templateSource).toContain("horizontal overflow is not scrollable");
    expect(templateSource).toContain('console.error("[serialized-table] Layout check failed"');
    expect(templateSource).toContain('window.addEventListener("resize", scheduleTableLayoutCheck)');
    expect(templateSource).toContain('window.removeEventListener("resize", scheduleTableLayoutCheck)');
  });

  it("keeps mouse wheel scrolling bound to the table scroller", () => {
    expect(templateSource).toContain("function handleTableWheel(event: WheelEvent)");
    expect(templateSource).toContain('@wheel.capture="handleTableWheel"');
    expect(templateSource).toContain("scroller.scrollTop += event.deltaY;");
    expect(templateSource).toContain("scroller.scrollLeft += event.deltaX || event.deltaY;");
    expect(templateSource).toMatch(/\.table-workspace\s*\{[\s\S]*height:\s*100%;[\s\S]*display:\s*flex;/);
    expect(templateSource).toMatch(/\.table-pane\s*\{[\s\S]*flex:\s*1;[\s\S]*height:\s*100%;/);
  });

  it("keeps row header content vertically centered in tall rows", () => {
    expect(templateSource).toContain('class="asset-cell-content"');
    expect(templateSource).toContain('class="status-cell-content"');
    expect(templateSource).toMatch(/\.asset-cell,\s*[\s\S]*\.status-cell\s*\{[\s\S]*vertical-align:\s*middle;/);
    expect(templateSource).toMatch(/\.asset-cell-content,\s*[\s\S]*\.status-cell-content\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;[\s\S]*justify-content:\s*center;/);
  });
});
