// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => undefined),
  hasTauriWindowRuntime: vi.fn(() => true),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../services/tauriRuntime", () => ({ hasTauriWindowRuntime: mocks.hasTauriWindowRuntime }));

import {
  MAX_CHAT_MESSAGE_IMAGE_PIXELS,
  chatMessageImageScale,
  findChatMessageImageTarget,
  writePngToClipboard,
} from "../composables/chatMessageImage";

class FakeClipboardItem {
  constructor(readonly items: Record<string, unknown>) {}
}

function fakeWindow(write?: (items: FakeClipboardItem[]) => Promise<void>) {
  return {
    navigator: { clipboard: write ? { write } : undefined },
    ClipboardItem: write ? FakeClipboardItem : undefined,
  } as unknown as Window;
}

const pngBlob = () => new Blob([new Uint8Array([137, 80, 78, 71])], { type: "image/png" });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.hasTauriWindowRuntime.mockReturnValue(true);
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("chatMessageImageScale", () => {
  it("follows the device pixel ratio between 1x and 2x", () => {
    expect(chatMessageImageScale(800, 600, 1.5)).toBe(1.5);
    expect(chatMessageImageScale(800, 600, 0.8)).toBe(1);
    expect(chatMessageImageScale(800, 600, 3)).toBe(2);
    expect(chatMessageImageScale(800, 600, Number.NaN)).toBe(1);
  });

  it("scales tall messages down to the canvas limits", () => {
    const scale = chatMessageImageScale(1000, 20_000, 2)!;
    expect(scale).toBeGreaterThanOrEqual(1);
    expect(scale).toBeLessThan(2);
    expect(1000 * scale * 20_000 * scale).toBeLessThanOrEqual(MAX_CHAT_MESSAGE_IMAGE_PIXELS + 1);
    expect(chatMessageImageScale(400, 32_000, 2)).toBeLessThanOrEqual(32_767 / 32_000);
  });

  it("rejects messages that do not fit even at 1x", () => {
    expect(chatMessageImageScale(800, 40_000, 1)).toBeNull();
    expect(chatMessageImageScale(0, 100, 1)).toBeNull();
  });
});

describe("findChatMessageImageTarget", () => {
  it("returns the highlighted item stack rather than the whole message group", () => {
    document.body.innerHTML = `
      <div class="scroll">
        <div class="chat-transcript-message" data-chat-message-id="a1" data-chat-message-group-end-id="a2">
          <div class="chat-transcript-item-stack" data-scroll-anchor-id="a2" data-chat-message-id="a1">
            <div data-chat-message-id="a2">answer</div>
          </div>
        </div>
      </div>`;
    const root = document.querySelector<HTMLElement>(".scroll")!;
    expect(findChatMessageImageTarget(root, "a2")?.classList.contains("chat-transcript-item-stack")).toBe(true);
    expect(findChatMessageImageTarget(root, "missing")).toBeNull();
  });

  it("falls back to the message element and escapes the id", () => {
    document.body.innerHTML = `<div class="scroll"><div data-chat-message-id='quo"te'>x</div></div>`;
    const root = document.querySelector<HTMLElement>(".scroll")!;
    expect(findChatMessageImageTarget(root, 'quo"te')?.textContent).toBe("x");
  });
});

describe("writePngToClipboard", () => {
  it("hands the pending PNG to the async clipboard", async () => {
    const write = vi.fn(async (_items: FakeClipboardItem[]) => undefined);
    const png = Promise.resolve(pngBlob());
    await writePngToClipboard(png, fakeWindow(write));
    expect(write).toHaveBeenCalledOnce();
    expect(write.mock.calls[0][0][0].items["image/png"]).toBe(png);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("falls back to the native clipboard with the raw PNG bytes", async () => {
    const write = vi.fn(async () => { throw new DOMException("Document is not focused.", "NotAllowedError"); });
    await writePngToClipboard(Promise.resolve(pngBlob()), fakeWindow(write));
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("write_clipboard_png", new Uint8Array([137, 80, 78, 71]));

    mocks.invoke.mockClear();
    await writePngToClipboard(Promise.resolve(pngBlob()), fakeWindow());
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });

  it("reports the render failure instead of the clipboard error", async () => {
    const renderError = new Error("render failed");
    const write = vi.fn(async () => { throw new DOMException("rejected", "NotAllowedError"); });
    await expect(writePngToClipboard(Promise.reject(renderError), fakeWindow(write))).rejects.toBe(renderError);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("surfaces the clipboard error outside the desktop app", async () => {
    mocks.hasTauriWindowRuntime.mockReturnValue(false);
    const clipboardError = new DOMException("Document is not focused.", "NotAllowedError");
    const write = vi.fn(async () => { throw clipboardError; });
    await expect(writePngToClipboard(Promise.resolve(pngBlob()), fakeWindow(write))).rejects.toBe(clipboardError);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
