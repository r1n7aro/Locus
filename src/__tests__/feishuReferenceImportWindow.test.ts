import { beforeEach, describe, expect, it, vi } from "vitest";

const subWindowMocks = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  getByLabelMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: subWindowMocks.invokeMock,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: vi.fn(() => ({ label: "main" })),
  WebviewWindow: class {
    static getByLabel = subWindowMocks.getByLabelMock;
  },
}));

import {
  FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  buildFeishuReferenceImportWindowUrl,
  getFeishuReferenceImportWindowPayload,
  openFeishuReferenceImportProgressWindow,
} from "../services/feishuReferenceImportWindow";

describe("feishuReferenceImportWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
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

  it("builds the window url", () => {
    expect(buildFeishuReferenceImportWindowUrl()).toBe(
      "/window.html?feishuReferenceImport=1",
    );
    const url = buildFeishuReferenceImportWindowUrl({ targetPath: "feishu/docs" });
    expect(url).toContain("targetPath=feishu%2Fdocs");
    expect(getFeishuReferenceImportWindowPayload(url.slice(url.indexOf("?"))).targetPath)
      .toBe("feishu/docs");
  });

  it("reuses an existing window without payload", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-6",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openFeishuReferenceImportProgressWindow();

    expect(existingWindow.emit).not.toHaveBeenCalled();
  });

  it("switches an existing window to a specific reference folder", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "feishu-reference-import-progress",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openFeishuReferenceImportProgressWindow({ targetPath: "feishu/docs" });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      expect.objectContaining({ targetPath: "feishu/docs" }),
    );
  });

  it("creates the import window through the sub-window command", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "feishu-reference-import-progress",
      existing: false,
      pooled: true,
    });

    await openFeishuReferenceImportProgressWindow({ targetPath: "feishu/docs" });

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "feishu-reference-import-progress",
        title: "Locus Feishu Knowledge Base",
        width: 760,
        height: 760,
        resizable: true,
        closable: false,
        query: expect.stringContaining("feishuReferenceImport=1"),
      }),
    });
  });
});
