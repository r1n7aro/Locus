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

    expect(service).toContain("export interface ViewFrontendLogRequest");
    expect(service).toContain("export function viewAppendFrontendLog");
    expect(service).toContain('"view_append_frontend_log"');

    expect(commands).toContain("pub async fn view_append_frontend_log");
    expect(commands).toContain("append_view_frontend_log_sync");
    expect(runtime).toContain("VIEW_FRONTEND_LOG_REL_PATH");
    expect(runtime).toContain(".locus/logs/frontend.log");
    expect(runtime).toContain("OpenOptions::new()");
    expect(lib).toContain("commands::view_append_frontend_log");
  });
});
