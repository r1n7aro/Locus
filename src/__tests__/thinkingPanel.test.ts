// @vitest-environment jsdom
import { createApp, h, nextTick, shallowReactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ThinkingPanel from "../components/ThinkingPanel.vue";
import { STREAMING_RENDER_THROTTLE_MS } from "../composables/streamingRenderThrottle";
import { StreamingTextChunks, type StreamingTextSource } from "../composables/streamingTextChunks";

vi.mock("../i18n", () => ({ t: (key: string) => key }));

let app: App | null = null;
let root: HTMLElement;
let nextFrameId: number;
const frames = new Map<number, FrameRequestCallback>();

function mount(props: { stream?: StreamingTextSource | null; text?: string; isThinking: boolean }) {
  const state = shallowReactive(props);
  const errors: unknown[] = [];
  app = createApp({ setup: () => () => h(ThinkingPanel, state) });
  app.config.errorHandler = (error) => errors.push(error);
  app.mount(root);
  expect(errors).toEqual([]);
  return state;
}

function flushFrame() {
  const pending = [...frames.values()];
  frames.clear();
  pending.forEach((callback) => callback(16));
}

beforeEach(() => {
  vi.useFakeTimers();
  root = document.createElement("div");
  document.body.appendChild(root);
  nextFrameId = 1;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextFrameId++;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
});

afterEach(() => {
  app?.unmount();
  app = null;
  root.remove();
  frames.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("ThinkingPanel runtime", () => {
  it.each(["empty", "history", "stream"] as const)("mounts %s content and scrolls on the first frame", (mode) => {
    const stream = new StreamingTextChunks();
    const liveText = "a".repeat(4096) + "tail";
    if (mode !== "empty") {
      stream.append("a".repeat(4096));
      stream.append("tail");
    }
    mount({ stream, text: mode === "history" ? "Saved thinking" : "", isThinking: mode !== "history" });

    if (mode === "empty") {
      expect(root.querySelector(".empty-hint")?.textContent).toBe("thinking.panel.empty");
    } else {
      expect(root.querySelector(".thinking-text")?.textContent).toBe(mode === "history" ? "Saved thinking" : liveText);
    }
    const content = root.querySelector<HTMLElement>(".thinking-content")!;
    Object.defineProperty(content, "scrollHeight", { value: 600 });
    expect(content.scrollTop).toBe(0);
    expect(frames.size).toBe(1);
    flushFrame();
    expect(content.scrollTop).toBe(600);
  });

  it("throttles live appends and schedules another scroll after rendering", async () => {
    const stream = new StreamingTextChunks();
    stream.append("First");
    mount({ stream, isThinking: true });
    flushFrame();

    stream.append(" second");
    await nextTick();
    stream.append(" third");
    await nextTick();
    expect(root.querySelector(".thinking-text")?.textContent).toBe("First");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(STREAMING_RENDER_THROTTLE_MS - 1);
    await nextTick();
    expect(root.querySelector(".thinking-text")?.textContent).toBe("First");
    vi.advanceTimersByTime(1);
    await nextTick();
    expect(root.querySelector(".thinking-text")?.textContent).toBe("First second third");

    const content = root.querySelector<HTMLElement>(".thinking-content")!;
    Object.defineProperty(content, "scrollHeight", { value: 800 });
    expect(frames.size).toBe(1);
    flushFrame();
    expect(content.scrollTop).toBe(800);
  });

  it("switches immediately between history and replacement streams", async () => {
    const stream = new StreamingTextChunks();
    stream.append("Live thinking");
    const props = mount({ stream, text: "", isThinking: true });
    stream.append(" pending");
    await nextTick();
    expect(vi.getTimerCount()).toBe(1);

    props.text = "Saved thinking";
    await nextTick();
    expect(root.querySelector(".thinking-text")?.textContent).toBe("Saved thinking");
    expect(vi.getTimerCount()).toBe(0);

    const replacement = new StreamingTextChunks();
    replacement.append("Replacement thinking");
    props.stream = replacement;
    props.text = "";
    await nextTick();
    expect(root.querySelector(".thinking-text")?.textContent).toBe("Replacement thinking");
    expect(vi.getTimerCount()).toBe(0);
    expect(frames.size).toBe(1);
  });

  it("cancels pending stream and scroll work when unmounted", async () => {
    const stream = new StreamingTextChunks();
    mount({ stream, isThinking: true });
    stream.append("Pending thinking");
    await nextTick();
    expect(vi.getTimerCount()).toBe(1);
    expect(frames.size).toBe(1);

    app!.unmount();
    app = null;
    expect(vi.getTimerCount()).toBe(0);
    expect(frames.size).toBe(0);
    stream.append(" after unmount");
    await nextTick();
    expect(vi.getTimerCount()).toBe(0);
    expect(frames.size).toBe(0);
  });
});
