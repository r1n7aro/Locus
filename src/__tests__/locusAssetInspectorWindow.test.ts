import { beforeEach, describe, expect, it, vi } from "vitest";

const eventMocks = vi.hoisted(() => ({
  emitToMock: vi.fn(),
}));

const tauriRuntimeMocks = vi.hoisted(() => ({
  hasTauriWindowRuntimeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  emitTo: eventMocks.emitToMock,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));

vi.mock("../services/workbenchWindow", () => ({
  isWorkbenchWindowLabel: (label: string) => label === "main" || label.startsWith("workbench-"),
}));

vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: tauriRuntimeMocks.hasTauriWindowRuntimeMock,
}));

import {
  LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX,
  WORKBENCH_INSPECTOR_OPEN_EVENT,
  buildLocusAssetInspectorTabId,
  isLocusAssetInspectorTabId,
  locusAssetInspectorTabTitle,
  locusAssetInspectorTargetPath,
  openLocusAssetInspectorWindow,
  parseLocusAssetInspectorTabId,
} from "../services/locusAssetInspectorWindow";

describe("locusAssetInspectorWindow", () => {
  const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 3 };
  const assetPath = "Assets/Prefabs/Characters/NPCs_BasePrefabs/Gluecose.prefab";
  const scenePath = "Assets/Scenes/WIP/TestingGround.unity";
  const objectPath = "BardHare/DialogueShot/cm[1]";

  beforeEach(() => {
    eventMocks.emitToMock.mockReset();
    eventMocks.emitToMock.mockResolvedValue(undefined);
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

  it("opens asset inspectors through the Workbench tab group", async () => {
    const opened = await openLocusAssetInspectorWindow(workspaceRef, { assetPath });

    expect(opened).toBe(true);
    expect(eventMocks.emitToMock).toHaveBeenCalledTimes(1);
    expect(eventMocks.emitToMock).toHaveBeenCalledWith("main", WORKBENCH_INSPECTOR_OPEN_EVENT, {
      targetLabel: "main",
      workspaceRef,
      inspector: { assetPath },
    });
  });

  it("opens scene object inspectors through the Workbench tab group", async () => {
    const opened = await openLocusAssetInspectorWindow(workspaceRef, {
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    expect(opened).toBe(true);
    expect(eventMocks.emitToMock).toHaveBeenCalledWith("main", WORKBENCH_INSPECTOR_OPEN_EVENT, {
      targetLabel: "main",
      workspaceRef,
      inspector: { kind: "sceneObject", scenePath, objectPath },
    });
  });

  it("rejects invalid payloads without touching the backend", async () => {
    const opened = await openLocusAssetInspectorWindow(workspaceRef, { assetPath: "   " });

    expect(opened).toBe(false);
    expect(eventMocks.emitToMock).not.toHaveBeenCalled();
  });

  it("does nothing without a Tauri window runtime", async () => {
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReturnValue(false);

    const opened = await openLocusAssetInspectorWindow(workspaceRef, { assetPath });

    expect(opened).toBe(false);
    expect(eventMocks.emitToMock).not.toHaveBeenCalled();
  });
});
