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
  REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
  buildReferenceExternalImportWindowUrl,
  openReferenceExternalImportWindow,
} from "../services/referenceExternalImportWindow";

describe("referenceExternalImportWindow", () => {
  beforeEach(() => {
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReturnValue({ label: "main" });
    webviewWindowMocks.createdWindows.length = 0;
  });

  it("builds the dedicated window url", () => {
    expect(buildReferenceExternalImportWindowUrl()).toBe(
      "/reference-external-import?referenceExternalImport=1",
    );
    expect(buildReferenceExternalImportWindowUrl({
      parentDir: "reference/gameplay",
      initialSource: "unity",
    })).toBe(
      "/reference-external-import?referenceExternalImport=1&parentDir=reference%2Fgameplay&initialSource=unity",
    );
  });

  it("focuses an existing window and updates its payload", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openReferenceExternalImportWindow({
      parentDir: "reference/gameplay",
      initialSource: "feishu",
    });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
      {
        parentDir: "reference/gameplay",
        initialSource: "feishu",
      },
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("creates a frameless child window bound to the current parent window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    await openReferenceExternalImportWindow({
      fixedTargetPath: "unity-official-docs",
      initialSource: "unity",
    });

    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [, options] = webviewWindowMocks.createdWindows[0] as [string, Record<string, unknown>];
    expect(options.parent).toEqual({ label: "main" });
    expect(options.decorations).toBe(false);
    expect(options.center).toBe(true);
    expect(options.shadow).toBe(true);
    expect(options.resizable).toBe(true);
    expect(options.width).toBe(1180);
    expect(options.height).toBe(900);
  });
});
