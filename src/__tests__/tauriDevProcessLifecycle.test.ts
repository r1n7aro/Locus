import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8").replace(/\r\n?/g, "\n");
}

describe("Tauri dev process lifecycle", () => {
  it("keeps Vite under the repository launcher lifecycle", () => {
    const config = read("src-tauri/tauri.conf.json");
    const launcher = read("scripts/run-tauri.mjs");

    expect(config).not.toContain('"beforeDevCommand"');
    expect(launcher).toContain("startManagedDevServer");
    expect(launcher).toContain("superviseTauriDev");
    expect(launcher).toContain("terminateManagedChild(devServer)");
    expect(launcher).toContain("terminateManagedChild(tauri)");
    expect(launcher).toContain("installShutdownHandlers");
  });

  it("uses a pipe watchdog to reap full Windows process trees", () => {
    const launcher = read("scripts/run-tauri.mjs");
    const watchdog = read("scripts/process-tree-watchdog.mjs");

    expect(launcher).toContain("startProcessTreeWatchdog(child)");
    expect(launcher).toContain('stdio: ["pipe", "ignore", "ignore"]');
    expect(watchdog).toContain('process.stdin.on("end"');
    expect(watchdog).toContain('process.stdin.on("close"');
    expect(watchdog).toContain('["/pid", String(pid), "/T", "/F"]');
  });
});
