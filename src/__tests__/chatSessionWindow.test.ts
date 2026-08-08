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
  CHAT_SESSION_WINDOW_EVENT,
  buildChatSessionWindowUrl,
  chatSessionWindowKind,
  getChatSessionWindowPayload,
  newChatSessionWindowKind,
  openChatSessionWindow,
  openNewChatSessionWindow,
} from "../services/chatSessionWindow";

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

describe("chatSessionWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
    stubTauriWindow();
  });

  it("builds and parses a lightweight-window URL", () => {
    const url = buildChatSessionWindowUrl({
      sessionId: "session-1",
      title: "材质检查",
    });

    expect(url).toContain("/window.html?chatSessionWindow=1");
    expect(getChatSessionWindowPayload(url.slice(url.indexOf("?")))).toEqual({
      sessionId: "session-1",
      title: "材质检查",
      newChat: false,
    });
  });

  it("creates stable capability-safe kinds per session", () => {
    const first = chatSessionWindowKind("session:one/1");
    const second = chatSessionWindowKind("session:two/2");

    expect(first).toMatch(/^chat-session-[a-zA-Z0-9_-]+$/);
    expect(first).toBe(chatSessionWindowKind("session:one/1"));
    expect(first).not.toBe(second);
  });

  it("creates a distinct capability-safe kind for each new-session window", () => {
    const first = newChatSessionWindowKind();
    const second = newChatSessionWindowKind();

    expect(first).toMatch(/^chat-session-new-[a-z0-9-]+$/);
    expect(first).not.toBe(second);
  });

  it("opens one resizable, minimizable window for the selected session", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "chat-session-session-1-12345678",
      existing: false,
      pooled: false,
    });

    await expect(openChatSessionWindow({
      sessionId: "session-1",
      title: "Player movement",
    })).resolves.toBe(true);

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: chatSessionWindowKind("session-1"),
        title: "Locus - Player movement",
        width: 1040,
        height: 780,
        minWidth: 660,
        minHeight: 480,
        resizable: true,
        maximizable: true,
        minimizable: true,
        query: expect.stringContaining("sessionId=session-1"),
      }),
    });
  });

  it("opens each new session in its own independent window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "chat-session-new-window",
      existing: false,
      pooled: false,
    });

    await expect(openNewChatSessionWindow()).resolves.toBe(true);

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: expect.stringMatching(/^chat-session-new-/),
        title: "Locus - New session",
        minimizable: true,
        closable: true,
        query: expect.stringContaining("newChat=1"),
      }),
    });
  });

  it("focuses and refreshes the matching existing session window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-5",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openChatSessionWindow({ sessionId: "session-1", title: "Session 1" });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      CHAT_SESSION_WINDOW_EVENT,
      { sessionId: "session-1", title: "Session 1" },
    );
  });

  it("wires the session context menu to an independently selected full chat workspace", () => {
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const windowApp = read("src/WindowApp.vue");
    const sessionWindow = read("src/components/ChatSessionWindow.vue");
    const chatView = read("src/components/ChatView.vue");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(sessionPanel).toContain("ctxOpenSessionInWindow");
    expect(sessionPanel).toContain("openNewChatSessionWindow");
    expect(sessionPanel).toContain("onNewSessionContextMenu");
    expect(sessionPanel).toContain("if (e.ctrlKey)");
    expect(sessionPanel).toContain("chat.session.openInWindow");
    expect(windowApp).toContain('kind: "chat-session"');
    expect(sessionWindow).toContain("syncActiveSessionSelection: false");
    expect(sessionWindow).toContain("setActiveSessionSelectionPersistence(false)");
    expect(sessionWindow).toContain(':show-session-navigation="false"');
    expect(sessionWindow).toContain(':persist-session-selection="false"');
    expect(sessionWindow).toContain("SessionCompactPicker");
    expect(sessionWindow).toContain('@select-session="selectWindowSession"');
    expect(sessionWindow).toContain("ChatWorkspaceView");
    expect(sessionWindow).toContain("handleTitlebarPointerDown");
    expect(sessionWindow).toContain("chat-session-window-controls");
    expect(chatView).toContain("showSessionNavigation !== false");
    expect(chatView).toContain("<ModelEffortSelector");
    expect(capabilities).toContain('"chat-session-*"');
  });
});
