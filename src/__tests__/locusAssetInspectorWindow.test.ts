import { beforeEach, describe, expect, it, vi } from "vitest";

const webviewWindowMocks = vi.hoisted(() => ({
  getByLabelMock: vi.fn(),
  getCurrentWebviewWindowMock: vi.fn(),
  createdWindows: [] as Array<unknown[]>,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: webviewWindowMocks.getCurrentWebviewWindowMock,
  WebviewWindow: class {
    static getByLabel = webviewWindowMocks.getByLabelMock;

    constructor(...args: unknown[]) {
      webviewWindowMocks.createdWindows.push(args);
    }

    once(event: string, callback: (...args: unknown[]) => void) {
      if (event === "tauri://created") {
        callback();
      }
    }
  },
}));

import {
  LOCUS_ASSET_INSPECTOR_WINDOW_EVENT,
  buildLocusAssetInspectorWindowUrl,
  getLocusAssetInspectorWindowPayload,
  openLocusAssetInspectorWindow,
} from "../services/locusAssetInspectorWindow";

describe("locusAssetInspectorWindow", () => {
  const assetPath = "Assets/Prefabs/Characters/NPCs_BasePrefabs/Gluecose.prefab";
  const scenePath = "Assets/Scenes/WIP/TestingGround.unity";
  const objectPath = "BardHare/DialogueShot/cm[1]";

  beforeEach(() => {
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReturnValue({ label: "main" });
    webviewWindowMocks.createdWindows.length = 0;
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: {
        location: { pathname: "/", search: "" },
        __TAURI_INTERNALS__: {
          invoke: vi.fn(),
          metadata: { currentWindow: { label: "main" } },
        },
      },
    });
  });

  it("builds and parses asset URLs for the dedicated inspector window", () => {
    const url = buildLocusAssetInspectorWindowUrl({ assetPath });

    expect(url).toContain("/locus-asset-inspector?locusAssetInspector=1");
    expect(getLocusAssetInspectorWindowPayload(url.slice(url.indexOf("?"))).assetPath).toBe(assetPath);
  });

  it("builds and parses scene object URLs for the dedicated inspector window", () => {
    const url = buildLocusAssetInspectorWindowUrl({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    const payload = getLocusAssetInspectorWindowPayload(url.slice(url.indexOf("?")));
    expect(url).toContain("kind=sceneObject");
    expect(payload).toEqual({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });
  });

  it("normalizes full scene object paths passed through assetPath", () => {
    const url = buildLocusAssetInspectorWindowUrl({
      assetPath: `${scenePath}/${objectPath}`,
    });

    expect(getLocusAssetInspectorWindowPayload(url.slice(url.indexOf("?")))).toEqual({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });
  });

  it("focuses an existing inspector window and sends the next asset path", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openLocusAssetInspectorWindow({ assetPath });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      LOCUS_ASSET_INSPECTOR_WINDOW_EVENT,
      { assetPath },
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("focuses an existing inspector window and sends the next scene object target", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openLocusAssetInspectorWindow({
      kind: "sceneObject",
      scenePath,
      objectPath,
    });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      LOCUS_ASSET_INSPECTOR_WINDOW_EVENT,
      {
        kind: "sceneObject",
        scenePath,
        objectPath,
      },
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("creates a frameless child window bound to the current parent window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    const opened = await openLocusAssetInspectorWindow({ assetPath });

    expect(opened).toBe(true);
    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [label, options] = webviewWindowMocks.createdWindows[0] as [string, Record<string, unknown>];
    expect(label).toBe("locus-asset-inspector");
    expect(options.parent).toEqual({ label: "main" });
    expect(options.decorations).toBe(false);
    expect(options.center).toBe(true);
    expect(options.shadow).toBe(true);
    expect(options.resizable).toBe(true);
    expect(options.closable).toBe(true);
    expect(options.width).toBe(1040);
    expect(options.height).toBe(720);
  });
});
