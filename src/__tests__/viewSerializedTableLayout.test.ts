import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const templateSource = readFileSync(
  resolve(process.cwd(), "src-tauri/src/view/templates/serialized_table.rs"),
  "utf8",
);
const componentSource = readFileSync(
  resolve(process.cwd(), "src/components/table/SerializedTableView.vue"),
  "utf8",
);

describe("serialized table View layout", () => {
  it("uses measured column widths and horizontal scrolling instead of compressing headers", () => {
    expect(componentSource).toContain("const MIN_DATA_COLUMN_WIDTH = 88;");
    expect(componentSource).toContain("const MAX_DATA_COLUMN_WIDTH = 220;");
    expect(componentSource).toContain("const assetColumnWidth = computed(() =>");
    expect(componentSource).toContain("const statusColumnWidth = computed(() =>");
    expect(componentSource).toContain("widthFromOverride(columnKey(column), autoColumnWidth(column), MIN_DATA_COLUMN_WIDTH)");
    expect(componentSource).toContain("width: `max(100%, ${tablePixelWidth.value}px)`");
    expect(componentSource).toContain("<colgroup>");
    expect(componentSource).toContain(":style=\"{ width: columnWidths[index] + 'px' }\"");
    expect(componentSource).toContain("class=\"column-label\"");
    expect(componentSource).toContain("scrollbar-gutter: stable both-edges;");
    expect(componentSource).toContain("overscroll-behavior: contain;");
    expect(componentSource).toContain("touch-action: pan-x pan-y;");
    expect(componentSource).toContain(".value-cell :deep(.unity-property-tree)");
  });

  it("supports resizable column separators with persisted widths", () => {
    expect(componentSource).toContain("const columnWidthOverrides = ref<Record<string, number>>({ ...props.columnWidths });");
    expect(componentSource).toContain("function startColumnResize(event: PointerEvent");
    expect(componentSource).toContain("function updateColumnResize(event: PointerEvent)");
    expect(templateSource).toContain("function persistColumnWidths(widths: Record<string, number>)");
    expect(templateSource).toContain(":column-widths=\"columnWidths\"");
    expect(componentSource).toContain("class=\"column-resize-handle\"");
    expect(componentSource).toContain('v-for="(column, index) in columns"');
    expect(componentSource).toContain("@pointerdown.prevent.stop=\"startColumnResize");
    expect(componentSource).toContain("@dblclick.prevent.stop=\"resetColumnWidth");
    expect(componentSource).toContain("border-right: 1px solid");
    expect(componentSource).toContain("body.locus-serialized-table-resizing");
    expect(componentSource).toContain("stopColumnResize(false);");
  });

  it("logs table layout failures when rendered columns overlap or lose scrollability", () => {
    expect(componentSource).toContain("function checkTableLayout()");
    expect(componentSource).toContain("header ${index + 1} overlaps previous column");
    expect(componentSource).toContain("horizontal overflow is not scrollable");
    expect(componentSource).toContain('console.error("[serialized-table] Layout check failed"');
    expect(componentSource).toContain('window.addEventListener("resize", scheduleLayoutCheck)');
    expect(componentSource).toContain('window.removeEventListener("resize", scheduleLayoutCheck)');
  });

  it("keeps mouse wheel scrolling bound to the table scroller", () => {
    expect(componentSource).toContain("function handleTableWheel(event: WheelEvent)");
    expect(componentSource).toContain('@wheel.capture="handleTableWheel"');
    expect(componentSource).toContain("scroller.scrollTop += event.deltaY;");
    expect(componentSource).toContain("scroller.scrollLeft += event.deltaX || event.deltaY;");
    expect(componentSource).toMatch(/\.locus-serialized-table\s*\{[\s\S]*height:\s*100%;[\s\S]*display:\s*flex;/);
    expect(componentSource).toMatch(/\.locus-serialized-table-scroller\s*\{[\s\S]*flex:\s*1;[\s\S]*overflow:\s*auto;/);
  });

  it("keeps row header content vertically centered in tall rows", () => {
    expect(componentSource).toContain('class="asset-cell-content"');
    expect(componentSource).toContain('class="status-cell-content"');
    expect(componentSource).toMatch(/\.asset-cell,\s*[\s\S]*\.status-cell\s*\{[\s\S]*vertical-align:\s*middle;/);
    expect(componentSource).toMatch(/\.asset-cell-content,\s*[\s\S]*\.status-cell-content\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;[\s\S]*justify-content:\s*center;/);
  });

  it("wires the generated template through the shared SerializedTableView component", () => {
    expect(templateSource).toContain("import { SerializedTableView } from \"@locus/components\";");
    expect(templateSource).toContain("resolveSerializedTableSources");
    expect(templateSource).toContain("<SerializedTableView");
    expect(templateSource).toContain("@update:column-widths=\"persistColumnWidths\"");
    expect(templateSource).toContain("@commit=\"commitCell\"");
  });
});
