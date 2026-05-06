import { afterEach, describe, expect, it, vi } from "vitest";
import { createAnimationFrameResizeObserver } from "../composables/resizeObserver";

describe("createAnimationFrameResizeObserver", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("coalesces resize notifications into one animation frame", () => {
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 1;
    const observers: FakeResizeObserver[] = [];

    class FakeResizeObserver {
      public observedTargets: Element[] = [];
      public disconnected = false;

      constructor(public readonly callback: ResizeObserverCallback) {
        observers.push(this);
      }

      observe(target: Element) {
        this.observedTargets.push(target);
      }

      unobserve(target: Element) {
        this.observedTargets = this.observedTargets.filter((item) => item !== target);
      }

      disconnect() {
        this.disconnected = true;
      }

      emit(entries: ResizeObserverEntry[]) {
        this.callback(entries, this as unknown as ResizeObserver);
      }
    }

    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("window", {
      requestAnimationFrame: vi.fn((callback: FrameRequestCallback) => {
        const id = nextFrameId++;
        frames.set(id, callback);
        return id;
      }),
      cancelAnimationFrame: vi.fn((id: number) => {
        frames.delete(id);
      }),
    });

    const callback = vi.fn();
    const handle = createAnimationFrameResizeObserver(callback);
    const target = {} as Element;
    const firstEntry = { target, contentRect: { width: 100 } } as unknown as ResizeObserverEntry;
    const secondEntry = { target, contentRect: { width: 120 } } as unknown as ResizeObserverEntry;

    handle?.observe(target);
    observers[0].emit([firstEntry]);
    observers[0].emit([secondEntry]);

    expect(callback).not.toHaveBeenCalled();
    expect(frames).toHaveLength(1);

    Array.from(frames.values())[0]?.(16);

    expect(callback).toHaveBeenCalledTimes(1);
    expect(callback.mock.calls[0]?.[0]).toEqual([secondEntry]);

    handle?.disconnect();
    expect(observers[0].disconnected).toBe(true);
  });

  it("cancels pending callbacks on disconnect", () => {
    const frames = new Map<number, FrameRequestCallback>();
    const observers: FakeResizeObserver[] = [];

    class FakeResizeObserver {
      constructor(public readonly callback: ResizeObserverCallback) {
        observers.push(this);
      }

      observe(_target: Element) {}
      unobserve(_target: Element) {}
      disconnect() {}
      emit(entries: ResizeObserverEntry[]) {
        this.callback(entries, this as unknown as ResizeObserver);
      }
    }

    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("window", {
      requestAnimationFrame: vi.fn((callback: FrameRequestCallback) => {
        frames.set(1, callback);
        return 1;
      }),
      cancelAnimationFrame: vi.fn((id: number) => {
        frames.delete(id);
      }),
    });

    const callback = vi.fn();
    const target = {} as Element;
    const handle = createAnimationFrameResizeObserver(callback);

    observers[0].emit([{ target } as ResizeObserverEntry]);
    handle?.disconnect();

    expect(frames).toHaveLength(0);
    expect(callback).not.toHaveBeenCalled();
  });
});
