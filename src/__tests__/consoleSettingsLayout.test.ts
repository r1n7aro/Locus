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

  it("renders newest logs first and keeps auto-scroll pinned to the latest row", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");

    expect(source).toContain(".map((entry, index) => ({ entry, index }))");
    expect(source).toContain("right.entry.timestampMs - left.entry.timestampMs || right.index - left.index");
    expect(source).toContain("function scrollToLatest()");
    expect(source).toContain("listRef.value.scrollTop = 0");
    expect(source).toContain("filteredEntries.value[0]?.id");
  });

  it("caps very long log messages with an expandable preview", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain("const CONSOLE_MESSAGE_MAX_HEIGHT = 132");
    expect(source).toContain("const CONSOLE_MESSAGE_PREVIEW_LIMIT = 4_000");
    expect(source).toContain("const expandedEntryIds = ref<Set<string>>(new Set())");
    expect(source).toContain("function displayMessage(entry: DebugConsoleEntry): string");
    expect(source).toContain("return entry.message.slice(0, CONSOLE_MESSAGE_PREVIEW_LIMIT)");
    expect(source).toContain('class="console-message-cell"');
    expect(source).toContain('class="console-message-meta"');
    expect(source).toContain('class="console-message-toggle"');
    expect(source).toMatch(/\.console-message\s*\{[\s\S]*max-height:\s*var\(--console-message-max-height\);[\s\S]*overflow:\s*auto;/);
    expect(zh).toContain('"settings.console.hiddenChars": "已隐藏 {0} 个字符"');
    expect(zh).toContain('"settings.console.expandMessage": "展开全部"');
    expect(zh).toContain('"settings.console.collapseMessage": "收起"');
    expect(en).toContain('"settings.console.hiddenChars": "{0} chars hidden"');
    expect(en).toContain('"settings.console.expandMessage": "Show All"');
    expect(en).toContain('"settings.console.collapseMessage": "Collapse"');
  });

  it("highlights visible search matches in log rows", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");

    expect(source).toContain("interface HighlightSegment");
    expect(source).toContain("const highlightPattern = computed<RegExp | null>");
    expect(source).toContain("function escapeRegExp(value: string): string");
    expect(source).toContain("function highlightSegments(text: string): HighlightSegment[]");
    expect(source).toContain("visibleSource.includes(query)");
    expect(source).toContain("highlightSegments(formatSource(entry.source))");
    expect(source).toContain("highlightSegments(entry.module)");
    expect(source).toContain("highlightSegments(displayMessage(entry))");
    expect(source).toContain('class="console-search-hit"');
    expect(source).toMatch(/\.console-search-hit\s*\{[\s\S]*background:\s*color-mix\(in srgb,\s*var\(--accent-color\)\s*38%,\s*transparent\);/);
    expect(source).toMatch(/\.console-search-hit\s*\{[\s\S]*color:\s*inherit;/);
  });

  it("adds a log export action with a .log save dialog", () => {
    const source = read("src/components/settings/ConsoleSettings.vue");
    const service = read("src/services/debugConsole.ts");
    const rustCommand = read("src-tauri/src/commands/log.rs");
    const rustLib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain('import { save } from "@tauri-apps/plugin-dialog"');
    expect(source).toContain("const isExporting = ref(false)");
    expect(source).toContain("function defaultExportFileName(): string");
    expect(source).toContain("return `locus-console-${year}${month}${day}-${hours}${minutes}${seconds}.log`");
    expect(source).toContain("async function exportLogs()");
    expect(source).toContain("filters: [{ name: \"Log\", extensions: [\"log\"] }]");
    expect(source).toContain("saveDebugConsoleLogExport(filePath, snapshot)");
    expect(source).toContain(':disabled="entries.length === 0 || isExporting"');
    expect(source).toContain('t("settings.console.export")');
    expect(service).toContain("export function formatDebugConsoleEntriesForLogExport");
    expect(service).toContain("export async function saveDebugConsoleLogExport");
    expect(service).toContain('invoke<string>("save_log_export"');
    expect(rustCommand).toContain("pub async fn save_log_export");
    expect(rustCommand).toContain("path.set_extension(\"log\")");
    expect(rustCommand).toContain("std::fs::write(&path, content.as_bytes())");
    expect(rustLib).toContain("commands::save_log_export");
    expect(zh).toContain('"settings.console.export": "导出日志"');
    expect(zh).toContain('"settings.console.exported": "已导出日志: {0}"');
    expect(en).toContain('"settings.console.export": "Export Logs"');
    expect(en).toContain('"settings.console.exported": "Exported logs: {0}"');
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
