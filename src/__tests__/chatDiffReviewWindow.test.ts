import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileDiffRequest } from "../types";

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

  it("builds and parses request URLs for the lightweight window entry", () => {
    const url = buildChatDiffReviewWindowUrl({ request });

    expect(url).toContain("/window.html?chatDiffReview=1");
    expect(getChatDiffReviewWindowPayload(url.slice(url.indexOf("?"))).request).toEqual(request);
  });

  it("sends the next request to an existing review window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-1",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openChatDiffReviewWindow({ request });

    expect(subWindowMocks.getByLabelMock).toHaveBeenCalledWith("sub-pool-1");
    expect(existingWindow.emit).toHaveBeenCalledWith(
      CHAT_DIFF_REVIEW_WINDOW_EVENT,
      { request },
    );
  });

  it("opens a new review window through the pooled sub-window command", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "chat-diff-review",
      existing: false,
      pooled: true,
    });
    const newWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(newWindow);

    const opened = await openChatDiffReviewWindow({ request });

    expect(opened).toBe(true);
    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "chat-diff-review",
        title: "Locus File Review",
        width: 1180,
        height: 760,
        minWidth: 760,
        minHeight: 520,
        resizable: true,
        maximizable: true,
        minimizable: false,
        query: expect.stringContaining("chatDiffReview=1"),
      }),
    });
    expect(newWindow.emit).not.toHaveBeenCalled();
  });
});
