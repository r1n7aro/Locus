import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function occurrences(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

describe("extra workdirs (additional working directories)", () => {
  it("persists per-workspace config under Library/Locus and exposes commands", () => {
    const rustCore = read("src-tauri/src/extra_workdirs.rs");
    const rustCommands = read("src-tauri/src/commands/extra_workdirs.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const service = read("src/services/extraWorkdirs.ts");

    // The attachment list lives in the Unity Library folder, which never
    // syncs with the project — attachments hold machine-local absolute paths.
    expect(rustCore).toContain('const EXTRA_WORKDIRS_FILE: &str = "extra_workdirs.json"');
    expect(rustCore).toContain("library_dir_for_working_dir");

    expect(rustCommands).toContain("pub async fn extra_workdirs_get");
    expect(rustCommands).toContain("pub async fn extra_workdirs_set");
    expect(rustCommands).toContain("pub async fn extra_workdirs_map");
    expect(rustApp).toContain("commands::extra_workdirs_get");
    expect(rustApp).toContain("commands::extra_workdirs_set");
    expect(rustApp).toContain("commands::extra_workdirs_map");

    expect(service).toContain('ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_get"');
    expect(service).toContain('ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_set"');
    expect(service).toContain('ipcInvoke<Record<string, ExtraWorkdirStatus[]>>("extra_workdirs_map"');
  });

  it("injects attached directories into the agent env prompt with an independent injected item", () => {
    const instance = read("src-tauri/src/agent/instance/mod.rs");

    expect(occurrences(instance, "crate::extra_workdirs::build_env_prompt_block(&self.working_dir)"))
      .toBeGreaterThanOrEqual(2); // env prompt append + injected-items listing
    expect(instance).toContain('id: "extra_workdirs".to_string()');
    expect(instance).toContain('title: "Additional Working Directories".to_string()');
    // Sorts directly behind the env.md entry in the injected panel.
    expect(instance).toContain('"extra_workdirs" => (0, usize::MAX)');
  });

  it("offers configuration from the workspace dropdown context menu and shows attachments inline", () => {
    const app = read("src/App.vue");

    expect(app).toContain("configureContextRecentDirExtraWorkdirs");
    expect(app).toContain('t("app.dir.configureExtraWorkdirs")');
    expect(app).toContain('class="dir-item-extras"');
    expect(app).toContain("extraWorkdirsFor(dir)");
    expect(app).toContain('t("extraWorkdirs.missingBadge")');
    expect(app).toContain('<ExtraWorkdirsConfigWindow v-else-if="isExtraWorkdirsWindow" />');
    expect(app).toContain("listenExtraWorkdirsUpdated");
  });

  it("validates attachment existence when a workspace opens", () => {
    const store = read("src/stores/project.ts");

    expect(store).toContain("async function checkCurrentExtraWorkdirs()");
    expect(store).toContain('t("app.dir.extraWorkdirsMissing"');
    // Both open paths run the check: restoring the last workspace at startup
    // (loadWorkingDir) and switching workspaces (setWorkingDir).
    expect(occurrences(store, "void checkCurrentExtraWorkdirs();")).toBeGreaterThanOrEqual(2);
  });

  it("honors attached directories in the file-tool workspace boundary", () => {
    const instance = read("src-tauri/src/agent/instance/mod.rs");

    // The env prompt authorizes attached dirs as project scope; the boundary
    // check must exempt them alongside skill-package and app-temp roots.
    expect(instance).toContain("crate::extra_workdirs::load_entries(working_dir)");
    expect(instance).toContain("an attached additional working directory");
  });

  it("guards the config window against stale loads and pre-listener payloads", () => {
    const window = read("src/components/ExtraWorkdirsConfigWindow.vue");
    const rustSubWindow = read("src-tauri/src/commands/sub_window.rs");
    const rustApp = read("src-tauri/src/lib.rs");

    // Generation guard: a late extra_workdirs_get response from a previous
    // workspace must not overwrite the currently shown workspace's form.
    expect(window).toContain("loadEntriesRequestId");
    expect(window).toContain("requestId !== loadEntriesRequestId");
    // Missed-payload recovery: payloads emitted to an existing window before
    // its listener registered are pulled back via the recorded open query.
    expect(window).toContain("getSubWindowClaimedQuery(EXTRA_WORKDIRS_WINDOW_LABEL)");
    expect(window).toContain("payloadEventSeen");
    expect(rustSubWindow).toContain("claimed_queries");
    expect(rustSubWindow).toContain("pub async fn sub_window_claimed_query");
    expect(rustApp).toContain("commands::sub_window_claimed_query");
  });

  it("registers the standalone config window with capabilities and i18n", () => {
    const capabilities = read("src-tauri/capabilities/default.json");
    const windowService = read("src/services/extraWorkdirsWindow.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(capabilities).toContain('"extra-workdirs-config"');
    expect(windowService).toContain('EXTRA_WORKDIRS_WINDOW_LABEL = "extra-workdirs-config"');
    expect(windowService).toContain('EXTRA_WORKDIRS_UPDATED_EVENT = "extra-workdirs:updated"');

    for (const lang of [zh, en]) {
      expect(lang).toContain('"app.dir.configureExtraWorkdirs"');
      expect(lang).toContain('"app.dir.extraWorkdirsMissing"');
      expect(lang).toContain('"extraWorkdirs.windowTitle"');
      expect(lang).toContain('"extraWorkdirs.commentPlaceholder"');
      expect(lang).toContain('"extraWorkdirs.missingBadge"');
    }
  });
});
