import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Locus asset inspector integration", () => {
  it("routes asset context menu actions to the Locus Inspector", () => {
    const chat = read("src/components/ChatView.vue");
    const app = read("src/App.vue");
    const pane = read("src/components/LocusAssetInspectorPane.vue");
    const service = read("src/services/locusAssetInspectorWindow.ts");
    const tauriCapability = read("src-tauri/capabilities/default.json");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(chat).toContain("openLocusAssetInspector");
    expect(chat).toContain("assetRefContextCanOpenLocusInspector");
    expect(chat).toContain("doAssetRefOpenInLocusInspector");
    expect(chat).toContain("doAssetRefOpenInLocusInspectorWindow");
    expect(chat).toContain('target.kind === "sceneObject"');
    expect(chat).toContain('kind: "sceneObject"');
    expect(chat).toContain("openUnitySceneObjectInspector(target.scenePath, target.objectPath)");
    expect(chat).toContain(".unity-object-identity[data-unity-ref-kind]");
    expect(chat).toContain('t("common.openInLocusInspector")');
    expect(chat).toContain('t("common.openInLocusInspectorWindow")');
    const selectInUnityIndex = chat.indexOf('t("common.selectInUnity")');
    const openInspectorIndex = chat.indexOf('t("common.openInLocusInspector")');
    expect(selectInUnityIndex).toBeGreaterThan(0);
    expect(openInspectorIndex).toBeGreaterThan(selectInUnityIndex);
    expect(chat.slice(selectInUnityIndex, openInspectorIndex)).toContain('class="asset-ref-ctx-sep"');

    // The standalone inspector window is gone; inspector targets open as tabs
    // inside the View host window system.
    expect(app).not.toContain("LocusAssetInspectorWindow");
    expect(app).not.toContain("isLocusAssetInspectorWindowLocation");
    expect(service).toContain('LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX = "locus-inspector:"');
    expect(service).toContain("buildLocusAssetInspectorTabId");
    expect(service).toContain("parseLocusAssetInspectorTabId");
    expect(service).toContain("viewOpenInspectorTab({ tabId: buildLocusAssetInspectorTabId(nextPayload) })");
    expect(service).toContain("scenePath");
    expect(service).toContain("objectPath");

    expect(pane).toContain("UnityObjectPreview");
    expect(pane).toContain('kind === "sceneObject"');
    expect(pane).toContain("objectPath");
    expect(pane).toContain('level="inspector"');
    expect(pane).toContain(":auto-load-preview=\"true\"");
    // The preview header already names the target; no extra path/source row.
    expect(pane).not.toContain("locus-asset-inspector-pane-header");

    expect(tauriCapability).not.toContain('"locus-asset-inspector"');
    expect(tauriCapability).toContain('"view-*"');
    expect(tauriCapability).toContain('"core:window:allow-close"');
    expect(tauriCapability).toContain('"core:window:allow-destroy"');
    expect(zh).toContain('"common.openInLocusInspector": "在 Locus Inspector 中打开"');
    expect(zh).toContain('"common.openInLocusInspectorWindow": "在独立 Inspector 窗口中打开"');
    expect(zh).toContain('"asset.inspector.source.disk": "磁盘"');
    expect(zh).toContain('"asset.inspector.source.live": "Live"');
    expect(en).toContain('"common.openInLocusInspector": "Open in Locus Inspector"');
    expect(en).toContain('"common.openInLocusInspectorWindow": "Open in standalone Inspector window"');
    expect(en).toContain('"asset.inspector.source.disk": "Disk"');
    expect(en).toContain('"asset.inspector.source.live": "Live"');
  });

  it("hosts inspector tabs inside the View host window system", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const viewService = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const app = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    // Host window renders inspector tabs inline next to View tabs.
    expect(host).toContain("import LocusAssetInspectorPane from \"./LocusAssetInspectorPane.vue\"");
    expect(host).toContain("isLocusAssetInspectorTabId");
    expect(host).toContain("inspectorPaneRecords");
    expect(host).toContain("<LocusAssetInspectorPane :payload=\"record.payload\" />");
    expect(host).toContain("activeTabIsInspector");
    expect(host).toContain("inspectorTabFromId");

    // Every tab can be closed in place; the last tab closes the window.
    expect(host).toContain("async function closeTab(tabId: string)");
    expect(host).toContain("@click.stop=\"closeTab(tab.id)\"");
    expect(host).toContain("onTabAuxClick");
    expect(host).toContain('t(\'view.host.closeTab\')');
    expect(zh).toContain('"view.host.closeTab"');
    expect(en).toContain('"view.host.closeTab"');

    // Inspector opens route through the shared host/tab registry.
    expect(viewService).toContain('ipcInvoke<ViewRunResult>("view_open_inspector_tab"');
    expect(commands).toContain("pub async fn view_open_inspector_tab");
    expect(app).toContain("commands::view_open_inspector_tab");
    expect(runtime).toContain('LOCUS_INSPECTOR_TAB_ID_PREFIX: &str = "locus-inspector:"');
    expect(runtime).toContain("fn normalize_view_tab_id");
    expect(runtime).toContain("pub async fn open_inspector_tab_window");
    expect(runtime).toContain("fn detach_inspector_tab_window");
    expect(runtime).toContain("fn encode_view_host_tab_id");
    expect(runtime).toContain("reusable_view_host_window_label(app_handle, &id)");
  });

  it("hosts an embedded floating inspector panel in the main window", () => {
    const app = read("src/App.vue");
    const composable = read("src/composables/useLocusAssetInspectorPanel.ts");
    const panel = read("src/components/LocusAssetInspectorPanel.vue");

    expect(app).toContain("const LocusAssetInspectorPanel = defineAsyncComponent");
    expect(app).toContain("setLocusAssetInspectorPanelHostAvailable(!isStandaloneWindow)");
    expect(app).toContain("<LocusAssetInspectorPanel v-if=\"!isStandaloneWindow && locusAssetInspectorPanel.state.open\" />");

    // Embedded mode must fall back to the standalone window when no panel host
    // exists; auto mode only embeds when the window fits the panel.
    expect(composable).toContain("export async function openLocusAssetInspector(");
    expect(composable).toContain("canFitEmbeddedLocusAssetInspectorPanel");
    expect(composable).toContain('|| (mode === "auto" && canFitEmbeddedLocusAssetInspectorPanel())');
    expect(composable).toContain("if (preferEmbedded && openLocusAssetInspectorPanel(payload))");
    expect(composable).toContain("return openLocusAssetInspectorWindow(payload);");
    expect(composable).toContain("normalizeLocusAssetInspectorPayload");
    expect(composable).toContain("isValidLocusAssetInspectorPayload");

    // Draggable / resizable floating panel with the shared inspector preview.
    expect(panel).toContain("UnityObjectPreview");
    expect(panel).toContain('level="inspector"');
    expect(panel).toContain(":auto-load-preview=\"true\"");
    expect(panel).toContain("@source-change=\"handlePreviewSourceChange\"");
    expect(panel).toContain("handleTitlebarPointerDown");
    expect(panel).toContain("handleResizePointerDown");
    // Carve the panel out of the native window drag region so it can float
    // over the tab bar without dragging the whole main window.
    expect(panel).toContain("-webkit-app-region: no-drag;");
    // Resizable from every edge and corner, anchoring the opposite side.
    expect(panel).toContain("handleResizePointerDown($event, 'se')");
    expect(panel).toContain('["n", "s", "e", "w", "nw", "ne", "sw"]');
    expect(panel).toContain("function resizeRect(");
    expect(panel).toContain("setPointerCapture");
    expect(panel).toContain("clampRect");
    expect(panel).toContain("locus:assetInspectorPanelRect");
    expect(panel).toContain("openLocusAssetInspectorWindow");
    expect(panel).toContain("closeLocusAssetInspectorPanel");
  });

  it("routes session asset reference clicks through the configurable click action", () => {
    const chat = read("src/components/ChatView.vue");
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("export type AssetRefClickAction =");
    expect(displaySettings).toContain("assetRefClickAction: AssetRefClickAction;");
    // Adaptive is the default: embed when the window fits, otherwise window.
    expect(displaySettings).toContain('| "locusInspectorAuto"');
    expect(displaySettings).toContain('assetRefClickAction: "locusInspectorAuto",');
    // The Unity embed window has its own click action, defaulting to the
    // editor's native Inspector.
    expect(displaySettings).toContain('| "unityInspector"');
    expect(displaySettings).toContain("unityEmbedAssetRefClickAction: AssetRefClickAction;");
    expect(displaySettings).toContain('unityEmbedAssetRefClickAction: "unityInspector",');

    expect(chat).toContain("runAssetRefClickAction");
    expect(chat).toContain("displaySettings.assetRefClickAction");
    expect(chat).toContain("displaySettings.unityEmbedAssetRefClickAction");
    expect(chat).toContain("isUnityEmbeddedWindow()");
    expect(chat).toContain('action === "unityInspector"');
    expect(chat).toContain("openAssetRefInUnityInspector");
    expect(chat).toContain("legacyAssetRefClick");
    expect(chat).toContain('action === "locusInspectorAuto"');
    expect(chat).toContain('action === "locusInspectorWindow"');
    expect(chat).toContain(': "auto"');
    expect(chat).toContain('if (action === "fileBrowser")');

    expect(displayPanel).toContain("assetRefClickActionOptions");
    expect(displayPanel).toContain("unityEmbedAssetRefClickActionOptions");
    // Both pickers are dropdowns with a per-option description (hint).
    expect(displayPanel).toContain('import BaseDropdown from "../ui/BaseDropdown.vue";');
    expect(displayPanel).toContain('value: "locusInspectorAuto"');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickInspectorAutoDesc")');
    expect(displayPanel).toContain('value: "unityInspector"');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickUnityInspectorDesc")');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickUnitySelectDesc")');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickFileBrowserDesc")');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickInspectorEmbeddedDesc")');
    expect(displayPanel).toContain('hint: t("settings.display.assetRefClickInspectorWindowDesc")');
    expect(displayPanel).toContain(":model-value=\"display.assetRefClickAction\"");
    expect(displayPanel).toContain(":model-value=\"display.unityEmbedAssetRefClickAction\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('assetRefClickAction', $event as AssetRefClickAction)\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('unityEmbedAssetRefClickAction', $event as AssetRefClickAction)\"");

    for (const lang of [zh, en]) {
      expect(lang).toContain('"settings.display.assetRefClickTitle"');
      expect(lang).toContain('"settings.display.assetRefClickUnityEmbedTarget"');
      expect(lang).toContain('"settings.display.assetRefClickUnityInspector"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorAuto"');
      expect(lang).toContain('"settings.display.assetRefClickUnitySelect"');
      expect(lang).toContain('"settings.display.assetRefClickFileBrowser"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorEmbedded"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorWindow"');
      expect(lang).toContain('"settings.display.assetRefClickUnityInspectorDesc"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorAutoDesc"');
      expect(lang).toContain('"settings.display.assetRefClickUnitySelectDesc"');
      expect(lang).toContain('"settings.display.assetRefClickFileBrowserDesc"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorEmbeddedDesc"');
      expect(lang).toContain('"settings.display.assetRefClickInspectorWindowDesc"');
    }
  });
});
