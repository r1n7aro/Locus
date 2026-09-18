import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
describe("native View logging and automation", () => {
  it("batches instance logs without intercepting the native console", () => {
    const host = readFileSync("src/components/view/ViewRuntimeHost.vue", "utf8");
    expect(host).toContain("viewAppendFrontendLogs");
    expect(host).not.toContain("consoleForCapture");
    expect(host).not.toContain("MutationObserver");
    expect(host).toContain("flushLogs()");
  });
  it("exposes one TypeScript tool instead of parallel View debugging tools", () => {
    const registry = readFileSync("src-tauri/src/tool/builtins/mod.rs", "utf8");
    const prompt = JSON.parse(readFileSync("tools/execute_typescript.json", "utf8"));
    expect(registry).toContain("view::execute_typescript()");
    expect(registry).not.toMatch(/view::view_(snapshot|capture|action|wait|debug_eval|console_read)\(/);
    expect(prompt.parameters.required).toEqual(["code"]);
  });
});
