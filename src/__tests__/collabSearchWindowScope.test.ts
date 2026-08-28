import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildCollabSearchWindowQuery,
  getCollabSearchWindowWorkspaceRef,
  openCollabSearchWindow,
} from "../services/collabSearchWindow";
import type { WorkspaceRef } from "../services/project";

const subWindowMocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  getByLabel: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: subWindowMocks.invoke,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: vi.fn(() => ({ label: "main" })),
  WebviewWindow: class {
    static getByLabel = subWindowMocks.getByLabel;
  },
}));

describe("Collab search window workspace scope", () => {
  const workspaceRef: WorkspaceRef = {
    checkoutId: "checkout-feature/worktree",
    expectedGeneration: 23,
  };

  beforeEach(() => {
    subWindowMocks.invoke.mockReset();
    subWindowMocks.invoke.mockResolvedValue({ label: "collab", existing: false, pooled: false });
    subWindowMocks.getByLabel.mockReset();
    subWindowMocks.getByLabel.mockResolvedValue(null);
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

  it("round-trips an explicit checkout binding through the window URL", () => {
    const query = buildCollabSearchWindowQuery(workspaceRef);

    expect(getCollabSearchWindowWorkspaceRef(`?${query}`)).toEqual(workspaceRef);
  });

  it("rejects legacy search URLs without a checkout generation", () => {
    expect(getCollabSearchWindowWorkspaceRef("?collabSearch=1")).toBeNull();
    expect(getCollabSearchWindowWorkspaceRef("?collabSearch=1&checkoutId=checkout-a")).toBeNull();
  });

  it("uses a distinct immutable window kind for each checkout generation", async () => {
    await openCollabSearchWindow({ checkoutId: "checkout-a", expectedGeneration: 7 });
    await openCollabSearchWindow({ checkoutId: "checkout-a", expectedGeneration: 8 });

    const kinds = subWindowMocks.invoke.mock.calls.map((call) => call[1].request.kind);
    expect(kinds).toHaveLength(2);
    expect(kinds[0]).toMatch(/^collab-history-search-[0-9a-f]+-g7$/);
    expect(kinds[1]).toMatch(/^collab-history-search-[0-9a-f]+-g8$/);
    expect(kinds[0]).not.toBe(kinds[1]);
  });
});
