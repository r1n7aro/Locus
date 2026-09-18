// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { observeCsvGridResize } from "../components/csv/csvGridResize";

let callback: ResizeObserverCallback;
let handle: ReturnType<typeof observeCsvGridResize>;
let element: HTMLElement;
let allowed: boolean;
const redraw = vi.fn();
const disconnect = vi.fn();

function resize(width: number, height = 640) {
  callback([{ target: element, contentRect: { width, height } } as unknown as ResizeObserverEntry], {} as ResizeObserver);
}

beforeEach(() => {
  vi.useFakeTimers(); allowed = true;
  vi.stubGlobal("ResizeObserver", class {
    constructor(fn: ResizeObserverCallback) { callback = fn; }
    observe() {} unobserve() {} disconnect = disconnect;
  });
  element = document.createElement("div");
  handle = observeCsvGridResize(element, redraw, () => allowed);
});
afterEach(() => { handle.disconnect(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

describe("CSV viewport resize scheduling", () => {
  it("coalesces an animated resize and never redraws inside the observer callback", async () => {
    resize(1100); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).not.toHaveBeenCalled();
    for (let index = 1; index <= 18; index++) {
      resize(1100 - index * 15);
      expect(redraw).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(16);
    }
    await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
    resize(830.4, 640.4); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
  });

  it("defers hidden and editing grids until resumed, using their latest size", async () => {
    resize(1100); await vi.advanceTimersByTimeAsync(120);
    resize(1000); await vi.advanceTimersByTimeAsync(16);
    allowed = false; handle.pause();
    resize(900); await vi.advanceTimersByTimeAsync(200);
    expect(redraw).not.toHaveBeenCalled();
    allowed = true; handle.resume(); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
  });

  it("rechecks edit state when a scheduled redraw executes", async () => {
    resize(1100); await vi.advanceTimersByTimeAsync(120);
    resize(900); await vi.advanceTimersByTimeAsync(16);
    allowed = false; await vi.advanceTimersByTimeAsync(120);
    expect(redraw).not.toHaveBeenCalled();
    allowed = true; handle.resume(); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
  });

  it("redraws a table mounted at zero size after it becomes visible", async () => {
    resize(0, 0); await vi.advanceTimersByTimeAsync(120);
    handle.request(); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).not.toHaveBeenCalled();
    resize(1000); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
  });

  it("refreshes a reactivated table even when its size did not change", async () => {
    resize(1100); await vi.advanceTimersByTimeAsync(120);
    handle.request(); handle.request(); await vi.advanceTimersByTimeAsync(120);
    expect(redraw).toHaveBeenCalledTimes(1);
  });

  it("cancels queued animation frames and timers on disposal", async () => {
    resize(1100); await vi.advanceTimersByTimeAsync(120);
    resize(900); await vi.advanceTimersByTimeAsync(16);
    resize(800); handle.disconnect(); await vi.advanceTimersByTimeAsync(200);
    handle.resume(); handle.request(); await vi.advanceTimersByTimeAsync(200);
    expect(disconnect).toHaveBeenCalledTimes(1);
    expect(redraw).not.toHaveBeenCalled();
  });
});
