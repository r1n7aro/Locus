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
  CONTEXT_COMPACTION_WINDOW_EVENT,
  buildContextCompactionWindowUrl,
  getContextCompactionWindowPayload,
  openContextCompactionWindow,
} from "../services/contextCompactionWindow";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

describe("contextCompactionWindow", () => {
  const payload = { sessionId: "session-1", messageId: "handoff-1" };

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

  it("builds and parses lightweight-window URLs", () => {
    const url = buildContextCompactionWindowUrl(payload);

    expect(url).toContain("/window.html?contextCompaction=1");
    expect(getContextCompactionWindowPayload(url.slice(url.indexOf("?")))).toEqual(payload);
  });

  it("updates an existing compacted-context window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-4",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openContextCompactionWindow(payload);

    expect(existingWindow.emit).toHaveBeenCalledWith(
      CONTEXT_COMPACTION_WINDOW_EVENT,
      payload,
    );
  });

  it("opens through the pooled sub-window route", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "context-compaction",
      existing: false,
      pooled: false,
    });

    await expect(openContextCompactionWindow(payload)).resolves.toBe(true);
    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "context-compaction",
        query: expect.stringContaining("messageId=handoff-1"),
        width: 920,
        height: 720,
      }),
    });
  });

  it("wires the transcript marker to a context-complete read-only view", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const chatView = read("src/components/ChatView.vue");
    const windowApp = read("src/WindowApp.vue");
    const viewer = read("src/components/ContextCompactionWindow.vue");
    const capabilities = read("src-tauri/capabilities/default.json");
    const backend = read("src-tauri/src/session/store.rs");
    const commands = read("src-tauri/src/lib.rs");

    expect(transcript).toContain('emit("openCompactedContext", messageId)');
    expect(transcript).toContain("enableCompactedContextOpen");
    expect(chatView).toContain('@open-compacted-context="openCompactedContext"');
    expect(windowApp).toContain('kind: "context-compaction"');
    expect(viewer).toContain('chat.compactedContext.systemPlaceholder');
    expect(viewer).toContain("promptPrefixPlaceholder");
    expect(viewer).toContain("codexEncrypted");
    expect(viewer).toContain("enable-file-refs");
    expect(capabilities).toContain('"context-compaction"');
    expect(backend).toContain("persist_compacted_context_snapshot");
    expect(commands).toContain("commands::get_compacted_context_output");
  });
});
