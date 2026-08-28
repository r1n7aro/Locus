import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin status notice", () => {
  it("routes plugin attention through the global error notice style and top warning strip", () => {
    const app = read("src/App.vue") + read("src/styles/app-global.css");
    const projectStore = read("src/stores/project.ts");

    expect(projectStore).toContain('const PLUGIN_STATUS_NOTICE_OPERATION = "unity-plugin-status";');
    expect(projectStore).toContain('const plan = await unityService.checkUnityPluginInstallPlan(requireWorkspaceRef());');
    expect(projectStore).toContain("plan.dllUpdateRequired && plan.unityRunning");
    expect(projectStore).toContain('title: t("app.plugin.closeUnityConfirmTitle")');
    expect(projectStore).toContain("forceCloseUnity = true");
    expect(projectStore).toContain("await unityService.installUnityPlugin(requireWorkspaceRef(), { forceCloseUnity })");
    expect(projectStore).toContain('notificationStore.addNotice("error", pluginStatusLabel(status)');
    expect(projectStore).toContain("replaceOperation: true");
    expect(projectStore).toContain("notificationStore.clearByOperation(PLUGIN_STATUS_NOTICE_OPERATION)");
    expect(app).toContain('class="tab-plugin-warn"');
    expect(app).toContain('class="tab-plugin-icon"');
    expect(app).toContain("var(--status-danger-bg)");
    expect(app).toContain("var(--status-danger-fg)");
    expect(app).toContain("border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color) 28%);");
  });

  it("exposes a Unity plugin install preflight for DLL close confirmation", () => {
    const unityService = read("src/services/unity.ts");
    const workspaceCommands = read("src-tauri/src/commands/workspace.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(unityService).toContain('"check_unity_plugin_install_plan"');
    expect(workspaceCommands).toContain("pub async fn check_unity_plugin_install_plan");
    expect(lib).toContain("commands::check_unity_plugin_install_plan");
    expect(zh).toContain('"app.plugin.closeUnityConfirmTitle": "关闭 Unity 后更新插件"');
    expect(en).toContain('"app.plugin.closeUnityConfirmTitle": "Update after closing Unity"');
  });

  it("uses the same DLL close confirmation from onboarding install", () => {
    const onboarding = read("src/components/OnboardingView.vue");

    expect(onboarding).toContain("checkUnityPluginInstallPlan");
    expect(onboarding).toContain("plan.dllUpdateRequired && plan.unityRunning");
    expect(onboarding).toContain('okLabel: t("app.plugin.closeUnityConfirmAction")');
    expect(onboarding).toContain("await installUnityPlugin(requireOnboardingWorkspaceRef(), { forceCloseUnity })");
  });

  it("keeps the top tabs single-line when the plugin notice is visible", () => {
    const app = read("src/App.vue") + read("src/styles/app-global.css");

    expect(app).toMatch(/\.tab-item\s*\{[\s\S]*flex:\s*0 0 auto;[\s\S]*white-space:\s*nowrap;/);
    expect(app).toMatch(/\.tab-plugin-warn\s*\{[\s\S]*flex:\s*0 0 auto;[\s\S]*white-space:\s*nowrap;/);
    expect(app).not.toContain(".workspace-selector");
    expect(app).not.toContain(".workspace-btn");
  });
});
