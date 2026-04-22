import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("ConsoleSettings layout", () => {
  it("supports resizable log columns with persisted widths", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");

    expect(source).toContain('const CONSOLE_COLUMN_STORAGE_KEY = "locus.settings.console.columns.v1"');
    expect(source).toContain('const activeResizeColumn = ref<ResizableConsoleColumn | null>(null)');
    expect(source).toContain("const columnWidths = ref<ConsoleColumnWidths>(loadStoredColumnWidths())");
    expect(source).toContain("function loadStoredColumnWidths(): ConsoleColumnWidths {");
    expect(source).toContain("function persistColumnWidths()");
    expect(source).toContain("function onColumnResizeStart(event: MouseEvent, column: ResizableConsoleColumn)");
    expect(source).toContain("document.body.style.cursor = \"col-resize\"");
    expect(source).toContain("releaseColumnResizeSelectionLock = acquireSelectionLock()");
    expect(source).toContain('class="console-header"');
    expect(source).toContain('class="console-column-handle"');
    expect(source).toContain("@mousedown=\"onColumnResizeStart($event, 'timeWidth')\"");
    expect(source).toContain("@mousedown=\"onColumnResizeStart($event, 'sourceWidth')\"");
    expect(source).toContain("@mousedown=\"onColumnResizeStart($event, 'moduleWidth')\"");
    expect(source).toContain("@keydown.left.prevent=\"nudgeColumnWidth('timeWidth', -12)\"");
    expect(source).toMatch(/\.console-header,\s*\.console-row\s*\{[\s\S]*grid-template-columns:[\s\S]*var\(--console-time-width\)[\s\S]*var\(--console-source-width\)[\s\S]*var\(--console-module-width\)[\s\S]*minmax\(var\(--console-message-min-width\),\s*1fr\);/);
    expect(source).toMatch(/\.console-column-handle\s*\{[\s\S]*cursor:\s*col-resize;/);
  });

  it("formats timestamps to seconds instead of milliseconds", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");

    expect(source).toContain("function formatTime(timestampMs: number): string {");
    expect(source).toContain("const seconds = String(date.getSeconds()).padStart(2, \"0\")");
    expect(source).toContain("return `${hours}:${minutes}:${seconds}`;");
    expect(source).not.toContain("getMilliseconds");
  });

  it("defines localized console column labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.console.column.time": "时间"');
    expect(zh).toContain('"settings.console.column.source": "来源"');
    expect(zh).toContain('"settings.console.column.module": "模块"');
    expect(zh).toContain('"settings.console.column.message": "内容"');
    expect(en).toContain('"settings.console.column.time": "Time"');
    expect(en).toContain('"settings.console.column.source": "Source"');
    expect(en).toContain('"settings.console.column.module": "Module"');
    expect(en).toContain('"settings.console.column.message": "Message"');
  });
});
