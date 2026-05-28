import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View frontend logs", () => {
  it("captures View host console output into the package log command", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const service = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(host).toContain("installViewConsoleLogCapture");
    expect(host).toContain("CONSOLE_LOG_LEVELS");
    expect(host).toContain("viewAppendFrontendLog({ viewId: activeViewId, level, message })");
    expect(host).toContain('window.addEventListener("unhandledrejection"');
    expect(host).toContain("viewReadFrontendLog({ viewId, limit: 1 })");
    expect(host).toContain("viewOpenFrontendLog(viewId)");
    expect(host).toContain("view-host-logbar");
    expect(host).toContain("RUNTIME_STATUSBAR_SELECTOR");
    expect(host).toContain("embeddedLogbarSlot");
    expect(host).toContain("view-host-logbar-inline");
    expect(host).toContain("view-host-logbar-slot");
    expect(host).toContain("Double-click to open frontend.log");

    expect(service).toContain("export interface ViewFrontendLogRequest");
    expect(service).toContain("export interface ViewFrontendLogEntry");
    expect(service).toContain("export function viewAppendFrontendLog");
    expect(service).toContain("export function viewReadFrontendLog");
    expect(service).toContain("export function viewOpenFrontendLog");
    expect(service).toContain('"view_append_frontend_log"');
    expect(service).toContain('"view_read_frontend_log"');
    expect(service).toContain('"view_open_frontend_log"');

    expect(commands).toContain("pub async fn view_append_frontend_log");
    expect(commands).toContain("pub async fn view_read_frontend_log");
    expect(commands).toContain("pub async fn view_open_frontend_log");
    expect(commands).toContain("append_view_frontend_log_sync");
    expect(commands).toContain("read_view_frontend_log_sync");
    expect(commands).toContain("open_view_frontend_log_sync");
    expect(runtime).toContain("VIEW_FRONTEND_LOG_REL_PATH");
    expect(runtime).toContain(".locus/logs/frontend.log");
    expect(runtime).toContain("OpenOptions::new()");
    expect(runtime).toContain("pub fn read_view_frontend_log_sync");
    expect(runtime).toContain("pub fn open_view_frontend_log_sync");
    expect(lib).toContain("commands::view_append_frontend_log");
    expect(lib).toContain("commands::view_read_frontend_log");
    expect(lib).toContain("commands::view_open_frontend_log");
  });

  it("logs caught View template errors to the frontend console", () => {
    const serializedTable = read("src-tauri/src/view/templates/serialized_table.rs");
    const fieldBlocks = read("src-tauri/src/view/templates/field_blocks.rs");
    const nodeGraph = read("src-tauri/src/view/templates/node_graph.rs");

    expect(serializedTable).toContain("console.error(`[serialized-table] Source provider failed: ${label}`, error);");
    expect(serializedTable).toContain('console.error("[serialized-table] Read failed", error);');
    expect(serializedTable).toContain('console.error("[serialized-table] Write failed", error);');
    expect(fieldBlocks).toContain('console.error("[field-blocks] Read failed", error);');
    expect(fieldBlocks).toContain('console.error("[field-blocks] Write failed", error);');
    expect(nodeGraph).toContain('console.error("[node-graph] Read failed", error);');
  });

  it("keeps View automation requests resilient during host startup and pointer drag", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const runtime = read("src-tauri/src/view.rs");
    const canvas = read("src/components/canvas/LocusCanvasView.ts");
    const serializedTable = read("src-tauri/src/view/templates/serialized_table.rs");

    expect(runtime).toContain("let retry_interval = Duration::from_millis(200)");
    expect(runtime).toContain("window.emit(VIEW_AUTOMATION_REQUEST_EVENT, event.clone())");
    expect(host).toContain("function shouldHandleAutomationRequest");
    const mountedBlock = host.slice(host.indexOf("onMounted(async () => {"), host.indexOf("onUnmounted(() => {"));
    expect(mountedBlock.indexOf("unsubscribeAutomation = await")).toBeLessThan(mountedBlock.indexOf("await loadView()"));
    expect(host).toContain("function dispatchDragSequence");
    expect(host).toContain("new PointerEvent");
    expect(canvas).toContain("function trySetPointerCapture");
    expect(serializedTable).toContain("Synthetic View automation pointer events");
  });
});
