// @vitest-environment jsdom
import { createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sessionResultIsVisible, useSessionAttentionReader } from "../composables/useSessionAttentionReader";
import { applySessionAttentionEvent, emptySessionAttentionState, isSessionUnread, sessionAttention, updateSessionAttention } from "../services/sessionAttention";

describe("visible session read acknowledgement", () => {
  const cleanups: Array<() => void> = [];
  beforeEach(() => { localStorage.clear(); sessionAttention.value = emptySessionAttentionState(); vi.useFakeTimers(); });
  afterEach(async () => {
    cleanups.splice(0).forEach((dispose) => dispose());
    await updateSessionAttention(() => {});
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function mountReader(options: { sessionId?: () => string | null; rendered?: () => boolean; visible?: boolean } = {}) {
    const element = document.createElement("div");
    document.body.append(element);
    Object.defineProperties(element, { clientHeight: { value: 100 }, scrollHeight: { value: 500 } });
    element.scrollTop = 400;
    const rects = vi.spyOn(element, "getClientRects").mockReturnValue(
      (options.visible === false ? [] : [{}]) as unknown as DOMRectList,
    );
    const app = createApp({ setup() {
      useSessionAttentionReader(options.sessionId ?? (() => "a"), () => element, options.rendered ?? (() => true));
      return () => h("div");
    } });
    app.mount(element);
    const dispose = () => { app.unmount(); element.remove(); };
    cleanups.push(dispose);
    return { element, rects, dispose };
  }

  async function complete(sessionId = "a", runId = "first") {
    await updateSessionAttention((state) => applySessionAttentionEvent(state, {
      type: "done", sessionId, runId, messageId: `${runId}-reply`, fullText: "done",
    }, true));
    await nextTick();
  }

  it("does not mistake an older result in the DOM for a visible result", () => {
    const scroll = document.createElement("div");
    const message = document.createElement("div");
    message.dataset.chatMessageId = "result";
    scroll.append(message);
    vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue({ top: 100, bottom: 500 } as DOMRect);
    const bounds = vi.spyOn(message, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 80, width: 200, height: 80 } as DOMRect);
    expect(sessionResultIsVisible(scroll, "result")).toBe(false);
    bounds.mockReturnValue({ top: 300, bottom: 450, width: 200, height: 150 } as DOMRect);
    expect(sessionResultIsVisible(scroll, "result")).toBe(true);
  });

  it("requires the actual result, focused window and bottom viewport, without consuming a newer completion", async () => {
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const rendered = ref(false);
    const { element } = mountReader({ rendered: () => rendered.value });
    element.scrollTop = 0;
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    focused.mockReturnValue(true);
    rendered.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(true); // still reading history
    element.scrollTop = 400;
    element.dispatchEvent(new Event("scroll"));
    expect(isSessionUnread("a")).toBe(false);
    expect(sessionAttention.value.sessions.a!.completionSequence).toBe(1);
    rendered.value = false;
    await complete("a", "second");
    expect(isSessionUnread("a")).toBe(true);
    rendered.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
  });

  it("acknowledges a selected session after rendering without waiting for a timer", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    await complete("a");
    await complete("b");
    const selected = ref<string | null>(null);
    mountReader({ sessionId: () => selected.value });
    selected.value = "a";
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
    expect(isSessionUnread("b")).toBe(true);
    selected.value = "b";
    await nextTick();
    expect(isSessionUnread("b")).toBe(false);
  });

  it("rechecks focus and document visibility immediately", async () => {
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    mountReader();
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    focused.mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    expect(isSessionUnread("a")).toBe(false);
    visibility.mockReturnValue("hidden");
    await complete("a", "second");
    expect(isSessionUnread("a")).toBe(true);
    visibility.mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(isSessionUnread("a")).toBe(false);
  });

  it("waits for the actual message DOM when result rendering finishes later", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    let element: HTMLElement;
    ({ element } = mountReader({ rendered: () => !!element?.querySelector('[data-chat-message-id="first-reply"]') }));
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    const message = document.createElement("div");
    message.dataset.chatMessageId = "first-reply";
    element.append(message);
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
  });

  it("restores unread state after a failed save without an immediate retry loop", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    await complete();
    const request = vi.fn().mockRejectedValue(new Error("lock unavailable"));
    vi.stubGlobal("navigator", { locks: { request } });
    const warned = vi.spyOn(console, "warn").mockImplementation(() => {});
    mountReader();
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(isSessionUnread("a")).toBe(true);
    expect(request).toHaveBeenCalledOnce();
    expect(warned).toHaveBeenCalledOnce();
    vi.unstubAllGlobals();
  });

  it("leaves a kept-mounted hidden pane unread until its viewport becomes visible", async () => {
    let resize: () => void = () => {};
    const disconnect = vi.fn();
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { resize = callback; }
      observe() {}
      disconnect = disconnect;
    });
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const { rects } = mountReader({ visible: false });
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    rects.mockReturnValue([{}] as unknown as DOMRectList);
    resize();
    expect(isSessionUnread("a")).toBe(false);
    cleanups.pop()!();
    expect(disconnect).toHaveBeenCalledOnce();
    await complete("a", "second");
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(isSessionUnread("a")).toBe(true);
  });
});
