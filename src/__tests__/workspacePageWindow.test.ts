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
  buildWorkspacePageWindowUrl,
  getWorkspacePageWindowPayload,
  isWorkspacePageWindowLocation,
  openWorkspacePageWindow,
  workspacePageWindowKind,
} from "../services/workspacePageWindow";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

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

describe("workspacePageWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
    stubTauriWindow();
  });

  it("builds and parses a workspace page window URL", () => {
    const url = buildWorkspacePageWindowUrl({ page: "knowledge", title: "知识" });
    expect(url).toContain("/window.html?workspacePageWindow=1");
    expect(isWorkspacePageWindowLocation({ search: url.slice(url.indexOf("?")) } as Location))
      .toBe(true);
    expect(getWorkspacePageWindowPayload(url.slice(url.indexOf("?")))).toEqual({
      page: "knowledge",
      title: "知识",
    });
    expect(isWorkspacePageWindowLocation({ search: "?page=knowledge" } as Location)).toBe(false);
    expect(isWorkspacePageWindowLocation({
      search: "?workspacePageWindow=1&page=chat",
    } as Location)).toBe(false);
  });

  it("opens one resizable standalone window per page", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "workspace-page-asset",
      existing: false,
      pooled: false,
    });

    await expect(openWorkspacePageWindow({ page: "asset", title: "资产" }))
      .resolves.toBe(true);

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: workspacePageWindowKind("asset"),
        title: "Locus - 资产",
        width: 1280,
        height: 820,
        minimizable: true,
        closable: true,
        query: expect.stringContaining("page=asset"),
      }),
    });
  });

  it("wires top-tab context and Ctrl-click actions to the standalone shell", () => {
    const app = read("src/App.vue");
    const windowApp = read("src/WindowApp.vue");
    const pageWindow = read("src/components/WorkspacePageWindow.vue");
    const pageBootstrap = read("src/composables/useWorkspacePageBootstrap.ts");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(app).toContain("openTopTabContextMenu");
    expect(app).toContain("event.ctrlKey && canOpenTopTabInWindow");
    expect(app).toContain("openWorkspacePageWindow");
    expect(app).toContain("app.tab.openInWindow");
    expect(windowApp).toContain('kind: "workspace-page"');
    expect(app).toContain("isWorkspacePageWindowLocation");
    expect(app).toContain('<WorkspacePageWindow v-else-if="isWorkspacePageWindow" />');
    expect(pageWindow).toContain("CollabView.vue");
    expect(pageWindow).toContain("KnowledgeView.vue");
    expect(pageWindow).toContain("AssetView.vue");
    expect(pageWindow).toContain("SettingsView.vue");
    expect(app).toContain("WORKSPACE_PAGE_RESET_ONBOARDING_EVENT");
    expect(pageWindow).toContain("workspace-page-window-controls");
    expect(pageWindow).toContain("useWorkspacePageBootstrap");
    expect(pageWindow).not.toContain("useAppBootstrap");
    expect(pageBootstrap).toContain('page === "knowledge" || page === "collab" || page === "settings"');
    expect(pageBootstrap).not.toContain("refreshSessions");
    expect(pageBootstrap).not.toContain("loadSkills");
    expect(pageBootstrap).not.toContain("registerListeners");
    expect(capabilities).toContain('"workspace-page-*"');
  });
});
