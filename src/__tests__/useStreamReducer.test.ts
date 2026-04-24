import { describe, it, expect } from "vitest";
import { buildToolResultMessages, reduceStreamEvent, type StreamState } from "../composables/useStreamReducer";
import type { StreamEvent, ToolCallDisplay } from "../types";

function makeState(overrides?: Partial<StreamState>): StreamState {
  return {
    messages: [],
    streamingText: "",
    rawStreamText: "",
    streamingThinking: "",
    isStreaming: false,
    isThinking: false,
    thinkingStartTime: 0,
    thinkingDuration: 0,
    activeToolCalls: [],
    tokenUsage: {
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCacheReadTokens: 0,
      totalCacheWriteTokens: 0,
      totalCostUsd: 0,
      pricedRounds: 0,
      contextTokens: 0,
      contextLimit: 0,
    },
    todos: [],
    showTodoPanel: false,
    pendingQuestion: null,
    pendingToolConfirms: [],
    undoableMessageIds: new Set(),
    ...overrides,
  };
}

describe("reduceStreamEvent", () => {
  describe("buildToolResultMessages", () => {
    it("materializes completed tool outputs as hidden tool result messages", () => {
      expect(buildToolResultMessages([
        {
          id: "tc-1",
          name: "read",
          arguments: "{}",
          status: "done",
          output: "file contents",
        },
        {
          id: "tc-2",
          name: "grep",
          arguments: "{}",
          status: "running",
        },
      ], 123)).toEqual([
        {
          id: "tool_result_tc-1",
          role: "tool",
          content: "file contents",
          createdAt: 123,
          toolCallId: "tc-1",
        },
      ]);
    });
  });

  describe("textDelta", () => {
    it("appends text without auto-activating streaming (streaming controlled by chat store)", () => {
      const state = makeState();
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "hello" };
      const mutations = reduceStreamEvent(state, event);

      // Auto-activation removed — streaming now controlled by runStart/cancel
      expect(mutations.filter((m) => m.type === "setStreaming")).toHaveLength(0);
      expect(mutations).toContainEqual({ type: "appendRawText", text: "hello" });
      expect(mutations).toContainEqual({ type: "setThinking", value: false });
    });

    it("does not set streaming state on text delta", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "world" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.filter((m) => m.type === "setStreaming")).toHaveLength(0);
    });

    it("updates thinking duration if thinking was active", () => {
      const state = makeState({ isStreaming: true, isThinking: true, thinkingStartTime: Date.now() - 5000 });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "x" };
      const mutations = reduceStreamEvent(state, event);

      const durationMut = mutations.find((m) => m.type === "updateThinkingDuration");
      expect(durationMut).toBeDefined();
      if (durationMut?.type === "updateThinkingDuration") {
        expect(durationMut.duration).toBeGreaterThanOrEqual(4);
        expect(durationMut.duration).toBeLessThanOrEqual(6);
      }
    });
  });

  describe("thinkingDelta", () => {
    it("appends thinking and starts thinking mode", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "thinking..." };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendThinking", text: "thinking..." });
      const setThinking = mutations.find((m) => m.type === "setThinking");
      expect(setThinking).toBeDefined();
      if (setThinking?.type === "setThinking") {
        expect(setThinking.value).toBe(true);
        expect(setThinking.startTime).toBeGreaterThan(0);
      }
    });

    it("does not reset thinking start if already thinking", () => {
      const state = makeState({ isStreaming: true, isThinking: true, thinkingStartTime: 1000 });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "more" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendThinking", text: "more" });
      expect(mutations.find((m) => m.type === "setThinking")).toBeUndefined();
    });
  });

  describe("toolCallStart", () => {
    it("adds a new tool call", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"foo.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);

      const addMut = mutations.find((m) => m.type === "addToolCall");
      expect(addMut).toBeDefined();
      if (addMut?.type === "addToolCall") {
        expect(addMut.toolCall.id).toBe("tc1");
        expect(addMut.toolCall.name).toBe("read");
        expect(addMut.toolCall.status).toBe("running");
      }
    });

    it("updates existing tool call arguments", () => {
      const existingTc: ToolCallDisplay = { id: "tc1", name: "read", arguments: "{}", status: "running" };
      const state = makeState({ isStreaming: true, activeToolCalls: [existingTc] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"bar.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "addToolCall")).toBeUndefined();
      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { arguments: '{"path":"bar.ts"}' },
      });
    });
  });

  describe("toolCallDone", () => {
    it("marks tool call as done", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "file contents",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "done", output: "file contents" },
      });
    });

    it("marks tool call as error when outcome is error", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "not found",
        outcome: "error",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "error", output: "not found" },
      });
    });

    it("marks tool call as interrupted when outcome is interrupted", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "工具执行被用户中止，未返回结果。",
        outcome: "interrupted",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "interrupted", output: "工具执行被用户中止，未返回结果。" },
      });
    });

    it("parses todowrite output", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "todowrite", arguments: "{}", status: "running" }] });
      const todos = [{ content: "do thing", status: "pending", priority: "medium" }];
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "todowrite",
        output: `Todos updated: ${JSON.stringify(todos)}`,
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      const todoMut = mutations.find((m) => m.type === "setTodos");
      expect(todoMut).toBeDefined();
      if (todoMut?.type === "setTodos") {
        expect(todoMut.runId).toBe("test-run");
        expect(todoMut.todos).toHaveLength(1);
        expect(todoMut.todos[0].content).toBe("do thing");
      }
    });

    it("emits canvasAutoOpen for canvas tool", () => {
      const spec = { title: "Test", fields: [] };
      const tc: ToolCallDisplay = { id: "tc1", name: "canvas", arguments: JSON.stringify({ spec }), status: "running" };
      const state = makeState({ isStreaming: true, activeToolCalls: [tc] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "canvas",
        output: "ok",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      const canvasMut = mutations.find((m) => m.type === "canvasAutoOpen");
      expect(canvasMut).toBeDefined();
      if (canvasMut?.type === "canvasAutoOpen") {
        expect(canvasMut.toolCallId).toBe("tc1");
        expect(canvasMut.spec).toEqual(spec);
      }
    });
  });

  describe("toolCallDelta", () => {
    it("appends delta to tool call", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "toolCallDelta", sessionId: "s1", toolCallId: "tc1", delta: "partial" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendToolDelta", id: "tc1", delta: "partial" });
    });
  });

  describe("knowledgeProposal", () => {
    it("upserts proposal messages into the stream", () => {
      const event: StreamEvent = {
        runId: "test-run",
        type: "knowledgeProposal",
        sessionId: "s1",
        message: {
          id: "kp-msg-1",
          role: "assistant",
          content: "",
          createdAt: 1,
          knowledgeProposal: {
            proposalId: "kp-1",
            status: "pending",
            confidence: 0.82,
            verify: "required",
            estTokens: 1200,
            items: [
              {
                kind: "memory",
                mode: "replace",
                target: "project-understanding.md",
                draft: "# Project Understanding",
              },
            ],
            createdAt: 1,
            updatedAt: 1,
          },
        },
      };

      const mutations = reduceStreamEvent(makeState(), event);
      expect(mutations).toContainEqual({
        type: "upsertMessage",
        message: event.message,
      });
    });
  });

  describe("subagent tool calls", () => {
    it("adds nested tool call under parent", () => {
      const parent: ToolCallDisplay = { id: "p1", name: "agent", arguments: "{}", status: "running", nestedToolCalls: [] };
      const state = makeState({ isStreaming: true, activeToolCalls: [parent] });
      const event: StreamEvent = { runId: "test-run",
        type: "subagentToolCallStart",
        sessionId: "s1",
        parentToolCallId: "p1",
        toolCallId: "c1",
        toolName: "read",
        arguments: "{}",
      };
      const mutations = reduceStreamEvent(state, event);

      const addMut = mutations.find((m) => m.type === "addNestedToolCall");
      expect(addMut).toBeDefined();
      if (addMut?.type === "addNestedToolCall") {
        expect(addMut.parentId).toBe("p1");
        expect(addMut.toolCall.id).toBe("c1");
      }
    });

    it("marks nested tool call done", () => {
      const child: ToolCallDisplay = { id: "c1", name: "read", arguments: "{}", status: "running" };
      const parent: ToolCallDisplay = { id: "p1", name: "agent", arguments: "{}", status: "running", nestedToolCalls: [child] };
      const state = makeState({ isStreaming: true, activeToolCalls: [parent] });
      const event: StreamEvent = { runId: "test-run",
        type: "subagentToolCallDone",
        sessionId: "s1",
        parentToolCallId: "p1",
        toolCallId: "c1",
        toolName: "read",
        output: "ok",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateNestedToolCall",
        parentId: "p1",
        childId: "c1",
        updates: { status: "done", output: "ok" },
      });
    });
  });

  describe("toolCallRoundDone", () => {
    it("pushes assistant message, tool results, and resets round", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "thought", thinkingDuration: 3 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallRoundDone",
        sessionId: "s1",
        messageId: "m1",
        fullText: "result text",
        toolCalls: [{ id: "tc1", name: "read", arguments: "{}" }],
      };
      const mutations = reduceStreamEvent(state, event);

      const pushMsg = mutations.find((m) => m.type === "pushMessage");
      expect(pushMsg).toBeDefined();
      if (pushMsg?.type === "pushMessage") {
        expect(pushMsg.message.id).toBe("m1");
        expect(pushMsg.message.role).toBe("assistant");
        expect(pushMsg.message.content).toBe("result text");
        expect(pushMsg.message.thinkingContent).toBe("thought");
        expect(pushMsg.message.thinkingDuration).toBe(3);
      }
      expect(mutations.find((m) => m.type === "pushToolResults")).toBeDefined();
      expect(mutations.find((m) => m.type === "resetRound")).toBeDefined();
    });
  });

  describe("usageUpdate", () => {
    it("updates all usage fields", () => {
      const state = makeState();
      const event: StreamEvent = { runId: "test-run",
        type: "usageUpdate",
        sessionId: "s1",
        inputTokens: 100,
        outputTokens: 50,
        cacheReadTokens: 10,
        cacheWriteTokens: 5,
        totalInputTokens: 200,
        totalOutputTokens: 100,
        totalCacheReadTokens: 20,
        totalCacheWriteTokens: 10,
        totalCostUsd: 0.05,
        pricedRounds: 3,
        contextTokens: 5000,
        contextLimit: 100000,
      };
      const mutations = reduceStreamEvent(state, event);

      const usageMut = mutations.find((m) => m.type === "updateUsage");
      expect(usageMut).toBeDefined();
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.totalInputTokens).toBe(200);
        expect(usageMut.usage.totalCostUsd).toBe(0.05);
        expect(usageMut.usage.contextTokens).toBe(5000);
      }
    });

    it("preserves existing contextTokens when event has 0", () => {
      const state = makeState({
        tokenUsage: { totalInputTokens: 0, totalOutputTokens: 0, totalCacheReadTokens: 0, totalCacheWriteTokens: 0, totalCostUsd: 0, pricedRounds: 0, contextTokens: 3000, contextLimit: 100000 },
      });
      const event: StreamEvent = { runId: "test-run",
        type: "usageUpdate",
        sessionId: "s1",
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalInputTokens: 100,
        totalOutputTokens: 50,
        totalCacheReadTokens: 0,
        totalCacheWriteTokens: 0,
        totalCostUsd: 0.01,
        pricedRounds: 1,
        contextTokens: 0,
        contextLimit: 0,
      };
      const mutations = reduceStreamEvent(state, event);

      const usageMut = mutations.find((m) => m.type === "updateUsage");
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.contextTokens).toBe(3000);
        expect(usageMut.usage.contextLimit).toBe(100000);
      }
    });
  });

  describe("compactDone", () => {
    it("clears stale context usage after compaction completes", () => {
      const state = makeState({
        tokenUsage: {
          totalInputTokens: 100,
          totalOutputTokens: 50,
          totalCacheReadTokens: 10,
          totalCacheWriteTokens: 5,
          totalCostUsd: 0.01,
          pricedRounds: 1,
          contextTokens: 8000,
          contextLimit: 100000,
        },
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "compactDone",
        sessionId: "s1",
        messagesBefore: 40,
        messagesAfter: 8,
        messages: [
          {
            id: "handoff-1",
            role: "assistant",
            content: "## Context Handoff",
            createdAt: 100,
          },
        ],
      };

      const mutations = reduceStreamEvent(state, event);
      const replaceMut = mutations.find((m) => m.type === "replaceMessages");
      expect(replaceMut).toBeDefined();
      if (replaceMut?.type === "replaceMessages") {
        expect(replaceMut.messages).toHaveLength(1);
        expect(replaceMut.messages[0]?.content).toContain("Context Handoff");
      }
      const usageMut = mutations.find((m) => m.type === "updateUsage");
      expect(usageMut).toBeDefined();
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.contextTokens).toBe(0);
        expect(usageMut.usage.contextLimit).toBe(100000);
        expect(usageMut.usage.totalInputTokens).toBe(100);
      }
    });
  });

  describe("askUser", () => {
    it("sets pending question", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "askUser",
        sessionId: "s1",
        questionId: "q1",
        toolCallId: "tc1",
        question: "What file?",
        options: [{ label: "foo", description: "foo.ts" }],
      };
      const mutations = reduceStreamEvent(state, event);

      const qMut = mutations.find((m) => m.type === "setQuestion");
      expect(qMut).toBeDefined();
      if (qMut?.type === "setQuestion") {
        expect(qMut.question?.questionId).toBe("q1");
        expect(qMut.question?.question).toBe("What file?");
      }
    });
  });

  describe("toolConfirm", () => {
    it("sets pending tool confirm", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "toolConfirm",
        sessionId: "s1",
        questionId: "q1",
        toolCallId: "tc1",
        display: {
          kind: "knowledge",
          operation: "edit",
          targetKind: "document",
          docType: "design",
          path: "design/core.md",
          directoryPath: "design",
          directoryMode: "approval",
          documentBeforeText: "before",
          documentAfterText: "after",
        },
      };
      const mutations = reduceStreamEvent(state, event);

      const cMut = mutations.find((m) => m.type === "enqueueToolConfirm");
      expect(cMut).toBeDefined();
      if (cMut?.type === "enqueueToolConfirm") {
        expect(cMut.confirm?.display.kind).toBe("knowledge");
      }
    });
  });

  describe("undoAvailable", () => {
    it("adds message id to undoable set", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "undoAvailable", sessionId: "s1", assistantMessageId: "m1" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "addUndoable", messageId: "m1" });
    });
  });

  describe("done", () => {
    it("upserts final message, resets round, and stops streaming", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "", thinkingDuration: 0 });
      const event: StreamEvent = { runId: "test-run", type: "done", sessionId: "s1", messageId: "m1", fullText: "final text" };
      const mutations = reduceStreamEvent(state, event);

      const upsertMsg = mutations.find((m) => m.type === "upsertMessage");
      expect(upsertMsg).toBeDefined();
      if (upsertMsg?.type === "upsertMessage") {
        expect(upsertMsg.message.content).toBe("final text");
        expect(upsertMsg.message.thinkingContent).toBeUndefined();
      }
      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });

    it("stops streaming without pushing an empty assistant message", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "thought", thinkingDuration: 2 });
      const event: StreamEvent = { runId: "test-run", type: "done", sessionId: "s1", messageId: "", fullText: "" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "upsertMessage")).toBeUndefined();
      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });

    it("reuses the same assistant message id when a server-tool round already inserted the message", () => {
      const state = makeState({
        isStreaming: true,
        messages: [
          {
            id: "m1",
            role: "assistant",
            content: "final text",
            createdAt: 1,
            toolCalls: [{ id: "ws-1", name: "web_search", arguments: "{\"query\":\"unity\"}", serverToolOutput: "Searched: unity" }],
          },
        ],
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "done",
        sessionId: "s1",
        messageId: "m1",
        fullText: "final text",
      };
      const mutations = reduceStreamEvent(state, event);

      const upsertMsg = mutations.find((m) => m.type === "upsertMessage");
      expect(upsertMsg).toBeDefined();
      if (upsertMsg?.type === "upsertMessage") {
        expect(upsertMsg.message.id).toBe("m1");
        expect(upsertMsg.message.content).toBe("final text");
        expect(upsertMsg.message.toolCalls).toEqual([
          {
            id: "ws-1",
            name: "web_search",
            arguments: "{\"query\":\"unity\"}",
            serverToolOutput: "Searched: unity",
          },
        ]);
      }
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });
  });

  describe("error", () => {
    it("stops streaming without producing inline error state", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "error",
        sessionId: "s1",
        error: { code: "test.error", message: "something broke", retryable: false, severity: "error" },
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "pushMessage")).toBeUndefined();
      expect(mutations.find((m) => m.type === "resetRound")).toBeDefined();
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });
  });

  describe("cancelled", () => {
    it("stops streaming silently when the run is cancelled", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "thinking" });
      const event: StreamEvent = {
        runId: "test-run",
        type: "cancelled",
        sessionId: "s1",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
      expect(mutations.find((m) => m.type === "pushMessage")).toBeUndefined();
    });
  });
});
