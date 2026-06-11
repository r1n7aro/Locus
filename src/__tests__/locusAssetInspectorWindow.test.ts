import { beforeEach, describe, expect, it, vi } from "vitest";

const viewServiceMocks = vi.hoisted(() => ({
  viewOpenInspectorTabMock: vi.fn(),
}));

const tauriRuntimeMocks = vi.hoisted(() => ({
  hasTauriWindowRuntimeMock: vi.fn(),
}));

vi.mock("../services/view", () => ({
  viewOpenInspectorTab: viewServiceMocks.viewOpenInspectorTabMock,
}));

vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: tauriRuntimeMocks.hasTauriWindowRuntimeMock,
}));

import {
  LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX,
  buildLocusAssetInspectorTabId,
  isLocusAssetInspectorTabId,
  locusAssetInspectorTabTitle,
  locusAssetInspectorTargetPath,
  openLocusAssetInspectorWindow,
  parseLocusAssetInspectorTabId,
} from "../services/locusAssetInspectorWindow";

describe("locusAssetInspectorWindow", () => {
  const assetPath = "Assets/Prefabs/Characters/NPCs_BasePrefabs/Gluecose.prefab";
  const scenePath = "Assets/Scenes/WIP/TestingGround.unity";
  const objectPath = "BardHare/DialogueShot/cm[1]";

  beforeEach(() => {
    viewServiceMocks.viewOpenInspectorTabMock.mockReset();
    viewServiceMocks.viewOpenInspectorTabMock.mockResolvedValue({
      id: "",
      windowLabel: "view-inspector-abc123",
      hostUrl: "/view-host?id=...",
      packageRoot: "",
    });
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReset();
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReturnValue(true);
  });

  it("builds and parses asset tab ids", () => {
    const tabId = buildLocusAssetInspectorTabId({ assetPath });

    expect(tabId.startsWith(LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX)).toBe(true);
    expect(isLocusAssetInspectorTabId(tabId)).toBe(true);
    expect(parseLocusAssetInspectorTabId(tabId)).toEqual({ assetPath });
  });

  it("builds and parses scene object tab ids", () => {
    const tabId = buildLocusAssetInspectorTabId({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    expect(parseLocusAssetInspectorTabId(tabId)).toEqual({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });
  });

  it("normalizes full scene object paths passed through assetPath", () => {
    const tabId = buildLocusAssetInspectorTabId({
      assetPath: `${scenePath}/${objectPath}`,
    });

    expect(parseLocusAssetInspectorTabId(tabId)).toEqual({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });
  });

  it("produces identical tab ids for identical targets (dedupe key)", () => {
    expect(buildLocusAssetInspectorTabId({ assetPath }))
      .toBe(buildLocusAssetInspectorTabId({ assetPath: `${assetPath}/` }));
  });

  it("keeps tab ids ASCII-safe for window registries and host URLs", () => {
    const tabId = buildLocusAssetInspectorTabId({ assetPath: "Assets/特效 粒子/烟雾.prefab" });

    expect([...tabId].every((ch) => ch.charCodeAt(0) > 0x20 && ch.charCodeAt(0) < 0x7f)).toBe(true);
    expect(parseLocusAssetInspectorTabId(tabId)).toEqual({ assetPath: "Assets/特效 粒子/烟雾.prefab" });
  });

  it("derives tab titles and target paths from the payload", () => {
    expect(locusAssetInspectorTabTitle({ assetPath })).toBe("Gluecose.prefab");
    expect(locusAssetInspectorTabTitle({ kind: "sceneObject", scenePath, objectPath })).toBe("cm[1]");
    expect(locusAssetInspectorTargetPath({ kind: "sceneObject", scenePath, objectPath }))
      .toBe(`${scenePath}/${objectPath}`);
    expect(parseLocusAssetInspectorTabId("not-an-inspector-tab")).toBeNull();
  });

  it("opens inspector tabs through the View host tab system", async () => {
    const opened = await openLocusAssetInspectorWindow({ assetPath });

    expect(opened).toBe(true);
    expect(viewServiceMocks.viewOpenInspectorTabMock).toHaveBeenCalledTimes(1);
    expect(viewServiceMocks.viewOpenInspectorTabMock).toHaveBeenCalledWith({
      tabId: buildLocusAssetInspectorTabId({ assetPath }),
    });
  });

  it("opens scene object targets through the View host tab system", async () => {
    const opened = await openLocusAssetInspectorWindow({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    expect(opened).toBe(true);
    expect(viewServiceMocks.viewOpenInspectorTabMock).toHaveBeenCalledWith({
      tabId: buildLocusAssetInspectorTabId({ kind: "sceneObject", scenePath, objectPath }),
    });
  });

  it("rejects invalid payloads without touching the backend", async () => {
    const opened = await openLocusAssetInspectorWindow({ assetPath: "   " });

    expect(opened).toBe(false);
    expect(viewServiceMocks.viewOpenInspectorTabMock).not.toHaveBeenCalled();
  });

  it("does nothing without a Tauri window runtime", async () => {
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReturnValue(false);

    const opened = await openLocusAssetInspectorWindow({ assetPath });

    expect(opened).toBe(false);
    expect(viewServiceMocks.viewOpenInspectorTabMock).not.toHaveBeenCalled();
  });
});
