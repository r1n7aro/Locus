import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Locus asset Inspector integration", () => {
  it("routes the single Locus Inspector action into Workbench", () => {
    const chat = read("src/components/ChatView.vue");
    const service = read("src/services/locusAssetInspector.ts");

    expect(chat).toContain("openLocusAssetInspectorWorkbenchTab");
    expect(chat).toContain("assetRefContextCanOpenLocusInspector");
    expect(chat).toContain("doAssetRefOpenInLocusInspector");
    expect(chat).toContain('action === "locusInspector"');
    expect(chat).toContain('t("common.openInLocusInspector")');
    expect(chat).not.toContain("doAssetRefOpenInLocusInspectorWindow");
    expect(chat).not.toContain("common.openInLocusInspectorWindow");
    expect(chat).not.toContain("openLocusAssetInspectorPanel");

    expect(service).toContain("WORKBENCH_INSPECTOR_OPEN_EVENT");
    expect(service).toContain("emitTo<WorkbenchInspectorOpenPayload>");
    expect(service).toContain("isWorkbenchWindowLabel(currentLabel)");
    expect(service).toContain('targetLabel = isWorkbenchWindowLabel(currentLabel) ? currentLabel : "main"');
  });

  it("hosts Inspector targets inside standard Workbench editor groups", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const editor = read("src/components/workbench/WorkbenchAssetEditor.vue");
    const preview = read("src/components/asset/WorkspaceAssetPreview.vue");

    expect(workbench).toContain("WORKBENCH_INSPECTOR_OPEN_EVENT");
    expect(workbench).toContain("openInspectorInWorkbench(event.payload)");
    expect(workbench).toContain('kind: "sceneObject"');
    expect(workbench).toContain('kind: "asset"');
    expect(workbench).toContain("workbenchResourceKey(resource)");
    expect(workbench).toContain("<WorkbenchAssetEditor");
    expect(editor).toContain("WorkspaceAssetPreview");
    expect(editor).toContain(':show-header="false"');
    expect(preview).toContain("UnityObjectPreview");
    expect(preview).toContain('level="inspector"');
    expect(preview).toContain(':collapsible="false"');
  });

  it("removes the floating panel and legacy View-host Inspector surfaces", () => {
    const app = read("src/App.vue");
    const viewHost = read("src/components/ViewHostWindow.vue");
    const viewService = read("src/services/view.ts");
    const viewBackend = read("src-tauri/src/view.rs");

    expect(app).not.toContain("LocusAssetInspectorPanel");
    expect(app).not.toContain("useLocusAssetInspectorPanel");
    expect(viewHost).not.toContain("LocusAssetInspectorPane");
    expect(viewHost).not.toContain("isLocusAssetInspectorTabId");
    expect(viewService).not.toContain("viewOpenInspectorTab");
    expect(viewBackend).not.toContain("LOCUS_INSPECTOR_TAB_ID_PREFIX");
    expect(viewBackend).not.toContain("open_inspector_tab_window");

    for (const removed of [
      "src/components/LocusAssetInspectorPanel.vue",
      "src/components/LocusAssetInspectorPane.vue",
      "src/composables/useLocusAssetInspectorPanel.ts",
      "src/services/locusAssetInspectorWindow.ts",
    ]) {
      expect(existsSync(resolve(cwd, removed))).toBe(false);
    }
  });

  it("exposes one Workbench Inspector preference and migrates legacy values", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain('| "locusInspector";');
    expect(displaySettings).toContain('assetRefClickAction: "locusInspector",');
    expect(displaySettings).toContain("LEGACY_LOCUS_INSPECTOR_ACTIONS");
    expect(displaySettings).toContain('return "locusInspector";');
    expect(displayPanel).toContain('value: "locusInspector"');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickInspectorDesc")');
    expect(displayPanel).not.toContain('value: "locusInspectorAuto"');
    expect(displayPanel).not.toContain('value: "locusInspectorEmbedded"');
    expect(displayPanel).not.toContain('value: "locusInspectorWindow"');
    expect(zh).toContain('"settings.display.assetRefClickInspector": "Locus Inspector"');
    expect(zh).toContain("在当前工作台中打开资产或 GameObject 标签页");
    expect(en).toContain('"settings.display.assetRefClickInspector": "Locus Inspector"');
    expect(en).toContain("Opens the asset or GameObject as a tab in the current Workbench");
  });
});
