import { beforeEach, describe, expect, it, vi } from "vitest";

const eventMocks = vi.hoisted(() => ({
  emitToMock: vi.fn(),
}));

const tauriRuntimeMocks = vi.hoisted(() => ({
  hasTauriWindowRuntimeMock: vi.fn(),
}));

const windowMocks = vi.hoisted(() => ({
  label: "main",
}));

vi.mock("@tauri-apps/api/event", () => ({
  emitTo: eventMocks.emitToMock,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: windowMocks.label }),
}));

vi.mock("../services/workbenchWindow", () => ({
  isWorkbenchWindowLabel: (label: string) => label === "main" || label.startsWith("workbench-"),
}));

vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: tauriRuntimeMocks.hasTauriWindowRuntimeMock,
}));

import {
  WORKBENCH_INSPECTOR_OPEN_EVENT,
  locusAssetInspectorTabTitle,
  normalizeLocusAssetInspectorTarget,
  openLocusAssetInspectorWorkbenchTab,
} from "../services/locusAssetInspector";
import { normalizeAssetRefClickAction } from "../composables/useDisplaySettings";

describe("Locus asset Inspector Workbench routing", () => {
  const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 3 };
  const assetPath = "Assets/Prefabs/Characters/NPCs_BasePrefabs/Gluecose.prefab";
  const scenePath = "Assets/Scenes/WIP/TestingGround.unity";
  const objectPath = "BardHare/DialogueShot/cm[1]";

  beforeEach(() => {
    eventMocks.emitToMock.mockReset();
    eventMocks.emitToMock.mockResolvedValue(undefined);
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReset();
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReturnValue(true);
    windowMocks.label = "main";
  });

  it("normalizes asset and scene-object targets", () => {
    expect(normalizeLocusAssetInspectorTarget({ assetPath: `${assetPath}/` })).toEqual({ assetPath });
    expect(normalizeLocusAssetInspectorTarget({ assetPath: `${scenePath}/${objectPath}` })).toEqual({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });
  });

  it("migrates every legacy Locus Inspector preference to Workbench", () => {
    for (const action of [
      "locusInspectorAuto",
      "locusInspectorEmbedded",
      "locusInspectorWindow",
    ]) {
      expect(normalizeAssetRefClickAction(action, "fileBrowser")).toBe("locusInspector");
    }
  });

  it("derives Workbench tab titles from targets", () => {
    expect(locusAssetInspectorTabTitle({ assetPath })).toBe("Gluecose.prefab");
    expect(locusAssetInspectorTabTitle({ kind: "sceneObject", scenePath, objectPath })).toBe("cm[1]");
  });

  it("opens asset targets in the current Workbench tab group", async () => {
    const opened = await openLocusAssetInspectorWorkbenchTab(workspaceRef, { assetPath });

    expect(opened).toBe(true);
    expect(eventMocks.emitToMock).toHaveBeenCalledWith("main", WORKBENCH_INSPECTOR_OPEN_EVENT, {
      targetLabel: "main",
      workspaceRef,
      inspector: { assetPath },
    });
  });

  it("keeps an auxiliary Workbench as the target host", async () => {
    windowMocks.label = "workbench-asset-a";

    await openLocusAssetInspectorWorkbenchTab(workspaceRef, {
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    expect(eventMocks.emitToMock).toHaveBeenCalledWith(
      "workbench-asset-a",
      WORKBENCH_INSPECTOR_OPEN_EVENT,
      {
        targetLabel: "workbench-asset-a",
        workspaceRef,
        inspector: { kind: "sceneObject", scenePath, objectPath },
      },
    );
  });

  it("routes non-Workbench windows back to the main Workbench", async () => {
    windowMocks.label = "workspace-page-knowledge";

    await openLocusAssetInspectorWorkbenchTab(workspaceRef, { assetPath });

    expect(eventMocks.emitToMock).toHaveBeenCalledWith(
      "main",
      WORKBENCH_INSPECTOR_OPEN_EVENT,
      expect.any(Object),
    );
  });

  it("rejects invalid targets and unavailable Tauri runtimes", async () => {
    expect(await openLocusAssetInspectorWorkbenchTab(workspaceRef, { assetPath: "   " })).toBe(false);
    tauriRuntimeMocks.hasTauriWindowRuntimeMock.mockReturnValue(false);
    expect(await openLocusAssetInspectorWorkbenchTab(workspaceRef, { assetPath })).toBe(false);
    expect(eventMocks.emitToMock).not.toHaveBeenCalled();
  });
});
