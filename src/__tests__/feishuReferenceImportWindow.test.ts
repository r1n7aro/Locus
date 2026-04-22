import { beforeEach, describe, expect, it, vi } from "vitest";

const webviewWindowMocks = vi.hoisted(() => ({
  getByLabelMock: vi.fn(),
  createdWindows: [] as Array<unknown[]>,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
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
  FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  buildFeishuReferenceImportWindowUrl,
  openFeishuReferenceImportProgressWindow,
} from "../services/feishuReferenceImportWindow";

describe("feishuReferenceImportWindow", () => {
  beforeEach(() => {
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.createdWindows.length = 0;
  });

  it("builds the window url", () => {
    expect(buildFeishuReferenceImportWindowUrl()).toBe(
      "/feishu-reference-import?feishuReferenceImport=1",
    );
    expect(
      buildFeishuReferenceImportWindowUrl({ targetPath: "reference-folder" }),
    ).toBe(
      "/feishu-reference-import?feishuReferenceImport=1&targetPath=reference-folder",
    );
  });

  it("focuses an existing window", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openFeishuReferenceImportProgressWindow();

    expect(existingWindow.emit).not.toHaveBeenCalled();
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("switches an existing window to a specific reference folder", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openFeishuReferenceImportProgressWindow({
      targetPath: "reference-folder",
    });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      { targetPath: "reference-folder" },
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
  });

  it("creates an independent top-level window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    await openFeishuReferenceImportProgressWindow();

    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [, options] = webviewWindowMocks.createdWindows[0] as [string, Record<string, unknown>];
    expect(options.parent).toBeUndefined();
    expect(options.center).toBe(true);
    expect(options.shadow).toBe(true);
    expect(options.resizable).toBe(true);
  });
});
