import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool permission settings", () => {
  it("keeps the global tool permission mode in settings", () => {
    const settingsView = read("src/components/SettingsView.vue");
    const toolPermissions = read("src/components/settings/ToolPermissions.vue");
    const settingsState = read("src/composables/useSettingsState.ts");

    expect(settingsView).toContain(':tool-permission-mode="chatStore.toolPermissionMode"');
    expect(settingsView).toContain(':behavior-list="approvalBehaviorList"');
    expect(settingsView).toContain(':file-workspace-boundary-enabled="fileToolWorkspaceBoundary"');
    expect(settingsView).toContain('@set-global-permission-mode="chatStore.setToolPermissionMode"');
    expect(settingsView).toContain('@set-file-workspace-boundary="setFileToolWorkspaceBoundaryEnabled"');
    expect(toolPermissions).toContain("settings.perms.globalMode");
    expect(toolPermissions).toContain("settings.perms.globalModeDesc");
    expect(toolPermissions).toContain("settings.perms.fileBoundaryTitle");
    expect(toolPermissions).toContain("settings.perms.fileBoundaryScope");
    expect(toolPermissions).toContain("settings.perms.fileBoundaryAll");
    expect(toolPermissions).toContain("settings.perms.fileBoundaryWorkspace");
    expect(toolPermissions).toContain("settings.perms.behaviorTitle");
    expect(toolPermissions).toContain("BaseSegmented");
    expect(toolPermissions).toContain("fileBoundaryMode");
    expect(toolPermissions).toContain("setFileBoundaryMode");
    expect(toolPermissions).toContain("settings.perms.columnMode");
    expect(toolPermissions).toContain("settings.perms.toolTitle");
    expect(toolPermissions).toContain("emit('setGlobalPermissionMode', $event as ToolMode)");
    expect(toolPermissions).toContain('emit("setFileWorkspaceBoundary", mode === "workspace")');
    expect(toolPermissions.indexOf("settings.perms.behaviorTitle")).toBeLessThan(
      toolPermissions.indexOf("settings.perms.globalMode"),
    );
    expect(settingsState).toMatch(/name:\s*"behavior\.unity_editor_status_change",[\s\S]*defaultMode:\s*"auto"/);
    expect(settingsState).toMatch(/name:\s*"behavior\.knowledge_governance",[\s\S]*defaultMode:\s*"auto"/);
  });
});

