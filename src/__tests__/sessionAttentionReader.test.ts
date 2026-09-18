// @vitest-environment jsdom
import { createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionAttentionReader } from "../composables/useSessionAttentionReader";
import { applySessionAttentionEvent, emptySessionAttentionState, isSessionUnread, sessionAttention, updateSessionAttention } from "../services/sessionAttention";

describe("focused session read acknowledgement", () => {
  const cleanups: Array<() => void> = [];
  beforeEach(() => { localStorage.clear(); sessionAttention.value = emptySessionAttentionState(); vi.useFakeTimers(); });
  afterEach(async () => {
    cleanups.splice(0).forEach((dispose) => dispose());
    await updateSessionAttention(() => {});
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function mountReader(options: {
    sessionId?: () => string | null;
    active?: () => boolean;
    ready?: () => boolean;
    visible?: boolean;
  } = {}) {
    const element = document.createElement("div");
    document.body.append(element);
    Object.defineProperties(element, { clientHeight: { value: 100 }, scrollHeight: { value: 500 } });
    element.scrollTop = 400;
    const rects = vi.spyOn(element, "getClientRects").mockReturnValue(
      (options.visible === false ? [] : [{}]) as unknown as DOMRectList,
    );
    const app = createApp({ setup() {
      useSessionAttentionReader(options.sessionId ?? (() => "a"), () => element,
        () => (options.active?.() ?? true) && (options.ready?.() ?? true));
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

  it("reads a loaded session on window focus even when its saved scroll position is in history", async () => {
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const ready = ref(false);
    const { element } = mountReader({ ready: () => ready.value });
    element.scrollTop = 0;
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    ready.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(true);
    focused.mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    expect(isSessionUnread("a")).toBe(false);
    expect(sessionAttention.value.sessions.a!.completionSequence).toBe(1);
    expect(element.scrollTop).toBe(0);
    ready.value = false;
    await complete("a", "second");
    expect(isSessionUnread("a")).toBe(true);
    ready.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
  });

  it("acknowledges a selected session without waiting for a timer", async () => {
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

  it("reads only the focused pane and rechecks already mounted tabs on activation", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const active = ref<string | null>(null);
    const { element } = mountReader({ active: () => active.value === "a" });
    mountReader({ sessionId: () => "b", active: () => active.value === "b" });
    element.scrollTop = 0;
    await complete("a");
    await complete("b");
    expect(isSessionUnread("a")).toBe(true);
    expect(isSessionUnread("b")).toBe(true);
    active.value = "a";
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
    expect(isSessionUnread("b")).toBe(true);
    expect(element.querySelector("[data-chat-message-id]")).toBeNull();
    active.value = "b";
    await nextTick();
    expect(isSessionUnread("b")).toBe(false);
    await complete("a", "second");
    expect(isSessionUnread("a")).toBe(true);
    active.value = "a";
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
  });

  it("waits for loading and scroll restoration after a tab is activated", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const active = ref(false);
    const ready = ref(false);
    mountReader({ active: () => active.value, ready: () => ready.value });
    await complete();
    active.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(true);
    ready.value = true;
    await nextTick();
    expect(isSessionUnread("a")).toBe(false);
  });

  it("rechecks when keyboard focus enters the document", async () => {
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const { element } = mountReader();
    await complete();
    expect(isSessionUnread("a")).toBe(true);
    focused.mockReturnValue(true);
    element.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
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
