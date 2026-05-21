import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { persistedOutputDisplay } from "../components/toolPersistedOutput";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function sliceBetween(source: string, start: string, end: string) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  expect(startIndex).toBeGreaterThanOrEqual(0);
  expect(endIndex).toBeGreaterThan(startIndex);
  return source.slice(startIndex, endIndex);
}

describe("app temporary files settings", () => {
  it("adds temporary file controls to general settings", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const service = read("src/services/storage.ts");
    const types = read("src/types.ts");
    const lib = read("src-tauri/src/lib.rs");
    const storageCommands = read("src-tauri/src/commands/storage.rs");

    expect(source).toContain("getAppTempInfo");
    expect(source).toContain("clearAppTempDir");
    expect(source).toContain("openAppTempDirectory");
    expect(source).toContain('t("settings.general.tempFiles")');
    expect(source).toContain('t("settings.general.tempClear")');
    expect(service).toContain('ipcInvoke<AppTempInfo>("get_app_temp_info")');
    expect(service).toContain('ipcInvoke<AppTempInfo>("clear_app_temp_dir")');
    expect(service).toContain('ipcInvoke("open_app_temp_dir")');
    expect(types).toContain("export interface AppTempInfo");
    expect(lib).toContain("commands::get_app_temp_info");
    expect(lib).toContain("commands::clear_app_temp_dir");
    expect(lib).toContain("commands::open_app_temp_dir");
    expect(lib).toContain("commands::set_app_temp_dir_override(data_dir.join(\"temp\"))");
    expect(storageCommands).toContain("pub struct AppTempInfo");
    expect(storageCommands).toContain("resolve_runtime_storage_dir(app_handle)?.join(\"temp\")");
    expect(storageCommands).toContain("clear_dir_contents");
    expect(storageCommands).toContain("pub async fn open_app_temp_dir");
  });

  it("keeps storage and temporary file blocks on the same field layout", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const storageBlock = sliceBetween(
      source,
      't("settings.general.storage")',
      't("settings.general.tempFiles")',
    );
    const tempBlock = sliceBetween(
      source,
      't("settings.general.tempFiles")',
      't("settings.general.gitRuntime")',
    );

    for (const block of [storageBlock, tempBlock]) {
      expect(block).toContain('t("settings.general.storageCurrentPath")');
      expect(block).toContain('t("settings.general.storageSize")');
      expect(block).toContain('t("settings.general.storageOpen")');
      expect(block).toContain('class="storage-actions"');
    }
  });

  it("renders deleted persisted tool outputs without raw wrapper text", () => {
    const parsed = persistedOutputDisplay(
      "<persisted-output-deleted>\nFull output file deleted: C:\\temp\\tool.txt\n</persisted-output-deleted>",
    );

    expect(parsed.kind).toBe("deleted");
    expect(parsed.path).toBe("C:\\temp\\tool.txt");
  });
});
