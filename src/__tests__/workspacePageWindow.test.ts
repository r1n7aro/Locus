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
  buildWorkspacePageWindowQuery,
  buildWorkspacePageWindowUrl,
  getWorkspacePageWindowPayload,
  isWorkspacePageWindowLocation,
  openWorkspacePageWindow,
  workspacePageWindowKind,
  type WorkspacePageWindowPayload,
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

const checkoutPayload = (
  checkoutId: string,
  page: "chat" | "knowledge" | "collab" | "asset" | "views" | "agent" = "knowledge",
): Extract<WorkspacePageWindowPayload, { scope: "checkout" }> => ({
  scope: "checkout",
  page,
  checkoutId,
  workspaceGeneration: 7,
  title: `${page} title`,
});

describe("workspacePageWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
    stubTauriWindow();
  });

  it("round-trips checkout and app payloads without an implicit workspace", () => {
    const checkout = checkoutPayload("checkout-A/unsafe?");
    const checkoutUrl = buildWorkspacePageWindowUrl(checkout);
    const checkoutSearch = checkoutUrl.slice(checkoutUrl.indexOf("?"));
    expect(checkoutUrl).toContain("/window.html?workspacePageWindow=1");
    expect(isWorkspacePageWindowLocation({ search: checkoutSearch } as Location)).toBe(true);
    expect(getWorkspacePageWindowPayload(checkoutSearch)).toEqual(checkout);

    const app: WorkspacePageWindowPayload = {
      scope: "app",
      page: "plugins",
      title: "插件",
    };
    const appQuery = buildWorkspacePageWindowQuery(app);
    expect(getWorkspacePageWindowPayload(`?${appQuery}`)).toEqual(app);
    expect(appQuery).not.toContain("checkoutId");
    expect(appQuery).not.toContain("workspaceGeneration");
  });

  it("keeps the same checkout page independent across A and B", () => {
    const kindA = workspacePageWindowKind(checkoutPayload("checkout-a"));
    const kindB = workspacePageWindowKind(checkoutPayload("checkout-b"));

    expect(kindA).not.toBe(kindB);
    expect(kindA).toMatch(/^[a-zA-Z0-9_-]+$/);
    expect(kindB).toMatch(/^[a-zA-Z0-9_-]+$/);
    expect(workspacePageWindowKind({
      ...checkoutPayload("checkout-a"),
      workspaceGeneration: 8,
    })).not.toBe(kindA);
    const agentKind = workspacePageWindowKind(checkoutPayload("checkout-a", "agent"));
    expect(agentKind).toContain("workspace-page-agent-");
  });

  it("maps legacy app URLs and rejects legacy checkout ambiguity", () => {
    expect(getWorkspacePageWindowPayload("?workspacePageWindow=1&page=settings&title=设置"))
      .toEqual({ scope: "app", page: "settings", title: "设置" });
    expect(getWorkspacePageWindowPayload("?workspacePageWindow=1&page=knowledge&title=知识"))
      .toBeNull();
    expect(isWorkspacePageWindowLocation({
      search: "?workspacePageWindow=1&page=knowledge",
    } as Location)).toBe(false);
  });

  it("rejects cross-scope pages and malformed checkout identities", () => {
    expect(getWorkspacePageWindowPayload(
      "?workspacePageWindow=1&scope=checkout&page=settings&checkoutId=a&workspaceGeneration=1",
    )).toBeNull();
    expect(getWorkspacePageWindowPayload(
      "?workspacePageWindow=1&scope=app&page=knowledge",
    )).toBeNull();
    expect(getWorkspacePageWindowPayload(
      "?workspacePageWindow=1&scope=app&page=agent&workspaceGeneration=x",
    )).toBeNull();
    expect(getWorkspacePageWindowPayload(
      "?workspacePageWindow=1&scope=checkout&page=knowledge&checkoutId=a&workspaceGeneration=x",
    )).toBeNull();
    expect(() => buildWorkspacePageWindowQuery({
      scope: "checkout",
      page: "settings",
      checkoutId: "checkout-a",
      workspaceGeneration: 1,
      title: "invalid",
    } as unknown as WorkspacePageWindowPayload)).toThrow("Invalid workspace page window payload");
  });

  it("opens a checkout-bound resizable standalone window", async () => {
    const payload = checkoutPayload("checkout-a", "asset");
    subWindowMocks.invokeMock.mockResolvedValue({
      label: workspacePageWindowKind(payload),
      existing: false,
      pooled: false,
    });

    await expect(openWorkspacePageWindow(payload)).resolves.toBe(true);

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: workspacePageWindowKind(payload),
        title: "Locus - asset title",
        width: 1280,
        height: 820,
        minimizable: true,
        closable: true,
        query: expect.stringContaining("checkoutId=checkout-a"),
      }),
    });
  });

  it("binds checkout pages through window context and keeps app pages process-level", () => {
    const app = read("src/App.vue");
    const windowApp = read("src/WindowApp.vue");
    const pageWindow = read("src/components/WorkspacePageWindow.vue");
    const pageBootstrap = read("src/composables/useWorkspacePageBootstrap.ts");
    const backend = read("src-tauri/src/lib.rs");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(app).toContain("openWorkspacePageWindow");
    expect(windowApp).toContain('kind: "workspace-page"');
    expect(pageWindow).toContain("KnowledgeView.vue");
    expect(pageWindow).toContain("WorkspaceChatPage.vue");
    expect(pageWindow).toContain("workspaceRef: checkoutWorkspaceRef.value");
    expect(pageWindow).toContain('workingDir: ""');
    expect(pageWindow).toContain("workspace-page-window-controls");
    expect(pageWindow).toContain("useWorkspacePageBootstrap");
    expect(pageWindow).toContain("workspaceContextStore.disposeWindow()");
    expect(pageWindow).not.toContain("useAppBootstrap");
    const chatPage = read("src/components/WorkspaceChatPage.vue");
    expect(chatPage).toContain("ChatWorkspaceView");
    expect(chatPage).toContain("registerListeners");
    expect(pageBootstrap).toContain("workspaceContextStore.initialize(currentWindowId, \"main\")");
    expect(pageBootstrap).toContain("workspaceContextStore.focusWorkspaceRef");
    expect(pageBootstrap).toContain("expectedGeneration: payload.workspaceGeneration");
    expect(pageBootstrap).not.toContain("projectStore.loadWorkingDir");
    expect(pageBootstrap).not.toContain("refreshSessions");
    expect(pageBootstrap).not.toContain("loadSkills");
    expect(pageBootstrap).not.toContain("registerListeners");
    expect(backend).toContain("WindowEvent::Destroyed");
    expect(backend).toContain("contexts.remove_window(window.label(), intent_epoch)");
    expect(capabilities).toContain('"workspace-page-*"');
  });

  it("keeps View inside Development and out of the primary navigation", () => {
    const app = read("src/App.vue");
    const developmentWorkbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const topTabs = app.slice(app.indexOf("const topTabs"), app.indexOf("const visibleTopTabs"));

    expect(topTabs).toContain('{ id: "development"');
    expect(topTabs).not.toContain('{ id: "views"');
    expect(topTabs).toContain('{ id: "plugins"');
    expect(topTabs).toContain('{ id: "agent"');
    expect(topTabs).not.toContain('{ id: "knowledge"');
    expect(topTabs).not.toContain('{ id: "collab"');
    expect(app).not.toContain("const projectTabs");
    expect(app).not.toContain('class="project-tab-context"');
    expect(app).toContain("DevelopmentWorkbench");
    expect(developmentWorkbench).toContain('editor.resource.section === \'views\'');
    expect(developmentWorkbench).toContain("<ViewPackageView");
    expect(app).toContain("openTopTabInWindow");
    expect(app).toContain('return tab.id !== "development"');
    expect(app).toMatch(
      /v-show="uiStore\.activePage === 'plugins'"[\s\S]{0,120}working-dir=""/,
    );
  });
});
