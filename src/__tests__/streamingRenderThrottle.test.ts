import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, ref } from "vue";
import type { AssistantRenderPart } from "../types";
import {
  STREAMING_RENDER_THROTTLE_MS,
  useThrottledLiveRenderParts,
  useThrottledStreamingText,
} from "../composables/streamingRenderThrottle";

function textPart(id: string, content: string): AssistantRenderPart {
  return { kind: "text", id, order: { runId: "run", seq: 1 }, content };
}

function thinkingPart(id: string, content: string, active = true): AssistantRenderPart {
  return { kind: "thinking", id, order: { runId: "run", seq: 1 }, content, active };
}

describe("useThrottledStreamingText", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("coalesces append-only growth to one trailing update", async () => {
    const source = ref("");
    const scope = effectScope();
    const throttled = scope.run(() => useThrottledStreamingText(() => source.value));
    if (!throttled) throw new Error("scope did not initialize");

    source.value = "hel";
    await nextTick();
    expect(throttled.text.value).toBe("");

    source.value = "hello";
    await nextTick();
    expect(throttled.text.value).toBe("");

    vi.advanceTimersByTime(STREAMING_RENDER_THROTTLE_MS - 1);
    expect(throttled.text.value).toBe("");

    vi.advanceTimersByTime(1);
    expect(throttled.text.value).toBe("hello");

    scope.stop();
  });

  it("applies resets and shrinking replacements immediately", async () => {
    const source = ref("streamed output");
    const scope = effectScope();
    const throttled = scope.run(() => useThrottledStreamingText(() => source.value));
    if (!throttled) throw new Error("scope did not initialize");

    expect(throttled.text.value).toBe("streamed output");

    source.value = "short";
    await nextTick();
    expect(throttled.text.value).toBe("short");

    source.value = "";
    await nextTick();
    expect(throttled.text.value).toBe("");

    scope.stop();
  });

  it("flush() applies the latest source value even before the watcher ran", () => {
    const source = ref("a");
    const scope = effectScope();
    const throttled = scope.run(() => useThrottledStreamingText(() => source.value));
    if (!throttled) throw new Error("scope did not initialize");

    source.value = "abc";
    throttled.flush();
    expect(throttled.text.value).toBe("abc");

    scope.stop();
  });

  it("stops pending updates when the scope is disposed", async () => {
    const source = ref("");
    const scope = effectScope();
    const throttled = scope.run(() => useThrottledStreamingText(() => source.value));
    if (!throttled) throw new Error("scope did not initialize");

    source.value = "pending";
    await nextTick();
    scope.stop();

    vi.advanceTimersByTime(STREAMING_RENDER_THROTTLE_MS * 2);
    expect(throttled.text.value).toBe("");
  });
});

describe("useThrottledLiveRenderParts", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("throttles pure content growth on existing parts", async () => {
    const source = ref<AssistantRenderPart[]>([thinkingPart("think", "a"), textPart("text", "h")]);
    const scope = effectScope();
    const displayed = scope.run(() => useThrottledLiveRenderParts(() => source.value));
    if (!displayed) throw new Error("scope did not initialize");

    source.value = [thinkingPart("think", "a"), textPart("text", "hello")];
    await nextTick();
    expect(displayed.value[1]).toMatchObject({ content: "h" });

    vi.advanceTimersByTime(STREAMING_RENDER_THROTTLE_MS);
    expect(displayed.value[1]).toMatchObject({ content: "hello" });

    scope.stop();
  });

  it("flushes immediately when parts are added", async () => {
    const source = ref<AssistantRenderPart[]>([thinkingPart("think", "a")]);
    const scope = effectScope();
    const displayed = scope.run(() => useThrottledLiveRenderParts(() => source.value));
    if (!displayed) throw new Error("scope did not initialize");

    source.value = [thinkingPart("think", "a", false), textPart("text", "h")];
    await nextTick();
    expect(displayed.value).toHaveLength(2);
    expect(displayed.value[0]).toMatchObject({ active: false });

    scope.stop();
  });

  it("flushes immediately when parts are cleared", async () => {
    const source = ref<AssistantRenderPart[]>([textPart("text", "hello")]);
    const scope = effectScope();
    const displayed = scope.run(() => useThrottledLiveRenderParts(() => source.value));
    if (!displayed) throw new Error("scope did not initialize");

    source.value = [];
    await nextTick();
    expect(displayed.value).toHaveLength(0);

    scope.stop();
  });

  it("flushes immediately when content is replaced with something shorter", async () => {
    const source = ref<AssistantRenderPart[]>([textPart("text", "long streamed text")]);
    const scope = effectScope();
    const displayed = scope.run(() => useThrottledLiveRenderParts(() => source.value));
    if (!displayed) throw new Error("scope did not initialize");

    source.value = [textPart("text", "short")];
    await nextTick();
    expect(displayed.value[0]).toMatchObject({ content: "short" });

    scope.stop();
  });

  it("flushes immediately when a part identity changes", async () => {
    const source = ref<AssistantRenderPart[]>([textPart("text-a", "hello")]);
    const scope = effectScope();
    const displayed = scope.run(() => useThrottledLiveRenderParts(() => source.value));
    if (!displayed) throw new Error("scope did not initialize");

    source.value = [textPart("text-b", "hello")];
    await nextTick();
    expect(displayed.value[0]).toMatchObject({ id: "text-b" });

    scope.stop();
  });
});
