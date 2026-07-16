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
  REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
  buildReferenceExternalImportWindowUrl,
  getReferenceExternalImportWindowPayload,
  openReferenceExternalImportWindow,
} from "../services/referenceExternalImportWindow";

describe("referenceExternalImportWindow", () => {
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

  it("builds the dedicated window url", () => {
    const url = buildReferenceExternalImportWindowUrl({
      parentDir: "reference/docs",
      initialSource: "feishu",
    });

    expect(url).toContain("/window.html?referenceExternalImport=1");
    expect(url).toContain("parentDir=reference%2Fdocs");
    expect(url).toContain("initialSource=feishu");

    const payload = getReferenceExternalImportWindowPayload(url.slice(url.indexOf("?")));
    expect(payload.parentDir).toBe("reference/docs");
    expect(payload.initialSource).toBe("feishu");
  });

  it("updates the payload of an existing window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-5",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openReferenceExternalImportWindow({ parentDir: "reference/docs" });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT,
      expect.objectContaining({ parentDir: "reference/docs" }),
    );
  });

  it("creates the import window through the sub-window command", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "reference-external-import",
      existing: false,
      pooled: true,
    });

    await openReferenceExternalImportWindow({ initialSource: "local" });

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "reference-external-import",
        title: "Locus External Import",
        width: 1180,
        height: 900,
        minWidth: 920,
        minHeight: 700,
        resizable: true,
        closable: false,
        query: expect.stringContaining("referenceExternalImport=1"),
      }),
    });
  });
});
