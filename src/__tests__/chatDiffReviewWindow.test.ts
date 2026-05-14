import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileDiffRequest } from "../types";

const webviewWindowMocks = vi.hoisted(() => ({
  getByLabelMock: vi.fn(),
  getCurrentWebviewWindowMock: vi.fn(),
  createdWindows: [] as Array<unknown[]>,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: webviewWindowMocks.getCurrentWebviewWindowMock,
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
  CHAT_DIFF_REVIEW_WINDOW_EVENT,
  buildChatDiffReviewWindowUrl,
  getChatDiffReviewWindowPayload,
  openChatDiffReviewWindow,
} from "../services/chatDiffReviewWindow";

describe("chatDiffReviewWindow", () => {
  const request: FileDiffRequest = {
    source: "chatCheckpoint",
    filePath: "Assets/Scripts/Player.cs",
    sessionId: "session-1",
    assistantMessageId: "assistant-1",
    detail: "full",
  };

  beforeEach(() => {
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReset();
    webviewWindowMocks.getCurrentWebviewWindowMock.mockReturnValue({ label: "main" });
    webviewWindowMocks.createdWindows.length = 0;
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

  it("builds and parses request URLs for the dedicated review window", () => {
    const url = buildChatDiffReviewWindowUrl({ request });

    expect(url).toContain("/chat-diff-review?chatDiffReview=1");
    expect(getChatDiffReviewWindowPayload(url.slice(url.indexOf("?"))).request).toEqual(request);
  });

  it("focuses an existing review window and sends the next request", async () => {
    const existingWindow = {
      emit: vi.fn(),
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openChatDiffReviewWindow({ request });

    expect(existingWindow.emit).toHaveBeenCalledWith(
      CHAT_DIFF_REVIEW_WINDOW_EVENT,
      { request },
    );
    expect(existingWindow.setFocus).toHaveBeenCalledTimes(1);
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("creates a frameless child window bound to the current parent window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    const opened = await openChatDiffReviewWindow({ request });

    expect(opened).toBe(true);
    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [label, options] = webviewWindowMocks.createdWindows[0] as [string, Record<string, unknown>];
    expect(label).toBe("chat-diff-review");
    expect(options.parent).toEqual({ label: "main" });
    expect(options.decorations).toBe(false);
    expect(options.center).toBe(true);
    expect(options.shadow).toBe(true);
    expect(options.resizable).toBe(true);
    expect(options.closable).toBe(true);
    expect(options.width).toBe(1180);
    expect(options.height).toBe(760);
  });
});
