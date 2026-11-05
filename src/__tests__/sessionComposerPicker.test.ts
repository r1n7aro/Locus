// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import SessionComposerPicker from "../components/chat/SessionComposerPicker.vue";
import type { SessionSummary } from "../types";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import {
  isSessionUnread, markSessionAttentionRead, resetSessionAttentionForTests, sessionAttention,
} from "../services/sessionAttention";
import { createSessionAttentionServer } from "./fixtures/sessionAttentionServer";

vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join(" ") }));

let app: App | undefined;
let attentionServer: ReturnType<typeof createSessionAttentionServer>;
beforeAll(() => { attentionServer = createSessionAttentionServer(); });
afterAll(() => attentionServer.uninstall());
beforeEach(() => {
  localStorage.clear();
  resetSessionAttentionForTests();
  useDisplaySettings().state.showSessionUnreadIndicators = true;
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  vi.setSystemTime(new Date("2026-10-12T04:00:00Z"));
  Object.defineProperty(Element.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
});
afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.useRealTimers();
  useDisplaySettings().state.showSessionUnreadIndicators = true;
  Reflect.deleteProperty(Element.prototype, "scrollIntoView");
});

async function flush() {
  await nextTick();
  await nextTick();
  // Vue ignores bubbled events stamped at the same millisecond as listener attachment.
  vi.advanceTimersByTime(1);
}

function mount(ownerDocument = document) {
  const host = ownerDocument.createElement("div");
  ownerDocument.body.append(host);
  const state = reactive({
    activeSessionId: "session-0",
    active: true,
    disabled: false,
    sessions: Array.from({ length: 20 }, (_, index): SessionSummary => ({
      id: `session-${index}`, title: `Session ${index}`, updatedAt: Date.now() / 1000 - (20 - index) * 60, sessionType: "chat",
    })),
  });
  const select = vi.fn();
  app = createApp({ render: () => h(SessionComposerPicker, { ...state, onSelectSession: select }) });
  app.mount(host);
  const root = host.querySelector<HTMLElement>(".session-composer-picker")!;
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 500, left: 100, bottom: 524, right: 124, width: 24, height: 24, x: 100, y: 500, toJSON() {} });
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  const panel = () => ownerDocument.querySelector<HTMLElement>(".session-composer-panel");
  const options = () => Array.from(panel()?.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? []);
  return { state, trigger, select, panel, options, async open() { trigger.click(); await flush(); } };
}

describe("composer session picker", () => {
  it("opens above the composer, lists every session by recency and marks the current one", async () => {
    const view = mount();
    await view.open();
    expect(view.panel()!.style.bottom).toBe(`${window.innerHeight - 500 + 6}px`);
    expect(view.panel()!.style.maxHeight).toBe("340px");
    expect(view.trigger.getAttribute("aria-expanded")).toBe("true");
    expect(view.options()).toHaveLength(20);
    expect(view.options()[0]!.querySelector(".session-composer-title")!.textContent).toBe("Session 19");
    expect(view.panel()!.querySelector('[aria-selected="true"] .session-composer-title')!.textContent).toBe("Session 0");
    expect(document.activeElement).toBe(view.panel()!.querySelector("input"));
  });

  it("filters titles, selects a result and closes with focus restored", async () => {
    const view = mount();
    await view.open();
    const search = view.panel()!.querySelector("input")!;
    search.value = "SESSION 19";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    expect(view.options()).toHaveLength(1);
    view.options()[0]!.click();
    await flush();
    expect(view.select).toHaveBeenCalledExactlyOnceWith("session-19");
    expect(view.panel()).toBeNull();
    expect(document.activeElement).toBe(view.trigger);
    await view.open();
    expect(view.options()).toHaveLength(20);
  });

  it("supports arrow navigation, Escape and outside dismissal without selecting", async () => {
    const view = mount();
    await view.open();
    view.panel()!.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(document.activeElement).toBe(view.options()[view.options().length - 1]);
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement).toBe(view.options()[0]);
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flush();
    expect(view.panel()).toBeNull();
    expect(document.activeElement).toBe(view.trigger);
    await view.open();
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    await flush();
    expect(view.panel()).toBeNull();
    expect(view.select).not.toHaveBeenCalled();
  });

  it("distinguishes an empty catalog from no matches and ignores IME confirmation", async () => {
    const view = mount();
    await view.open();
    const search = view.panel()!.querySelector("input")!;
    search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }));
    expect(view.select).not.toHaveBeenCalled();
    search.value = "missing";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    expect(view.panel()!.textContent).toContain("globalSearch.noResults");
    search.value = "";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    view.state.sessions = [];
    await flush();
    expect(view.panel()!.textContent).toContain("chat.session.noSessions");
  });

  it("does not reload the current session and closes when its tab becomes inactive", async () => {
    const view = mount();
    await view.open();
    view.panel()!.querySelector<HTMLButtonElement>('[aria-selected="true"]')!.click();
    await flush();
    expect(view.select).not.toHaveBeenCalled();
    expect(view.panel()).toBeNull();
    await view.open();
    view.state.active = false;
    await flush();
    expect(view.panel()).toBeNull();
  });

  it("keeps the popup in the document that owns the composer", async () => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const ownerDocument = frame.contentDocument!;
    const view = mount(ownerDocument);
    await view.open();
    expect(view.panel()!.parentElement).toBe(ownerDocument.body);
    expect(document.querySelector(".session-composer-panel")).toBeNull();
    view.options()[0]!.click();
    await flush();
    expect(view.select).toHaveBeenCalledExactlyOnceWith("session-19");
  });

  it("uses the workspace time format, refreshes while open and releases the timer on close", async () => {
    const view = mount();
    const now = Date.now() / 1000;
    view.state.sessions = [
      { id: "recent", title: "Recent", updatedAt: now - 30, sessionType: "chat" },
      { id: "hours", title: "Hours", updatedAt: now - 3 * 3600, sessionType: "chat" },
      { id: "older", title: "Older", updatedAt: now - 40 * 86400, sessionType: "chat" },
    ];
    await view.open();
    const times = () => view.options().map(option => option.querySelector(".session-composer-updated-time")!.textContent);
    expect(times()).toEqual(["time.justNow", "time.hoursAgo 3", "time.daysAgo 40"]);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(times()[0]).toBe("time.minutesAgo 1");
    view.trigger.click();
    await flush();
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    await view.open();
    expect(times()[0]).toBe("time.minutesAgo 6");
    app!.unmount();
    app = undefined;
    expect(vi.getTimerCount()).toBe(0);
  });

  it("shares unread state with the workspace without marking sessions read when opening or selecting", async () => {
    const view = mount();
    attentionServer.record("session-19");
    await view.open();
    const row = () => view.options()[0]!;
    expect(row().querySelector('.session-composer-unread-dot')!.getAttribute('aria-label')).toBe("chat.session.unread");
    expect(row().querySelector('.session-composer-updated-time')).toBeNull();
    expect(row().getAttribute("title")).toContain("time.minutesAgo 1");
    expect(row().getAttribute("aria-label")).toContain("chat.session.unread");
    expect(isSessionUnread("session-19")).toBe(true);
    row().click();
    await flush();
    expect(isSessionUnread("session-19")).toBe(true);
    await view.open();
    await markSessionAttentionRead("session-19", sessionAttention.value.sessions["session-19"]!.sequence);
    await flush();
    expect(row().querySelector('.session-composer-unread-dot')).toBeNull();
    expect(row().querySelector('.session-composer-updated-time')!.textContent).toBe("time.minutesAgo 1");
  });

  it("reacts to new unread events and the shared display preference while the list is open", async () => {
    const view = mount();
    await view.open();
    attentionServer.record("session-19");
    await flush();
    expect(view.options()[0]!.querySelector('.session-composer-unread-dot')).not.toBeNull();
    useDisplaySettings().state.showSessionUnreadIndicators = false;
    await flush();
    expect(view.options()[0]!.querySelector('.session-composer-unread-dot')).toBeNull();
    expect(view.options()[0]!.querySelector('.session-composer-updated-time')!.textContent).toBe("time.minutesAgo 1");
    expect(isSessionUnread("session-19")).toBe(true);
  });
});
