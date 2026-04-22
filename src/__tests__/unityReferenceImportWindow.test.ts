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
  UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  buildUnityReferenceImportWindowUrl,
  getUnityReferenceImportWindowPayload,
  openUnityReferenceImportProgressWindow,
} from "../services/unityReferenceImportWindow";

describe("unityReferenceImportWindow", () => {
  beforeEach(() => {
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.createdWindows.length = 0;
  });

  it("serializes the selected locale into the window url", () => {
    expect(buildUnityReferenceImportWindowUrl({
      targetPath: "reference-folder",
      projectVersion: "2022.3.47f1",
      docsVersion: "2022.3",
      locale: "zh-CN",
    })).toBe(
      "/unity-reference-import?unityReferenceImport=1&targetPath=reference-folder&projectVersion=2022.3.47f1&docsVersion=2022.3&locale=zh-CN",
    );
  });

  it("reads the locale from the window query string", () => {
    expect(
      getUnityReferenceImportWindowPayload(
        "?unityReferenceImport=1&targetPath=reference-folder&projectVersion=2022.3.47f1&docsVersion=2022.3&locale=en",
      ),
    ).toMatchObject({
      targetPath: "reference-folder",
      projectVersion: "2022.3.47f1",
      docsVersion: "2022.3",
      locale: "en",
    });
  });

  it("focuses an existing window without resetting state when no payload is provided", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openUnityReferenceImportProgressWindow();

    expect(existingWindow.emit).not.toHaveBeenCalled();
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("pushes payload updates into an existing window and focuses it", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openUnityReferenceImportProgressWindow({
      targetPath: "reference-folder",
      running: true,
      projectVersion: "2022.3.47f1",
      docsVersion: "2022.3",
      locale: "zh-CN",
    });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      expect.objectContaining({
        targetPath: "reference-folder",
        running: true,
        projectVersion: "2022.3.47f1",
        docsVersion: "2022.3",
        locale: "zh-CN",
      }),
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
  });

  it("creates an independent top-level window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    await openUnityReferenceImportProgressWindow({
      targetPath: "reference-folder",
      projectVersion: "2022.3.47f1",
      docsVersion: "2022.3",
      locale: "en",
    });

    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [, options] = webviewWindowMocks.createdWindows[0] as [string, Record<string, unknown>];
    expect(options.parent).toBeUndefined();
    expect(options.center).toBe(true);
    expect(options.shadow).toBe(true);
  });
});
