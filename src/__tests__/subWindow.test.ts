import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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
  buildSubWindowUrl,
  getSubWindowClaimedQuery,
  isSubWindowPoolLocation,
  openSubWindow,
  prepareSubWindowPool,
} from "../services/subWindow";

function stubTauriWindow() {
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
}

describe("subWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
    stubTauriWindow();
  });

  it("builds window.html urls from query strings", () => {
    expect(buildSubWindowUrl("planView=1&x=2")).toBe("/window.html?planView=1&x=2");
  });

  it("detects the pool window location", () => {
    expect(isSubWindowPoolLocation({ search: "?subWindowPool=1" })).toBe(true);
    expect(isSubWindowPoolLocation({ search: "?planView=1" })).toBe(false);
  });

  it("opens through the sub_window_open command with descriptor defaults", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "plan-view",
      existing: false,
      pooled: false,
    });
    const windowHandle = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(windowHandle);

    const result = await openSubWindow({
      kind: "plan-view",
      title: "Locus Plan Review",
      width: 920,
      height: 760,
      minWidth: 600,
      minHeight: 420,
    }, "planView=1&planFilePath=x");

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "plan-view",
        query: "planView=1&planFilePath=x",
        title: "Locus Plan Review",
        width: 920,
        height: 760,
        minWidth: 600,
        minHeight: 420,
        resizable: true,
        maximizable: true,
        minimizable: false,
        closable: true,
        focusExisting: true,
        backgroundColor: expect.stringMatching(/^#[0-9a-f]{6}$/i),
      }),
    });
    expect(result.existing).toBe(false);
    expect(result.label).toBe("plan-view");
    expect(result.window).toBe(windowHandle);
  });

  it("surfaces pool-claimed labels for existing windows", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-3",
      existing: true,
      pooled: false,
    });
    const windowHandle = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(windowHandle);

    const result = await openSubWindow({
      kind: "plan-view",
      title: "Locus Plan Review",
      width: 920,
      height: 760,
    }, "planView=1");

    expect(subWindowMocks.getByLabelMock).toHaveBeenCalledWith("sub-pool-3");
    expect(result.existing).toBe(true);
    expect(result.window).toBe(windowHandle);
  });

  it("pre-warms the pool with the current theme background", async () => {
    subWindowMocks.invokeMock.mockResolvedValue(undefined);

    await prepareSubWindowPool();

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_pool_prepare", {
      backgroundColor: expect.stringMatching(/^#[0-9a-f]{6}$/i),
    });
  });

  it("pulls the latest claimed query for a window kind", async () => {
    subWindowMocks.invokeMock.mockResolvedValue("extraWorkdirsConfig=1&workspacePath=C%3A%2FProj");

    await expect(getSubWindowClaimedQuery("extra-workdirs-config")).resolves.toBe(
      "extraWorkdirsConfig=1&workspacePath=C%3A%2FProj",
    );
    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_claimed_query", {
      kind: "extra-workdirs-config",
    });

    subWindowMocks.invokeMock.mockResolvedValue(null);
    await expect(getSubWindowClaimedQuery("plan-view")).resolves.toBeNull();
  });

  it("grants pool window labels the shared window capability", () => {
    // Pool windows carry sub-pool-N labels; without a capability entry the
    // shell cannot register its assign listener and the pool silently dies.
    const capabilities = readFileSync(
      resolve(process.cwd(), "src-tauri/capabilities/default.json"),
      "utf8",
    );
    expect(capabilities).toContain('"sub-pool-*"');
  });
});
