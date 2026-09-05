// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ConsoleSettings from "../components/settings/ConsoleSettings.vue";
import type { DebugConsoleEntry } from "../types";

const consoleState = vi.hoisted(() => ({
  entries: [] as DebugConsoleEntry[],
  listener: null as (() => void) | null,
}));

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: vi.fn() }) }));
vi.mock("../services/permissions", () => ({ getDebugMode: async () => false }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("../services/debugConsole", () => ({
  getDebugConsoleSnapshot: () => consoleState.entries.slice(),
  subscribeDebugConsole: (listener: () => void) => {
    consoleState.listener = listener;
    return () => { consoleState.listener = null; };
  },
  initDebugConsole: async () => {},
  refreshDebugConsole: async () => {},
  clearDebugConsole: async () => {},
  revealLogFile: vi.fn(),
  saveDebugConsoleLogExport: vi.fn(),
}));

let app: App | null = null;
let root: HTMLElement;
let nextFrameId = 1;
const frames = new Map<number, FrameRequestCallback>();
const observers: TestResizeObserver[] = [];

class TestResizeObserver {
  targets = new Set<Element>();
  constructor(private callback: ResizeObserverCallback) { observers.push(this); }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); }
  emit(targets: Element[], height: number) {
    this.callback(targets.map((target) => ({
      target,
      borderBoxSize: [{ blockSize: height, inlineSize: 1000 }],
    } as unknown as ResizeObserverEntry)), this as unknown as ResizeObserver);
  }
}

async function tick() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

async function frame() {
  const pending = [...frames.values()];
  frames.clear();
  pending.forEach((callback) => callback(16));
  await tick();
}

function rows() { return [...root.querySelectorAll<HTMLElement>(".console-row")]; }
function bodyHeight() { return root.querySelector<HTMLElement>(".console-virtual-body")!.style.height; }
function rowObserver() { return observers.find((observer) => observer.targets.has(rows()[0]!))!; }

async function mount() {
  app = createApp(ConsoleSettings);
  app.mount(root);
  await tick();
  await frame();
}

beforeEach(() => {
  consoleState.entries = Array.from({ length: 2000 }, (_, index) => ({
    id: `log-${index}`,
    timestampMs: index,
    level: "info",
    source: "frontend",
    module: "test",
    target: "test",
    message: `Message ${index}`,
  }));
  root = document.createElement("div");
  document.body.appendChild(root);
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextFrameId++;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("console-list") ? 520 : 0;
  });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1000);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { height: this.classList.contains("console-header") ? 32 : 36 } as DOMRect;
  });
});

afterEach(() => {
  app?.unmount();
  app = null;
  frames.clear();
  observers.length = 0;
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("console virtual list runtime", () => {
  it("renders a bounded window and coalesces row measurements outside observer delivery", async () => {
    await mount();
    expect(rows().length).toBeLessThan(40);
    expect(rows()[0]!.textContent).toContain("Message 1999");
    const height = parseFloat(bodyHeight());
    const measured = rows().slice(0, 3);
    rowObserver().emit(measured, 60);
    rowObserver().emit(measured, 72);
    await tick();
    expect(parseFloat(bodyHeight())).toBe(height);
    expect(frames.size).toBe(1);
    await frame();
    expect(parseFloat(bodyHeight())).toBe(height + 3 * (72 - 36));
    expect(rows()[1]!.style.transform).toBe("translateY(72px)");
  });

  it("ignores hidden rows and cancels stale resize work when unmounted", async () => {
    await mount();
    const height = bodyHeight();
    const observer = rowObserver();
    observer.emit(rows(), 0);
    await frame();
    expect(bodyHeight()).toBe(height);
    observer.emit(rows(), 90);
    app!.unmount();
    app = null;
    expect(frames.size).toBe(0);
    expect(observers.every((item) => item.targets.size === 0)).toBe(true);
    expect(consoleState.listener).toBeNull();
  });

  it("keeps manual scrolling on new logs and resets it on filtering or enabling auto-scroll", async () => {
    await mount();
    const list = root.querySelector<HTMLElement>(".console-list")!;
    const toggle = root.querySelector<HTMLButtonElement>('[role="switch"]')!;
    toggle.click();
    await tick();
    list.scrollTop = 36000;
    list.dispatchEvent(new Event("scroll"));
    await frame();
    expect(rows().length).toBeLessThan(50);
    expect(rows()[0]!.dataset.consoleEntryId).not.toBe("log-1999");
    consoleState.entries = [...consoleState.entries.slice(1), {
      ...consoleState.entries[0]!, id: "latest", timestampMs: 2001, message: "Latest message",
    }];
    consoleState.listener!();
    await tick();
    expect(list.scrollTop).toBe(36000);
    toggle.click();
    await tick();
    expect(list.scrollTop).toBe(0);
    expect(rows()[0]!.textContent).toContain("Latest message");
    toggle.click();
    list.scrollTop = 36000;
    list.dispatchEvent(new Event("scroll"));
    await frame();
    const search = root.querySelector<HTMLInputElement>(".console-search")!;
    search.value = "Latest message";
    search.dispatchEvent(new Event("input"));
    await tick();
    await frame();
    expect(list.scrollTop).toBe(0);
    expect(rows()).toHaveLength(1);
    expect(root.querySelector(".console-search-hit")!.textContent).toBe("Latest message");
  });

  it("remeasures visible rows even when a column change leaves their height unchanged", async () => {
    await mount();
    rowObserver().emit(rows(), 72);
    await frame();
    root.querySelector<HTMLElement>(".console-column-handle")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await tick();
    await frame();
    expect(rows()[1]!.style.transform).toBe("translateY(36px)");
  });

  it("preserves full long messages behind the expandable preview", async () => {
    consoleState.entries[1999]!.message = "x".repeat(5000);
    await mount();
    expect(root.querySelector(".console-message")!.textContent).toHaveLength(4000);
    root.querySelector<HTMLButtonElement>(".console-message-toggle")!.click();
    await tick();
    expect(root.querySelector(".console-message")!.textContent).toHaveLength(5000);
    root.querySelector<HTMLButtonElement>(".console-message-toggle")!.click();
    await tick();
    expect(root.querySelector(".console-message")!.textContent).toHaveLength(4000);
  });
});
