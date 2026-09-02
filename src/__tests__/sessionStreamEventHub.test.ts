import { describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../types";
import {
  bindSessionAsyncTaskUpdateConsumer,
  bindSessionStreamEventConsumer,
  publishSessionAsyncTaskUpdate,
  publishSessionExecutionState,
  publishSessionStreamEvent,
  sessionStreamSourceMatchesWorkspace,
  subscribeSessionAsyncTaskUpdateConsumer,
  subscribeSessionExecutionStateConsumer,
  subscribeSessionStreamEventConsumer,
  subscribeSessionStreamEvents,
  workspaceStreamEventSource,
} from "../services/sessionStreamEventHub";

describe("session stream event hub", () => {
  it("fans out each normalized event and removes subscribers synchronously", () => {
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = subscribeSessionStreamEvents(first);
    const unsubscribeSecond = subscribeSessionStreamEvents(second);
    const event: StreamEvent = {
      type: "runStart",
      sessionId: "session-1",
      runId: "run-1",
    };

    publishSessionStreamEvent({ event, source: { kind: "legacy" } });
    unsubscribeFirst();
    publishSessionStreamEvent({ event, source: { kind: "legacy" } });
    unsubscribeSecond();

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(2);
  });

  it("preserves workspace scope for pane-level routing", () => {
    const source = workspaceStreamEventSource({
      eventName: "stream-event",
      streamRevision: 12,
      projectId: "project-a",
      checkoutId: "checkout-a",
      workspaceGeneration: 4,
      payload: {
        type: "runStart",
        sessionId: "session-1",
        runId: "run-1",
      },
    });

    expect(sessionStreamSourceMatchesWorkspace(source, {
      checkoutId: "checkout-a",
      expectedGeneration: 4,
    })).toBe(true);
    expect(sessionStreamSourceMatchesWorkspace(source, {
      checkoutId: "checkout-a",
      expectedGeneration: 5,
    })).toBe(false);
    expect(sessionStreamSourceMatchesWorkspace(source, {
      checkoutId: "checkout-b",
      expectedGeneration: 4,
    })).toBe(false);
    expect(sessionStreamSourceMatchesWorkspace({ kind: "legacy" }, null)).toBe(true);
  });

  it("buffers events until an exact session consumer is bound", () => {
    const sessionId = "buffered-session";
    const runId = "buffered-run";
    const runStart: StreamEvent = { type: "runStart", sessionId, runId };
    const textDelta: StreamEvent = {
      type: "textDelta",
      sessionId,
      runId,
      text: "hello",
    };
    publishSessionStreamEvent({ event: runStart, source: { kind: "legacy" } });
    publishSessionStreamEvent({ event: textDelta, source: { kind: "legacy" } });

    expect(bindSessionStreamEventConsumer(sessionId).map(({ event }) => event)).toEqual([
      runStart,
      textDelta,
    ]);
    expect(bindSessionStreamEventConsumer(sessionId)).toEqual([]);
  });

  it("delivers one reduction when several panes resolve to the same state", () => {
    const sharedState = {};
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = subscribeSessionStreamEventConsumer(() => sharedState, first);
    const unsubscribeSecond = subscribeSessionStreamEventConsumer(() => sharedState, second);

    publishSessionStreamEvent({
      event: {
        type: "runStart",
        sessionId: "deduplicated-session",
        runId: "deduplicated-run",
      },
      source: { kind: "legacy" },
    });

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).not.toHaveBeenCalled();
    unsubscribeFirst();
    unsubscribeSecond();
  });

  it("buffers events received after a bound consumer unmounts", () => {
    const sessionId = "bound-remount-gap-session";
    const state = {};
    const listener = vi.fn();
    const unsubscribe = subscribeSessionStreamEventConsumer(
      (dispatch) => dispatch.event.sessionId === sessionId ? state : null,
      listener,
    );
    bindSessionStreamEventConsumer(sessionId);
    publishSessionStreamEvent({
      event: { type: "runStart", sessionId, runId: "bound-remount-gap-run" },
      source: { kind: "legacy" },
    });
    unsubscribe();

    const terminal: StreamEvent = {
      type: "done",
      sessionId,
      runId: "bound-remount-gap-run",
      messageId: "bound-remount-gap-assistant",
      fullText: "finished while unmounted",
    };
    publishSessionStreamEvent({ event: terminal, source: { kind: "legacy" } });

    expect(listener).toHaveBeenCalledTimes(1);
    expect(bindSessionStreamEventConsumer(sessionId).map(({ event }) => event)).toEqual([
      terminal,
    ]);
  });

  it("delivers one async task update to a shared reducer mounted in multiple panes", () => {
    const sharedState = {};
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = subscribeSessionAsyncTaskUpdateConsumer(() => sharedState, first);
    const unsubscribeSecond = subscribeSessionAsyncTaskUpdateConsumer(() => sharedState, second);
    const update = {
      sessionId: "async-shared-session",
      assistantMessageId: "assistant-1",
      toolCallId: "tool-1",
      taskId: "task-1",
      toolName: "bash",
      status: "running" as const,
      output: "line 1",
    };

    publishSessionAsyncTaskUpdate(update);

    expect(first).toHaveBeenCalledOnce();
    expect(first).toHaveBeenCalledWith(update);
    expect(second).not.toHaveBeenCalled();
    unsubscribeFirst();
    unsubscribeSecond();
  });

  it("replays async task updates received before a durable reducer binds", () => {
    const update = {
      sessionId: "async-pending-session",
      assistantMessageId: "assistant-2",
      toolCallId: "tool-2",
      taskId: "task-2",
      toolName: "bash",
      status: "completed" as const,
      output: "done",
    };

    publishSessionAsyncTaskUpdate(update);

    expect(bindSessionAsyncTaskUpdateConsumer(update.sessionId)).toEqual([update]);
    expect(bindSessionAsyncTaskUpdateConsumer(update.sessionId)).toEqual([]);
  });

  it("deduplicates execution-state updates for a session shared by multiple panes", () => {
    const sharedState = {};
    const first = vi.fn();
    const second = vi.fn();
    const resolve = (update: { sessionId: string }) => (
      update.sessionId === "execution-session" ? sharedState : null
    );
    const unsubscribeFirst = subscribeSessionExecutionStateConsumer(resolve, first);
    const unsubscribeSecond = subscribeSessionExecutionStateConsumer(resolve, second);
    const update = {
      sessionId: "execution-session",
      modelId: "gpt-scoped",
      effort: "high" as const,
      fastMode: true,
    };

    publishSessionExecutionState(update);

    expect(first).toHaveBeenCalledWith(update);
    expect(second).not.toHaveBeenCalled();
    unsubscribeFirst();
    unsubscribeSecond();
  });

});
