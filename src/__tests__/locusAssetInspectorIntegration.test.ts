import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Locus asset inspector integration", () => {
  it("routes asset context menu actions to the standalone Locus Inspector window", () => {
    const chat = read("src/components/ChatView.vue");
    const app = read("src/App.vue");
    const windowComponent = read("src/components/LocusAssetInspectorWindow.vue");
    const tauriCapability = read("src-tauri/capabilities/default.json");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(chat).toContain("openLocusAssetInspectorWindow");
    expect(chat).toContain("assetRefContextCanOpenLocusInspector");
    expect(chat).toContain("doAssetRefOpenInLocusInspector");
    expect(chat).toContain('target.kind === "sceneObject"');
    expect(chat).toContain('kind: "sceneObject"');
    expect(chat).toContain("openUnitySceneObjectInspector(target.scenePath, target.objectPath)");
    expect(chat).toContain(".unity-object-identity[data-unity-ref-kind]");
    expect(chat).toContain('t("common.openInLocusInspector")');
    const selectInUnityIndex = chat.indexOf('t("common.selectInUnity")');
    const openInspectorIndex = chat.indexOf('t("common.openInLocusInspector")');
    expect(selectInUnityIndex).toBeGreaterThan(0);
    expect(openInspectorIndex).toBeGreaterThan(selectInUnityIndex);
    expect(chat.slice(selectInUnityIndex, openInspectorIndex)).toContain('class="asset-ref-ctx-sep"');
    expect(app).toContain("isLocusAssetInspectorWindowLocation");
    expect(app).toContain("const LocusAssetInspectorWindow = defineAsyncComponent");
    expect(app).toContain("<LocusAssetInspectorWindow v-else-if=\"isLocusAssetInspectorWindow\" />");
    expect(windowComponent).toContain("UnityObjectPreview");
    expect(windowComponent).toContain('kind: "sceneObject"');
    expect(windowComponent).toContain("scenePath");
    expect(windowComponent).toContain("objectPath");
    expect(windowComponent).toContain('level="inspector"');
    expect(windowComponent).toContain(":auto-load-preview=\"true\"");
    expect(windowComponent).toContain("@source-change=\"handlePreviewSourceChange\"");
    expect(windowComponent).toContain("inspectorSourceState");
    expect(windowComponent).toContain("UnityObjectPreviewSourceState");
    expect(windowComponent).toContain("locus-asset-inspector-source");
    expect(windowComponent).toContain("data-window-no-drag");
    expect(windowComponent).toContain("@pointerdown.stop");
    expect(windowComponent).toContain("currentWindow.destroy()");
    expect(windowComponent).toContain(".locus-asset-inspector-close *");
    expect(tauriCapability).toContain('"locus-asset-inspector"');
    expect(tauriCapability).toContain('"core:window:allow-close"');
    expect(tauriCapability).toContain('"core:window:allow-destroy"');
    expect(zh).toContain('"common.openInLocusInspector": "在 Locus Inspector 中打开"');
    expect(zh).toContain('"asset.inspector.source.disk": "磁盘"');
    expect(zh).toContain('"asset.inspector.source.live": "Live"');
    expect(en).toContain('"common.openInLocusInspector": "Open in Locus Inspector"');
    expect(en).toContain('"asset.inspector.source.disk": "Disk"');
    expect(en).toContain('"asset.inspector.source.live": "Live"');
  });
});
