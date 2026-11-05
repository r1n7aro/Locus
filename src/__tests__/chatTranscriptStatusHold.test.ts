// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ChatTranscript from "../components/chat/ChatTranscript.vue";
import {
  resolveTransientStatus,
  transientShrinkHoldHeight,
} from "../components/chat/transientStatusHold";
import { StreamingTextChunks } from "../composables/streamingTextChunks";
import type { AssistantRenderPart, ChatMessage, ClaudeCodePhase, ToolCallDisplay } from "../types";

describe("resolveTransientStatus", () => {
  const units = (...values: string[]) => new Set(values);

  it("keeps the last status until a new output unit renders", () => {
    const shown = resolveTransientStatus({ current: "thinking", units: units("tool:a"), streaming: true, memory: null });
    expect(shown).toMatchObject({ status: "thinking", held: false });

    const held = resolveTransientStatus({ current: null, units: units("tool:a"), streaming: true, memory: shown.memory });
    expect(held).toMatchObject({ status: "thinking", held: true });

    // Output that only moved out of the live region is not new output.
    const moved = resolveTransientStatus({ current: null, units: units(), streaming: true, memory: held.memory });
    expect(moved).toMatchObject({ status: "thinking", held: true });

    const released = resolveTransientStatus({ current: null, units: units("tool:a", "tool:b"), streaming: true, memory: moved.memory });
    expect(released).toEqual({ status: null, held: false, memory: null });
  });

  it("does not retain a status of a finished discrete operation", () => {
    const compacting = resolveTransientStatus({ current: "compacting", currentRetainable: false, units: units(), streaming: true, memory: null });
    expect(compacting).toEqual({ status: "compacting", held: false, memory: null });
    expect(resolveTransientStatus({ current: null, units: units(), streaming: true, memory: compacting.memory }))
      .toEqual({ status: null, held: false, memory: null });
  });

  it("never retains a status after the stream ends", () => {
    const shown = resolveTransientStatus({ current: "waiting", units: units(), streaming: true, memory: null });
    expect(resolveTransientStatus({ current: null, units: units(), streaming: false, memory: shown.memory }))
      .toEqual({ status: null, held: false, memory: null });
  });
});

describe("transientShrinkHoldHeight", () => {
  it("holds only the shrink caused by the removed status", () => {
    expect(transientShrinkHoldHeight({ heightBefore: 100, heightAfter: 90, statusFootprint: 38 })).toBe(100);
    expect(transientShrinkHoldHeight({ heightBefore: 300, heightAfter: 100, statusFootprint: 38 })).toBe(138);
    expect(transientShrinkHoldHeight({ heightBefore: 100, heightAfter: 120, statusFootprint: 38 })).toBeNull();
    expect(transientShrinkHoldHeight({ heightBefore: 100, heightAfter: 90, statusFootprint: 0 })).toBeNull();
  });
});

const apps: App[] = [];

afterEach(() => {
  while (apps.length) apps.pop()!.unmount();
  document.body.innerHTML = "";
});

function mountTranscript(initial: Record<string, unknown>) {
  const props = reactive({
    messages: [{ id: "user", role: "user", content: "Run it", createdAt: 0 }] as ChatMessage[],
    streamingText: "",
    isStreaming: true,
    isThinking: false,
    activeToolCalls: [] as ToolCallDisplay[],
    liveRenderParts: [] as AssistantRenderPart[],
    livePartStreams: null as ReadonlyMap<string, StreamingTextChunks> | null,
    variant: "session" as const,
    sessionKey: "status-hold",
    claudeCodeMode: false,
    claudeCodePhase: null as ClaudeCodePhase | null,
    ...initial,
  });
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ render: () => h(ChatTranscript, props) });
  app.use(createPinia());
  app.mount(host);
  apps.push(app);
  return { props, host };
}

async function flush() {
  for (let index = 0; index < 3; index += 1) await nextTick();
}

describe("ChatTranscript live status rows", () => {
  it("keeps the Claude Code status until the next tool block renders", async () => {
    const { props, host } = mountTranscript({ claudeCodeMode: true, claudeCodePhase: "thinking" });
    await flush();
    const status = () => host.querySelector('[data-render-part-key="transient:claude-code"]');
    const statusRow = status();
    expect(statusRow?.textContent).toContain("Thinking");

    // The phase moves to tool input before any tool block exists.
    props.claudeCodePhase = "tool-input";
    await flush();
    expect(status()).toBe(statusRow);
    expect(status()?.textContent).toContain("Thinking");

    props.claudeCodePhase = "tool-use";
    props.activeToolCalls = [{ id: "tool-1", name: "bash", arguments: "{}", status: "running" }];
    await flush();
    expect(status()).toBeNull();
    expect(host.querySelector(".tool-call-block")).not.toBeNull();

    props.claudeCodePhase = "requesting";
    props.activeToolCalls = [{ id: "tool-1", name: "bash", arguments: "{}", status: "done", output: "ok" }];
    await flush();
    expect(status()).not.toBeNull();

    props.claudeCodePhase = "responding";
    await flush();
    expect(status()).not.toBeNull();
    props.isStreaming = false;
    await flush();
    expect(status()).toBeNull();
  });

  it("keeps the waiting row until streamed text becomes visible", async () => {
    const stream = new StreamingTextChunks();
    const { props, host } = mountTranscript({});
    await flush();
    const waiting = () => host.querySelector('[data-render-part-key="transient:waiting"]');
    expect(waiting()).not.toBeNull();

    props.livePartStreams = new Map([["answer", stream]]);
    props.liveRenderParts = [{ kind: "text", id: "answer", order: { runId: "run", seq: 1 }, content: "" }];
    await new Promise((resolve) => setTimeout(resolve, 120));
    await flush();
    expect(host.querySelector('[data-render-part-key="transient:answer"]')).not.toBeNull();
    expect(waiting()).not.toBeNull();

    stream.append("   ");
    await flush();
    expect(waiting()).not.toBeNull();

    stream.append("Hello");
    await flush();
    expect(waiting()).toBeNull();
  });

  it("holds the stack height when a status row is replaced by shorter output", async () => {
    // jsdom has no layout: a status row is 38px and each tool row 28px.
    const rectHeight = (element: Element): number => {
      if (element.matches('[data-transient-status="true"]')) return 38;
      if (element.matches(".chat-transcript-tool-calls-group")) return 28 * element.querySelectorAll(".tool-call-block").length;
      if (element.matches(".chat-transcript-item-stack")) {
        return Array.from(element.children).reduce((sum, child) => sum + rectHeight(child), 0);
      }
      return 0;
    };
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      const height = rectHeight(this);
      return { x: 0, y: 0, top: 0, left: 0, width: 100, right: 100, height, bottom: height, toJSON: () => ({}) } as DOMRect;
    });
    const observers: Array<{ callback: ResizeObserverCallback; disconnected: boolean }> = [];
    vi.stubGlobal("ResizeObserver", class {
      record: { callback: ResizeObserverCallback; disconnected: boolean };
      constructor(callback: ResizeObserverCallback) {
        this.record = { callback, disconnected: false };
        observers.push(this.record);
      }
      observe() {}
      unobserve() {}
      disconnect() { this.record.disconnected = true; }
    });
    try {
      const done = (id: string): ToolCallDisplay => ({ id, name: "bash", arguments: "{}", status: "done", output: "ok" });
      const { props, host } = mountTranscript({
        claudeCodeMode: true,
        claudeCodePhase: "requesting",
        activeToolCalls: [done("tool-1")],
      });
      await flush();
      const content = () => host.querySelector<HTMLElement>(".chat-transcript-message.transient .chat-transcript-message-content")!;
      expect(rectHeight(host.querySelector(".chat-transcript-message.transient .chat-transcript-item-stack")!)).toBe(66);

      // 28px tool row replaces the 38px status row: 10px would be lost.
      props.claudeCodePhase = "tool-use";
      props.activeToolCalls = [done("tool-1"), { id: "tool-2", name: "bash", arguments: "{}", status: "running" }];
      await flush();
      expect(host.querySelector('[data-transient-status="true"]')).toBeNull();
      expect(content().style.minHeight).toBe("66px");

      // The next status grows the stack past the hold, which releases it.
      props.claudeCodePhase = "requesting";
      props.activeToolCalls = [done("tool-1"), done("tool-2")];
      await flush();
      const live = observers.filter((observer) => !observer.disconnected);
      live.forEach((observer) => observer.callback([], {} as ResizeObserver));
      expect(content().style.minHeight).toBe("");

      // A hold still in place is dropped when the stream ends.
      props.claudeCodePhase = "tool-use";
      props.activeToolCalls = [done("tool-1"), done("tool-2"), { id: "tool-3", name: "bash", arguments: "{}", status: "running" }];
      await flush();
      expect(content().style.minHeight).toBe("94px");
      props.isStreaming = false;
      await flush();
      expect(host.querySelector<HTMLElement>(".chat-transcript-message-content[style*='min-height']")).toBeNull();
    } finally {
      rectSpy.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("drops a retained status when the session changes", async () => {
    const { props, host } = mountTranscript({ claudeCodeMode: true, claudeCodePhase: "requesting" });
    await flush();
    props.claudeCodePhase = "tool-input";
    await flush();
    expect(host.querySelector('[data-render-part-key="transient:claude-code"]')).not.toBeNull();

    props.sessionKey = "other-session";
    await flush();
    expect(host.querySelector('[data-render-part-key="transient:claude-code"]')).toBeNull();
  });
});
