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
  UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  buildUnityReferenceImportWindowUrl,
  getUnityReferenceImportWindowPayload,
  openUnityReferenceImportProgressWindow,
} from "../services/unityReferenceImportWindow";

describe("unityReferenceImportWindow", () => {
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

  it("serializes the selected locale into the window url", () => {
    const url = buildUnityReferenceImportWindowUrl({
      targetPath: "reference-folder",
      projectVersion: "2022.3.47f1",
      docsVersion: "2022.3",
      running: true,
      locale: "zh-CN",
    });

    expect(url).toContain("/window.html?unityReferenceImport=1");
    expect(url).toContain("locale=zh-CN");
  });

  it("reads the locale from the window query string", () => {
    const payload = getUnityReferenceImportWindowPayload(
      "?unityReferenceImport=1&targetPath=reference-folder&running=1&locale=zh-CN",
    );

    expect(payload.targetPath).toBe("reference-folder");
    expect(payload.running).toBe(true);
    expect(payload.locale).toBe("zh-CN");
  });

  it("focuses an existing window without resetting state when no payload is provided", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-4",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openUnityReferenceImportProgressWindow();

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({ focusExisting: true }),
    });
    expect(existingWindow.emit).not.toHaveBeenCalled();
  });

  it("pushes payload updates into an existing window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "unity-reference-import-progress",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openUnityReferenceImportProgressWindow({
      targetPath: "reference-folder",
      running: true,
      locale: "zh-CN",
    });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      expect.objectContaining({
        targetPath: "reference-folder",
        running: true,
        locale: "zh-CN",
      }),
    );
  });

  it("creates a fixed-size progress window through the sub-window command", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "unity-reference-import-progress",
      existing: false,
      pooled: true,
    });

    await openUnityReferenceImportProgressWindow({ targetPath: "reference-folder" });

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "unity-reference-import-progress",
        title: "Locus Unity Docs",
        width: 720,
        height: 560,
        resizable: false,
        maximizable: false,
        minimizable: false,
        closable: false,
        query: expect.stringContaining("unityReferenceImport=1"),
      }),
    });
  });
});
