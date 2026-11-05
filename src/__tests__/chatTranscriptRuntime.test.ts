// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive } from "vue";
import { describe, expect, it, vi } from "vitest";
import ChatTranscript from "../components/chat/ChatTranscript.vue";
import type { AssistantRenderPart, ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

function mountToolHandoffTranscript(lastToolName = "todowrite") {
  const historyTools: ToolCallInfo[] = [1, 2, 3, 4].map((order) => ({
    id: `tool-${order}`,
    name: "read",
    arguments: JSON.stringify({ filePath: `Assets/File${order}.cs` }),
    order,
    outcome: "done",
    recordedOutput: "file content",
  }));
  const lastTool: ToolCallInfo = {
    id: "last-tool",
    name: lastToolName,
    arguments: lastToolName === "todowrite"
      ? JSON.stringify({ todos: [{ content: "Verify changes", status: "completed", priority: "medium" }] })
      : JSON.stringify({ filePath: "Assets/Last.cs" }),
    order: 5,
    outcome: "done",
    recordedOutput: "done",
  };
  const toolMessage = (toolCall: ToolCallInfo): ChatMessage => ({
    id: `message-${toolCall.id}`,
    role: "assistant",
    content: "",
    createdAt: toolCall.order!,
    toolCalls: [toolCall],
    renderParts: [{
      kind: "toolCall",
      id: toolCall.id,
      order: { runId: "handoff-run", seq: toolCall.order! },
      toolCall,
    }],
  });
  const finalPart: AssistantRenderPart = {
    kind: "text",
    id: "final-part",
    order: { runId: "handoff-run", seq: 6 },
    content: "Final result",
  };
  const finalMessage: ChatMessage = {
    id: "final-message",
    role: "assistant",
    content: "Final result",
    createdAt: 6,
    contentOrder: 6,
    renderParts: [finalPart],
  };
  const props = reactive({
    messages: [
      { id: "user", role: "user", content: "Inspect the project", createdAt: 0 },
      ...historyTools.map(toolMessage),
    ] as ChatMessage[],
    streamingText: "",
    streamingTextOrder: 0,
    isStreaming: true,
    isThinking: false,
    activeToolCalls: [{ ...lastTool, status: "done", output: lastTool.recordedOutput }] as ToolCallDisplay[],
    liveRenderParts: [] as AssistantRenderPart[],
    variant: "session" as const,
    sessionKey: "tool-handoff-regression",
  });
  const runtimeErrors: unknown[] = [];
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ render: () => h(ChatTranscript, props) });
  app.config.errorHandler = (error) => runtimeErrors.push(error);
  app.use(createPinia());
  app.mount(host);

  return {
    props,
    host,
    runtimeErrors,
    toolIds: [...historyTools, lastTool].map((toolCall) => toolCall.id),
    async completeToolRound() {
      await nextTick();
      props.messages = [...props.messages, toolMessage(lastTool)];
      props.activeToolCalls = [];
      await nextTick();
    },
    async streamFinalText(delay: number) {
      props.streamingText = finalMessage.content;
      props.streamingTextOrder = 6;
      props.liveRenderParts = [finalPart];
      await nextTick();
      await vi.advanceTimersByTimeAsync(delay);
    },
    materializeFinalText() {
      props.messages = [...props.messages, finalMessage];
      props.streamingText = "";
      props.streamingTextOrder = 0;
      props.liveRenderParts = [];
    },
    dispose() {
      app.unmount();
      host.remove();
    },
  };
}

function expectSettledToolHistory(
  transcript: ReturnType<typeof mountToolHandoffTranscript>,
  withFinalText = true,
) {
  expect(transcript.runtimeErrors).toEqual([]);
  expect(transcript.host.querySelector('[data-render-part-scope="transient"]')).toBeNull();
  const parts = Array.from(transcript.host.querySelectorAll<HTMLElement>('[data-render-part-scope="history"]'));
  expect(parts.map((part) => part.dataset.renderPartKind)).toEqual(withFinalText ? ["toolCall", "text"] : ["toolCall"]);
  expect(parts[0]?.dataset.toolLayoutToolCallIds?.split(",")).toEqual(transcript.toolIds);
  if (withFinalText) expect(parts[1]?.textContent?.trim()).toBe("Final result");
}

describe("chat transcript runtime", () => {
  it("renders trailing history tools while a different live tool call is active", async () => {
    const historyTool: ToolCallInfo = {
      id: "history-tool",
      name: "read",
      arguments: JSON.stringify({ filePath: "Assets/History.cs" }),
      order: 1,
      outcome: "done",
      recordedOutput: "history output",
    };
    const messages: ChatMessage[] = [
      {
        id: "user-message",
        role: "user",
        content: "Inspect the project",
        createdAt: 1,
      },
      {
        id: "history-message",
        role: "assistant",
        content: "",
        createdAt: 2,
        toolCalls: [historyTool],
        renderParts: [{
          kind: "toolCall",
          id: "history-tool-part",
          order: { runId: "history-run", seq: 1 },
          toolCall: historyTool,
        }],
      },
    ];
    const activeToolCalls: ToolCallDisplay[] = [{
      id: "live-tool",
      name: "read",
      arguments: JSON.stringify({ filePath: "Assets/Live.cs" }),
      status: "running",
      order: 2,
    }];
    const runtimeErrors: unknown[] = [];
    const host = document.createElement("div");
    const app = createApp(ChatTranscript, {
      messages,
      streamingText: "",
      isStreaming: true,
      isThinking: false,
      activeToolCalls,
      variant: "session",
      sessionKey: "runtime-cycle-regression",
    });
    app.config.errorHandler = (error) => runtimeErrors.push(error);
    app.use(createPinia());

    app.mount(host);
    await nextTick();

    expect(runtimeErrors).toEqual([]);
    expect(host.querySelector(".chat-transcript-content")).not.toBeNull();
    expect(host.querySelectorAll(".chat-transcript-message").length).toBeGreaterThan(0);
    expect(host.textContent).toContain("Inspect the project");

    app.unmount();
  });

  it.each([
    { delay: 0, lastToolName: "todowrite" },
    { delay: 200, lastToolName: "todowrite" },
    { delay: 1000, lastToolName: "todowrite" },
    { delay: 200, lastToolName: "read" },
  ])("keeps $lastToolName before the final response when streaming ends after $delay ms", async ({ delay, lastToolName }) => {
    vi.useFakeTimers();
    const transcript = mountToolHandoffTranscript(lastToolName);
    try {
      await transcript.completeToolRound();
      await transcript.streamFinalText(delay);
      if (delay === 200) {
        // End the stream while the promoted five-tool batch is still collapsing.
        const group = transcript.host.querySelector<HTMLElement>('[data-render-part-scope="transient"][data-render-part-kind="toolCall"]');
        expect(group?.dataset.toolLayoutToolCallIds?.split(",")).toEqual(transcript.toolIds);
        expect(group?.dataset.toolLayoutAnimateCollapseOnMount).toBe("true");
      }
      transcript.materializeFinalText();
      transcript.props.isStreaming = false;
      await nextTick();

      expectSettledToolHistory(transcript);
      await vi.advanceTimersByTimeAsync(2000);
      expectSettledToolHistory(transcript);
    } finally {
      transcript.dispose();
      vi.useRealTimers();
    }
  });

  it.each(["messages-first", "stream-end-first"])("reconciles tool history with %s updates", async (updateOrder) => {
    vi.useFakeTimers();
    const transcript = mountToolHandoffTranscript();
    try {
      await transcript.completeToolRound();
      await transcript.streamFinalText(200);
      if (updateOrder === "messages-first") {
        transcript.materializeFinalText();
        await nextTick();
        transcript.props.isStreaming = false;
      } else {
        transcript.props.isStreaming = false;
        await nextTick();
        transcript.materializeFinalText();
      }
      await nextTick();

      expectSettledToolHistory(transcript);
      await vi.advanceTimersByTimeAsync(2000);
      expectSettledToolHistory(transcript);
    } finally {
      transcript.dispose();
      vi.useRealTimers();
    }
  });

  it("keeps one assistant group across internal user rows with nothing to show", async () => {
    const toolMessage = (seq: number): ChatMessage => {
      const toolCall: ToolCallInfo = {
        id: `tool-${seq}`,
        name: "read",
        arguments: JSON.stringify({ filePath: `Assets/File${seq}.cs` }),
        order: seq,
        outcome: "done",
        recordedOutput: "file content",
      };
      return {
        id: `message-${seq}`,
        role: "assistant",
        content: "",
        createdAt: seq,
        toolCalls: [toolCall],
        renderParts: [{ kind: "toolCall", id: toolCall.id, order: { runId: "notice-run", seq }, toolCall }],
      };
    };
    const toolResult = (seq: number): ChatMessage => ({
      id: `result-${seq}`, role: "tool", content: "file content", createdAt: seq, toolCallId: `tool-${seq}`,
    });
    const messages: ChatMessage[] = [
      { id: "user", role: "user", content: "Inspect the project", createdAt: 0 },
      toolMessage(1),
      toolResult(1),
      {
        id: "async-result:task_1",
        role: "user",
        content: "",
        createdAt: 1,
        promptSuffix: "<system-reminder>\nAsync task t1 (bash) attempt 1 finished with status completed. Use the result below.\nElapsed: 1 ms\n\nResult:\nok\n</system-reminder>",
      },
      toolMessage(2),
      toolResult(2),
      { id: "continuation", role: "user", content: "", createdAt: 2, promptSuffix: "Continue from the existing conversation context." },
      {
        id: "final",
        role: "assistant",
        content: "Final result",
        createdAt: 3,
        contentOrder: 3,
        renderParts: [{ kind: "text", id: "final-part", order: { runId: "notice-run", seq: 3 }, content: "Final result" }],
      },
    ];
    const runtimeErrors: unknown[] = [];
    const host = document.createElement("div");
    const app = createApp(ChatTranscript, {
      messages,
      streamingText: "",
      isStreaming: false,
      isThinking: false,
      activeToolCalls: [],
      variant: "session",
      sessionKey: "internal-user-row-grouping",
    });
    app.config.errorHandler = (error) => runtimeErrors.push(error);
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    expect(runtimeErrors).toEqual([]);
    expect(host.querySelectorAll(".chat-transcript-message.user")).toHaveLength(1);
    expect(host.querySelectorAll(".chat-transcript-message.assistant")).toHaveLength(1);
    const parts = Array.from(host.querySelectorAll<HTMLElement>('[data-render-part-scope="history"]'));
    expect(parts.map((part) => part.dataset.renderPartKind)).toEqual(["toolCall", "text"]);
    expect(parts[0]?.dataset.toolLayoutToolCallIds?.split(",")).toEqual(["tool-1", "tool-2"]);

    app.unmount();
  });

  it("preserves a completed tool-only response when there is no final text", async () => {
    vi.useFakeTimers();
    const transcript = mountToolHandoffTranscript();
    try {
      await transcript.completeToolRound();
      transcript.props.isStreaming = false;
      await nextTick();
      await vi.advanceTimersByTimeAsync(2000);

      expectSettledToolHistory(transcript, false);
    } finally {
      transcript.dispose();
      vi.useRealTimers();
    }
  });
});
