import { describe, expect, it, vi } from "vitest";
import {
  collectPendingContinuationToolItemIds,
  collectPendingContinuationToolSegmentItemIds,
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  findTrailingAssistantToolMessageId,
  hasRunningToolCall,
  shouldAutoScrollToBottom,
  shouldShowAssistantContinuation,
  shouldShowWaitingPlaceholder,
} from "../composables/chatViewStability";

type TestFrameCallback = (time: number) => void;

describe("chatViewStability", () => {
  it("shows assistant continuation only while a transient assistant block exists", () => {
    expect(shouldShowAssistantContinuation("assistant", true)).toBe(true);
    expect(shouldShowAssistantContinuation("assistant", false)).toBe(false);
    expect(shouldShowAssistantContinuation("user", true)).toBe(false);
    expect(shouldShowAssistantContinuation(null, true)).toBe(false);
  });

  it("keeps trailing tool-only assistant rounds expanded while the run is still streaming", () => {
    const pendingIds = collectPendingContinuationToolItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      items: [
        { id: "m1", content: "已读取项目结构。", toolCallCount: 2 },
        { id: "m2", content: "", toolCallCount: 3 },
        { id: "m3", content: "", toolCallCount: 2 },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m3", "m2"]);
  });

  it("keeps the tool segment after the latest body expanded until another body arrives", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m1"] },
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m2"]);
  });

  it("keeps trailing tool segments expanded across non-body assistant parts", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
        { type: "other" },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m2"]);
  });

  it("stops pinning tool segments once a later body exists", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
        { type: "content" },
      ],
    });

    expect(Array.from(pendingIds)).toEqual([]);
  });

  it("stops pinning historical tool rounds once the final response arrives", () => {
    const pendingIds = collectPendingContinuationToolItemIds({
      isStreaming: false,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: false,
      items: [
        { id: "m1", content: "", toolCallCount: 3 },
        { id: "m2", content: "", toolCallCount: 2 },
      ],
    });

    expect(Array.from(pendingIds)).toEqual([]);
  });

  it("pins only the latest visible assistant message when it contains tool calls", () => {
    expect(findTrailingAssistantToolMessageId([
      {
        id: "a1",
        role: "assistant",
        toolCalls: [{ id: "tc-1", name: "read_file", arguments: "{\"path\":\"Assets/Player.cs\"}" }],
      },
      {
        id: "tool-1",
        role: "tool",
        content: "ok",
      } as any,
    ])).toBe("a1");

    expect(findTrailingAssistantToolMessageId([
      {
        id: "a1",
        role: "assistant",
        toolCalls: [{ id: "tc-1", name: "read_file", arguments: "{\"path\":\"Assets/Player.cs\"}" }],
      },
      {
        id: "u1",
        role: "user",
        content: "继续",
      } as any,
    ])).toBeNull();

    expect(findTrailingAssistantToolMessageId([
      {
        id: "a2",
        role: "assistant",
        content: "这里是最终答复",
      } as any,
    ])).toBeNull();
  });

  it("shows the waiting placeholder whenever the run is idle between stream events", () => {
    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: false,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(true);

    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: false,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(true);

    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: true,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(false);
  });

  it("treats completed tool calls as idle while nested running tools stay active", () => {
    expect(hasRunningToolCall([
      { status: "done" },
      { status: "error" },
      { status: "interrupted" },
    ])).toBe(false);

    expect(hasRunningToolCall([
      {
        status: "done",
        nestedToolCalls: [
          { id: "nested-1", name: "read", arguments: "{}", status: "running" },
        ],
      },
    ])).toBe(true);
  });

  it("skips auto-scroll when the user is reviewing older content", () => {
    expect(shouldAutoScrollToBottom({
      force: false,
      remembered: { mode: "offset", scrollTop: 240 },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(false);

    expect(shouldAutoScrollToBottom({
      force: false,
      remembered: {
        mode: "anchor",
        anchorId: "m2",
        offsetTop: 20,
        fallbackScrollTop: 240,
      },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(false);

    expect(shouldAutoScrollToBottom({
      force: true,
      remembered: { mode: "offset", scrollTop: 240 },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(true);
  });

  it("coalesces repeated scroll requests into one frame and preserves force", () => {
    const calls: boolean[] = [];
    let scheduledFrame: TestFrameCallback | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 1;
    });
    const cancelFrame = vi.fn();

    const scheduler = createCoalescedScrollScheduler(
      (force) => calls.push(force),
      requestFrame,
      cancelFrame,
    );

    scheduler.schedule(false);
    scheduler.schedule(false);
    scheduler.schedule(true);

    expect(requestFrame).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([]);

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }

    expect(calls).toEqual([true]);
  });

  it("cancels pending scroll work cleanly", () => {
    let scheduledFrame: TestFrameCallback | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 7;
    });
    const cancelFrame = vi.fn();
    const run = vi.fn();

    const scheduler = createCoalescedScrollScheduler(run, requestFrame, cancelFrame);

    scheduler.schedule();
    scheduler.cancel();
    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }

    expect(cancelFrame).toHaveBeenCalledWith(7);
    expect(run).not.toHaveBeenCalled();
  });

  it("retries scroll work after the next frame and after layout settles", () => {
    const calls: number[] = [];
    let scheduledFrame: TestFrameCallback | null = null;
    let scheduledTimeout: (() => void) | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 3;
    });
    const cancelFrame = vi.fn();
    const requestTimeout = vi.fn((cb: () => void, delay: number) => {
      scheduledTimeout = cb;
      expect(delay).toBe(320);
      return 9;
    });
    const cancelTimeout = vi.fn();

    const scheduler = createSettledScrollScheduler(
      () => calls.push(calls.length),
      320,
      requestFrame,
      cancelFrame,
      requestTimeout,
      cancelTimeout,
    );

    scheduler.schedule();

    expect(calls).toEqual([0]);
    expect(requestFrame).toHaveBeenCalledTimes(1);
    expect(requestTimeout).toHaveBeenCalledTimes(1);

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }
    expect(calls).toEqual([0, 1]);

    const flushTimeout = scheduledTimeout as (() => void) | null;
    if (flushTimeout) {
      flushTimeout();
    }
    expect(calls).toEqual([0, 1, 2]);
  });

  it("cancels pending settled scroll retries", () => {
    const run = vi.fn();
    let scheduledFrame: TestFrameCallback | null = null;
    let scheduledTimeout: (() => void) | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 11;
    });
    const cancelFrame = vi.fn();
    const requestTimeout = vi.fn((cb: () => void) => {
      scheduledTimeout = cb;
      return 13;
    });
    const cancelTimeout = vi.fn();

    const scheduler = createSettledScrollScheduler(
      run,
      320,
      requestFrame,
      cancelFrame,
      requestTimeout,
      cancelTimeout,
    );

    scheduler.schedule();
    scheduler.cancel();

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }
    const flushTimeout = scheduledTimeout as (() => void) | null;
    if (flushTimeout) {
      flushTimeout();
    }

    expect(run).toHaveBeenCalledTimes(1);
    expect(cancelFrame).toHaveBeenCalledWith(11);
    expect(cancelTimeout).toHaveBeenCalledWith(13);
  });
});
