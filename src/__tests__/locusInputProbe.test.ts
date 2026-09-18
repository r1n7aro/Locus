// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { installInputProbe } from "../../scripts/locus-input-probe.mjs";

const key = "__LOCUS_INPUT_CAPTURE_TEST";
const debugKey = "locus:webview-bridge:debug-enabled:v1";
function probe() {
  return (window as unknown as Record<string, { sample: () => { enabled: boolean; counts: Record<string, number> }; stop: () => void }>)[key];
}
beforeEach(() => { vi.useFakeTimers(); localStorage.clear(); });
afterEach(() => {
  probe()?.stop();
  Reflect.deleteProperty(window, key);
  Reflect.deleteProperty(window, "__LOCUS_DEBUG_ENABLED__");
  vi.restoreAllMocks(); vi.useRealTimers();
});

describe("debug-only input capture", () => {
  it("does not install listeners or timers when debug mode is off, even with a stale startup flag", () => {
    Reflect.set(window, "__LOCUS_DEBUG_ENABLED__", true);
    const add = vi.spyOn(window, "addEventListener");
    expect(installInputProbe(key, 1000)).toEqual({ enabled: false });
    expect(add).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(probe()).toBeUndefined();
  });

  it("removes all observers and its timer when debug mode is disabled during capture", () => {
    localStorage.setItem(debugKey, "1");
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    expect(installInputProbe(key, 10000).enabled).toBe(true);
    const observers = add.mock.calls.slice();
    localStorage.removeItem(debugKey);
    vi.advanceTimersByTime(250);
    for (const [type, listener] of observers) expect(remove).toHaveBeenCalledWith(type, listener, true);
    expect(probe()).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("leaves real page events alone and excludes synthetic clicks from evidence", () => {
    localStorage.setItem(debugKey, "1");
    installInputProbe(key, 10000);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    const handler = vi.fn();
    const button = document.createElement("button");
    button.addEventListener("click", handler);
    document.body.append(button);
    button.dispatchEvent(click);
    expect(handler).toHaveBeenCalledOnce();
    expect(click.defaultPrevented).toBe(false);
    expect(probe()!.sample().counts).toEqual({});
    button.remove();
  });

  it("stops before processing the next event when debug mode has just been disabled", () => {
    localStorage.setItem(debugKey, "1");
    installInputProbe(key, 10000);
    localStorage.removeItem(debugKey);
    window.dispatchEvent(new MouseEvent("pointermove"));
    expect(probe()).toBeUndefined();
    // jsdom queues storage notifications at 0ms; the probe interval is 250ms.
    vi.advanceTimersByTime(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("self-cleans if the collector disconnects and cannot request cleanup", () => {
    localStorage.setItem(debugKey, "1");
    installInputProbe(key, 1000);
    vi.advanceTimersByTime(1000);
    expect(probe()).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});
